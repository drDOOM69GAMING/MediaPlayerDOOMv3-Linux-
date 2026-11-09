#![allow(unused_imports)]
#![allow(dead_code)]
// Audio file collection, tagging, transcoding, recording, art discovery.
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use lofty::prelude::*;
use crate::consts::NO_WINDOW;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
/// Directories that never contain music and can be huge (or recursive symlink
/// traps). Matching is case-insensitive.
fn skip_scan_dir(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    matches!(
        n.as_str(),
        "system volume information"
            | "$recycle.bin"
            | "recycler"
            | "lost+found"
            | "node_modules"
            | "__macosx"
            | ".git"
            | ".cache"
            | ".trash"
    ) || n.starts_with(".trash-")
}

fn walk_audio<F: FnMut(usize)>(
    dir: &Path,
    out: &mut Vec<String>,
    progress: &mut F,
    visited: &mut HashSet<PathBuf>,
) {
    // Symlinked music folders are common on Linux. Follow them, but canonical
    // paths are tracked so a link back to a parent cannot make this recurse.
    let canon = dir.canonicalize().ok();
    if let Some(c) = &canon {
        if !visited.insert(c.clone()) {
            return;
        }
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                if skip_scan_dir(&name) {
                    continue;
                }
            }
            walk_audio(&p, out, progress, visited);
        } else if ft.is_symlink() && p.is_dir() {
            if let Some(name) = p.file_name().map(|n| n.to_string_lossy().to_string()) {
                if !skip_scan_dir(&name) {
                    walk_audio(&p, out, progress, visited);
                }
            }
        } else if ft.is_file() || (ft.is_symlink() && p.is_file()) {
            let s = p.to_string_lossy().to_string();
            if is_audio(&s) {
                out.push(s);
                progress(out.len());
            }
        }
    }
}

pub fn collect_audio(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    walk_audio(Path::new(dir), &mut out, &mut |_| {}, &mut visited);
    out.sort();
    out
}

pub fn collect_audio_report(dir: &str, tx: &Sender<Msg>) -> Vec<String> {
    let mut out = Vec::new();
    let mut last = std::time::Instant::now();
    let mut progress = |found: usize| {
        if last.elapsed().as_millis() >= 300 {
            let _ = tx.send(Msg::ScanProgress { found });
            last = std::time::Instant::now();
        }
    };
    let mut visited = HashSet::new();
    walk_audio(Path::new(dir), &mut out, &mut progress, &mut visited);
    out.sort();
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
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i", audio, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .no_window()
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
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i", audio, "-vn", "-ac", "2", "-ar", "44100", "-f", "wav", &po])
        .no_window()
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
        .no_window()
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
    if let Some(home) = crate::util::home_dir() {
        check(&home.join("Music"), 100);
        check(&home.join("OneDrive").join("Music"), 100);
        if let Some(music) = crate::tools::music_dir() {
            check(&music, 120);
        }
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
        // Common Linux mount points. Only `Music` directly below each root is
        // checked, so this stays cheap even with several mounted drives.
        roots.push(PathBuf::from("/"));
        roots.push(PathBuf::from("/media"));
        roots.push(PathBuf::from("/mnt"));
        if let Some(home) = crate::util::home_dir() {
            let user = home.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let removable = PathBuf::from("/run/media").join(&user);
            if removable.is_dir() {
                roots.push(removable.clone());
                if let Ok(rd) = std::fs::read_dir(&removable) {
                    roots.extend(rd.flatten().map(|e| e.path()));
                }
            }
            roots.push(home);
        }
        for base in ["/media", "/mnt"] {
            if let Ok(rd) = std::fs::read_dir(base) {
                roots.extend(rd.flatten().map(|e| e.path()));
            }
        }
    }
    roots
}

pub fn find_local_art(song_path: &str) -> Option<Vec<u8>> {
    let p = Path::new(song_path);
    if let Some(dir) = p.parent() {
        if let Some(b) = find_dir_art(dir) {
            return Some(b);
        }
    }
    if let Some(probed) = load_tagged(song_path) {
        if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
            for pic in tag.pictures() {
                let data = pic.data();
                if art_bytes_ok(data, 16) {
                    return Some(data.to_vec());
                }
            }
        }
    }
    None
}

