#![allow(unused_imports)]
#![allow(dead_code)]
// Audio file collection, tagging, transcoding, recording, art discovery.
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use lofty::prelude::*;
use crate::consts::NO_WINDOW;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
/// Directories that never contain music and can be huge (or recursive symlink
/// traps). Matching is case-insensitive.
fn skip_scan_dir(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    matches!(
        n.as_str(),
        "system volume information"
            | "$recycle.bin"
            | "recycler"
            | "lost+found"
            | "node_modules"
            | "__macosx"
            | ".git"
            | ".cache"
            | ".trash"
    ) || n.starts_with(".trash-")
}

fn walk_audio<F: FnMut(usize)>(
    dir: &Path,
    out: &mut Vec<String>,
    cues: &mut Vec<String>,
    progress: &mut F,
    visited: &mut HashSet<PathBuf>,
) {
    // Symlinked music folders are common on Linux. Follow them, but canonical
    // paths are tracked so a link back to a parent cannot make this recurse.
    let canon = dir.canonicalize().ok();
    if let Some(c) = &canon {
        if !visited.insert(c.clone()) {
            return;
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                if skip_scan_dir(&name) {
                    continue;
                }
            }
            walk_audio(&p, out, cues, progress, visited);
        } else if ft.is_symlink() && p.is_dir() {
            if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                if !skip_scan_dir(&name) {
                    walk_audio(&p, out, cues, progress, visited);
                }
            }
        } else if ft.is_file() || (ft.is_symlink() && p.is_file()) {
            let s = p.to_string_lossy().to_string();
            if is_audio(&s) {
                out.push(s);
                progress(out.len() + cues.len());
            } else if crate::cue::is_cue_file(&s) {
                cues.push(s);
            }
        }
    }
}

pub fn collect_audio(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cues = Vec::new();
    let mut visited = HashSet::new();
    walk_audio(Path::new(dir), &mut out, &mut cues, &mut |_| {}, &mut visited);
    expand_cues(&mut out, cues);
    out.sort();
    out
}

/// Same as `collect_audio`, but stops promptly when `cancel` is set. Used by
/// folder conversion so closing the app during a large-library scan does not
/// freeze the UI while it waits for the walk to finish.
pub fn collect_audio_cancellable(
    dir: &str,
    cancel: &std::sync::atomic::AtomicBool,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut cues = Vec::new();
    let mut visited = HashSet::new();
    walk_audio_cancellable(Path::new(dir), &mut out, &mut cues, &mut visited, cancel);
    expand_cues(&mut out, cues);
    out.sort();
    out
}

fn walk_audio_cancellable(
    dir: &Path,
    out: &mut Vec<String>,
    cues: &mut Vec<String>,
    visited: &mut HashSet<PathBuf>,
    cancel: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let canon = dir.canonicalize().ok();
    if let Some(c) = &canon {
        if !visited.insert(c.clone()) {
            return;
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                if skip_scan_dir(&name) {
                    continue;
                }
            }
            walk_audio_cancellable(&p, out, cues, visited, cancel);
        } else if ft.is_symlink() && p.is_dir() {
            if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                if !skip_scan_dir(&name) {
                    walk_audio_cancellable(&p, out, cues, visited, cancel);
                }
            }
        } else if ft.is_file() || (ft.is_symlink() && p.is_file()) {
            let s = p.to_string_lossy().to_string();
            if is_audio(&s) {
                out.push(s);
            } else if crate::cue::is_cue_file(&s) {
                cues.push(s);
            }
        }
    }
}

/// One directory remembered by an incremental scan. `mtime` changes when
/// entries are added, removed or renamed, which is exactly when the listing
/// must be re-read.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct ScanDirEntry {
    mtime: u64,
    /// Audio and CUE files directly in this directory.
    files: Vec<String>,
    dirs: Vec<String>,
}

static SCAN_CACHE_LOCK: Mutex<()> = Mutex::new(());

fn scan_cache_path() -> PathBuf {
    crate::util::data_dir().join("scan_cache.json")
}

