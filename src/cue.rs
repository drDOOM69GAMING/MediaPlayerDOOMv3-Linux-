// CUE sheet support for single-file albums.
//
// A CUE sheet can describe many tracks inside one APE/FLAC/WAV image. The
// library normally stores one path per track, so these albums used to appear as
// one very long song. We expand them into lightweight virtual paths:
//
//     cue:///music/Artist/Album/Album.ape#0002
//
// The prefix is intercepted by the player, tag reader, art lookup and
// converter; playback seeks to the track's start and stops at its end.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use lofty::prelude::*;

pub const PREFIX: &str = "cue://";

#[derive(Clone, Debug)]
pub struct CueTrack {
    pub number: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub start: f64,
    pub end: f64,
}

impl CueTrack {
    pub fn secs(&self) -> f32 {
        if self.end > self.start {
            (self.end - self.start) as f32
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct CueSheet {
    pub cue_path: String,
    pub audio_path: String,
    pub artist: String,
    pub album: String,
    pub tracks: Vec<CueTrack>,
}

pub fn is_cue(path: &str) -> bool {
    path.starts_with(PREFIX)
}

pub fn is_cue_file(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("cue"))
        .unwrap_or(false)
}

/// True when the path is a real file, or a CUE track whose image exists.
pub fn exists(path: &str) -> bool {
    if is_cue(path) {
        base_audio(path)
            .map(|b| Path::new(&b).is_file())
            .unwrap_or(false)
    } else {
        Path::new(path).is_file()
    }
}

pub fn make_path(cue_path: &str, number: u32) -> String {
    format!("{}{}#{:04}", PREFIX, cue_path, number)
}

pub fn split(path: &str) -> Option<(&str, u32)> {
    let rest = path.strip_prefix(PREFIX)?;
    let (cue, num) = rest.rsplit_once('#')?;
    let num = num.parse().ok()?;
    Some((cue, num))
}

fn cache() -> &'static Mutex<HashMap<String, CueSheet>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CueSheet>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn sheet(cue_path: &str) -> Option<CueSheet> {
    let lock = cache();
    if let Some(s) = lock
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(cue_path)
    {
        return Some(s.clone());
    }
    let parsed = parse_sheet(cue_path)?;
    lock.lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(cue_path.to_string(), parsed.clone());
    Some(parsed)
}

pub fn track_for(path: &str) -> Option<CueTrack> {
    let (cue, number) = split(path)?;
    let sheet = sheet(cue)?;
    sheet
        .tracks
        .iter()
        .find(|t| t.number == number)
        .cloned()
}

/// Physical audio file behind a virtual CUE track.
pub fn base_audio(path: &str) -> Option<String> {
    let (cue, _) = split(path)?;
    sheet(cue).map(|s| s.audio_path)
}

/// Folder that owns the CUE album (used for art and band grouping).
pub fn parent_dir(path: &str) -> Option<PathBuf> {
    let base = base_audio(path)?;
    Path::new(&base).parent().map(|p| p.to_path_buf())
}

/// `Artist - Title` for sorting/status text.
pub fn stem_title(path: &str) -> Option<String> {
    let t = track_for(path)?;
    let who = if !crate::util::is_placeholder_meta(&t.artist) {
        Some(t.artist.as_str())
    } else if !crate::util::is_placeholder_meta(&t.album) {
        Some(t.album.as_str())
    } else {
        None
    };
    Some(match who {
        Some(w) => format!("{} - {}", w, t.title),
        None => format!("{:02} - {}", t.number, t.title),
    })
}

/// `Album - NN Title` fallback row label before tags load.
pub fn display(path: &str) -> Option<String> {
    let t = track_for(path)?;
    let album = if crate::util::is_placeholder_meta(&t.album) {
        parent_dir(path)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "CUE".to_string())
    } else {
        t.album.clone()
    };
    Some(format!("{} - {:02} {}", album, t.number, t.title))
}

pub fn expand_one(cue_path: &str) -> Vec<String> {
    let Some(sheet) = sheet(cue_path) else {
        return Vec::new();
    };
    sheet
        .tracks
        .iter()
        .map(|t| make_path(cue_path, t.number))
        .collect()
}

/// Path used for filesystem operations (folders, tag pictures): the real audio
/// image for a virtual CUE track, otherwise the path itself.
pub fn disk_path(path: &str) -> String {
    base_audio(path).unwrap_or_else(|| path.to_string())
}

fn split_keyword(line: &str) -> (&str, &str) {
    match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim()),
        None => (line, ""),
    }
}

fn unquote(rest: &str) -> String {
    let s = rest.trim();
    if let Some(body) = s.strip_prefix('"') {
        if let Some(end) = body.find('"') {
            return body[..end].to_string();
        }
        return body.to_string();
    }
    // Unquoted values may still carry a trailing type (FILE x.ape WAVE).
    s.split_whitespace().next().unwrap_or("").to_string()
}

