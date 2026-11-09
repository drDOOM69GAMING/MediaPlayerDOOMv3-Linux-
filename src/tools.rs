#![allow(unused_imports)]
#![allow(dead_code)]
// Locating/downloading bundled external tools (ffmpeg, ffprobe, yt-dlp, mkvmerge).
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;
use crate::{EMBEDDED_FFMPEG, EMBEDDED_FFPROBE, EMBEDDED_MKVMERGE, EMBEDDED_YTDLP};
use crate::types::Msg;
use crate::util::{data_dir, home_dir};

/// Executable suffix for helper binaries (`.exe` on Windows, empty elsewhere).
pub const EXE_SUFFIX: &str = if cfg!(windows) { ".exe" } else { "" };

pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

pub fn tools_dir() -> PathBuf {
    let d = data_dir().join("tools");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Look a tool up on `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Resolve a helper binary: prefer the app-managed copy in the data dir,
/// then fall back to a copy installed system-wide (`PATH`). This is what lets
/// the Linux build use the distro's ffmpeg/mkvmerge instead of downloading
/// Windows executables.
fn tool_path(base: &str) -> PathBuf {
    let local = tools_dir().join(format!("{}{}", base, EXE_SUFFIX));
    if local.is_file() {
        return local;
    }
    which(&format!("{}{}", base, EXE_SUFFIX))
        .or_else(|| which(base))
        .unwrap_or(local)
}

pub fn ytdlp_path() -> PathBuf {
    tool_path("yt-dlp")
}

pub fn ffmpeg_path() -> PathBuf {
    tool_path("ffmpeg")
}

pub fn ffprobe_path() -> PathBuf {
    tool_path("ffprobe")
}

pub fn mkvmerge_path() -> PathBuf {
    tool_path("mkvmerge")
}

/// Folder where REC saves WAV files.
///
/// Windows keeps the historical `<exe dir>/Recordings`; Linux writes under the
/// user's Music folder, because the install directory is usually read-only
/// (`/usr/bin`) and must never be written to by the app.
pub fn recordings_dir() -> PathBuf {
    #[cfg(windows)]
    let d = exe_dir().join("Recordings");
    #[cfg(not(windows))]
    let d = music_folder().join("Recordings");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Default folder for downloaded music. Uses the XDG music dir on Linux and
/// `%USERPROFILE%\Music` on Windows.
pub fn music_folder() -> PathBuf {
    let p = music_dir().unwrap_or_else(|| {
        home_dir().unwrap_or_else(exe_dir).join("Music")
    });
    if !p.is_dir() {
        let _ = std::fs::create_dir_all(&p);
    }
    p
}

/// The user's music directory, honouring `user-dirs.dirs` on Linux.
pub fn music_dir() -> Option<PathBuf> {
    #[cfg(not(windows))]
    if let Some(d) = xdg_user_dir("XDG_MUSIC_DIR") {
        return Some(d);
    }
    home_dir().map(|h| h.join("Music"))
}

/// Read an XDG user directory (e.g. `XDG_MUSIC_DIR` from user-dirs.dirs).
#[cfg(not(windows))]
pub fn xdg_user_dir(key: &str) -> Option<PathBuf> {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".config")))?;
    let text = std::fs::read_to_string(cfg.join("user-dirs.dirs")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(key) else { continue };
        let Some(value) = rest.trim().strip_prefix('=') else { continue };
        let value = value.trim().trim_matches('"');
        if let Some(stripped) = value.strip_prefix("$HOME/").or_else(|| value.strip_prefix("~/")) {
            return home_dir().map(|h| h.join(stripped));
        }
        if value.starts_with('/') {
            return Some(PathBuf::from(value));
        }
    }
    None
}

pub fn download_tool(url: &str, dst: &Path, label: &str, tx: &Sender<Msg>) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
    let _ = tx.send(Msg::YtStatus(format!("Downloading {}...", label)));
    let mut resp = client.get(url).send().map_err(|e| format!("{} download failed: {}", label, e))?;
    if !resp.status().is_success() {
        return Err(format!("{} download failed: HTTP {}", label, resp.status()));
    }
    let mut out = File::create(dst).map_err(|e| e.to_string())?;
    std::io::copy(&mut resp, &mut out).map_err(|e| e.to_string())?;
    make_executable(dst);
    Ok(())
}

pub fn write_bin(dst: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    std::fs::write(dst, bytes).map_err(|e| format!("could not extract {}: {}", label, e))?;
    make_executable(dst);
    Ok(())
}

/// Helper binaries must be executable on Unix; harmless no-op on Windows.
fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(perms.mode() | 0o755);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn ytdlp_download_url() -> &'static str {
    #[cfg(windows)]
    { "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" }
    #[cfg(all(not(windows), target_arch = "x86_64"))]
    { "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux" }
    #[cfg(all(not(windows), target_arch = "aarch64"))]
    { "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64" }
    #[cfg(all(not(windows), not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
    { "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp" }
}

pub fn ensure_tools(tx: &Sender<Msg>) -> Result<(), String> {
    let _ = std::fs::create_dir_all(tools_dir());

    // yt-dlp: embedded (Windows release build), already installed locally, on
    // PATH, or downloaded on demand.
    if !ytdlp_path().is_file() {
        match EMBEDDED_YTDLP {
            Some(b) => write_bin(&ytdlp_path(), b, "yt-dlp")?,
            None => download_tool(ytdlp_download_url(), &ytdlp_path(), "yt-dlp", tx)?,
        }
    }

    // ffmpeg / ffprobe: embedded on Windows release builds; on Linux the
    // distro packages are used when present, otherwise download is skipped and
    // only features that need them report a clear error.
    let ff = ffmpeg_path();
    if !ff.is_file() {
        match EMBEDDED_FFMPEG {
            Some(b) => write_bin(&ff, b, "ffmpeg")?,
            None => {
                #[cfg(windows)]
                download_ffmpeg_windows(tx)?;
                #[cfg(not(windows))]
                let _ = tx.send(Msg::YtStatus(
                    "ffmpeg not found - install it (sudo pacman -S ffmpeg) for radio, video and recording".into(),
                ));
            }
        }
    }
    let fp = ffprobe_path();
    if !fp.is_file() {
        match EMBEDDED_FFPROBE {
            Some(b) => write_bin(&fp, b, "ffprobe")?,
            None => {
                #[cfg(windows)]
                if ff.is_file() && ff.parent() == Some(tools_dir().as_path()) {
                    download_ffmpeg_windows(tx)?;
                }
                #[cfg(not(windows))]
                let _ = ();
            }
        }
    }
    let mm = mkvmerge_path();
    if !mm.is_file() {
        match EMBEDDED_MKVMERGE {
            Some(b) => write_bin(&mm, b, "mkvmerge")?,
            None => { /* mkvmerge is optional; cut feature unavailable */ }
        }
    }
    Ok(())
}

/// Windows fallback: fetch the gyan.dev essentials zip and pull the two exes.
#[cfg(windows)]
fn download_ffmpeg_windows(tx: &Sender<Msg>) -> Result<(), String> {
    use std::io::Write as _;
    let _ = tx.send(Msg::YtStatus("Downloading ffmpeg...".into()));
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
    let resp = client.get("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip").send().map_err(|e| format!("ffmpeg download failed: {}", e))?;
    let bytes = resp.bytes().map_err(|e| e.to_string())?;
    let _ = tx.send(Msg::YtStatus("Extracting ffmpeg...".into()));
    let mut cur = std::io::Cursor::new(bytes);
    let mut za = zip::ZipArchive::new(&mut cur).map_err(|e| format!("bad ffmpeg archive: {}", e))?;
    for i in 0..za.len() {
        let mut f = za.by_index(i).map_err(|e| e.to_string())?;
        let name = f.name().replace('\\', "/");
        let fname = name.rsplit('/').next().unwrap_or("").to_string();
        if fname == "ffmpeg.exe" || fname == "ffprobe.exe" {
            let target = tools_dir().join(&fname);
            let mut out = File::create(&target).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
        }
    }
    if !ffmpeg_path().is_file() {
        return Err("ffmpeg extraction produced no ffmpeg.exe".into());
    }
    Ok(())
}