fn load_scan_cache(path: &Path) -> HashMap<String, ScanDirEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_scan_cache(path: &Path, cache: &HashMap<String, ScanDirEntry>) {
    if let Ok(text) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, text);
    }
}

fn dir_mtime(dir: &Path) -> u64 {
    std::fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn walk_audio_cached<F: FnMut(usize)>(
    dir: &Path,
    out: &mut Vec<String>,
    cues: &mut Vec<String>,
    progress: &mut F,
    visited: &mut HashSet<PathBuf>,
    cache: &HashMap<String, ScanDirEntry>,
    new_cache: &mut HashMap<String, ScanDirEntry>,
) {
    let canon = dir.canonicalize().ok();
    if let Some(c) = &canon {
        if !visited.insert(c.clone()) {
            return;
        }
    }
    let key = dir.to_string_lossy().to_string();
    let mtime = dir_mtime(dir);

    if mtime != 0 {
        if let Some(entry) = cache.get(&key) {
            if entry.mtime == mtime {
                for f in &entry.files {
                    if is_audio(f) {
                        out.push(f.clone());
                        progress(out.len() + cues.len());
                    } else if crate::cue::is_cue_file(f) {
                        cues.push(f.clone());
                    }
                }
                for d in &entry.dirs {
                    walk_audio_cached(Path::new(d), out, cues, progress, visited, cache, new_cache);
                }
                new_cache.insert(key, entry.clone());
                return;
            }
        }
    }

    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        let name = p.file_name().map(|n| n.to_string_lossy().to_string());
        if ft.is_dir() || (ft.is_symlink() && p.is_dir()) {
            if let Some(n) = &name {
                if skip_scan_dir(n) {
                    continue;
                }
            }
            dirs.push(p.to_string_lossy().to_string());
        } else if ft.is_file() || (ft.is_symlink() && p.is_file()) {
            let s = p.to_string_lossy().to_string();
            if is_audio(&s) {
                out.push(s.clone());
                files.push(s);
                progress(out.len() + cues.len());
            } else if crate::cue::is_cue_file(&s) {
                cues.push(s.clone());
                files.push(s);
            }
        }
    }
    for d in &dirs {
        walk_audio_cached(Path::new(d), out, cues, progress, visited, cache, new_cache);
    }
    new_cache.insert(key, ScanDirEntry { mtime, files, dirs });
}

fn collect_audio_report_cached(
    dir: &str,
    tx: &Sender<Msg>,
    cache_path: &Path,
) -> Vec<String> {
    let _guard = SCAN_CACHE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cache = load_scan_cache(cache_path);
    let mut new_cache: HashMap<String, ScanDirEntry> = HashMap::new();
    let mut out = Vec::new();
    let mut cues = Vec::new();
    let mut last = std::time::Instant::now();
    let mut progress = |found: usize| {
        if last.elapsed().as_millis() >= 300 {
            let _ = tx.send(Msg::ScanProgress { found });
            last = std::time::Instant::now();
        }
    };
    let mut visited = HashSet::new();
    walk_audio_cached(
        Path::new(dir),
        &mut out,
        &mut cues,
        &mut progress,
        &mut visited,
        &cache,
        &mut new_cache,
    );
    if !new_cache.is_empty() {
        // Merge instead of replacing: AddMany may scan one subfolder and must
        // not throw away cache entries for the rest of the library.
        let mut merged = cache;
        merged.extend(new_cache);
        save_scan_cache(cache_path, &merged);
    }
    expand_cues(&mut out, cues);
    out.sort();
    out
}

pub fn collect_audio_report(dir: &str, tx: &Sender<Msg>) -> Vec<String> {
    let path = scan_cache_path();
    collect_audio_report_cached(dir, tx, &path)
}

fn normalized_path_key(path: &str) -> String {
    path.replace('\\', "/").replace("/./", "/")
}

