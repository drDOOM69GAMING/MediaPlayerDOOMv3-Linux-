#![allow(unused_imports)]
#![allow(dead_code)]
// Library helpers: bands, sorting, tag reading, weighted picks.
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use lofty::prelude::*;
use rodio::{Decoder, Source};
use crate::files::*;
use crate::util::*;
pub fn weighted_pick(pl: &[String], weights: &HashMap<String, f32>) -> String {
    let mut r = rand::random::<f64>();
    let total: f64 = pl.iter().map(|s| weights.get(s).copied().unwrap_or(1.0) as f64).sum();
    if total <= 0.0 { r = 0.0; } else { r *= total; }
    for s in pl {
        let w = weights.get(s).copied().unwrap_or(1.0) as f64;
        if r < w {
            return s.clone();
        }
        r -= w;
    }
    pl[0].clone()
}

pub fn random_next(current: Option<&str>, pl: &[String], weights: &HashMap<String, f32>) -> String {
    let s = weighted_pick(pl, weights);
    if pl.len() > 1 && current == Some(s.as_str()) {
        if let Some(i) = pl.iter().position(|p| *p == s) {
            return pl[(i + 1) % pl.len()].clone();
        }
    }
    s
}

thread_local! {
    /// Set while a `Decoder::new` may legitimately hit rodio's symphonia
    /// init-seek `unreachable!` panic (M4A/AAC-in-MP4 bug, rodio#846). The
    /// panic hook checks this and stays quiet; `open_decoder` logs a clean
    /// decode-skip line instead.
    pub static EXPECTING_DECODE_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn open_decoder(path: &str) -> Option<Decoder<BufReader<File>>> {
    let f = File::open(path).ok()?;
    EXPECTING_DECODE_PANIC.with(|fl| fl.set(true));
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Decoder::new(BufReader::new(f))));
    EXPECTING_DECODE_PANIC.with(|fl| fl.set(false));
    match res {
        Ok(Ok(d)) => Some(d),
        Ok(Err(_)) => None,
        Err(_) => {
            // rodio panicked inside Decoder::new (symphonia init seek). The panic
            // hook is muted for this; record which file so it can be fixed/removed.
            log_decode_skip(path);
            None
        }
    }
}

fn log_decode_skip(path: &str) {
    use std::io::Write as _;
    let log = data_dir().join("crash.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "\n===== decode-skip =====");
        let _ = writeln!(f, "{}", path);
        let _ = writeln!(f, "file could not be opened by the built-in decoder - use ffmpeg or replace the file");
    }
}

/// Stable per-song key for the on-disk art cache (%APPDATA%\..\artcache).
pub fn art_cache_path(path: &str) -> PathBuf {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    artcache_dir().join(format!("{:016x}.art", h.finish()))
}

pub fn read_art_cache(path: &str) -> Option<Vec<u8>> {
    std::fs::read(art_cache_path(path)).ok()
}

pub fn write_art_cache(path: &str, bytes: &[u8]) -> bool {
    let _ = std::fs::create_dir_all(artcache_dir());
    std::fs::write(art_cache_path(path), bytes).is_ok()
}

pub fn make_display(path: &str) -> String {
    let folder = Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("{} - {}", folder, beautify_name(&stem(path)))
}

/// Playlist row label from real tags, falling back to the filename/folder form.
pub fn playlist_display(artist: &str, title: &str, path: &str) -> String {
    let a = artist.trim();
    let t = title.trim();
    if !t.is_empty() {
        if !a.is_empty() {
            format!("{} - {}", a, t)
        } else {
            t.to_string()
        }
    } else if !a.is_empty() {
        format!("{} - {}", a, beautify_name(&stem(path)))
    } else {
        make_display(path)
    }
}

pub fn song_sort_key(path: &str) -> (String, String) {
    let s = stem(path);
    if let Some(pos) = s.find(" - ") {
        let after = &s[pos + 3..];
        let (title, artist) = if let Some(open) = after.rfind('(') {
            if after.ends_with(')') {
                (&after[..open], after[open + 1..after.len() - 1].trim())
            } else {
                (after, "")
            }
        } else {
            (after, "")
        };
        let artist = if !artist.is_empty() {
            artist.to_string()
        } else {
            Path::new(path)
                .parent()
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        (
            artist.to_lowercase(),
            title.trim().to_lowercase(),
        )
    } else {
        let folder = Path::new(path)
            .parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        (folder.to_lowercase(), s.to_lowercase())
    }
}

pub fn band_of(path: &str) -> String {
    let parent = Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if parent.is_empty() {
        return parent;
    }
    if let Some(open) = parent.rfind('(') {
        if parent.ends_with(')') {
            let inner = parent[open + 1..parent.len() - 1].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    if let Some(pos) = parent.find(" - ") {
        return parent[..pos].trim().to_string();
    }
    parent
}

pub fn band_of_file(path: &str) -> String {
    let s = stem(path);
    if s.is_empty() {
        return band_of(path);
    }
    if let Some(pos) = s.find(" - ") {
        let after = &s[pos + 3..];
        if let Some(open) = after.rfind('(') {
            if after.ends_with(')') {
                let artist = after[open + 1..after.len() - 1].trim();
                if !artist.is_empty() {
                    return artist.to_string();
                }
            }
        }
        return s[..pos].trim().to_string();
    }
    band_of(path)
}

pub fn folder_artist_album(path: &str) -> Option<(String, String)> {
    let parent = Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())?;
    let mut name = parent.trim().to_string();
    if name.is_empty() {
        return None;
    }
    if let Some(open) = name.rfind('(') {
        if name.ends_with(')') {
            name = name[..open].trim().to_string();
        }
    }
    if let Some(pos) = name.find(" - ") {
        let artist = name[..pos].trim().to_string();
        let album = name[pos + 3..].trim().to_string();
        if !artist.is_empty() {
            return Some((artist, album));
        }
    }
    Some((name, String::new()))
}

pub fn strip_track_no(title: &str) -> Option<String> {
    if let Some(sep) = title.find(" - ") {
        let head = title[..sep].trim();
        let digits = !head.is_empty() && head.chars().all(|c| c.is_ascii_digit() || c == '.');
        if digits {
            return Some(title[sep + 3..].trim().to_string());
        }
    }
    None
}

pub fn load_bands(path: &std::path::Path) -> HashMap<String, Vec<String>> {
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(map) = serde_json::from_str::<HashMap<String, Vec<String>>>(&text) {
            return map;
        }
    }
    HashMap::new()
}

// A/V debugging log for developers.
// When true, every av_log() call appends a line to %APPDATA%\MediaPlayerOFDOOM\avsync.log.
// Useful for tracking intro/credits skip, seek, and audio-clock behavior.
// Set to true to re-enable diagnostics; keep false for release builds (av_log is a no-op).
