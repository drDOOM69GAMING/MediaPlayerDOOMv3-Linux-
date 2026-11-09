// DVD-Video sources (disc drive or ISO image).
//
// Playback itself is done by ffmpeg's `dvdvideo` demuxer (libdvdread/libdvdnav,
// plus libdvdcss for encrypted discs). Sources are tagged with a prefix so the
// generic video pipeline can tell them apart from ordinary files.
#![allow(dead_code)]

pub const PREFIX: &str = "dvdvideo:";
pub const DVD_INPUT_FORMAT: &str = "dvdvideo";

pub fn is_dvd(path: &str) -> bool {
    path.starts_with(PREFIX)
}

pub fn wrap(path: &str) -> String {
    if is_dvd(path) {
        path.to_string()
    } else {
        format!("{}{}", PREFIX, path)
    }
}

pub fn source(path: &str) -> &str {
    path.strip_prefix(PREFIX).unwrap_or(path)
}

/// Human-readable label for queue/status text.
pub fn display_name(path: &str) -> String {
    if !is_dvd(path) {
        return crate::util::stem(path);
    }
    let src = source(path);
    if src.starts_with("/dev/") {
        format!("DVD ({})", src)
    } else {
        std::path::Path::new(src)
            .file_name()
            .map(|n| format!("DVD {}", n.to_string_lossy()))
            .unwrap_or_else(|| "DVD".to_string())
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
    out.dedup_by(|a, b| {
        std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok()
    });
    out
}

#[cfg(not(target_os = "linux"))]
pub fn drives() -> Vec<String> {
    Vec::new()
}

pub fn first_drive() -> Option<String> {
    drives().into_iter().next()
}

/// Eject the tray if the source is a device. ISO images have nothing to eject.
pub fn eject(path: &str) -> bool {
    if !is_dvd(path) {
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