/// Replace a single-file image with its CUE tracks. Multi-FILE sheets and
/// sheets whose image is missing are ignored; those folders already contain
/// one file per track.
fn expand_cues(files: &mut Vec<String>, cues: Vec<String>) {
    let mut done: HashSet<String> = HashSet::new();
    for cue in cues {
        let Some(sheet) = crate::cue::sheet(&cue) else { continue };
        let base = normalized_path_key(&sheet.audio_path);
        if !done.insert(base.clone()) {
            continue;
        }
        let before = files.len();
        files.retain(|p| normalized_path_key(p) != base);
        if files.len() == before {
            continue;
        }
        for t in &sheet.tracks {
            files.push(crate::cue::make_path(&cue, t.number));
        }
    }
}

/// Some files carry ID3 tags with bytes that make ffmpeg's content probe pick
/// the wrong demuxer (two MP3s in the scanned library were mistaken for WAV).
/// Hint the demuxer from the extension so those files still decode.
pub(crate) fn input_format_hint(audio: &str) -> Option<&'static str> {
    let ext = Path::new(audio)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => Some("mp3"),
        "flac" => Some("flac"),
        "m4a" | "m4b" | "m4p" => Some("mov"),
        "ape" => Some("ape"),
        "wav" | "wave" => Some("wav"),
        "aif" | "aiff" | "aifc" => Some("aiff"),
        "wma" | "asf" => Some("asf"),
        "wv" => Some("wv"),
        "mpc" => Some("mpc"),
        _ => None,
    }
}

pub fn transcode_with_ffmpeg(audio: &str, ffmpeg: &Path) -> Result<PathBuf, String> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    audio.hash(&mut h);
    let slug = format!("{:016x}", h.finish());
    let dir = work_dir().join("tc");
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join(format!("{}.wav", slug));
    if out.exists() && out.metadata().map(|m| m.len() > 4096).unwrap_or(false) {
        return Ok(out);
    }
    // Write to a partial name and rename only after ffmpeg succeeds. A large
    // APE album can take a minute to convert; quitting midway must never leave
    // something that the next run treats as a complete cached WAV.
    let part = dir.join(format!("{}.wav.part", slug));
    let _ = std::fs::remove_file(&part);
    let ff = ffmpeg.to_string_lossy().to_string();
    let po = part.to_string_lossy().to_string();
    let cue = crate::cue::track_for(audio);
    let input = crate::cue::base_audio(audio).unwrap_or_else(|| audio.to_string());
    let mut cmd = std::process::Command::new(ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if let Some(fmt) = input_format_hint(&input) {
        cmd.args(["-f", fmt]);
    }
    if let Some(t) = &cue {
        if t.start > 0.0 {
            cmd.args(["-ss", &format!("{:.3}", t.start)]);
        }
        if t.end > t.start {
            cmd.args(["-t", &format!("{:.3}", t.end - t.start)]);
        }
    }
    let st = cmd
        .args(["-i", &input, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .no_window()
        .output()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.status.success() && part.exists() {
        if let Err(e) = std::fs::rename(&part, &out) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("could not cache converted audio: {}", e));
        }
        Ok(out)
    } else {
        let _ = std::fs::remove_file(&part);
        let tail = String::from_utf8_lossy(&st.stderr)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        Err(if tail.is_empty() {
            "ffmpeg could not convert this file".to_string()
        } else {
            format!("ffmpeg could not convert this file: {}", tail)
        })
    }
}

