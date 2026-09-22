#![allow(unused_imports)]
#![allow(dead_code)]
// yt-dlp download/search helpers.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::io::BufReader;
use std::os::windows::process::CommandExt;
use crate::consts::NO_WINDOW;
use crate::network::percent_encode;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
pub fn parse_yt_progress(s: &str) -> Option<(f32, String, String)> {
    let t = s.trim();
    if !t.starts_with("[download]") || !t.contains('%') {
        return None;
    }
    let rest = t.trim_start_matches("[download]").trim();
    let pct: f32 = rest.split('%').next()?.trim().parse().ok()?;
    let speed = rest
        .split(" at ")
        .nth(1)
        .map(|x| x.split(" ETA").next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let eta = rest
        .split(" ETA ")
        .nth(1)
        .map(|x| x.trim().to_string())
        .unwrap_or_default();
    Some((pct, speed, eta))
}

pub fn is_yt_log_line(s: &str) -> bool {
    s.contains("% of") || s.contains("[download]") || s.contains("[ExtractAudio]")
        || s.starts_with("Destination:") || s.starts_with("Deleting original")
        || s.starts_with("[info]") || s.starts_with("[ffmpeg]") || s.starts_with("ERROR")
}
pub fn yt_lookup(yt: &Path, target: &str) -> Result<(String, String, String, String, String), String> {
    let out = std::process::Command::new(yt)
        .creation_flags(NO_WINDOW)
        .args(["--skip-download", "--no-warnings", "--newline", "--print", "%(id)s|%(title)s|%(artist)s|%(uploader)s|%(album)s", target])
        .output()
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;
    let first = String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").to_string();
    if first.is_empty() {
        return Err(format!("yt-dlp returned no metadata (exit {})", out.status.code().unwrap_or(-1)));
    }
    let mut it = first.split('|');
    Ok((
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
        it.next().unwrap_or("").to_string(),
    ))
}

pub fn yt_resolve_title(query: &str) -> String {
    let q = query.trim();
    if q.is_empty() || !ytdlp_path().is_file() {
        return query.to_string();
    }
    let target = if q.starts_with("http") { q.to_string() } else { format!("ytsearch1:{}", q) };
    match yt_lookup(&ytdlp_path(), &target) {
        Ok((_, title, _, _, _)) if !title.is_empty() => title,
        _ => query.to_string(),
    }
}

pub fn yt_grab(yt: &Path, ffmpeg: &Path, target: &str, id: &str, chunks: u32, tx: &Sender<Msg>) -> Result<PathBuf, String> {
    use std::io::BufRead;
    use std::process::Stdio;
    let tmp = work_dir().join("yt");
    let _ = std::fs::create_dir_all(&tmp);
    let tpl = tmp.join(format!("{}.%(ext)s", id));
    let _ = std::fs::remove_file(tmp.join(format!("{}.mp3", id)));
    let mut child = std::process::Command::new(yt)
        .creation_flags(NO_WINDOW)
        .args([
            "--newline",
            "--no-warnings",
            "--ffmpeg-location",
            ffmpeg.parent().and_then(|p| p.to_str()).unwrap_or(""),
            "-f", "bestaudio/best",
            "-x",
            "--audio-format", "mp3",
            "--audio-quality", "320k",
            "--embed-metadata",
            "--embed-thumbnail",
            "--convert-thumbnails", "jpg",
            "--parse-metadata", "%(uploader)s:%(artist)s",
            "-N", &chunks.to_string(),
            "-o", &tpl.to_string_lossy(),
            target,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;
    let mut last_err = String::new();
    if let Some(se) = child.stderr.take() {
        let mut reader = BufReader::new(se);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            let s = line.trim_end().to_string();
            if !s.is_empty() {
                last_err = s.clone();
                if is_yt_log_line(&s) {
                    let _ = tx.send(Msg::YtLog(s));
                }
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let final_mp3 = tmp.join(format!("{}.mp3", id));
    if !status.success() || !final_mp3.is_file() {
        let tail: Vec<&str> = last_err.lines().rev().take(4).collect();
        let err = tail.join("\n");
        return Err(if err.is_empty() { format!("yt-dlp exited {}", status.code().unwrap_or(-1)) } else { err });
    }
    Ok(final_mp3)
}

pub fn yt_download_and_store(query: &str, chunks: u32, tx: &Sender<Msg>) -> Result<PathBuf, String> {
    ensure_tools(tx)?;
    let q = query.trim();
    if q.is_empty() {
        return Err("Enter a YouTube URL or search term".into());
    }
    let target = if q.starts_with("http") { q.to_string() } else { format!("ytsearch1:{}", q) };
    let (id, title, artist, uploader, album) = yt_lookup(&ytdlp_path(), &target)?;
    let mut display = title.clone();
    if display.is_empty() {
        display = id.clone();
    }
    let _ = tx.send(Msg::YtStatus(format!("Downloading \"{}\"...", display)));
    let mp3 = yt_grab(&ytdlp_path(), &ffmpeg_path(), &target, &id, chunks, tx)?;
    let artist = if artist.is_empty() { uploader } else { artist };
    let artist = if artist.is_empty() { "Unknown".to_string() } else { artist };
    let folder = artist_album_folder(&music_folder(), &artist, &album);
    let num = next_track_num(&folder);
    let mut fname = format!("{:03} - {}.mp3", num, sanitize_win(&title));
    let mut final_path = folder.join(&fname);
    let mut extra = 0u32;
    while final_path.exists() {
        extra += 1;
        fname = format!("{:03}.{} - {}.mp3", num, extra, sanitize_win(&title));
        final_path = folder.join(&fname);
    }
    std::fs::rename(&mp3, &final_path).map_err(|e| format!("could not save to Music folder: {}", e))?;
    Ok(final_path)
}

pub fn artist_album_folder(root: &Path, artist: &str, album: &str) -> PathBuf {
    let album_d = if album.trim().is_empty() { "Singles".to_string() } else { album.to_string() };
    let want = format!("{} - {}", sanitize_win(artist), sanitize_win(&album_d));
    let want_l = want.to_lowercase();
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(n) = p.file_name().map(|f| f.to_string_lossy().to_lowercase()) {
                if n == want_l {
                    return p;
                }
            }
        }
    }
    let folder = root.join(&want);
    let _ = std::fs::create_dir_all(&folder);
    folder
}