fn parse_time(text: &str) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    let num = |s: &str| s.trim().parse::<f64>().ok();
    match parts.as_slice() {
        [s] => num(s),
        [m, s] => Some(num(m)? * 60.0 + num(s)?),
        [m, s, f] => Some(num(m)? * 60.0 + num(s)? + num(f)? / 75.0),
        [h, m, s, f] => Some(num(h)? * 3600.0 + num(m)? * 60.0 + num(s)? + num(f)? / 75.0),
        _ => None,
    }
}

fn resolve_audio(cue_dir: &Path, raw: &str) -> Option<PathBuf> {
    let cleaned = raw.replace('\\', "/");
    let p = Path::new(&cleaned);
    let cand = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cue_dir.join(p)
    };
    if cand.is_file() {
        return Some(cand);
    }
    // Windows-created sheets sometimes disagree on case.
    let want = p.file_name()?.to_string_lossy().to_ascii_lowercase();
    for e in std::fs::read_dir(cue_dir).ok()?.flatten() {
        if e.file_name().to_string_lossy().to_ascii_lowercase() == want {
            return Some(e.path());
        }
    }
    None
}

fn audio_duration(path: &Path) -> f64 {
    let s = path.to_string_lossy();
    if let Some(probed) = crate::files::load_tagged(&s) {
        let d = probed.properties().duration().as_secs_f64();
        if d > 0.0 {
            return d;
        }
    }
    let d = crate::ffmpeg::probe_video_duration(&s);
    if d.is_finite() && d > 0.0 {
        d as f64
    } else {
        0.0
    }
}