pub fn record_with_ffmpeg(audio: &str, ffmpeg: &Path, dest: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let po = dest.to_string_lossy().to_string();
    let _ = std::fs::remove_file(dest);
    let cue = crate::cue::track_for(audio);
    let input = crate::cue::base_audio(audio).unwrap_or_else(|| audio.to_string());
    let mut cmd = std::process::Command::new(ffmpeg.to_string_lossy().to_string());
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    if let Some(fmt) = input_format_hint(&input) {
        cmd.args(["-f", fmt]);
    }
    if let Some(t) = &cue {
        if t.start > 0.0 {
            cmd.args(["-ss", &format!("{:.3}", t.start)]);
        }
        if t.end > t.start {
            cmd.args(["-t", &format!("{:.3}", t.end - t.start)]);
        }
    }
    let st = cmd
        .args(["-i", &input, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .no_window()
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && dest.exists() {
        Ok(dest.to_path_buf())
    } else {
        Err("ffmpeg could not record this file".into())
    }
}

pub fn record_flight_ffmpeg(url: &str, ffmpeg: &Path, dest: &Path, secs: u32) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let po = dest.to_string_lossy().to_string();
    let _ = std::fs::remove_file(dest);
    let secs = secs.to_string();
    let st = std::process::Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-y", "-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", "-t", &secs, &po])
        .no_window()
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && dest.exists() {
        Ok(dest.to_path_buf())
    } else {
        Err("ffmpeg could not record this stream".into())
    }
}

pub fn find_music_folder() -> Option<String> {
    let mut best: Option<(PathBuf, usize)> = None;
    let mut check = |d: &Path, prio: usize| {
        let files = collect_audio(&d.to_string_lossy());
        if !files.is_empty() {
            let score = files.len() + prio;
            if best.as_ref().map_or(true, |(_, s)| score > *s) {
                best = Some((d.to_path_buf(), score));
            }
        }
    };
    if let Some(home) = crate::util::home_dir() {
        check(&home.join("Music"), 100);
        check(&home.join("OneDrive").join("Music"), 100);
        if let Some(music) = crate::tools::music_dir() {
            check(&music, 120);
        }
    }
    for root in existing_drive_roots() {
        check(&root.join("Music"), 40);
    }
    best.map(|(p, _)| p.to_string_lossy().to_string())
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
}

pub fn existing_drive_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        let mask = unsafe { GetLogicalDrives() };
        for bit in 0..26u32 {
            if mask & (1 << bit) != 0 {
                let ch = char::from_u32(b'A' as u32 + bit).unwrap_or('A');
                roots.push(PathBuf::from(format!("{}:\\", ch)));
            }
        }
    }
    #[cfg(not(windows))]
    {
        // Common Linux mount points. Only `Music` directly below each root is
        // checked, so this stays cheap even with several mounted drives.
        roots.push(PathBuf::from("/"));
        roots.push(PathBuf::from("/media"));
        roots.push(PathBuf::from("/mnt"));
        if let Some(home) = crate::util::home_dir() {
            let user = home.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let removable = PathBuf::from("/run/media").join(&user);
            if removable.is_dir() {
                roots.push(removable.clone());
                if let Ok(rd) = std::fs::read_dir(&removable) {
                    roots.extend(rd.flatten().map(|e| e.path()));
                }
            }
            roots.push(home);
        }
        for base in ["/media", "/mnt"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                roots.extend(rd.flatten().map(|e| e.path()));
            }
        }
    }
    roots
}

pub fn find_local_art(song_path: &str) -> Option<Vec<u8>> {
    let disk = crate::cue::disk_path(song_path);
    let p = Path::new(&disk);
    if let Some(dir) = p.parent() {
        if let Some(b) = find_dir_art(dir) {
            return Some(b);
        }
    }
    if let Some(probed) = load_tagged(&disk) {
        if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
            for pic in tag.pictures() {
                let data = pic.data();
                if art_bytes_ok(data, 16) {
                    return Some(data.to_vec());
                }
            }
        }
    }
    None
}

/// Rank image files inside one folder. Cover-ish names win over arbitrary
/// scans, and back/disc/booklet images come last so they never get chosen as
/// the album cover just because they sort first.
fn find_dir_art(dir: &Path) -> Option<Vec<u8>> {
    const EXTS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "jfif"];
    let mut candidates: Vec<(u8, String, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return None };
    for e in rd.flatten() {
        let ep = e.path();
        if !ep.is_file() {
            continue;
        }
        let Some(ext) = ep.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()) else { continue };
        if !EXTS.contains(&ext.as_str()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let base = name.split('.').next().unwrap_or("").trim().to_string();
        let prio = match base.as_str() {
            "cover" | "folder" | "front" => 0,
            "album" | "albumart" | "albumartlarge" | "albumartsmall" => 1,
            "artwork" | "default" | "art" => 2,
            "back" | "backcover" | "cd" | "disc" | "inside" | "booklet" | "scan" => 40,
            _ => 20,
        };
        candidates.push((prio, name, ep));
    }
    candidates.sort();
    for (_, _, path) in candidates {
        if let Ok(b) = std::fs::read(&path) {
            if art_bytes_ok(&b, 16) {
                return Some(b);
            }
        }
    }
    None
}

