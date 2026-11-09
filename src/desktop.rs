// Self-installing desktop integration.
//
// Linux file managers cannot show an icon for a bare ELF binary, so the release
// build keeps an icon-bearing launcher in the application menu and on the
// desktop. It is rewritten when the executable or icon changes, making it
// survive updates, a deleted shortcut, or moving the binary.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use crate::consts::APP_NAME;
use crate::tools::xdg_user_dir;
use crate::util::{data_home, home_dir};

pub const ICON_PNG: &[u8] = include_bytes!("../assets/mediaplayerofdoom.png");

fn quote_exec(path: &Path) -> String {
    let s = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{}\"", s)
}

fn entry_text(exe: &Path, icon: &Path) -> String {
    let exec = quote_exec(exe);
    format!(
        "[Desktop Entry]\n\
Type=Application\n\
Name=MediaPlayerDOOM\n\
GenericName=Media Player\n\
Comment=Retro DOOM-styled music and video player\n\
Exec={exec} %F\n\
TryExec={exec}\n\
Icon={icon}\n\
Terminal=false\n\
Categories=AudioVideo;Audio;Video;Player;\n\
MimeType=audio/mpeg;audio/flac;audio/x-flac;audio/x-wav;audio/ogg;audio/opus;audio/mp4;audio/x-m4a;video/mp4;video/x-matroska;video/x-msvideo;video/webm;\n\
Keywords=music;audio;video;player;doom;retro;mp3;flac;mkv;\n\
StartupNotify=true\n\
StartupWMClass=mediaplayerofdoom\n",
        exec = exec,
        icon = icon.display()
    )
}

fn write_if_changed(path: &Path, text: &str) -> bool {
    if std::fs::read_to_string(path).ok().as_deref() == Some(text) {
        return false;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, text).is_ok()
}

#[cfg(unix)]
fn make_launchable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut p = meta.permissions();
        p.set_mode(p.mode() | 0o755);
        let _ = std::fs::set_permissions(path, p);
    }
    let _ = Command::new("gio")
        .args(["set"])
        .arg(path)
        .args(["metadata::trusted", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(unix))]
fn make_launchable(_path: &Path) {}

fn refresh_caches(icon_theme: &Path) {
    // Fire and forget: never delay startup for a cache rebuild.
    let _ = Command::new("kbuildsycoca6")
        .arg("--noincremental")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(icon_theme)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Ensure the desktop shortcut exists, and nothing more.
///
/// Called when the player opens. If `Desktop/MediaPlayerDOOM.desktop` is
/// already present this returns immediately and touches nothing (no rewrite,
/// no other launchers, no cache churn). Only when it is missing do we create
/// the icon (if needed) and the shortcut.
pub fn ensure_desktop_shortcut() -> bool {
    // Cargo's debug binary is a development artifact; don't advertise it.
    if cfg!(debug_assertions) {
        return false;
    }
    let Some(home) = home_dir() else { return false };
    let desktop_dir = xdg_user_dir("XDG_DESKTOP_DIR")
        .or_else(|| Some(home.join("Desktop")).filter(|p| p.is_dir()));
    let Some(dir) = desktop_dir.filter(|d| d.is_dir()) else { return false };
    let shortcut = dir.join("MediaPlayerDOOM.desktop");
    if shortcut.exists() {
        // Already there: deliberately do nothing at all.
        return false;
    }

    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
    else {
        return false;
    };
    let icon = data_home().join("icons/hicolor/512x512/apps/mediaplayerofdoom.png");
    if !icon.is_file() {
        if let Some(parent) = icon.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&icon, ICON_PNG).is_err() {
            return false;
        }
    }
    let text = entry_text(&exe, &icon);
    if !write_if_changed(&shortcut, &text) {
        return false;
    }
    make_launchable(&shortcut);
    true
}
