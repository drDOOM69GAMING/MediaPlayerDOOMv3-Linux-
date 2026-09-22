#![allow(unused_imports)]
#![allow(dead_code)]
// Small shared helpers.
use std::path::{Path, PathBuf};
use std::time::Duration;
use crate::consts::AUDIO_FORMATS;
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

pub fn data_dir() -> PathBuf {
    let base = if let Ok(a) = std::env::var("APPDATA") {
        PathBuf::from(a)
    } else if let Ok(h) = std::env::var("USERPROFILE") {
        PathBuf::from(h)
    } else {
        PathBuf::from(".")
    };
    let d = base.join("MediaPlayerOFDOOM");
    let _ = std::fs::create_dir_all(&d);
    d
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