pub fn load_tagged(path: &str) -> Option<lofty::file::TaggedFile> {
    let probe = lofty::probe::Probe::open(path).ok()?;
    let probe = probe.guess_file_type().ok()?;
    probe.read().ok()
}

/// Normalize cover art for the local cache: decode, downscale to <=220px and
/// re-encode as JPEG so the whole-playlist cache stays small (~10-30 KB/file).
/// Returns None for huge/unreadable images (those are skipped, not cached).
pub fn normalize_art(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let thumb = image::imageops::thumbnail(&rgba, 220, 220);
    let dimg = image::DynamicImage::ImageRgba8(thumb);
    let mut out = std::io::Cursor::new(Vec::new());
    dimg.write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    Some(out.into_inner())
}


#[cfg(test)]
mod file_pipeline_tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch() -> PathBuf {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let d = std::env::temp_dir().join(format!("mpdoom-files-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_audio(dir: &Path) -> PathBuf {
        let out = dir.join("tone.mp3");
        let st = Command::new(ffmpeg_path())
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
                "-c:a", "libmp3lame",
            ])
            .arg(&out)
            .status()
            .expect("ffmpeg should start");
        assert!(st.success(), "could not create test mp3");
        out
    }

    #[test]
    #[ignore]
    fn scan_real_music_from_env() {
        let Some(dir) = std::env::var_os("MPDOOM_TEST_MUSIC") else {
            return;
        };
        let files = collect_audio(&dir.to_string_lossy());
        let cue_tracks = files.iter().filter(|p| crate::cue::is_cue(p)).count();
        println!("total={} cue_tracks={}", files.len(), cue_tracks);
        if let Some(p) = files.iter().find(|p| crate::cue::is_cue(p)) {
            println!(
                "sample={} stem={} display={} base={:?}",
                p,
                stem(p),
                crate::library::make_display(p),
                crate::cue::base_audio(p)
            );
        }
        assert!(files.len() >= 18_000, "only found {} files", files.len());
        assert!(cue_tracks >= 40, "CUE sheets were not expanded: {}", cue_tracks);
    }

    #[test]
    #[ignore]
    fn transcode_real_file_from_env() {
        let Some(p) = std::env::var_os("MPDOOM_TEST_AUDIO") else {
            return;
        };
        let path = PathBuf::from(p);
        let wav = transcode_with_ffmpeg(&path.to_string_lossy(), &ffmpeg_path())
            .expect("real file should transcode");
        assert!(wav.metadata().unwrap().len() > 4096);
        let _ = std::fs::remove_file(&wav);
    }

    #[test]
    fn transcode_hints_demuxer_from_extension() {
        assert_eq!(input_format_hint("a.mp3"), Some("mp3"));
        assert_eq!(input_format_hint("a.MP3"), Some("mp3"));
        assert_eq!(input_format_hint("a.ape"), Some("ape"));
        assert_eq!(input_format_hint("a.m4a"), Some("mov"));
        assert_eq!(input_format_hint("a.flac"), Some("flac"));
        assert_eq!(input_format_hint("a.unknown"), None);
    }

    #[test]
    fn transcode_writes_complete_cached_wav() {
        let dir = scratch();
        let mp3 = make_audio(&dir);
        let wav = transcode_with_ffmpeg(&mp3.to_string_lossy(), &ffmpeg_path())
            .expect("mp3 should transcode");
        assert!(wav.is_file());
        assert!(wav.metadata().unwrap().len() > 4096);
        let part = wav.with_extension("wav.part");
        assert!(!part.exists(), "partial cache file must not remain");
        let again = transcode_with_ffmpeg(&mp3.to_string_lossy(), &ffmpeg_path())
            .expect("cached transcode should return");
        assert_eq!(wav, again);
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collects_supported_audio_recursively() {
        let dir = scratch();
        let a = make_audio(&dir);
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let b = sub.join("copy.flac");
        std::fs::copy(&a, &b).unwrap();
        std::fs::write(dir.join("notes.txt"), "not audio").unwrap();
        let found = collect_audio(&dir.to_string_lossy());
        assert_eq!(found.len(), 2, "found: {found:?}");
        assert!(found.iter().any(|p| p.ends_with("tone.mp3")));
        assert!(found.iter().any(|p| p.ends_with("copy.flac")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_skips_junk_and_symlink_loops() {
        let dir = scratch();
        let a = make_audio(&dir);
        let junk = dir.join("System Volume Information");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::copy(&a, junk.join("fake.mp3")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        }
        let extra = dir.join("linked-album");
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::copy(&a, extra.join("linked.mp3")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&extra, dir.join("music-link")).unwrap();
        }
        let found = collect_audio(&dir.to_string_lossy());
        // The real folder and a symlink to it are the same physical directory,
        // so its tracks are included exactly once.
        assert_eq!(found.len(), 2, "found: {found:?}");
        assert!(found.iter().any(|p| p.ends_with("tone.mp3")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcodes_and_records_with_ffmpeg() {
        let dir = scratch();
        let a = make_audio(&dir);
        let tc = transcode_with_ffmpeg(&a.to_string_lossy(), &ffmpeg_path()).expect("transcode");
        assert!(tc.is_file());
        assert!(std::fs::metadata(&tc).unwrap().len() > 4096);
        let dest = dir.join("rec.wav");
        let rec = record_with_ffmpeg(&a.to_string_lossy(), &ffmpeg_path(), &dest).expect("record");
        assert!(rec.is_file());
        assert!(std::fs::metadata(&rec).unwrap().len() > 4096);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incremental_scan_reuses_and_invalidates_directory_cache() {
        let dir = scratch();
        let root = dir.join("Music");
        let album = root.join("Artist").join("Album");
        std::fs::create_dir_all(&album).unwrap();
        let a = album.join("a.mp3");
        let b = album.join("b.mp3");
        std::fs::write(&a, b"x").unwrap();
        let cache = dir.join("scan_cache.json");
        let (tx, _rx) = std::sync::mpsc::channel();

        let first = collect_audio_report_cached(&root.to_string_lossy(), &tx, &cache);
        assert_eq!(first.len(), 1);
        assert!(cache.is_file(), "scan cache should be written");

        // Unchanged: the cached listing must still return the same file.
        let again = collect_audio_report_cached(&root.to_string_lossy(), &tx, &cache);
        assert_eq!(again, first);

        // New file: the album directory mtime changes, so the listing refreshes.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&b, b"y").unwrap();
        let after_add = collect_audio_report_cached(&root.to_string_lossy(), &tx, &cache);
        assert_eq!(after_add.len(), 2);

        // Deleted file: same invalidation path must drop it.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::remove_file(&a).unwrap();
        let after_del = collect_audio_report_cached(&root.to_string_lossy(), &tx, &cache);
        assert_eq!(after_del.len(), 1);
        assert!(after_del[0].ends_with("b.mp3"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_tags_and_normalizes_art() {
        let dir = scratch();
        let a = make_audio(&dir);
        // No tags were written, but lofty must still be able to open the file.
        let tagged = load_tagged(&a.to_string_lossy());
        assert!(tagged.is_some(), "lofty could not open the generated mp3");
        let img = image::RgbaImage::from_pixel(400, 400, image::Rgba([10, 200, 30, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let small = normalize_art(&png.into_inner()).expect("normalize art");
        let decoded = image::load_from_memory(&small).expect("cached jpeg should decode");
        assert!(decoded.width() <= 220 && decoded.height() <= 220);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
