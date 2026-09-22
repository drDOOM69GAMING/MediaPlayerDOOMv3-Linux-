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
use crate::util::data_dir;
pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

pub fn tools_dir() -> PathBuf {
    let d = data_dir().join("tools");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn ytdlp_path() -> PathBuf {
    tools_dir().join("yt-dlp.exe")
}

pub fn ffmpeg_path() -> PathBuf {
    tools_dir().join("ffmpeg.exe")
}

pub fn mkvmerge_path() -> PathBuf {
    tools_dir().join("mkvmerge.exe")
}

pub fn music_folder() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| exe_dir());
    let p = base.join("Music");
    if !p.is_dir() {
        let _ = std::fs::create_dir_all(&p);
    }
    p
}

pub fn download_tool(url: &str, dst: &Path, label: &str, tx: &Sender<Msg>) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
    let _ = tx.send(Msg::YtStatus(format!("Downloading {}...", label)));
    let mut resp = client.get(url).send().map_err(|e| format!("{} download failed: {}", label, e))?;
    let mut out = File::create(dst).map_err(|e| e.to_string())?;
    std::io::copy(&mut resp, &mut out).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_bin(dst: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    std::fs::write(dst, bytes).map_err(|e| format!("could not extract {}: {}", label, e))
}

pub fn ensure_tools(tx: &Sender<Msg>) -> Result<(), String> {
    let _ = std::fs::create_dir_all(tools_dir());
    let ytdlp = ytdlp_path();
    if !ytdlp.is_file() {
        match EMBEDDED_YTDLP {
            Some(b) => write_bin(&ytdlp, b, "yt-dlp")?,
            None => download_tool(
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
                &ytdlp,
                "yt-dlp",
                tx,
            )?,
        }
    }
    let ff = ffmpeg_path();
    let fp = tools_dir().join("ffprobe.exe");
    if !ff.is_file() {
        let _ = tx.send(Msg::YtStatus("Extracting ffmpeg...".into()));
        match EMBEDDED_FFMPEG {
            Some(b) => write_bin(&ff, b, "ffmpeg")?,
            None => {
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
                if !ff.is_file() {
                    return Err("ffmpeg extraction produced no ffmpeg.exe".into());
                }
            }
        }
    }
    if !fp.is_file() {
        match EMBEDDED_FFPROBE {
            Some(b) => write_bin(&fp, b, "ffprobe")?,
            None => {
                if !ff.is_file() {
                    return Err("ffprobe missing and not embedded; run online once".into());
                }
                let _ = tx.send(Msg::YtStatus("Extracting ffprobe...".into()));
                let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
                let resp = client.get("https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip").send().map_err(|e| format!("ffprobe download failed: {}", e))?;
                let bytes = resp.bytes().map_err(|e| e.to_string())?;
                let mut cur = std::io::Cursor::new(bytes);
                let mut za = zip::ZipArchive::new(&mut cur).map_err(|e| format!("bad ffmpeg archive: {}", e))?;
                for i in 0..za.len() {
                    let mut f = za.by_index(i).map_err(|e| e.to_string())?;
                    let name = f.name().replace('\\', "/");
                    let fname = name.rsplit('/').next().unwrap_or("").to_string();
                    if fname == "ffprobe.exe" {
                        let target = tools_dir().join(&fname);
                        let mut out = File::create(&target).map_err(|e| e.to_string())?;
                        std::io::copy(&mut f, &mut out).map_err(|e| e.to_string())?;
                    }
                }
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

