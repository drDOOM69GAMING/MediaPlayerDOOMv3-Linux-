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

pub fn source(path: &str) -> &str {
    path.strip_prefix(PREFIX).unwrap_or(path)
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
