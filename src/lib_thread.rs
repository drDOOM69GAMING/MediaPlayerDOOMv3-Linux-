#![allow(unused_imports)]
#![allow(dead_code)]
// Background lib thread: video/audio decode via ffmpeg, frame pacing.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};
use lofty::prelude::*;
use crate::consts::NO_WINDOW;
use crate::ffmpeg::{bounds_from_chapters, cut_video_file, detect_credit_start, detect_intro_end, fit_video_dims, fit_video_dims_upscale, probe_chapters, probe_dvd_meta, probe_video_duration, probe_video_info, probe_video_streams, video_audio_levels};
use crate::files::*;
use crate::library::{art_cache_ok, art_cache_path, write_art_cache};
use crate::log::av_log;
use crate::pipe::spawn_video_audio;
use crate::tools::*;
use crate::types::*;
use crate::util::*;

/// A library walk is background maintenance. Lower its CPU and I/O priority
/// so it cannot compete with the first paint, the startup auto-play, or the
/// audio stream on a cold spinning disk. Linux only; other platforms keep the
/// normal thread priority.
#[cfg(target_os = "linux")]
fn lower_scan_priority() {
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        // IOPRIO_WHO_PROCESS = 1, class best-effort = 2, level 7 (lowest)
        let prio = (2 << 13) | 7;
        libc::syscall(
            libc::SYS_ioprio_set,
            1 as libc::c_long,
            tid as libc::c_long,
            prio as libc::c_long,
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn lower_scan_priority() {}

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
                    if let Some(t) = crate::cue::track_for(&p) {
                        secs = t.secs();
                        title = t.title;
                        artist = t.artist;
                        album = t.album;
                    } else if let Some(probed) = load_tagged(&p) {
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
                let mut done: Vec<String> = Vec::new();
                let mut missing: Vec<String> = Vec::new();
                for p in paths {
                    if art_cache_ok(&p) {
                        done.push(p);
                        continue;
                    }
                    if let Some(b) = find_local_art(&p) {
                        if let Some(nb) = normalize_art(&b) {
                            if write_art_cache(&p, &nb) {
                                done.push(p);
                                continue;
                            }
                        }
                    }
                    missing.push(p);
                }
                let _ = tx2.send(Msg::ArtAll { done, missing });
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

fn hwaccel_args() -> Vec<&'static str> {
    let vendor = std::fs::read_dir("/sys/class/drm")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .filter_map(|e| std::fs::read_to_string(e.path().join("device/vendor")).ok())
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let render = std::fs::read_dir("/sys/class/drm")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().starts_with("renderD"))
        .map(|e| format!("/dev/dri/{}", e.file_name().to_string_lossy()))
        .filter(|p| std::path::Path::new(p).exists());
    if vendor == "0x10de" && std::path::Path::new("/dev/nvidia0").exists() {
        // NVIDIA proprietary. If NVDEC isn't available ffmpeg falls back itself.
        return vec!["-hwaccel", "cuda"];
    }
    if (vendor == "0x1002" || vendor == "0x8086") && render.is_some() {
        // AMD or Intel through Mesa VAAPI - avoids ffmpeg trying CUDA first.
        return vec!["-hwaccel", "vaapi", "-hwaccel_device", "/dev/dri/renderD128"];
    }
    vec!["-hwaccel", "auto"]
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
                    lower_scan_priority();
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
                        } else if crate::cue::is_cue_file(&e) {
                            for f in crate::cue::expand_one(&e) {
                                if !added.contains(&f) {
                                    added.push(f);
                                }
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
                if let Some(t) = crate::cue::track_for(&path) {
                    duration = Duration::from_secs_f32(t.secs());
                    title = t.title;
                    artist = t.artist;
                    album = t.album;
                } else if let Some(probed) = load_tagged(&path) {
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
                // Decode/downscale on this worker, not the UI thread. A 4000px
                // folder scan would otherwise hitch every track change.
                let bytes = find_local_art(&path).and_then(|b| normalize_art(&b));
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
            LibCmd::VideoOpen { path, gen, seek, subtitle, sub_style, dvd_upscale, probe } => {
                av_log(&format!("open: RECEIVED gen={} (queue behind previous cmd?)", gen));
                // A paused reader never reads its pipe, so it can't notice the
                // old ffmpeg child being killed below and would never exit -
                // join() on it would deadlock the lib thread and leave the video
                // stuck on a black screen forever. Un-pause it first so it
                // resumes, hits the dead pipe, and exits cleanly.
                video_pause.store(false, std::sync::atomic::Ordering::Relaxed);
                if let Some(mut c) = video_child.take() { let _ = c.kill(); let _ = c.wait(); }
                if let Some(t) = video_thread.take() { let _ = t.join(); }
                av_log("open: old video killed+joined");
                let dvd = crate::dvd::is_dvd(&path);
                let bd = crate::bluray::is_bluray(&path);
                let t_open = std::time::Instant::now();
                // Optical discs are slow to open and every ffprobe pass makes
                // the drive seek. Use known broadcast/BD dimensions for the
                // first frame; local files still get a real probe.
                // DVD metadata is probed once and cached. Re-probing on every
                // seek added another disc reader while the new video/audio
                // readers were opening, which corrupted DVD playback. PAL
                // discs also need their real 720x576/25 values, not NTSC.
                let dvd_meta = if dvd {
                    probe_dvd_meta(crate::dvd::source(&path))
                } else {
                    None
                };
                let (sw, sh, sfps) = if bd {
                    (1920, 1080, 23.976)
                } else if let Some(m) = dvd_meta {
                    (m.w, m.h, if m.fps > 0.0 { m.fps } else { 29.97 })
                } else {
                    let p = probe_video_info(&path);
                    if p.0 == 0 || p.1 == 0 {
                        if dvd { (720, 480, 29.97) } else { p }
                    } else {
                        p
                    }
                };
                av_log(&format!("open: probe_info {}x{} fps={} took {}ms", sw, sh, sfps, t_open.elapsed().as_millis()));
                let (tw, th) = if dvd && dvd_upscale {
                    fit_video_dims_upscale(sw, sh, 1920, 1080)
                } else {
                    fit_video_dims(sw, sh, 1920, 1080)
                };
                let fps = if sfps > 0.0 { sfps } else { 24.0 };
                // libbluray's own time seek works even though ffmpeg's
                // bluray: protocol cannot seek this disc. Stream the title
                // from the requested timestamp into ffmpeg's stdin.
                let bd_stream = if bd {
                    crate::bluray::playlist(&path).and_then(|pl| {
                        crate::bdstream::spawn_stream(
                            crate::bluray::source(&path),
                            pl,
                            seek.max(0.0),
                        )
                    })
                } else {
                    None
                };
                let using_stream = bd_stream.is_some();
                if using_stream {
                    av_log(&format!(
                        "video: libbluray stream playlist={} seek={:.1}s",
                        crate::bluray::playlist(&path).unwrap_or(0),
                        seek
                    ));
                }
                let input = if using_stream {
                    "pipe:0".to_string()
                } else if dvd {
                    crate::dvd::source(&path).to_string()
                } else if bd {
                    format!("bluray:{}", crate::bluray::source(&path))
                } else {
                    path.clone()
                };
                let mut cmd = std::process::Command::new(ffmpeg_path());
                cmd.args(["-hide_banner", "-loglevel", "error"]);
                // Pick the GPU backend explicitly instead of letting ffmpeg
                // probe CUDA first on an AMD machine. Users can disable it with
                // MPDOOM_NO_HWACCEL=1 if a driver misbehaves.
                // DVD uses software MPEG-2 decode: it is only 720x480 and
                // software is completely reliable across seeks. VAAPI's DVD
                // seek/format reinitialisation is what produced green frames.
                let hw: Vec<&'static str> =
                    if dvd || std::env::var_os("MPDOOM_NO_HWACCEL").is_some() {
                        Vec::new()
                    } else {
                        hwaccel_args()
                    };
                if !hw.is_empty() {
                    av_log(&format!("video hwaccel: {:?}", hw));
                    cmd.args(&hw);
                }
                // Blu-ray gets the full GPU filter chain (scale/pad/RGBA).
                // DVD deliberately stays on the known-good CPU filter path:
                // MPEG-2 is only 720x480 (cheap), and reinitialising the VAAPI
                // filter graph after a DVD seek produced green/stuttering
                // frames on some drives. Decoding still uses VAAPI.
                let vaapi_filters = hw.get(1) == Some(&"vaapi") && bd && subtitle.is_none();
                if vaapi_filters {
                    cmd.args(["-hwaccel_output_format", "vaapi"]);
                }
                if dvd {
                    cmd.args(["-f", "dvdvideo"]);
                }
                if using_stream {
                    cmd.args(["-f", "mpegts"]);
                } else if bd {
                    if let Some(pl) = crate::bluray::playlist(&path) {
                        cmd.args(["-playlist"]).arg(pl.to_string());
                    }
                    if let Some(ch) = crate::bluray::chapter(&path) {
                        cmd.args(["-chapter"]).arg(ch.to_string());
                    }
                }
                if seek > 0.0 && !using_stream {
                    cmd.args(["-ss"]).arg(format!("{:.3}", seek));
                }
                if let Some(out) = bd_stream {
                    cmd.stdin(std::process::Stdio::from(out));
                }
                if bd {
                    crate::bluray::configure_command(&mut cmd);
                }
                let mut vf = if vaapi_filters {
                    format!(
                        "scale_vaapi=w={tw}:h={th}:force_original_aspect_ratio=decrease:format=rgba,pad_vaapi=w={tw}:h={th}:x=(ow-iw)/2:y=(oh-ih)/2,hwdownload,format=rgba"
                    )
                } else if dvd {
                    // DVDs are usually 480i/576i (or soft-telecined). Scaling
                    // interlaced fields directly produces combing/tearing that
                    // looks like corruption, so deinterlace first. Progressive
                    // frames are passed through by deint=interlaced. Lanczos
                    // and a mild unsharp pass make the 480/576 upscale cleaner.
                    if dvd_upscale {
                        let sharpen = if crate::ffmpeg::cas_supported() {
                            "cas=0.6"
                        } else {
                            "unsharp=5:5:1.0:3:3:0.0"
                        };
                        format!(
                            "yadif=deint=interlaced,scale={tw}:{th}:force_original_aspect_ratio=decrease:flags=lanczos,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2,{sharpen}"
                        )
                    } else {
                        format!(
                            "yadif=deint=interlaced,scale={tw}:{th}:force_original_aspect_ratio=decrease,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2"
                        )
                    }
                } else {
                    format!(
                        "scale={tw}:{th}:force_original_aspect_ratio=decrease,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2"
                    )
                };
                if let Some(sub) = subtitle.as_ref() {
                    vf = format!("{},{}", vf, crate::ffmpeg::subtitle_filter(sub, &sub_style));
                }
                cmd.arg("-i").arg(&input)
                    .args(["-an", "-sn", "-vf", &vf]);
                // DVD streams often report 29.97 fps while the decoded film
                // runs at 23.976 with soft-telecine repeat flags. Letting the
                // raw pipe keep that variable cadence made the reader send
                // frames in bursts. Force the probed rate into a constant
                // output rate so every frame has the same presentation slot.
                if dvd {
                    cmd.args(["-r", &format!("{:.6}", fps)]);
                }
                cmd.args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .no_window();
                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Msg::Error(format!("ffmpeg could not start video: {}", e)));
                        return;
                    }
                };
                av_log(&format!("open: ffmpeg spawned at {}ms", t_open.elapsed().as_millis()));
                if probe && !dvd && !bd {
                    let tx2 = tx.clone();
                    let p = path.clone();
                    std::thread::spawn(move || {
                        let (audio, subs) = probe_video_streams(&p);
                        let _ = tx2.send(Msg::VideoStreams { gen, audio, subs });
                    });
                }
                if !bd {
                    if let Some(m) = dvd_meta.filter(|m| m.dur > 0.0) {
                        let _ = tx.send(Msg::VideoMeta { gen, dur: m.dur });
                    } else {
                        let tx2 = tx.clone();
                        let p = path.clone();
                        std::thread::spawn(move || {
                            let dur = probe_video_duration(&p);
                            let _ = tx2.send(Msg::VideoMeta { gen, dur });
                        });
                    }
                }
                if let Some(stderr) = child.stderr.take() {
                    // ffmpeg's stderr used to go to Stdio::null(), so a file it
                    // refused to decode just showed a silent black screen.
                    // Forward the tail of it instead; the UI reports it if no
                    // frame ever arrives.
                    let tx2 = tx.clone();
                    let gen2 = gen;
                    std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader};
                        let mut tail: Vec<String> = Vec::new();
                        for line in BufReader::new(stderr).lines() {
                            let Ok(s) = line else { break };
                            let s = s.trim().to_string();
                            if s.is_empty() {
                                continue;
                            }
                            if tail.len() >= 4 {
                                tail.remove(0);
                            }
                            tail.push(s);
                        }
                        let err = tail.join(" | ");
                        if !err.is_empty() {
                            let _ = tx2.send(Msg::VideoDecodeErr { gen: gen2, err });
                        }
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
                        let frame = (w * h * 4) as usize;
                        let mut buf = vec![0u8; frame];
                        let mut pipe = stdout;
                        let fps = fps;
                        let mygen = (gen2 & 0xFFFF) as u16;
                        let frame_t = std::time::Duration::from_secs_f64(1.0 / fps as f64);
                        let mut next_t = std::time::Instant::now();
                        let mut was_paused = false;
                        let mut frames: u64 = 0;
                        let frame_log = std::env::var_os("MPDOOM_FRAME_LOG").is_some();
                        let mut last_sent_at = std::time::Instant::now();
                        let mut last_send: Option<std::time::Instant> = None;
                        let min_gap = frame_t.mul_f32(0.70);
                        let mut last_drift_at = std::time::Instant::now();
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
                                    if tx2.send(Msg::VideoFrame { w, h, rgba: buf, gen: gen2 }).is_err() {
                                        break;
                                    }
                                    buf = vec![0u8; frame];
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
                            // Present on a monotonic schedule. The audio
                            // position arrives in buffer-sized jumps, and
                            // gating every frame on it made the intervals
                            // alternate between very short and very long.
                            // Correct slow drift against the audio clock once
                            // per second instead.
                            let mut now = std::time::Instant::now();
                            if now > next_t + frame_t {
                                // More than one frame late: restart the
                                // schedule rather than sending a burst.
                                next_t = now;
                            }
                            while std::time::Instant::now() < next_t {
                                now = std::time::Instant::now();
                                let remain = next_t.saturating_duration_since(now);
                                if remain > std::time::Duration::from_millis(4) {
                                    std::thread::sleep(remain - std::time::Duration::from_millis(2));
                                } else {
                                    std::thread::sleep(std::time::Duration::from_millis(1));
                                }
                            }
                            if last_drift_at.elapsed() >= std::time::Duration::from_secs(1) {
                                let c = clock.load(std::sync::atomic::Ordering::Relaxed);
                                if c != 0 && ((c >> 48) as u16) == mygen {
                                    let cpos = c & ((1u64 << 48) - 1);
                                    if cpos != 0 {
                                        let apos = (cpos - 1) as f32 / 1000.0;
                                        let vpos = frames as f32 / fps;
                                        let drift = apos - vpos;
                                        if drift.abs() > 0.08 {
                                            // Nudge the schedule by at most
                                            // 10 ms per second. No frame is
                                            // skipped and no burst is shown.
                                            let step = drift.clamp(-0.01, 0.01);
                                            if step > 0.0 {
                                                next_t = next_t
                                                    .checked_sub(
                                                        std::time::Duration::from_secs_f32(step),
                                                    )
                                                    .unwrap_or(next_t);
                                            } else {
                                                next_t += std::time::Duration::from_secs_f32(-step);
                                            }
                                        }
                                    }
                                }
                                last_drift_at = std::time::Instant::now();
                            }
                            if let Some(last) = last_send {
                                let elapsed = last.elapsed();
                                if elapsed < min_gap {
                                    std::thread::sleep(min_gap - elapsed);
                                }
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
                                if tx2.send(Msg::VideoFrame { w, h, rgba: buf, gen: gen2 }).is_err() {
                                    break;
                                }
                                buf = vec![0u8; frame];
                                last_send = Some(std::time::Instant::now());
                                if frames == 0 {
                                    av_log("reader: first frame sent");
                                }
                            }
                            frames += 1;
                            if frame_log {
                                let now = std::time::Instant::now();
                                av_log(&format!(
                                    "frame {} dt={:.1}ms",
                                    frames,
                                    now.duration_since(last_sent_at).as_secs_f32() * 1000.0
                                ));
                                last_sent_at = now;
                            }
                            next_t += frame_t;
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
                if let Some(mut c) = video_child.take() { let _ = c.kill(); let _ = c.wait(); }
                if let Some(t) = video_thread.take() { let _ = t.join(); }
                let _ = tx.send(Msg::VideoClosed { gen: 0 });
            }
            LibCmd::VideoPause(p) => {
                video_pause.store(p, std::sync::atomic::Ordering::Relaxed);
            }
            LibCmd::VideoAnalyze { path, gen } => {
                let tx2 = tx.clone();
                std::thread::spawn(move || {
                    if crate::dvd::is_dvd(&path) || crate::bluray::is_bluray(&path) {
                        let _ = tx2.send(Msg::VideoBounds { gen, path, intro_end: 0.0, credits_start: 0.0 });
                        return;
                    }
                    let dur = probe_video_duration(&path);
                    // Named chapters are the most trustworthy source. A file
                    // with "Opening"/"Ending" chapters needs no guessing (and
                    // no audio extraction at all).
                    let (chapter_intro, chapter_credits) = bounds_from_chapters(&probe_chapters(&path));
                    let mut intro = chapter_intro;
                    let mut credits = chapter_credits;
                    // Audio fallback only for episodic-length videos; short
                    // clips and feature films are too easy to misdetect.
                    if (intro <= 0.0 || credits <= 0.0) && (600.0..=3600.0).contains(&dur) {
                        let ff = ffmpeg_path();
                        if intro <= 0.0 {
                            let front_len = dur.min(420.0);
                            let front = video_audio_levels(&path, &ff, 0.0, front_len);
                            intro = detect_intro_end(&front, 0.5);
                        }
                        if credits <= 0.0 && dur > 240.0 {
                            let back_len = dur.min(420.0);
                            let back_start = (dur - back_len).max(0.0);
                            let back = video_audio_levels(&path, &ff, back_start, back_len);
                            let rel = detect_credit_start(&back, 0.5);
                            if rel > 0.0 {
                                credits = back_start + rel;
                            }
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

