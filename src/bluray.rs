// Blu-ray sources (disc drive or ISO image).
//
// ffmpeg's `bluray:` protocol uses libbluray, and libaacs reads AACS keys from
// ~/.config/aacs/KEYDB.cfg (or $XDG_CONFIG_HOME/aacs/KEYDB.cfg) automatically.
#![allow(dead_code)]

pub const PREFIX: &str = "bluray:";

pub fn is_bluray(path: &str) -> bool {
    path.starts_with(PREFIX)
}

pub fn wrap(path: &str) -> String {
    if is_bluray(path) {
        path.to_string()
    } else {
        format!("{}{}", PREFIX, path)
    }
}

pub fn wrap_playlist(source: &str, playlist: Option<u32>) -> String {
    match playlist {
        Some(n) => format!("{}{}:{}", PREFIX, n, source),
        None => format!("{}{}", PREFIX, source),
    }
}

pub fn wrap_chapter(source: &str, playlist: Option<u32>, chapter: u32) -> String {
    match playlist {
        Some(n) => format!("{}{}:{}:{}", PREFIX, n, chapter, source),
        None => format!("{}{}", PREFIX, source),
    }
}

/// Path after the optional `bluray:<playlist>:` marker.
pub fn source(path: &str) -> &str {
    let mut rest = path.strip_prefix(PREFIX).unwrap_or(path);
    // Up to two numeric markers: playlist then chapter.
    for _ in 0..2 {
        if let Some((first, tail)) = rest.split_once(':') {
            if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) && tail.starts_with('/') {
                return tail;
            }
            if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) {
                rest = tail;
                continue;
            }
        }
        break;
    }
    rest
}

pub fn playlist(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PREFIX)?;
    let (first, _) = rest.split_once(':')?;
    first.parse::<u32>().ok()
}

pub fn chapter(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PREFIX)?;
    let (_pl, rest) = rest.split_once(':')?;
    let (ch, _) = rest.split_once(':')?;
    if ch.starts_with('/') {
        return None;
    }
    ch.parse::<u32>().ok()
}

#[derive(Clone, Copy)]
pub struct TitleInfo {
    pub playlist: u32,
    pub secs: f32,
    pub chapters: u32,
    /// 0-based audio stream index whose language is English, when known.
    pub english_audio: Option<usize>,
}

/// Ask libbluray which playlist is the main title and how long it is. The
/// duration lets the player skip a second optical probe before the first frame.
pub fn main_title_info(source: &str) -> Option<TitleInfo> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<TitleInfo>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(m) = cache.lock() {
        if let Some(v) = m.get(source) {
            return *v;
        }
    }
    let info = (|| {
        let out = std::process::Command::new("bd_list_titles")
            .args(["-l", "-a"])
            .arg(source)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut main_idx: Option<usize> = None;
        let mut current_idx: Option<usize> = None;
        let mut audio_langs: Vec<String> = Vec::new();
        let mut main_audio: Option<usize> = None;
        let mut titles: Vec<(usize, u32, u32, u32)> = Vec::new(); // index, secs, playlist, chapters
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Main title:") {
                main_idx = v.trim().parse().ok();
            }
            if let Some(v) = line.trim().strip_prefix("AUD:") {
                if current_idx == main_idx {
                    audio_langs = v.split_whitespace().map(|x| x.to_ascii_lowercase()).collect();
                }
                continue;
            }
            if !line.trim_start().starts_with("index:") {
                continue;
            }
            current_idx = line.split_whitespace().nth(1).and_then(|v| v.parse::<usize>().ok());
            let idx = line.split_whitespace().nth(1).and_then(|v| v.parse::<usize>().ok());
            let dur = line.split("duration:").nth(1).and_then(|rest| {
                let hm = rest.split_whitespace().next()?;
                let mut it = hm.split(':');
                let h: u32 = it.next()?.parse().ok()?;
                let m: u32 = it.next()?.parse().ok()?;
                let s: u32 = it.next()?.parse().ok()?;
                Some(h * 3600 + m * 60 + s)
            });
            let pl = line.split("(playlist:").nth(1).and_then(|rest| {
                rest.split_whitespace()
                    .next()?
                    .trim_end_matches(')')
                    .trim_end_matches(".mpls")
                    .parse::<u32>()
                    .ok()
            });
            let chapters = line.split("chapters:").nth(1).and_then(|rest| {
                rest.split_whitespace().next()?.parse::<u32>().ok()
            }).unwrap_or(0);
            if let (Some(i), Some(d), Some(p)) = (idx, dur, pl) {
                titles.push((i, d, p, chapters));
            }
        }
        if !audio_langs.is_empty() {
            main_audio = audio_langs.iter().position(|l| matches!(l.as_str(), "eng" | "en" | "english"));
        }
        let chosen = main_idx
            .and_then(|mi| titles.iter().find(|(i, _, _, _)| *i == mi).copied())
            .or_else(|| titles.iter().max_by_key(|(_, d, _, _)| *d).copied())?;
        Some(TitleInfo { playlist: chosen.2, secs: chosen.1 as f32, chapters: chosen.3, english_audio: main_audio })
    })();
    if let Ok(mut m) = cache.lock() {
        m.insert(source.to_string(), info);
    }
    info
}

