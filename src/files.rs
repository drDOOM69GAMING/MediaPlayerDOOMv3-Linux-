#![allow(unused_imports)]
#![allow(dead_code)]
// Audio file collection, tagging, transcoding, recording, art discovery.
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;
use std::sync::mpsc::Sender;
use lofty::prelude::*;
use crate::consts::NO_WINDOW;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
pub fn collect_audio(dir: &str) -> Vec<String> {
    fn walk(d: &Path, out: &mut Vec<String>) {
        let rd = match std::fs::read_dir(d) { Ok(rd) => rd, Err(_) => return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if is_audio(&p.to_string_lossy()) {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(Path::new(dir), &mut out);
    out.sort();
    out
}

pub fn collect_audio_report(dir: &str, tx: &Sender<Msg>) -> Vec<String> {
    fn walk(d: &Path, out: &mut Vec<String>, last: &mut std::time::Instant, tx: &Sender<Msg>) {
        let rd = match std::fs::read_dir(d) { Ok(rd) => rd, Err(_) => return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out, last, tx);
            } else if is_audio(&p.to_string_lossy()) {
                out.push(p.to_string_lossy().to_string());
            }
            if last.elapsed().as_millis() >= 300 {
                let _ = tx.send(Msg::ScanProgress { found: out.len() });
                *last = std::time::Instant::now();
            }
        }
    }
    let mut out = Vec::new();
    let mut last = std::time::Instant::now();
    walk(Path::new(dir), &mut out, &mut last, tx);
    out
}

pub fn transcode_with_ffmpeg(audio: &str, ffmpeg: &Path) -> Result<PathBuf, String> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    audio.hash(&mut h);
    let slug = format!("{:016x}", h.finish());
    let dir = work_dir().join("tc");
    let _ = std::fs::create_dir_all(&dir);
    let out = dir.join(format!("{}.wav", slug));
    if out.exists() && out.metadata().map(|m| m.len() > 4096).unwrap_or(false) {
        return Ok(out);
    }
    let ff = ffmpeg.to_string_lossy().to_string();
    let po = out.to_string_lossy().to_string();
    let st = std::process::Command::new(ff)
        .args(["-y", "-i", audio, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && out.exists() {
        Ok(out)
    } else {
        Err("ffmpeg could not convert this file".into())
    }
}

pub fn record_with_ffmpeg(audio: &str, ffmpeg: &Path, dest: &Path) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let po = dest.to_string_lossy().to_string();
    let _ = std::fs::remove_file(dest);
    let st = std::process::Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-y", "-i", audio, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && dest.exists() {
        Ok(dest.to_path_buf())
    } else {
        Err("ffmpeg could not record this file".into())
    }
}

pub fn record_flight_ffmpeg(url: &str, ffmpeg: &Path, dest: &Path, secs: u32) -> Result<PathBuf, String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let po = dest.to_string_lossy().to_string();
    let _ = std::fs::remove_file(dest);
    let secs = secs.to_string();
    let st = std::process::Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-y", "-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", "-t", &secs, &po])
        .creation_flags(NO_WINDOW)
        .status()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if st.success() && dest.exists() {
        Ok(dest.to_path_buf())
    } else {
        Err("ffmpeg could not record this stream".into())
    }
}

pub fn find_music_folder() -> Option<String> {
    let mut best: Option<(PathBuf, usize)> = None;
    let mut check = |d: &Path, prio: usize| {
        let files = collect_audio(&d.to_string_lossy());
        if !files.is_empty() {
            let score = files.len() + prio;
            if best.as_ref().map_or(true, |(_, s)| score > *s) {
                best = Some((d.to_path_buf(), score));
            }
        }
    };
    if let Ok(h) = std::env::var("USERPROFILE") {
        let home = PathBuf::from(&h);
        check(&home.join("Music"), 100);
        check(&home.join("OneDrive").join("Music"), 100);
    }
    for root in existing_drive_roots() {
        check(&root.join("Music"), 40);
    }
    best.map(|(p, _)| p.to_string_lossy().to_string())
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLogicalDrives() -> u32;
}

pub fn existing_drive_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        let mask = unsafe { GetLogicalDrives() };
        for bit in 0..26u32 {
            if mask & (1 << bit) != 0 {
                let ch = char::from_u32(b'A' as u32 + bit).unwrap_or('A');
                roots.push(PathBuf::from(format!("{}:\\", ch)));
            }
        }
    }
    #[cfg(not(windows))]
    {
        roots.push(PathBuf::from("/"));
    }
    roots
}

pub fn find_local_art(song_path: &str) -> Option<Vec<u8>> {
    let p = Path::new(song_path);
    if let Some(dir) = p.parent() {
        for name in ["cover.jpg", "cover.png", "folder.jpg", "album.jpg", "front.jpg", "default.jpg"] {
            let p2 = dir.join(name);
            if p2.is_file() {
                if let Ok(b) = std::fs::read(&p2) { return Some(b); }
            }
        }
        let mut imgs: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let ep = e.path();
                if let Some(ext) = ep.extension().and_then(|x| x.to_str()) {
                    if ["jpg", "jpeg", "png"].contains(&ext.to_lowercase().as_str()) {
                        imgs.push(ep);
                    }
                }
            }
        }
        imgs.sort();
        for ip in imgs {
            if let Ok(b) = std::fs::read(&ip) { return Some(b); }
        }
    }
    if let Some(probed) = load_tagged(song_path) {
        if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
            if let Some(pic) = tag.pictures().first() {
                return Some(pic.data().to_vec());
            }
        }
    }
    None
}

pub fn load_tagged(path: &str) -> Option<lofty::file::TaggedFile> {
    let probe = lofty::probe::Probe::open(path).ok()?;
    let probe = probe.guess_file_type().ok()?;
    probe.read().ok()
}

/// Normalize cover art for the local cache: decode, downscale to <=220px and
/// re-encode as JPEG so the whole-playlist cache stays small (~10-30 KB/file).
/// Returns None for huge/unreadable images (those are skipped, not cached).
pub fn normalize_art(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() > 4 * 1024 * 1024 {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let rgba = img.to_rgba8();
    let thumb = image::imageops::thumbnail(&rgba, 220, 220);
    let dimg = image::DynamicImage::ImageRgba8(thumb);
    let mut out = std::io::Cursor::new(Vec::new());
    dimg.write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    Some(out.into_inner())
}