fn parse_sheet(cue_path: &str) -> Option<CueSheet> {
    let bytes = std::fs::read(cue_path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    let cue_dir = Path::new(cue_path).parent()?.to_path_buf();
    let folder_album = cue_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut files: Vec<String> = Vec::new();
    let mut album_artist = String::new();
    let mut album_title = String::new();
    let mut tracks: Vec<CueTrack> = Vec::new();
    let mut current: Option<CueTrack> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.to_ascii_uppercase().starts_with("REM ") {
            continue;
        }
        let (kw, rest) = split_keyword(line);
        match kw.to_ascii_uppercase().as_str() {
            "FILE" => {
                let name = unquote(rest);
                if !name.is_empty() {
                    files.push(name);
                }
            }
            "TRACK" => {
                if let Some(t) = current.take() {
                    tracks.push(t);
                }
                let number = rest
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())?;
                current = Some(CueTrack {
                    number,
                    title: String::new(),
                    artist: album_artist.clone(),
                    album: album_title.clone(),
                    start: 0.0,
                    end: 0.0,
                });
            }
            "TITLE" => {
                let v = unquote(rest);
                if let Some(t) = current.as_mut() {
                    t.title = v;
                } else {
                    album_title = v;
                }
            }
            "PERFORMER" => {
                let v = unquote(rest);
                if let Some(t) = current.as_mut() {
                    t.artist = v;
                } else {
                    album_artist = v;
                }
            }
            "INDEX" => {
                let mut it = rest.split_whitespace();
                if it.next() == Some("01") {
                    if let (Some(t), Some(time)) = (current.as_mut(), it.next()) {
                        if let Some(secs) = parse_time(time) {
                            t.start = secs;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(t) = current.take() {
        tracks.push(t);
    }

    // Only single-image sheets are expandable. Multi-FILE sheets already have
    // one file per track and are handled as ordinary audio.
    if files.len() != 1 || tracks.len() < 2 {
        return None;
    }
    let audio_path = resolve_audio(&cue_dir, &files[0])?;
    if !audio_path.is_file() {
        return None;
    }
    let audio_str = audio_path.to_string_lossy().to_string();
    let duration = audio_duration(&audio_path);
    tracks.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for i in 0..tracks.len() {
        let end = if i + 1 < tracks.len() {
            tracks[i + 1].start
        } else {
            duration
        };
        tracks[i].end = if end > tracks[i].start { end } else { 0.0 };
        if tracks[i].title.trim().is_empty() {
            tracks[i].title = format!("Track {:02}", tracks[i].number);
        }
        if tracks[i].artist.trim().is_empty() {
            tracks[i].artist = album_artist.clone();
        }
        if tracks[i].album.trim().is_empty() {
            tracks[i].album = if album_title.trim().is_empty() {
                folder_album.clone()
            } else {
                album_title.clone()
            };
        }
    }
    Some(CueSheet {
        cue_path: cue_path.to_string(),
        audio_path: audio_str,
        artist: album_artist,
        album: if album_title.trim().is_empty() {
            folder_album
        } else {
            album_title
        },
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("mpdoom-cue-{}-{}-{}", std::process::id(), name, n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn parses_times_including_frames() {
        assert_eq!(parse_time("03:19:36"), Some(3.0 * 60.0 + 19.0 + 36.0 / 75.0));
        assert_eq!(parse_time("01:02:03:04"), Some(3600.0 + 2.0 * 60.0 + 3.0 + 4.0 / 75.0));
        assert_eq!(parse_time("12.5"), Some(12.5));
        assert_eq!(parse_time("bad"), None);
    }

    #[test]
    fn virtual_paths_round_trip() {
        let p = make_path("/music/Artist/Album/image.ape", 7);
        assert!(is_cue(&p));
        assert_eq!(split(&p), Some(("/music/Artist/Album/image.ape", 7)));
    }

    #[test]
    #[ignore]
    fn inspect_real_cue_from_env() {
        let Some(p) = std::env::var_os("MPDOOM_TEST_CUE") else {
            return;
        };
        let p = p.to_string_lossy().to_string();
        let s = sheet(&p).expect("cue should parse");
        println!(
            "cue={} audio={} artist={} album={} tracks={}",
            s.cue_path,
            s.audio_path,
            s.artist,
            s.album,
            s.tracks.len()
        );
        for t in &s.tracks {
            println!(
                "{:02}  {:>8.2} - {:>8.2}  {} / {}",
                t.number, t.start, t.end, t.artist, t.title
            );
        }
    }

    #[test]
    #[ignore]
    fn inspect_real_cue_track_from_env() {
        let Some(p) = std::env::var_os("MPDOOM_TEST_CUE_TRACK") else {
            return;
        };
        let p = p.to_string_lossy().to_string();
        println!("is_cue={} split={:?}", is_cue(&p), split(&p));
        println!("base={:?}", base_audio(&p));
        println!("track={:?}", track_for(&p));
    }

    #[test]
    fn virtual_paths_display_like_real_tracks() {
        let dir = scratch("display");
        let audio = dir.join("Album.ape");
        std::fs::write(&audio, b"not really ape").unwrap();
        let cue = dir.join("Album.cue");
        std::fs::write(
            &cue,
            "PERFORMER \"Test Artist\"\nTITLE \"Test Album\"\nFILE \"Album.ape\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"One\"\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    TITLE \"Two\"\n    INDEX 01 02:00:00\n",
        )
        .unwrap();
        let p = make_path(&cue.to_string_lossy(), 2);
        assert_eq!(crate::util::stem(&p), "Test Artist - Two");
        assert_eq!(crate::library::make_display(&p), "Test Album - 02 Two");
        assert_eq!(crate::library::playlist_display("Test Artist", "Two", &p), "Test Artist - Two");
        assert_eq!(crate::library::band_of(&p), "Test Artist");
        assert_eq!(
            crate::library::folder_artist_album(&p),
            Some(("Test Artist".to_string(), "Test Album".to_string()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expands_single_file_sheet() {
        let dir = scratch("expand");
        let audio = dir.join("Album.ape");
        std::fs::write(&audio, b"not really ape").unwrap();
        let cue = dir.join("Album.cue");
        std::fs::write(
            &cue,
            "PERFORMER \"Artist\"\nTITLE \"Album\"\nFILE \"Album.ape\" WAVE\n  TRACK 01 AUDIO\n    TITLE \"One\"\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    TITLE \"Two\"\n    INDEX 01 03:00:00\n  TRACK 03 AUDIO\n    TITLE \"Three\"\n    INDEX 01 05:30:00\n",
        )
        .unwrap();
        let s = sheet(&cue.to_string_lossy()).expect("sheet should parse");
        assert_eq!(s.tracks.len(), 3);
        assert_eq!(s.tracks[0].title, "One");
        assert_eq!(s.tracks[1].start, 180.0);
        assert_eq!(s.tracks[0].end, 180.0);
        assert_eq!(s.tracks[1].end, 330.0);
        let expanded = expand_one(&cue.to_string_lossy());
        assert_eq!(expanded.len(), 3);
        let t = track_for(&expanded[1]).unwrap();
        assert_eq!(t.title, "Two");
        assert_eq!(t.artist, "Artist");
        assert_eq!(t.album, "Album");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_multi_file_and_missing_image() {
        let dir = scratch("multi");
        let cue = dir.join("multi.cue");
        std::fs::write(
            &cue,
            "FILE \"a.flac\" WAVE\n TRACK 01 AUDIO\n TITLE \"A\"\n INDEX 01 00:00:00\nFILE \"b.flac\" WAVE\n TRACK 02 AUDIO\n TITLE \"B\"\n INDEX 01 00:00:00\n",
        )
        .unwrap();
        assert!(sheet(&cue.to_string_lossy()).is_none());
        std::fs::write(dir.join("gone.cue"), "FILE \"gone.ape\" WAVE\n TRACK 01 AUDIO\n INDEX 01 00:00:00\n TRACK 02 AUDIO\n INDEX 01 01:00:00\n").unwrap();
        assert!(sheet(&dir.join("gone.cue").to_string_lossy()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
