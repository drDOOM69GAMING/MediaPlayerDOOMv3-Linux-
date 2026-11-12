#![allow(unused_imports)]
#![allow(dead_code)]
// Small shared helpers.
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::consts::AUDIO_FORMATS;

/// Hide the console window for spawned helper processes on Windows.
/// On Unix this is a no-op (there is no console to hide), which keeps every
/// call site identical on all platforms.
/// Reap a killed child off the UI thread. `wait()` can block if the process is
/// stuck in network or optical I/O; never let that freeze the window.
pub fn reap_child_async(mut child: std::process::Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

pub trait CommandNoWindow {
    fn no_window(&mut self) -> &mut Self;
    /// Kill this child when the player exits, even on SIGKILL, so a crash
    /// cannot leave ffmpeg holding the optical drive.
    fn die_with_parent(&mut self) -> &mut Self;
}

impl CommandNoWindow for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(crate::consts::NO_WINDOW)
        }
        #[cfg(not(windows))]
        {
            self
        }
    }

    fn die_with_parent(&mut self) -> &mut Self {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                self.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        self
    }
}
pub static BEAUTIFY_RE: [std::sync::OnceLock<regex::Regex>; 4] = [
    std::sync::OnceLock::new(),
    std::sync::OnceLock::new(),
    std::sync::OnceLock::new(),
    std::sync::OnceLock::new(),
];

pub fn beautify_name(filename: &str) -> String {
    let r1 = BEAUTIFY_RE[0].get_or_init(|| regex::Regex::new(r"^(CD\s?\d+[-]?\s*|\d+[-.]?\s*)+").unwrap());
    let r2 = BEAUTIFY_RE[1].get_or_init(|| regex::Regex::new(r"\s*\([^)]*\)\s*").unwrap());
    let r3 = BEAUTIFY_RE[2].get_or_init(|| regex::Regex::new(r"[\[\(]?\d{3}\s?(kbps|kb)?[\)]?$").unwrap());
    let r4 = BEAUTIFY_RE[3].get_or_init(|| regex::Regex::new(r"\s*-\s*\d{4}\s*$").unwrap());
    let mut s = r1.replace(filename, "").to_string();
    s = r2.replace(&s, "").to_string();
    s = r3.replace(&s, "").to_string();
    s = r4.replace(&s, "").to_string();
    let s = s.trim().to_string();
    if s.is_empty() { filename.to_string() } else { s }
}

pub fn is_audio(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_FORMATS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn stem(path: &str) -> String {
    if let Some((_, track, _)) = crate::cd::cdda_parts(path) {
        return format!("Track {:02}", track);
    }
    if let Some(t) = crate::cue::stem_title(path) {
        return t;
    }
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

pub fn fmt_secs(s: f32) -> String {
    let s = s.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn fmt_freq(f: f32) -> String {
    if f >= 400.0 {
        format!("{} kHz", f.round())
    } else {
        format!("{:.1} FM", f)
    }
}

pub fn fmt_run(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Per-user application data directory.
///
/// Windows keeps `%APPDATA%\MediaPlayerOFDOOM` (unchanged from v3).
/// Linux follows the XDG spec: `$XDG_DATA_HOME/MediaPlayerOFDOOM`, falling
/// back to `~/.local/share/MediaPlayerOFDOOM`.
pub fn data_home() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base
}

pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(windows)]
    let app_folder = "MediaPlayerOFDOOM";
    #[cfg(not(windows))]
    let app_folder = "mediaplayerofdoom";
    let d = base.join(app_folder);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Per-user config directory (Linux: `$XDG_CONFIG_HOME/MediaPlayerOFDOOM`).
/// Windows still uses the data dir, so existing installs keep their files.
pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    {
        data_dir()
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        let d = base.join("mediaplayerofdoom");
        let _ = std::fs::create_dir_all(&d);
        d
    }
}

/// User's home directory, cross-platform.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

/// Scratch/work area for transient files (video cuts, transcodes, yt-dlp
/// downloads). Lives inside %APPDATA%\MediaPlayerOFDOOM\work so nothing is
/// dropped into %TEMP% or %LOCALAPPDATA%.
pub fn work_dir() -> PathBuf {
    let d = data_dir().join("work");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Persistent tag cache (%APPDATA%\MediaPlayerOFDOOM\metacache.json). Survives
/// restarts so the playlist needs no lofty re-scan on every launch.
pub fn meta_cache_path() -> PathBuf {
    data_dir().join("metacache.json")
}

/// Persistent list of paths confirmed to have no cover art (no folder/image
/// file, no embedded picture, and no web match). Lets "Meta All" skip tracks
/// that already came up empty instead of re-scanning them every run.
pub fn missing_art_path() -> PathBuf {
    data_dir().join("missing_art.json")
}

/// On-disk cover art cache keyed by song path hash, normalized to small JPEGs.
pub fn artcache_dir() -> PathBuf {
    let d = data_dir().join("artcache");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn fmt_time(s: f32) -> String {
    let s = s.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

pub fn truncate_mid(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3) / 2;
    let head: String = s.chars().take(keep).collect();
    let tail: String = s.chars().rev().take(keep).collect::<Vec<char>>().into_iter().rev().collect();
    format!("{}...{}", head, tail)
}

pub fn sanitize_win(s: &str) -> String {
    let bad = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut out: String = s.trim().chars().map(|c| if bad.contains(&c) { '_' } else { c }).collect();
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    if out.is_empty() {
        out = "untitled".to_string();
    }
    out
}

/// Move a file, falling back to copy+delete when `rename` cannot cross
/// filesystems. On Linux the download scratch area lives on the home
/// filesystem while the Music library is often on another disk (`/run/media`,
/// `/mnt`, a separate partition), where `rename` fails with EXDEV.
pub fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)?;
            Ok(())
        }
    }
}