pub fn main_playlist(source: &str) -> Option<u32> {
    main_title_info(source).map(|i| i.playlist)
}

pub fn english_audio_map(source: &str) -> Option<String> {
    main_title_info(source)
        .and_then(|i| i.english_audio)
        .map(|n| format!("0:a:{}", n))
}

pub fn display_name(path: &str) -> String {
    if !is_bluray(path) {
        return crate::util::stem(path);
    }
    let src = source(path);
    if src.starts_with("/dev/") {
        format!("BLU-RAY ({})", src)
    } else {
        std::path::Path::new(src)
            .file_name()
            .map(|n| format!("BLU-RAY {}", n.to_string_lossy()))
            .unwrap_or_else(|| "BLU-RAY".to_string())
    }
}

#[cfg(target_os = "linux")]
pub fn drives() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("sr") && name[2..].chars().all(|c| c.is_ascii_digit()) {
                out.push(e.path().to_string_lossy().to_string());
            }
        }
    }
    for fixed in ["/dev/dvd", "/dev/cdrom"] {
        if std::path::Path::new(fixed).exists() {
            out.push(fixed.to_string());
        }
    }
    out.sort();
    out.dedup_by(|a, b| std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok());
    out
}

#[cfg(not(target_os = "linux"))]
pub fn drives() -> Vec<String> {
    Vec::new()
}

pub fn first_drive() -> Option<String> {
    drives().into_iter().next()
}

/// Search the usual locations for a user-supplied AACS KEYDB.cfg. Keys are
/// never bundled with the app; only the user's own database is used.
pub fn keydb_candidates() -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute());
    let mut out = Vec::new();
    if let Some(x) = &xdg {
        out.push(x.join("aacs").join("KEYDB.cfg"));
    }
    if let Some(h) = &home {
        out.push(h.join(".config/aacs/KEYDB.cfg"));
        out.push(h.join(".aacs/KEYDB.cfg"));
    }
    out.push(std::path::PathBuf::from("/etc/xdg/aacs/KEYDB.cfg"));
    out.push(std::path::PathBuf::from("/usr/share/aacs/KEYDB.cfg"));
    out
}

pub fn keydb_path() -> Option<std::path::PathBuf> {
    keydb_candidates().into_iter().find(|p| p.is_file())
}

/// libaacs only looks at the effective XDG config directory. If the user's
/// KEYDB lives elsewhere, point ffmpeg's child environment at a small config
/// directory containing a symlink to it.
pub fn aacs_env() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static ENV: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    ENV.get_or_init(|| {
        let key = keydb_path()?;
        let effective = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
        if key == effective.join("aacs/KEYDB.cfg") {
            return None;
        }
        let dir = crate::util::data_dir().join("aacs-env");
        let aacs = dir.join("aacs");
        std::fs::create_dir_all(&aacs).ok()?;
        let link = aacs.join("KEYDB.cfg");
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&key, &link).ok()?;
        #[cfg(not(unix))]
        std::fs::copy(&key, &link).ok()?;
        Some(dir)
    })
    .clone()
}

pub fn configure_command(cmd: &mut std::process::Command) {
    if let Some(dir) = aacs_env() {
        cmd.env("XDG_CONFIG_HOME", dir);
    }
}

pub fn eject(path: &str) -> bool {
    if !is_bluray(path) {
        return false;
    }
    let src = source(path);
    if !src.starts_with("/dev/") {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(file) = std::fs::OpenOptions::new().read(true).open(src) {
            use std::os::unix::io::AsRawFd;
            if unsafe { libc::ioctl(file.as_raw_fd(), 0x5309, 0) } == 0 {
                return true;
            }
        }
        return std::process::Command::new("eject")
            .arg(src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_parsing() {
        let p = wrap_chapter("/dev/sr0", Some(1), 5);
        assert_eq!(p, "bluray:1:5:/dev/sr0");
        assert_eq!(source(&p), "/dev/sr0");
        assert_eq!(playlist(&p), Some(1));
        assert_eq!(chapter(&p), Some(5));
        let iso = wrap_playlist("/home/user/Movie.iso", Some(100));
        assert_eq!(source(&iso), "/home/user/Movie.iso");
        assert_eq!(playlist(&iso), Some(100));
        assert_eq!(chapter(&iso), None);
    }
}