/// Rank image files inside one folder. Cover-ish names win over arbitrary
/// scans, and back/disc/booklet images come last so they never get chosen as
/// the album cover just because they sort first.
fn find_dir_art(dir: &Path) -> Option<Vec<u8>> {
    const EXTS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "jfif"];
    let mut candidates: Vec<(u8, String, PathBuf)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return None };
    for e in rd.flatten() {
        let ep = e.path();
        if !ep.is_file() {
            continue;
        }
        let Some(ext) = ep.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()) else { continue };
        if !EXTS.contains(&ext.as_str()) {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_ascii_lowercase();
        let base = name.split('.').next().unwrap_or("").trim().to_string();
        let prio = match base.as_str() {
            "cover" | "folder" | "front" => 0,
            "album" | "albumart" | "albumartlarge" | "albumartsmall" => 1,
            "artwork" | "default" | "art" => 2,
            "back" | "backcover" | "cd" | "disc" | "inside" | "booklet" | "scan" => 40,
            _ => 20,
        };
        candidates.push((prio, name, ep));
    }
    candidates.sort();
    for (_, _, path) in candidates {
        if let Ok(b) = std::fs::read(&path) {
            if art_bytes_ok(&b, 16) {
                return Some(b);
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


#[cfg(test)]
mod file_pipeline_tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch() -> PathBuf {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let d = std::env::temp_dir().join(format!("mpdoom-files-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_audio(dir: &Path) -> PathBuf {
        let out = dir.join("tone.mp3");
        let st = Command::new(ffmpeg_path())
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
                "-c:a", "libmp3lame",
            ])
            .arg(&out)
            .status()
            .expect("ffmpeg should start");
        assert!(st.success(), "could not create test mp3");
        out
    }

    #[test]
    fn collects_supported_audio_recursively() {
        let dir = scratch();
        let a = make_audio(&dir);
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        let b = sub.join("copy.flac");
        std::fs::copy(&a, &b).unwrap();
        std::fs::write(dir.join("notes.txt"), "not audio").unwrap();
        let found = collect_audio(&dir.to_string_lossy());
        assert_eq!(found.len(), 2, "found: {found:?}");
        assert!(found.iter().any(|p| p.ends_with("tone.mp3")));
        assert!(found.iter().any(|p| p.ends_with("copy.flac")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_skips_junk_and_symlink_loops() {
        let dir = scratch();
        let a = make_audio(&dir);
        let junk = dir.join("System Volume Information");
        std::fs::create_dir_all(&junk).unwrap();
        std::fs::copy(&a, junk.join("fake.mp3")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        }
        let extra = dir.join("linked-album");
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::copy(&a, extra.join("linked.mp3")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&extra, dir.join("music-link")).unwrap();
        }
        let found = collect_audio(&dir.to_string_lossy());
        // The real folder and a symlink to it are the same physical directory,
        // so its tracks are included exactly once.
        assert_eq!(found.len(), 2, "found: {found:?}");
        assert!(found.iter().any(|p| p.ends_with("tone.mp3")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn transcodes_and_records_with_ffmpeg() {
        let dir = scratch();
        let a = make_audio(&dir);
        let tc = transcode_with_ffmpeg(&a.to_string_lossy(), &ffmpeg_path()).expect("transcode");
        assert!(tc.is_file());
        assert!(std::fs::metadata(&tc).unwrap().len() > 4096);
        let dest = dir.join("rec.wav");
        let rec = record_with_ffmpeg(&a.to_string_lossy(), &ffmpeg_path(), &dest).expect("record");
        assert!(rec.is_file());
        assert!(std::fs::metadata(&rec).unwrap().len() > 4096);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_tags_and_normalizes_art() {
        let dir = scratch();
        let a = make_audio(&dir);
        // No tags were written, but lofty must still be able to open the file.
        let tagged = load_tagged(&a.to_string_lossy());
        assert!(tagged.is_some(), "lofty could not open the generated mp3");
        let img = image::RgbaImage::from_pixel(400, 400, image::Rgba([10, 200, 30, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let small = normalize_art(&png.into_inner()).expect("normalize art");
        let decoded = image::load_from_memory(&small).expect("cached jpeg should decode");
        assert!(decoded.width() <= 220 && decoded.height() <= 220);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
