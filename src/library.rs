#![allow(unused_imports)]
#![allow(dead_code)]
// Library helpers: bands, sorting, tag reading, weighted picks.
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use lofty::prelude::*;
use rodio::{Decoder, Source};
use crate::files::*;
use crate::util::*;
pub fn weighted_pick(pl: &[String], weights: &HashMap<String, f32>) -> String {
    let mut r = rand::random::<f64>();
    let total: f64 = pl.iter().map(|s| weights.get(s).copied().unwrap_or(1.0) as f64).sum();
    if total <= 0.0 { r = 0.0; } else { r *= total; }
    for s in pl {
        let w = weights.get(s).copied().unwrap_or(1.0) as f64;
        if r < w {
            return s.clone();
        }
        r -= w;
    }
    pl[0].clone()
}

pub fn random_next(current: Option<&str>, pl: &[String], weights: &HashMap<String, f32>) -> String {
    let s = weighted_pick(pl, weights);
    if pl.len() > 1 && current == Some(s.as_str()) {
        if let Some(i) = pl.iter().position(|p| *p == s) {
            return pl[(i + 1) % pl.len()].clone();
        }
    }
    s
}

thread_local! {
    /// Set while a `Decoder::new` may legitimately hit rodio's symphonia
    /// init-seek `unreachable!` panic (M4A/AAC-in-MP4 bug, rodio#846). The
    /// panic hook checks this and stays quiet; `open_decoder` logs a clean
    /// decode-skip line instead.
    pub static EXPECTING_DECODE_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn open_decoder(path: &str) -> Option<Decoder<BufReader<File>>> {
    let f = File::open(path).ok()?;
    // Tell the kernel this is a forward, streaming read. On spinning disks it
    // noticeably reduces seek stalls while decoding long tracks.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_SEQUENTIAL);
        }
    }
    // 8 KiB (BufReader's default) is tiny for FLAC/MP3 on a HDD.
    let reader = BufReader::with_capacity(256 * 1024, f);
    EXPECTING_DECODE_PANIC.with(|fl| fl.set(true));
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Decoder::new(reader)));
    EXPECTING_DECODE_PANIC.with(|fl| fl.set(false));
    match res {
        Ok(Ok(d)) => Some(d),
        Ok(Err(_)) => None,
        Err(_) => {
            // rodio panicked inside Decoder::new (symphonia init seek). The panic
            // hook is muted for this; record which file so it can be fixed/removed.
            log_decode_skip(path);
            None
        }
    }
}

fn log_decode_skip(path: &str) {
    use std::io::Write as _;
    let log = data_dir().join("crash.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        let _ = writeln!(f, "\n===== decode-skip =====");
        let _ = writeln!(f, "{}", path);
        let _ = writeln!(f, "file could not be opened by the built-in decoder - use ffmpeg or replace the file");
    }
}

/// Stable per-song key for the on-disk art cache (%APPDATA%\..\artcache).
pub fn art_cache_path(path: &str) -> PathBuf {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    artcache_dir().join(format!("{:016x}.art", h.finish()))
}

pub fn read_art_cache(path: &str) -> Option<Vec<u8>> {
    let cache = art_cache_path(path);
    let bytes = std::fs::read(&cache).ok()?;
    if art_bytes_ok(&bytes, 32) {
        Some(bytes)
    } else {
        // Interrupted writes and old bad downloads used to stay forever, so a
        // track showed a blank/black square and never looked for art again.
        // Drop the bad file and report it as missing so it gets re-fetched.
        let _ = std::fs::remove_file(&cache);
        None
    }
}

/// True when a usable cached cover exists. Invalid files are removed so the
/// caller can treat the track as needing art again.
pub fn art_cache_ok(path: &str) -> bool {
    read_art_cache(path).is_some()
}

pub fn write_art_cache(path: &str, bytes: &[u8]) -> bool {
    let _ = std::fs::create_dir_all(artcache_dir());
    if !art_bytes_ok(bytes, 32) {
        return false;
    }
    let dst = art_cache_path(path);
    // Write beside the destination and rename, so a crash or a second worker
    // can never leave a half-written `.art` file behind.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dst.with_extension(format!("tmp-{}-{}", std::process::id(), stamp));
    if std::fs::write(&tmp, bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    let ok = crate::util::move_file(&tmp, &dst).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    ok
}

pub fn make_display(path: &str) -> String {
    if let Some((_, track, _)) = crate::cd::cdda_parts(path) {
        return format!("Track {:02}", track);
    }
    if let Some(d) = crate::cue::display(path) {
        return d;
    }
    let folder = Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| "?".to_string());
    format!("{} - {}", folder, beautify_name(&stem(path)))
}