/// True for tag values that mean "we don't actually know".
///
/// Searching an online service for "Unknown Artist / Unknown Album" returns a
/// random album whose cover then gets cached as if it belonged to the track.
pub fn is_placeholder_meta(s: &str) -> bool {
    let n = s.trim().to_ascii_lowercase();
    matches!(
        n.as_str(),
        "" | "unknown" | "unknown artist" | "unknown album" | "unknown title"
            | "artist" | "album" | "title" | "untitled" | "n/a" | "na" | "none"
    )
}

/// Validate image bytes before caching or displaying them.
///
/// `min_side` rejects thumbnails/placeholders that are too small to be album
/// art. A near-solid image (all sampled pixels within a few levels of each
/// other) is also rejected: several online sources return a blank white/black
/// square for "no cover", which used to get cached and shown as an empty box.
pub fn art_bytes_ok(bytes: &[u8], min_side: u32) -> bool {
    if bytes.len() < 512 {
        return false;
    }
    let Ok(img) = image::load_from_memory(bytes) else { return false };
    if img.width() < min_side || img.height() < min_side {
        return false;
    }
    let rgba = img.to_rgba8();
    let small = image::imageops::thumbnail(&rgba, 16, 16);
    let mut visible = 0u32;
    let (mut lo, mut hi) = (255u8, 0u8);
    for px in small.pixels() {
        if px[3] > 8 {
            visible += 1;
            for c in &px.0[..3] {
                lo = lo.min(*c);
                hi = hi.max(*c);
            }
        }
    }
    visible > 0 && hi.saturating_sub(lo) >= 2
}

pub fn next_track_num(dir: &Path) -> u32 {
    let mut max = 0u32;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = digits.parse::<u32>() {
                max = max.max(v);
            }
        }
    }
    max + 1
}


#[cfg(test)]
mod portability_tests {
    use super::*;

    fn unique(name: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("mpdoom-move-{}-{}-{}", std::process::id(), n, name))
    }

    #[test]
    fn audio_extensions_include_ape_and_ignore_case() {
        for p in [
            "album.ape",
            "album.APE",
            "album.flac",
            "album.FLAC",
            "album.m4a",
            "album.mp3",
            "album.wav",
        ] {
            assert!(is_audio(p), "{p} should be collected as audio");
        }
        for p in ["album.jpg", "album.cue", "album.log", "album"] {
            assert!(!is_audio(p), "{p} should not be collected as audio");
        }
    }

    #[test]
    fn placeholder_metadata_is_rejected() {
        for s in ["", "Unknown", "Unknown Artist", "unknown album", "Untitled", "N/A", "none"] {
            assert!(is_placeholder_meta(s), "{s:?} should be treated as unknown");
        }
        for s in ["Tool", "Lateralus", "Various Artists", "Type O Negative"] {
            assert!(!is_placeholder_meta(s), "{s:?} is real metadata");
        }
    }

    #[test]
    fn move_file_works_within_a_filesystem() {
        let from = unique("a.bin");
        let to = unique("b.bin");
        std::fs::write(&from, b"hello").unwrap();
        move_file(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"hello");
        let _ = std::fs::remove_file(&to);
    }

    #[test]
    fn move_file_copies_across_filesystems() {
        // /dev/shm is usually a separate tmpfs; this is exactly the EXDEV case
        // that broke YouTube downloads when Music lived on another drive.
        let shm = Path::new("/dev/shm");
        if !shm.is_dir() {
            return;
        }
        let from = unique("from.bin");
        let to = shm.join(unique("to.bin").file_name().unwrap());
        std::fs::write(&from, b"cross-device payload").unwrap();
        move_file(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"cross-device payload");
        let _ = std::fs::remove_file(&to);
    }

    #[test]
    fn linux_data_dir_follows_xdg_layout() {
        let d = data_dir();
        assert!(d.is_dir());
        assert!(d.ends_with(if cfg!(windows) { "MediaPlayerOFDOOM" } else { "mediaplayerofdoom" }));
    }
}
