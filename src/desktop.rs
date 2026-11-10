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

/// MIME types registered for the default media player action. Keep this in
/// sync with packaging/mediaplayerofdoom.desktop and install.sh.
pub const MIME_TYPES: [&str; 25] = [
    "audio/mpeg",
    "audio/flac",
    "audio/x-flac",
    "audio/mp4",
    "audio/x-m4a",
    "audio/aac",
    "audio/ogg",
    "audio/opus",
    "audio/x-wav",
    "audio/wav",
    "audio/aiff",
    "audio/x-aiff",
    "audio/x-ms-wma",
    "audio/x-ape",
    "audio/x-wavpack",
    "audio/x-musepack",
    "audio/webm",
    "video/mp4",
    "video/x-matroska",
    "video/webm",
    "video/x-msvideo",
    "video/mpeg",
    "video/quicktime",
    "video/x-ms-wmv",
    "video/mp2t",
];

fn log(msg: &str) {
    use std::io::Write as _;
    let path = crate::util::data_dir().join("desktop.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

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
    let mimes = format!("{};", MIME_TYPES.join(";"));
    format!(
        "[Desktop Entry]\n\
Type=Application\n\
Name=MediaPlayerDOOM\n\
GenericName=Media Player\n\
Comment=Retro DOOM-styled music and video player\n\
Exec={exec} %F\n\
Icon={icon}\n\
Terminal=false\n\
Categories=AudioVideo;Audio;Video;Player;\n\
MimeType={mimes}\n\
Keywords=music;audio;video;player;doom;retro;mp3;flac;mkv;\n\
StartupNotify=true\n\
StartupWMClass=mediaplayerofdoom\n",
        exec = exec,
        icon = icon.display(),
        mimes = mimes
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
    let Some(home) = home_dir() else {
        log("skip: no HOME");
        return false;
    };
    let desktop_dir = xdg_user_dir("XDG_DESKTOP_DIR")
        .or_else(|| Some(home.join("Desktop")).filter(|p| p.is_dir()));
    log(&format!("desktop dir candidate: {:?}", desktop_dir));
    let Some(dir) = desktop_dir.filter(|d| d.is_dir()) else {
        log("skip: desktop dir missing");
        return false;
    };
    let shortcut = dir.join("MediaPlayerDOOM.desktop");
    if shortcut.exists() {
        // Already there: deliberately do nothing at all.
        log(&format!("ok: already exists at {}", shortcut.display()));
        return false;
    }

    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
    else {
        log("skip: could not resolve current exe");
        return false;
    };
    let icon = data_home().join("icons/hicolor/512x512/apps/mediaplayerofdoom.png");
    if !icon.is_file() {
        if let Some(parent) = icon.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&icon, ICON_PNG) {
            Ok(()) => log(&format!("wrote icon {}", icon.display())),
            Err(e) => log(&format!("icon write failed: {e}; using theme icon name")),
        }
    }
    // Even if the icon file could not be written, still create the shortcut.
    // Falling back to the theme name keeps the launcher useful.
    let icon_for_entry = if icon.is_file() { icon } else { PathBuf::from("mediaplayerofdoom") };
    let text = entry_text(&exe, &icon_for_entry);
    if !write_if_changed(&shortcut, &text) {
        log(&format!("shortcut write failed: {}", shortcut.display()));
        return false;
    }
    make_launchable(&shortcut);
    log(&format!("created shortcut {}", shortcut.display()));
    true
}

/// Register MediaPlayerDOOM as the default handler for its audio and video
/// types. This writes the applications menu launcher and then asks xdg-mime to
/// point every supported MIME type at it. No root access is needed.
pub fn set_default_player() -> Result<usize, String> {
    #[cfg(not(unix))]
    {
        return Err("setting the default player is supported on Linux only".into());
    }
    #[cfg(unix)]
    {
        let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.canonicalize().ok())
        else {
            return Err("could not resolve the application executable".into());
        };
        let icon = data_home().join("icons/hicolor/512x512/apps/mediaplayerofdoom.png");
        if !icon.is_file() {
            if let Some(parent) = icon.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&icon, ICON_PNG);
        }
        let icon_for_entry = if icon.is_file() {
            icon
        } else {
            PathBuf::from("mediaplayerofdoom")
        };
        let apps = data_home().join("applications");
        if let Err(e) = std::fs::create_dir_all(&apps) {
            return Err(format!("could not create applications folder: {}", e));
        }
        let entry = apps.join("mediaplayerofdoom.desktop");
        let text = entry_text(&exe, &icon_for_entry);
        if let Err(e) = std::fs::write(&entry, text) {
            return Err(format!("could not write desktop entry: {}", e));
        }
        make_launchable(&entry);
        let _ = Command::new("update-desktop-database")
            .arg(&apps)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let mut set = 0usize;
        for mime in MIME_TYPES {
            match Command::new("xdg-mime")
                .args(["default", "mediaplayerofdoom.desktop", mime])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(s) if s.success() => set += 1,
                Ok(_) => {}
                Err(e) => {
                    return Err(format!(
                        "xdg-mime is required to set the default player: {}",
                        e
                    ))
                }
            }
        }
        if set == 0 {
            return Err("no MIME associations were changed; xdg-mime may be unavailable".into());
        }
        log(&format!("set default player for {} MIME types", set));
        let _ = Command::new("kbuildsycoca6")
            .arg("--noincremental")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        Ok(set)
    }
}