/// Playlist row label from real tags, falling back to the filename/folder form.
pub fn playlist_display(artist: &str, title: &str, path: &str) -> String {
    let a = artist.trim();
    let t = title.trim();
    if !t.is_empty() {
        if !a.is_empty() {
            format!("{} - {}", a, t)
        } else {
            t.to_string()
        }
    } else if !a.is_empty() {
        format!("{} - {}", a, beautify_name(&stem(path)))
    } else {
        make_display(path)
    }
}

pub fn song_sort_key(path: &str) -> (String, String) {
    let disk = crate::cue::disk_path(path);
    let s = stem(path);
    if let Some(pos) = s.find(" - ") {
        let after = &s[pos + 3..];
        let (title, artist) = if let Some(open) = after.rfind('(') {
            if after.ends_with(')') {
                (&after[..open], after[open + 1..after.len() - 1].trim())
            } else {
                (after, "")
            }
        } else {
            (after, "")
        };
        let artist = if !artist.is_empty() {
            artist.to_string()
        } else {
            Path::new(&disk)
                .parent()
                .and_then(|p| p.file_name())
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        (
            artist.to_lowercase(),
            title.trim().to_lowercase(),
        )
    } else {
        let folder = Path::new(&disk)
            .parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        (folder.to_lowercase(), s.to_lowercase())
    }
}

pub fn band_of(path: &str) -> String {
    if let Some(t) = crate::cue::track_for(path) {
        if !crate::util::is_placeholder_meta(&t.artist) {
            return t.artist;
        }
    }
    let disk = crate::cue::disk_path(path);
    let parent = Path::new(&disk)
        .parent()
        .and_then(|p| p.file_name())
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    if parent.is_empty() {
        return parent;
    }
    if let Some(open) = parent.rfind('(') {
        if parent.ends_with(')') {
            let inner = parent[open + 1..parent.len() - 1].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    if let Some(pos) = parent.find(" - ") {
        return parent[..pos].trim().to_string();
    }
    parent
}

pub fn band_of_file(path: &str) -> String {
    let s = stem(path);
    if s.is_empty() {
        return band_of(path);
    }
    if let Some(pos) = s.find(" - ") {
        let after = &s[pos + 3..];
        if let Some(open) = after.rfind('(') {
            if after.ends_with(')') {
                let artist = after[open + 1..after.len() - 1].trim();
                if !artist.is_empty() {
                    return artist.to_string();
                }
            }
        }
        return s[..pos].trim().to_string();
    }
    band_of(path)
}

pub fn folder_artist_album(path: &str) -> Option<(String, String)> {
    if let Some(t) = crate::cue::track_for(path) {
        if !crate::util::is_placeholder_meta(&t.artist) {
            return Some((t.artist, t.album));
        }
    }
    let disk = crate::cue::disk_path(path);
    let mut dir = Path::new(&disk).parent()?;
    let mut name = dir.file_name()?.to_string_lossy().trim().to_string();
    if name.is_empty() {
        return None;
    }
    // Multi-disc rips put tracks in "CD 1"/"Disc 2"; the album is one level up.
    if is_disc_dir(&name) {
        dir = dir.parent()?;
        name = dir.file_name()?.to_string_lossy().trim().to_string();
        if name.is_empty() {
            return None;
        }
    }
    if let Some(open) = name.rfind('(') {
        if name.ends_with(')') {
            name = name[..open].trim().to_string();
        }
    }
    if let Some(pos) = name.find(" - ") {
        let artist = name[..pos].trim().to_string();
        let album = name[pos + 3..].trim().to_string();
        if !artist.is_empty() {
            return Some((artist, album));
        }
    }
    // Standard Artist/Album/track layout: use the grandparent as the artist.
    // This is what lets untagged libraries still get artwork and lyrics.
    if let Some(artist) = dir.parent().and_then(|p| p.file_name()) {
        let artist = artist.to_string_lossy().trim().to_string();
        if !artist.is_empty() && !is_generic_music_dir(&artist) {
            return Some((artist, name));
        }
    }
    Some((name, String::new()))
}

fn is_disc_dir(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    let rest = n
        .strip_prefix("cd")
        .or_else(|| n.strip_prefix("disc"))
        .or_else(|| n.strip_prefix("disk"));
    match rest {
        Some(r) => {
            let r = r.trim();
            !r.is_empty() && r.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

fn is_generic_music_dir(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "music" | "music library" | "downloads" | "audio" | "media" | "songs" | "youtube" | "recordings"
    )
}

/// Fill missing/Unknown artist and album values from the folder layout.
///
/// When the folder gives a complete pair, it replaces both values: old cache
/// entries sometimes stored the album name as the artist (from a simpler
/// fallback), and keeping that would send artwork lookups to the wrong name.
pub fn merge_folder_meta(artist: &mut String, album: &mut String, path: &str) {
    let missing = crate::util::is_placeholder_meta;
    if !missing(artist) && !missing(album) {
        return;
    }
    let Some((a, al)) = folder_artist_album(path) else { return };
    if !a.is_empty() && !al.is_empty() {
        *artist = a;
        *album = al;
    } else {
        if missing(artist) && !a.is_empty() {
            *artist = a;
        }
        if missing(album) && !al.is_empty() {
            *album = al;
        }
    }
}

pub(crate) fn looks_like_track_number(s: &str) -> bool {
    let t = s.trim().to_ascii_lowercase();
    let t = t.strip_prefix("track").unwrap_or(&t).trim();
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-' || c == '_' || c == ' ')
}

/// Title derived from the filename when the file has no usable title tag.
pub fn filename_title(path: &str, artist: &str) -> String {
    let raw = stem(path).replace('_', " ");
    let stripped = strip_track_no(&raw).unwrap_or(raw);
    let cleaned = beautify_name(&stripped);
    if !artist.trim().is_empty() {
        if let Some((left, right)) = cleaned.split_once(" - ") {
            if left.trim().eq_ignore_ascii_case(artist.trim()) {
                let r = right.trim();
                if !r.is_empty() {
                    return r.to_string();
                }
            }
        }
    }
    cleaned.trim().to_string()
}

/// Fill missing/placeholder title, artist and album from the filename and the
/// folder layout. Valid tags are kept; a tag that is only a track number is
/// replaced. This keeps the library labels and the online art/rating lookups
/// consistent with the actual file instead of showing "Unknown".
pub fn repaired_meta(path: &str, title: &str, artist: &str, album: &str) -> (String, String, String) {
    let mut a = artist.trim().to_string();
    let mut al = album.trim().to_string();
    merge_folder_meta(&mut a, &mut al, path);
    let mut t = title.trim().to_string();
    if let Some(stripped) = strip_track_no(&t) {
        if !stripped.is_empty() {
            t = stripped;
        }
    }
    if t.is_empty() || is_placeholder_meta(&t) || looks_like_track_number(&t) {
        t = filename_title(path, &a);
    }
    if t.trim().is_empty() {
        t = beautify_name(&stem(path));
    }
    if a.is_empty() || is_placeholder_meta(&a) {
        a = "Unknown".to_string();
    }
    if al.is_empty() || is_placeholder_meta(&al) {
        al = "Unknown".to_string();
    }
    (t, a, al)
}

pub fn strip_track_no(title: &str) -> Option<String> {
    if let Some(sep) = title.find(" - ") {
        let head = title[..sep].trim();
        let digits = !head.is_empty() && head.chars().all(|c| c.is_ascii_digit() || c == '.');
        if digits {
            return Some(title[sep + 3..].trim().to_string());
        }
    }
    None
}

pub fn load_bands(path: &std::path::Path) -> HashMap<String, Vec<String>> {
    if let Ok(text) = std::fs::read_to_string(path) {
        if let Ok(map) = serde_json::from_str::<HashMap<String, Vec<String>>>(&text) {
            return map;
        }
    }
    HashMap::new()
}

// A/V debugging log for developers.
// When true, every av_log() call appends a line to %APPDATA%\MediaPlayerOFDOOM\avsync.log.
// Useful for tracking intro/credits skip, seek, and audio-clock behavior.
// Set to true to re-enable diagnostics; keep false for release builds (av_log is a no-op).

#[cfg(test)]
mod folder_tests {
    use super::*;

    fn p(parts: &[&str]) -> String {
        let mut b = std::path::PathBuf::new();
        for part in parts {
            b.push(part);
        }
        b.to_string_lossy().to_string()
    }

    #[test]
    fn repairs_metadata_from_filename_and_folder() {
        let path = p(&["Music", "Queen", "A Night at the Opera", "02 Queen - Doing All Right.mp3"]);
        let (t, a, al) = repaired_meta(&path, "", "", "");
        assert_eq!(t, "Doing All Right");
        assert_eq!(a, "Queen");
        assert_eq!(al, "A Night at the Opera");
    }

    #[test]
    fn keeps_valid_metadata_tags() {
        let path = p(&["Music", "Queen", "A Night at the Opera", "11 - Bohemian Rhapsody.mp3"]);
        let (t, a, al) = repaired_meta(&path, "Bohemian Rhapsody", "Queen", "A Night at the Opera");
        assert_eq!(t, "Bohemian Rhapsody");
        assert_eq!(a, "Queen");
        assert_eq!(al, "A Night at the Opera");
    }

    #[test]
    fn recognises_artist_album_layout() {
        assert_eq!(
            folder_artist_album(&p(&["Music", "Tool", "Lateralus", "01.flac"])),
            Some(("Tool".into(), "Lateralus".into()))
        );
    }

    #[test]
    fn recognises_artist_dash_album_folder() {
        assert_eq!(
            folder_artist_album(&p(&["Music", "Type O Negative - Bloody Kisses (1993)", "x.flac"])),
            Some(("Type O Negative".into(), "Bloody Kisses".into()))
        );
    }

    #[test]
    fn skips_disc_folders() {
        assert_eq!(
            folder_artist_album(&p(&["Music", "Tool", "Lateralus", "CD 2", "x.flac"])),
            Some(("Tool".into(), "Lateralus".into()))
        );
    }

    #[test]
    fn merge_replaces_old_wrong_artist_when_album_missing() {
        let mut artist = "Lateralus".to_string();
        let mut album = "Unknown".to_string();
        merge_folder_meta(&mut artist, &mut album, &p(&["Music", "Tool", "Lateralus", "x.flac"]));
        assert_eq!((artist.as_str(), album.as_str()), ("Tool", "Lateralus"));
    }

    #[test]
    fn merge_keeps_good_tags() {
        let mut artist = "Tool".to_string();
        let mut album = "Lateralus".to_string();
        merge_folder_meta(&mut artist, &mut album, &p(&["Downloads", "x.flac"]));
        assert_eq!((artist.as_str(), album.as_str()), ("Tool", "Lateralus"));
    }

    #[test]
    fn does_not_use_music_root_as_artist() {
        assert_eq!(
            folder_artist_album(&p(&["Music", "Lateralus", "x.flac"])),
            Some(("Lateralus".into(), String::new()))
        );
    }
}

#[cfg(test)]
mod art_cache_tests {
    use super::*;

    fn fake_song(name: &str) -> String {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("/nonexistent/mpdoom-art-test/{}-{}-{}.flac", std::process::id(), n, name)
    }

    fn jpeg(color: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(200, 200, image::Rgba(color));
        // Add a contrasting square so the image is not a solid placeholder.
        let mut img = img;
        for x in 80..120 {
            for y in 80..120 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn write_then_read_round_trips() {
        let song = fake_song("good");
        let bytes = jpeg([30, 90, 180, 255]);
        assert!(write_art_cache(&song, &bytes));
        let got = read_art_cache(&song).expect("cached art should read back");
        assert!(art_bytes_ok(&got, 32));
        assert!(art_cache_ok(&song));
        let _ = std::fs::remove_file(art_cache_path(&song));
    }

    #[test]
    fn corrupt_cache_is_deleted_and_reported_missing() {
        let song = fake_song("bad");
        assert!(write_art_cache(&song, &jpeg([200, 30, 30, 255])));
        let cache = art_cache_path(&song);
        std::fs::write(&cache, b"this is not a JPEG").unwrap();
        assert!(read_art_cache(&song).is_none(), "corrupt art must not load");
        assert!(!cache.exists(), "corrupt cache file must be removed");
    }

    #[test]
    fn invalid_bytes_are_never_cached() {
        let song = fake_song("invalid");
        assert!(!write_art_cache(&song, b"junk"));
        assert!(!art_cache_path(&song).exists());
    }

    #[test]
    fn solid_placeholder_is_not_cached() {
        let song = fake_song("solid");
        let img = image::RgbaImage::from_pixel(300, 300, image::Rgba([255, 255, 255, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        assert!(!write_art_cache(&song, &out.into_inner()));
    }
}
