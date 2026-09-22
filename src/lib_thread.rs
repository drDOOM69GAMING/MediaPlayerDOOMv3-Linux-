#![allow(unused_imports)]
#![allow(dead_code)]
// Background lib thread: video/audio decode via ffmpeg, frame pacing.
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use lofty::prelude::*;
use crate::consts::NO_WINDOW;
use crate::ffmpeg::{cut_video_file, detect_credit_start, detect_intro_end, fit_video_dims, probe_video_duration, probe_video_info, video_audio_levels};
use crate::files::*;
use crate::library::{art_cache_path, write_art_cache};
use crate::log::av_log;
use crate::pipe::spawn_video_audio;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
pub fn lib_loop(rx: Receiver<LibCmd>, tx: Sender<Msg>, clock: std::sync::Arc<std::sync::atomic::AtomicU64>) {
    let video_pause = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut video_child: Option<Child> = None;
    let mut video_thread: Option<std::thread::JoinHandle<()>> = None;
    // Bulk tag reads run on their own worker POOL so the lib loop is never
    // blocked and eighteen thousand files don't queue behind each other: a full
    // library can be tens of thousands of files and reading them inline would
    // starve Scan/Meta/ArtLocal sitting behind the batch. The receiver is
    // shared (mutex-wrapped) across TAG_WORKERS threads, so up to 8 chunks are
    // being lofty-read at the same time.
    const TAG_WORKERS: usize = 8;
    let (tag_tx, tag_rx) = std::sync::mpsc::channel::<Vec<String>>();
    let tag_rx = std::sync::Arc::new(std::sync::Mutex::new(tag_rx));
    for _ in 0..TAG_WORKERS {
        let rx = tag_rx.clone();
        let tx2 = tx.clone();
        std::thread::spawn(move || loop {
            let paths = {
                let guard = match rx.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                guard.recv()
            };
            let Ok(paths) = paths else { return; };
            // One bad file must not kill the whole tag pipeline (leftover
            // paths would stay in tags_pending forever), so run each batch
            // behind catch_unwind and keep draining.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut entries: Vec<(String, String, String)> = Vec::with_capacity(paths.len());
                let mut metas: Vec<(String, String, String, String, f32)> = Vec::with_capacity(paths.len());
                for p in paths {
                    let mut title = String::new();
                    let mut artist = String::new();
                    let mut album = String::new();
                    let mut secs = 0.0f32;
                    if let Some(probed) = load_tagged(&p) {
                        secs = probed.properties().duration().as_secs_f32();
                        if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
                            if let Some(t) = tag.title() { title = t.to_string(); }
                            if let Some(a) = tag.artist() { artist = a.to_string(); }
                            if let Some(al) = tag.album() { album = al.to_string(); }
                        }
                    }
                    entries.push((p.clone(), artist.clone(), title.clone()));
                    metas.push((p, title, artist, album, secs));
                }
                // Tags feeds the playlist row names; BatchMeta feeds the
                // persistent local cache (metacache.json).
                let _ = tx2.send(Msg::Tags { entries });
                let _ = tx2.send(Msg::BatchMeta { entries: metas });
            }));
        });
    }
    // Bulk local-cover-art scans run on their own worker POOL too (normalizing
    // images is CPU-heavy, so 8 parallel chunks wall-clock much faster) - they
    // read folder images / embedded pictures, write them into %APPDATA%\..\artcache
    // and report the paths that had no art so the app can fetch those from the web.
    const ART_WORKERS: usize = 8;
    let (art_tx, art_rx) = std::sync::mpsc::channel::<Vec<String>>();
    let art_rx = std::sync::Arc::new(std::sync::Mutex::new(art_rx));
    for _ in 0..ART_WORKERS {
        let rx = art_rx.clone();
        let tx2 = tx.clone();
        std::thread::spawn(move || loop {
            let paths = {
                let guard = match rx.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                guard.recv()
            };
            let Ok(paths) = paths else { return; };
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut missing: Vec<String> = Vec::new();
                for p in paths {
                    if art_cache_path(&p).is_file() {
                        continue;
                    }
                    if let Some(b) = find_local_art(&p) {
                        if let Some(nb) = normalize_art(&b) {
                            let _ = write_art_cache(&p, &nb);
                            continue;
                        }
                    }
                    missing.push(p);
                }
                let _ = tx2.send(Msg::ArtAll { missing });
            }));
        });
    }
    if let Err(e) = ensure_tools(&tx) {
        let _ = tx.send(Msg::YtStatus(format!("Tools unavailable: {}", e)));
    }
    while let Ok(cmd) = rx.recv() {
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle_lib(cmd, &tx, &tag_tx, &art_tx, &video_pause, &mut video_child, &mut video_thread, &clock);
        }));
        if let Err(p) = res {
            let msg = if let Some(s) = p.downcast_ref::<&str>() { (*s).to_string() }
                else if let Some(s) = p.downcast_ref::<String>() { s.clone() }
                else { "unknown panic".to_string() };
            let _ = tx.send(Msg::Error(format!("Background task crashed: {}", msg)));
        }
    }
}

pub fn handle_lib(cmd: LibCmd, tx: &Sender<Msg>, tag_tx: &Sender<Vec<String>>, art_tx: &Sender<Vec<String>>, video_pause: &std::sync::Arc<std::sync::atomic::AtomicBool>, video_child: &mut Option<Child>, video_thread: &mut Option<std::thread::JoinHandle<()>>, clock: &std::sync::Arc<std::sync::atomic::AtomicU64>) {
        av_log(match &cmd {
            LibCmd::Scan(_) => "lib: Scan",
            LibCmd::AddMany(_) => "lib: AddMany",
            LibCmd::LoadPlaylist(_) => "lib: LoadPlaylist",
            LibCmd::SavePlaylist(..) => "lib: SavePlaylist",
            LibCmd::Meta(_) => "lib: Meta",
            LibCmd::TagBatch(_) => "lib: TagBatch",
            LibCmd::ArtLocal(_) => "lib: ArtLocal",
            LibCmd::BatchMeta(_) => "lib: BatchMeta",
            LibCmd::ArtAll(_) => "lib: ArtAll",
            LibCmd::Transcode { .. } => "lib: Transcode",
            LibCmd::Record { .. } => "lib: Record",
            LibCmd::RecordRadio { .. } => "lib: RecordRadio",
            LibCmd::FindMusicFolder => "lib: FindMusicFolder",
            LibCmd::VideoOpen { .. } => "lib: VideoOpen",
            LibCmd::VideoClose => "lib: VideoClose",
            LibCmd::VideoPause(_) => "lib: VideoPause",
            LibCmd::VideoAnalyze { .. } => "lib: VideoAnalyze",
            LibCmd::VideoCut { .. } => "lib: VideoCut",
        });
        match cmd {
LibCmd::Scan(dir) => {
                // Run the directory walk on a worker so the lib loop never
                // blocks for a full library scan - VideoOpen/Meta/ArtLocal
                // queued behind a big scan would otherwise sit until the walk
                // finishes (the video player shows LOADING... the whole time).
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    let files = collect_audio_report(&dir, &tx2);
                    let _ = tx2.send(Msg::DirScanned { dir, files });
                });
            }
            LibCmd::AddMany(entries) => {
                // Same non-blocking treatment as Scan: walk dirs on a worker
                // so file drops never stall the lib loop.
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    let mut added: Vec<String> = Vec::new();
                    for e in entries {
                        let p = PathBuf::from(&e);
                        if p.is_dir() {
                            let sub = collect_audio_report(&e, &tx2);
                            for f in sub {
                                if !added.contains(&f) {
                                    added.push(f);
                                }
                            }
                        } else if is_audio(&e) {
                            if !added.contains(&e) {
                                added.push(e);
                            }
                        }
                        let _ = tx2.send(Msg::ScanProgress { found: added.len() });
                    }
                    added.sort();
                    let _ = tx2.send(Msg::Added { files: added });
                });
            }
            LibCmd::LoadPlaylist(path) => {
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        match serde_json::from_str::<Vec<String>>(&text) {
                            Ok(mut files) => {
                                files.sort();
                                let _ = tx.send(Msg::PlaylistLoaded { files });
                            }
                            Err(_) => { let _ = tx.send(Msg::Error("Invalid playlist file".into())); }
                        }
                    }
                    Err(_) => { let _ = tx.send(Msg::Error("Could not read playlist".into())); }
                }
            }
            LibCmd::SavePlaylist(path, playlist) => {
                let ok = serde_json::to_string(&playlist)
                    .map(|s| std::fs::write(&path, s).is_ok())
                    .unwrap_or(false);
                if ok { let _ = tx.send(Msg::PlaylistSaved); }
                else { let _ = tx.send(Msg::Error("Could not save playlist".into())); }
            }
            LibCmd::Meta(path) => {
                let mut title = stem(&path);
                let mut artist = "Unknown".to_string();
                let mut album = "Unknown".to_string();
                let mut duration = Duration::ZERO;
                if let Some(probed) = load_tagged(&path) {
                    duration = probed.properties().duration();
                    if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
                        if let Some(t) = tag.title() { title = t.to_string(); }
                        if let Some(a) = tag.artist() { artist = a.to_string(); }
                        if let Some(al) = tag.album() { album = al.to_string(); }
                    }
                }
                let _ = tx.send(Msg::Meta { path, title, artist, album, duration });
            }
            LibCmd::TagBatch(paths) => {
                // Handed to the dedicated tag worker - never run lofty probes
                // inline here or Scan/Meta/ArtLocal queued behind would starve.
                let _ = tag_tx.send(paths);
            }
            LibCmd::ArtLocal(path) => {
                let bytes = find_local_art(&path);
                let _ = tx.send(Msg::ArtLocal { bytes });
            }
            LibCmd::BatchMeta(paths) => {
                // Full tags for a whole batch - same worker as TagBatch so the
                // lib loop never stalls on tens of thousands of lofty probes.
                let _ = tag_tx.send(paths);
            }
            LibCmd::ArtAll(paths) => {
                // Local cover scans run on the art worker (per-batch panic guard).
                let _ = art_tx.send(paths);
            }
            LibCmd::Transcode { display, audio } => {
                match transcode_with_ffmpeg(&audio, &ffmpeg_path()) {
                    Ok(wav) => { let _ = tx.send(Msg::Transcoded { display, wav }); }
                    Err(e) => { let _ = tx.send(Msg::TranscodeFail { display, err: e }); }
                }
            }
            LibCmd::Record { display, audio, dest } => {
                match record_with_ffmpeg(&audio, &ffmpeg_path(), &Path::new(&dest)) {
                    Ok(p) => { let _ = tx.send(Msg::Recorded { display, path: p.to_string_lossy().to_string() }); }
                    Err(e) => { let _ = tx.send(Msg::RecordFail { display, err: e }); }
                }
            }
            LibCmd::RecordRadio { display, url, dest } => {
                match record_flight_ffmpeg(&url, &ffmpeg_path(), &Path::new(&dest), 30) {
                    Ok(p) => { let _ = tx.send(Msg::Recorded { display, path: p.to_string_lossy().to_string() }); }
                    Err(e) => { let _ = tx.send(Msg::RecordFail { display, err: e }); }
                }
            }
            LibCmd::FindMusicFolder => {
                // Walk candidate drives on a worker - find_music_folder can
                // touch every drive's Music folder recursively (a 18k-file
                // library takes ~1s each), and we do NOT want that stalling
                // VideoOpen/Meta queued behind it at startup.
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    let folder = find_music_folder();
                    let _ = tx2.send(Msg::FolderFound { folder });
                });
            }
            LibCmd::VideoOpen { path, gen, seek } => {
                av_log(&format!("open: RECEIVED gen={} (queue behind previous cmd?)", gen));
                // A paused reader never reads its pipe, so it can't notice the
                // old ffmpeg child being killed below and would never exit -
                // join() on it would deadlock the lib thread and leave the video
                // stuck on a black screen forever. Un-pause it first so it
                // resumes, hits the dead pipe, and exits cleanly.
                video_pause.store(false, std::sync::atomic::Ordering::Relaxed);
                if let Some(mut c) = video_child.take() { let _ = c.kill(); }
                if let Some(t) = video_thread.take() { let _ = t.join(); }
                av_log("open: old video killed+joined");
                let t_open = std::time::Instant::now();
                let (sw, sh, sfps) = probe_video_info(&path);
                av_log(&format!("open: probe_info {}x{} fps={} took {}ms", sw, sh, sfps, t_open.elapsed().as_millis()));
                let (tw, th) = fit_video_dims(sw, sh, 1920, 1080);
                let fps = if sfps > 0.0 { sfps } else { 24.0 };
                let mut cmd = std::process::Command::new(ffmpeg_path());
                cmd.args(["-hide_banner", "-loglevel", "error"]);
                if seek > 0.0 {
                    cmd.args(["-ss"]).arg(format!("{:.3}", seek));
                }
                cmd.arg("-i").arg(&path)
                    .args(["-an", "-vf", &format!("scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2", tw, th, tw, th), "-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .creation_flags(NO_WINDOW);
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Msg::Error(format!("ffmpeg could not start video: {}", e)));
                        return;
                    }
                };
                av_log(&format!("open: ffmpeg spawned at {}ms", t_open.elapsed().as_millis()));
                {
                    let tx2 = tx.clone();
                    let p = path.clone();
                    std::thread::spawn(move || {
                        let dur = probe_video_duration(&p);
                        let _ = tx2.send(Msg::VideoMeta { gen, dur });
                    });
                }
                if let Some(stdout) = child.stdout.take() {
                    let w = tw;
                    let h = th;
                    let pausef = std::sync::Arc::clone(video_pause);
                    let clock = std::sync::Arc::clone(clock);
                    let tx2 = tx.clone();
                    let gen2 = gen;
                    let pos_send = (fps as u64).max(1);
                    *video_thread = Some(std::thread::spawn(move || {
                        use std::io::Read;
                        let frame = (w * h * 3) as usize;
                        let mut buf = vec![0u8; frame];
                        let mut pipe = stdout;
                        let fps = fps;
                        let mygen = (gen2 & 0xFFFF) as u16;
                        let frame_t = std::time::Duration::from_secs_f64(1.0 / fps as f64);
                        let mut next_t = std::time::Instant::now();
                        let mut was_paused = false;
                        let mut frames: u64 = 0;
                        loop {
                            let paused = pausef.load(std::sync::atomic::Ordering::Relaxed);
                            if paused {
                                if was_paused {
                                    std::thread::sleep(std::time::Duration::from_millis(30));
                                    continue;
                                }
                                // Entering pause: read one more frame so the
                                // display shows it while paused (e.g. the frame
                                // at a new seek position after seeking while
                                // paused), then idle until unpaused. This is a
                                // single blocked read: if the child is killed by
                                // VideoOpen/VideoClose (which un-pause first),
                                // the pipe closes and read_exact errors -> exit.
                                was_paused = true;
                                if pipe.read_exact(&mut buf).is_err() {
                                    break;
                                }
                                {
                                    let mut rgba = Vec::with_capacity(frame / 3 * 4);
                                    for p in buf.chunks_exact(3) {
                                        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
                                    }
                                    if tx2.send(Msg::VideoFrame { w, h, rgba, gen: gen2 }).is_err() {
                                        break;
                                    }
                                }
                                next_t = std::time::Instant::now();
                                std::thread::sleep(std::time::Duration::from_millis(30));
                                continue;
                            }
                            if was_paused {
                                next_t = std::time::Instant::now();
                                was_paused = false;
                            }
                            let target = frames as f32 / fps;
                            let wait_start = std::time::Instant::now();
                            loop {
                                let c = clock.load(std::sync::atomic::Ordering::Relaxed);
                                if c != 0 && ((c >> 48) as u16) == mygen {
                                    let cpos = c & ((1u64 << 48) - 1);
                                    if cpos != 0 {
                                        let apos = (cpos - 1) as f32 / 1000.0;
                                        if apos + 0.030 >= target {
                                            break;
                                        }
                                        if wait_start.elapsed() > std::time::Duration::from_millis(250) {
                                            break;
                                        }
                                        std::thread::sleep(std::time::Duration::from_millis(4));
                                        continue;
                                    }
                                }
                                let now = std::time::Instant::now();
                                if now >= next_t {
                                    break;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(2));
                            }
                            if pipe.read_exact(&mut buf).is_err() {
                                break;
                            }
                            let mut drop_frame = false;
                            let c = clock.load(std::sync::atomic::Ordering::Relaxed);
                            if c != 0 && ((c >> 48) as u16) == mygen {
                                let cpos = c & ((1u64 << 48) - 1);
                                if cpos != 0 && frames > 0 {
                                    let apos = (cpos - 1) as f32 / 1000.0;
                                    if apos > target + 0.75 {
                                        drop_frame = true;
                                    }
                                }
                            }
                            if !drop_frame {
                                let mut rgba = Vec::with_capacity(frame / 3 * 4);
                                for p in buf.chunks_exact(3) {
                                    rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
                                }
                                if tx2.send(Msg::VideoFrame { w, h, rgba, gen: gen2 }).is_err() {
                                    break;
                                }
                                if frames == 0 {
                                    av_log("reader: first frame sent");
                                }
                            }
                            frames += 1;
                            next_t = std::time::Instant::now() + frame_t;
                            if frames % pos_send == 0 {
                                let secs = seek + frames as f32 / fps;
                                if tx2.send(Msg::VideoPos { gen, secs }).is_err() {
                                    break;
                                }
                            }
                        }
                        let _ = tx2.send(Msg::VideoClosed { gen });
                    }));
                }
                *video_child = Some(child);
            }
            LibCmd::VideoClose => {
                // Same unpause-first reasoning as VideoOpen: a paused reader
                // would otherwise never exit, deadlocking the join below.
                video_pause.store(false, std::sync::atomic::Ordering::Relaxed);
                if let Some(mut c) = video_child.take() { let _ = c.kill(); }
                if let Some(t) = video_thread.take() { let _ = t.join(); }
                let _ = tx.send(Msg::VideoClosed { gen: 0 });
            }
            LibCmd::VideoPause(p) => {
                video_pause.store(p, std::sync::atomic::Ordering::Relaxed);
            }
            LibCmd::VideoAnalyze { path, gen } => {
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    let dur = probe_video_duration(&path);
                    if dur <= 0.0 {
                        let _ = tx2.send(Msg::VideoBounds { gen, path, intro_end: 0.0, credits_start: 0.0 });
                        return;
                    }
                    let ff = ffmpeg_path();
                    let front_len = dur.min(420.0);
                    let front = video_audio_levels(&path, &ff, 0.0, front_len);
                    let mut intro = detect_intro_end(&front, 0.5);
                    let mut credits = 0.0f32;
                    if dur > 240.0 {
                        let back_len = dur.min(420.0);
                        let back_start = (dur - back_len).max(0.0);
                        let back = video_audio_levels(&path, &ff, back_start, back_len);
                        let rel = detect_credit_start(&back, 0.5);
                        if rel > 0.0 {
                            credits = back_start + rel;
                        }
                    }
                    if intro >= dur - 5.0 {
                        intro = 0.0;
                    }
                    if credits > 0.0 && credits > dur - 3.0 {
                        credits = 0.0;
                    }
                    let _ = tx2.send(Msg::VideoBounds { gen, path, intro_end: intro, credits_start: credits });
                });
            }
            LibCmd::VideoCut { path, gen, intro_end, credits_start } => {
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    let cut = cut_video_file(&path, intro_end, credits_start);
                    match cut {
                        Ok(c) => { let _ = tx2.send(Msg::VideoCutDone { gen, path, cut: Some(c), err: None }); }
                        Err(e) => { let _ = tx2.send(Msg::VideoCutDone { gen, path, cut: None, err: Some(e) }); }
                    }
                });
            }
        }
}

