#![allow(unused_imports)]
#![allow(dead_code)]
// yt-dlp download/search helpers.
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;
use std::collections::HashMap;
use std::io::{BufReader, Read};
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

/// Whether a yt-dlp line is worth showing in the YT log panel.
///
/// yt-dlp tags its stages with a bracketed prefix. The download itself is
/// reported as `[download] 12.3% of ...`, but everything after it - merging
/// fragments, fixing up the container, writing tags, embedding the thumbnail,
/// transcoding to mp3 - is what the user stares at a frozen progress bar
/// through, so those stages are shown too.
pub fn is_yt_log_line(s: &str) -> bool {
    s.contains("% of") || s.contains("[download]") || s.contains("[ExtractAudio]")
        || s.starts_with("Destination:") || s.starts_with("Deleting original")
        || s.starts_with("[info]") || s.starts_with("[ffmpeg]") || s.starts_with("ERROR")
        || s.starts_with("[youtube]") || s.starts_with("[Merger]") || s.starts_with("[Fixup]")
        || s.starts_with("[MetadataParser]") || s.starts_with("[EmbedThumbnail]")
        || s.starts_with("[ThumbnailsConvertor]")
}
pub fn yt_lookup(
    yt: &Path,
    target: &str,
) -> Result<(String, String, String, String, String, f64), String> {
    let mut child = std::process::Command::new(yt)
        .creation_flags(NO_WINDOW)
        .args(["--skip-download", "--no-warnings", "--newline", "--print", "%(id)s|%(title)s|%(artist)s|%(uploader)s|%(album)s|%(duration)s", target])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;
    // Both pipes are drained on their own threads. Reading one to EOF on this
    // thread would let the other fill its buffer and wedge the child.
    let out_h = child.stdout.take().map(|mut so| {
        std::thread::spawn(move || {
            let mut b = String::new();
            let _ = so.read_to_string(&mut b);
            b
        })
    });
    let err_h = child.stderr.take().map(|mut se| {
        std::thread::spawn(move || {
            let mut b = String::new();
            let _ = se.read_to_string(&mut b);
            b
        })
    });
    // `output()` used to have no timeout, so a lookup that stalled (network
    // hiccup, YouTube challenge page) blocked the download worker forever and
    // the UI just sat on a 0% bar with nothing to show. Bound it and kill the
    // child so the queue reports a real reason and moves on.
    const LOOKUP_TIMEOUT: Duration = Duration::from_secs(90);
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {}
            Err(e) => return Err(format!("yt-dlp lookup failed: {}", e)),
        }
        if t0.elapsed() > LOOKUP_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "yt-dlp lookup timed out after {}s (no response from YouTube): {}",
                LOOKUP_TIMEOUT.as_secs(),
                target
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = out_h.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let err = err_h.map(|h| h.join().unwrap_or_default()).unwrap_or_default();
    let first = out.lines().next().unwrap_or("").trim().to_string();
    // Read the six fields strictly in the order the template emits them:
    // id | title | artist | uploader | album | duration.
    //
    // These used to be read shifted by one, which put the *uploader* (the
    // uploading channel) into the artist slot and the *artist* into the title
    // slot - producing a folder named after the channel and a track called
    // "untitled". `parse_lookup_line` is the single source of truth and
    // `lookup_fields_are_not_shifted` pins the order.
    parse_lookup_line(&first).ok_or_else(|| {
        let detail = err
            .lines()
            .rev()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .unwrap_or("yt-dlp produced no output");
        format!(
            "yt-dlp returned no usable metadata (exit {}): {}",
            status.code().unwrap_or(-1),
            detail
        )
    })
}

/// Split one `--print` line into `(id, title, artist, uploader, album, secs)`.
///
/// Isolated and pure so the field order is directly testable - reading these
/// fields off-by-one is what put the uploading channel into the artist slot.
pub fn parse_lookup_line(line: &str) -> Option<(String, String, String, String, String, f64)> {
    let mut it = line.trim().split('|');
    let id = clean_field(it.next()?);
    let title = clean_field(it.next()?);
    let artist = clean_field(it.next()?);
    let uploader = clean_field(it.next()?);
    let album = clean_field(it.next()?);
    // Duration is the one reliable way to tell a full-album encode from a
    // single song, since YouTube tags neither.
    let secs = it.next().unwrap_or("").trim().parse::<f64>().unwrap_or(0.0);
    Some((id, title, artist, uploader, album, secs))
}

/// Normalise a `--print` field.
///
/// yt-dlp emits the literal string `NA` - not an empty string - for metadata a
/// video doesn't provide. The folder name only tested for empty, so a video
/// with no artist/album tags produced a literal `NA - NA` directory.
fn clean_field(s: &str) -> String {
    let t = s.trim();
    if t.is_empty()
        || t.eq_ignore_ascii_case("na")
        || t.eq_ignore_ascii_case("none")
        || t.eq_ignore_ascii_case("null")
    {
        String::new()
    } else {
        t.to_string()
    }
}

/// Guess the artist from a video title.
///
/// YouTube only supplies a real `artist` for tracks that live in YouTube Music.
/// An ordinary upload has none, and the `uploader` field is whoever reuploaded
/// the video - frequently a stock-footage or "topic" channel with no relation to
/// the music - so naming a folder after it produces folders like
/// "Media Canvas - Singles". The title is the only place the real name usually
/// appears, in the near-universal "Artist - Song" / "Artist : Song" form.
///
/// Gives up rather than inventing a name when the title doesn't look like that
/// shape, so the caller can fall back to one flat folder.
pub fn artist_from_title(title: &str) -> Option<String> {
    let norm = normalize_separators(title);
    if norm.trim().is_empty() {
        return None;
    }
    let (idx, _) = split_separator(&norm)?;
    let cleaned = clean_field(&norm[..idx]);
    // Reject noise: too long to be a name, too short, no letters at all, or a
    // structural label like "Episode 1" / "Part 2" / "Track 3".
    if cleaned.is_empty() || cleaned.len() > 60 {
        return None;
    }
    let letters = cleaned.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 2 {
        return None;
    }
    let first = cleaned
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(['.', ':'])
        .to_lowercase();
    const LABELS: [&str; 10] = [
        "episode", "ep", "part", "track", "vol", "volume", "no", "number", "pt", "disc",
    ];
    if LABELS.iter().any(|l| *l == first) {
        return None;
    }
    Some(cleaned)
}

/// Fold the decorative separators official uploads use into a plain hyphen.
/// "Queen × Bohemian Rhapsody" really does use U+00FB. Char-for-char, so byte
/// offsets in the result still line up with the input.
fn normalize_separators(title: &str) -> String {
    title
        .trim()
        .chars()
        .map(|c| match c {
            '\u{2013}' | '\u{2014}' | '\u{2012}' | '\u{2015}' | '\u{2212}' | '\u{2022}'
            | '\u{00B7}' | '\u{00D7}' | '\u{2217}' | '\u{00FB}' | '\u{2011}' => '-',
            other => other,
        })
        .collect()
}

/// Find the "Artist - Song" separator, returning `(byte index, length)`.
/// Spaced forms first: a bare ": " is tried last because it also shows up in
/// "Episode 1: Pilot" style titles.
fn split_separator(norm: &str) -> Option<(usize, usize)> {
    for sep in [" - ", " : ", ": "].iter() {
        if let Some(idx) = norm.find(sep) {
            // Only trust a colon that appears early enough to be an artist.
            if *sep == ": " && idx > 40 {
                continue;
            }
            return Some((idx, sep.len()));
        }
    }
    None
}

/// Drop trailing bracketed noise: "[Full Album]", "(Official Video)", ...
fn strip_bracketed_tail(s: &str) -> String {
    let mut t = s.trim();
    loop {
        let before = t.len();
        for (open, close) in [('[', ']'), ('(', ')')] {
            if t.ends_with(close) {
                if let Some(i) = t.rfind(open) {
                    t = t[..i].trim_end();
                }
            }
        }
        if t.len() == before {
            break;
        }
    }
    clean_field(t)
}

/// Detect a full-album encode and return the album name.
///
/// A long runtime or album wording in the title is the only signal YouTube
/// gives without real tagging. Without this, a full CD rip like
/// "Green Day - Dookie [Full Album]" would be filed as if the album name were a
/// song title, while a genuine single would be filed under a folder named after
/// the song - which is the awkward case this avoids.
pub fn detect_album(title: &str, duration_secs: f64) -> Option<String> {
    const ALBUM_MIN_SECS: f64 = 20.0 * 60.0;
    let lower = title.to_lowercase();
    const KEYWORDS: [&str; 8] = [
        "full album", "complete album", "entire album", "full cd", "whole album",
        "discography", "album", "all songs",
    ];
    let says_album = KEYWORDS.iter().any(|k| lower.contains(k));
    if duration_secs < ALBUM_MIN_SECS && !says_album {
        return None;
    }
    let norm = normalize_separators(title);
    let album = match split_separator(&norm) {
        Some((idx, len)) => strip_bracketed_tail(&norm[idx + len..]),
        None => strip_bracketed_tail(&norm),
    };
    if album.is_empty() || album.len() > 80 {
        return None;
    }
    Some(album)
}

/// Normalised key for matching a YouTube title against library metadata:
/// lowercase, bracketed noise removed, everything non-alphanumeric dropped.
pub fn norm_match_key(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut depth = 0i32;
    for c in lower.chars() {
        match c {
            '[' | '(' | '{' => depth += 1,
            ']' | ')' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 && c.is_alphanumeric() => out.push(c),
            _ => {}
        }
    }
    out
}

/// `(artist, title) -> album` index over the local library, used to file a
/// single-song download into the album it already belongs to.
///
/// Also carries a title-only key, but only for titles where every library entry
/// agrees on one album - otherwise a common song name would file a download
/// under whatever album happened to be seen first.
pub fn build_album_index(
    cache: &HashMap<String, crate::settings::MetaCacheEntry>,
) -> HashMap<String, String> {
    let mut idx: HashMap<String, String> = HashMap::new();
    let mut by_title: HashMap<String, Option<String>> = HashMap::new();
    for e in cache.values() {
        let album = clean_field(&e.album);
        if album.is_empty() {
            continue;
        }
        let tk = norm_match_key(&e.title);
        if tk.len() < 3 {
            continue;
        }
        let ak = norm_match_key(&clean_field(&e.artist));
        if !ak.is_empty() {
            idx.entry(format!("{}\u{1f}{}", ak, tk))
                .or_insert_with(|| album.clone());
        }
        // None means "ambiguous", which disqualifies the title-only key.
        let slot = by_title.entry(tk).or_insert_with(|| Some(album.clone()));
        if slot.as_deref() != Some(album.as_str()) {
            *slot = None;
        }
    }
    for (t, a) in by_title {
        if let Some(album) = a {
            idx.entry(format!("\u{1f}{}", t)).or_insert(album);
        }
    }
    idx
}

/// Find the album for a downloaded track in the local library index.
///
/// A YouTube title usually carries the "Artist - " prefix that a library tag
/// does not ("AC/DC - Thunderstruck" vs a tagged "Thunderstruck"), so both the
/// whole title and just the song part are tried. Artist + title is preferred;
/// the title-only key exists only for unambiguous titles.
pub fn lookup_album(index: &HashMap<String, String>, artist: &str, title: &str) -> Option<String> {
    let norm = normalize_separators(title);
    let subject = split_separator(&norm).map(|(i, l)| strip_bracketed_tail(&norm[i + l..]));
    let a = norm_match_key(artist);
    for cand in [title.to_string(), subject.unwrap_or_default()] {
        let tk = norm_match_key(&cand);
        if tk.len() < 3 {
            continue;
        }
        if !a.is_empty() {
            if let Some(album) = index.get(&format!("{}\u{1f}{}", a, tk)) {
                return Some(album.clone());
            }
        }
        if let Some(album) = index.get(&format!("\u{1f}{}", tk)) {
            return Some(album.clone());
        }
    }
    None
}

pub fn yt_resolve_title(query: &str) -> String {
    let q = query.trim();
    if q.is_empty() || !ytdlp_path().is_file() {
        return query.to_string();
    }
    let target = if q.starts_with("http") { q.to_string() } else { format!("ytsearch1:{}", q) };
    match yt_lookup(&ytdlp_path(), &target) {
        Ok((_, title, _, _, _, _)) if !title.is_empty() => title,
        _ => query.to_string(),
    }
}

/// Rewrite the ID3 tags on a finished download.
///
/// `yt-dlp --parse-metadata` can only copy one *metadata field* into another
/// field, so passing a literal name writes `NA` (the field does not exist).
/// That is what left downloads tagged `artist=NA`. The audio itself is already
/// correct, so this is a tag-only pass: `-c copy` remuxes without re-encoding,
/// which takes a fraction of a second and preserves the embedded cover art.
///
/// Failure is reported but never fatal - a completed download must not be
/// thrown away because a tag could not be written.
fn yt_retag(
    ffmpeg: &Path,
    src: &Path,
    dst: &Path,
    title: &str,
    artist: &str,
    album: &str,
) -> Result<(), String> {
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-nostats".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        src.to_string_lossy().to_string(),
        "-c".into(),
        "copy".into(),
        "-id3v2_version".into(),
        "3".into(),
    ];
    // Only known values are written; a blank tag is left off rather than
    // written as an empty string. The uploading channel is never used.
    for (k, v) in [("title", title), ("artist", artist), ("album", album)] {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        args.push("-metadata".into());
        args.push(format!("{}={}", k, v));
    }
    args.push(dst.to_string_lossy().to_string());
    let out = std::process::Command::new(ffmpeg)
        .creation_flags(NO_WINDOW)
        .args(&args)
        .output()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    if !out.status.success() || !dst.is_file() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err
            .lines()
            .rev()
            .take(2)
            .collect::<Vec<_>>()
            .join(" / "));
    }
    Ok(())
}

/// Download the audio for a video, convert it to mp3 and tag it.
pub fn yt_grab(
    yt: &Path,
    ffmpeg: &Path,
    target: &str,
    id: &str,
    artist: &str,
    title: &str,
    album: &str,
    chunks: u32,
    tx: &Sender<Msg>,
) -> Result<PathBuf, String> {
    use std::io::BufRead;
    use std::process::Stdio;
    let tmp = work_dir().join("yt");
    let _ = std::fs::create_dir_all(&tmp);
    let tpl = tmp.join(format!("{}.%(ext)s", id));
    let _ = std::fs::remove_file(tmp.join(format!("{}.mp3", id)));
    let mut args: Vec<String> = vec![
        "--newline".into(),
        "--no-warnings".into(),
        "--ffmpeg-location".into(),
        ffmpeg.parent().and_then(|p| p.to_str()).unwrap_or("").into(),
        "-f".into(),
        "bestaudio/best".into(),
        "-x".into(),
        "--audio-format".into(),
        "mp3".into(),
        "--audio-quality".into(),
        "320k".into(),
        "--embed-metadata".into(),
        "--embed-thumbnail".into(),
        "--convert-thumbnails".into(),
        "jpg".into(),
    ];
    // The artist used to be injected here with
    // `--parse-metadata "<artist>:artist"`. That form only copies one
    // *metadata field* into another, so a literal name resolved to nothing and
    // the tag came out as `NA`. Tags are now written by `yt_retag` below.
    args.push("-N".into());
    args.push(chunks.to_string());
    args.push("-o".into());
    args.push(tpl.to_string_lossy().to_string());
    args.push(target.to_string());
    let mut child = std::process::Command::new(yt)
        .creation_flags(NO_WINDOW)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp failed to start: {}", e))?;
    // yt-dlp writes ALL --newline progress/info output to STDOUT (stderr stays
    // empty on a successful run), so both pipes must be drained. They are read
    // concurrently - one on this thread, one on a helper - otherwise a full pipe
    // buffer deadlocks the child.
    let out_tail: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let out_handle = child.stdout.take().map(|so| {
        let tx = tx.clone();
        let tail = std::sync::Arc::clone(&out_tail);
        std::thread::spawn(move || {
            let reader = BufReader::new(so);
            for line in reader.lines() {
                let Ok(s) = line else { break };
                let s = s.trim_end().to_string();
                if s.is_empty() {
                    continue;
                }
                // Keep the tail so a failure that only printed to stdout can
                // still explain itself.
                if let Ok(mut t) = tail.lock() {
                    if t.len() >= 4 {
                        t.remove(0);
                    }
                    t.push(s.clone());
                }
                if is_yt_log_line(&s) {
                    // yt-dlp reports no percentage while ffmpeg re-encodes, so
                    // hand the bar to the transcode stage the moment the extract
                    // starts. Without this the bar sat pinned at the download's
                    // 100% for the whole encode and read as a hang.
                    if s.starts_with("[ExtractAudio]") {
                        yt_stage_bar(&tx, STAGE_TRANSCODE, "converting to mp3");
                    }
                    let _ = tx.send(Msg::YtLog(s));
                }
            }
        })
    });
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
    if let Some(h) = out_handle {
        let _ = h.join();
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let final_mp3 = tmp.join(format!("{}.mp3", id));
    if !status.success() || !final_mp3.is_file() {
        let mut tail: Vec<String> = last_err.lines().rev().take(4).map(|s| s.to_string()).collect();
        tail.reverse();
        if tail.is_empty() {
            if let Ok(t) = out_tail.lock() {
                let start = t.len().saturating_sub(4);
                tail = t[start..].to_vec();
            }
        }
        let err = tail.join("\n");
        return Err(if err.is_empty() { format!("yt-dlp exited {}", status.code().unwrap_or(-1)) } else { err });
    }
    // Tag-only pass: `-c copy`, so the audio is not re-encoded and the cover art
    // yt-dlp embedded is kept. A failure here is logged but the download still
    // succeeds - the audio is good even if a tag did not take.
    let tagged = tmp.join(format!("{}.tagged.mp3", id));
    let _ = std::fs::remove_file(&tagged);
    match yt_retag(ffmpeg, &final_mp3, &tagged, title, artist, album) {
        Ok(()) => {
            // Windows rename will not overwrite, so the original goes first.
            let _ = std::fs::remove_file(&final_mp3);
            if let Err(e) = std::fs::rename(&tagged, &final_mp3) {
                let _ = std::fs::remove_file(&tagged);
                yt_stage(tx, format!("[2/3] WARNING: could not replace tags: {}", e));
            }
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tagged);
            yt_stage(tx, format!("[2/3] WARNING: could not write tags: {}", e));
        }
    }
    Ok(final_mp3)
}

/// Push a line into the persistent YT log panel.
///
/// The panel keeps the last 60 lines, so unlike the 5-second status flash a
/// failure recorded here stays put. Each stage of a download logs one line, so
/// a stall is identifiable by the last line that made it through.
fn yt_stage(tx: &Sender<Msg>, s: impl Into<String>) {
    let _ = tx.send(Msg::YtLog(s.into()));
}

/// One slice of the single continuous progress bar.
#[derive(Debug, Clone, Copy)]
pub struct Stage {
    /// Where this stage starts, as a fraction of the whole job.
    pub lo: f32,
    /// Where it ends.
    pub hi: f32,
    /// Typical seconds this stage takes, measured on a real full-album rip
    /// (lookup 2.6-3.5s, download 3.4s, transcode+tagging 4.5s). Stages with no
    /// percentage of their own are paced against this so the bar keeps filling
    /// instead of freezing. `0.0` means "driven by a real percentage instead".
    pub nominal_secs: f32,
}

pub const STAGE_LOOKUP: u8 = 0;
pub const STAGE_DOWNLOAD: u8 = 1;
pub const STAGE_TRANSCODE: u8 = 2;
pub const STAGE_SAVE: u8 = 3;

/// The four stages, in order, spanning 0.0 to 1.0 with no gaps.
pub const STAGES: [Stage; 4] = [
    Stage { lo: 0.00, hi: 0.10, nominal_secs: 3.0 },
    Stage { lo: 0.10, hi: 0.80, nominal_secs: 0.0 },
    Stage { lo: 0.80, hi: 0.97, nominal_secs: 4.5 },
    Stage { lo: 0.97, hi: 1.00, nominal_secs: 0.3 },
];

/// Where the bar should be, as one continuous 0..1 fraction of the whole job.
///
/// `real_pct` is yt-dlp's own download percentage and is only meaningful for
/// the download stage; the caller passes `None` elsewhere. Clock-paced stages
/// stop just short of their own ceiling so the bar never claims to be finished
/// before the stage actually is - the next stage pushes it the rest of the way.
pub fn pipeline_frac(stage: u8, elapsed_in_stage: f32, real_pct: Option<f32>) -> f32 {
    let s = match STAGES.get(stage as usize) {
        Some(s) => s,
        None => return 1.0,
    };
    let span = s.hi - s.lo;
    if span <= 0.0 {
        return s.lo;
    }
    let frac = match real_pct {
        // A real percentage is authoritative while we have one.
        Some(p) => (p / 100.0).clamp(0.0, 1.0),
        None if s.nominal_secs <= 0.0 => 0.0,
        None => {
            let t = (elapsed_in_stage / s.nominal_secs).clamp(0.0, 1.0);
            // Ease out so the bar decelerates into the stage boundary instead of
            // slamming into it, which is what made it look like it lurched.
            1.0 - (1.0 - t).powi(2)
        }
    };
    // Leave a sliver so only a real stage change completes a stage.
    let ceiling = if frac >= 1.0 { s.hi } else { s.hi - span * 0.02 };
    (s.lo + span * frac).clamp(s.lo, ceiling).clamp(0.0, 1.0)
}

/// Log a stage line and point the bar at that stage.
fn yt_stage_bar(tx: &Sender<Msg>, stage: u8, label: impl Into<String>) {
    let _ = tx.send(Msg::YtStage { stage, label: label.into() });
}

/// Identity of the album a download belongs to.
///
/// A queue of one album's worth of videos should all land in the same folder,
/// so two items share a destination exactly when this matches. Compared
/// case-insensitively, and an absent album is treated as "Singles" to match the
/// folder naming.
pub fn album_group_key(artist: &str, album: &str) -> String {
    let a = artist.trim().to_lowercase();
    let b = album.trim().to_lowercase();
    let b = if b.is_empty() { "singles".to_string() } else { b };
    // Unit separator: can't occur in a real artist/album name.
    format!("{}\u{1f}{}", a, b)
}

/// Folder name for an album. `YouTube` when no artist could be determined,
/// rather than a folder named after the uploading channel.
pub fn group_folder_name(artist: &str, album: &str) -> String {
    if artist.trim().is_empty() {
        return "YouTube".to_string();
    }
    let album_d = if album.trim().is_empty() { "Singles".to_string() } else { album.trim().to_string() };
    format!("{} - {}", sanitize_win(artist.trim()), sanitize_win(&album_d))
}

pub fn yt_download_and_store(
    query: &str,
    chunks: u32,
    group: Option<(String, String)>,
    album_index: &HashMap<String, String>,
    tx: &Sender<Msg>,
) -> Result<PathBuf, String> {
    ensure_tools(tx)?;
    let q = query.trim();
    if q.is_empty() {
        return Err("Enter a YouTube URL or search term".into());
    }
    let target = if q.starts_with("http") { q.to_string() } else { format!("ytsearch1:{}", q) };
    yt_stage(tx, format!("[1/3] looking up metadata: {}", target));
    yt_stage_bar(tx, STAGE_LOOKUP, "looking up video");
    let (id, title, yt_artist, uploader, yt_album, duration) = yt_lookup(&ytdlp_path(), &target)
        .map_err(|e| {
            yt_stage(tx, format!("[1/3] FAILED: {}", e));
            e
        })?;
    let mut display = title.clone();
    if display.is_empty() {
        display = id.clone();
    }
    // The name the file is saved under. "[Official Music Video]", "[4K Upgrade]",
    // "(Lyrics)" and similar are video packaging, not part of the song, and they
    // were reaching the filename and the title tag because the raw YouTube
    // title was used for both. Only the bracketed tail is removed - the
    // "Artist - " prefix stays, and nothing here feeds album or artist
    // detection, which still see the original title.
    let name_title = {
        let stripped = strip_bracketed_tail(&title);
        if stripped.is_empty() { title.clone() } else { stripped }
    };
    // Prefer a real YouTube artist. Failing that, read it out of the title.
    // The uploader/channel is deliberately NOT used as a fallback - it is
    // whoever reuploaded the video and is usually unrelated to the music.
    let artist = if !yt_artist.is_empty() {
        yt_artist
    } else {
        artist_from_title(&title).unwrap_or_default()
    };
    // Album, in order of trustworthiness:
    //   1. a real album tag from YouTube
    //   2. a full-album encode, read out of the title
    //   3. the album this track already has in the local library
    //   4. nothing -> "Singles"
    // Step 2 is what keeps a full CD rip from being filed as if the album name
    // were a song title; step 3 is what puts a single song into the album it
    // belongs to instead of a folder named after the song.
    let album = if !yt_album.is_empty() {
        yt_album
    } else if let Some(a) = detect_album(&title, duration) {
        a
    } else {
        lookup_album(album_index, &artist, &display).unwrap_or_default()
    };
    yt_stage(tx, format!(
        "[1/3] artist={:?} album={:?} ({:.0}s)",
        if artist.is_empty() { "?" } else { &artist },
        if album.is_empty() { "Singles" } else { &album },
        duration
    ));
    let _ = tx.send(Msg::YtStatus(format!("Downloading \"{}\"...", display)));
    yt_stage(tx, format!("[2/3] downloading: {}", display));
    yt_stage_bar(tx, STAGE_DOWNLOAD, "downloading audio");
    let mp3 = yt_grab(
        &ytdlp_path(),
        &ffmpeg_path(),
        &target,
        &id,
        &artist,
        &name_title,
        &album,
        chunks,
        tx,
    )
    .map_err(|e| {
        yt_stage(tx, format!("[2/3] FAILED: {}", e));
        e
    })?;
    let _ = &uploader;
    // Reuse the queue's folder when this track belongs to the same album, so a
    // queue of one album stays together. A different album opens its own
    // folder, and reports it so the rest of the queue can follow.
    let key = album_group_key(&artist, &album);
    let (folder, new_group) = match group {
        Some((k, name)) if k == key => (music_folder().join(&name), None),
        _ => {
            let name = group_folder_name(&artist, &album);
            (music_folder().join(&name), Some((key, name)))
        }
    };
    if let Some((k, name)) = new_group {
        let _ = std::fs::create_dir_all(&folder);
        let _ = tx.send(Msg::YtGroupEstablished { key: k, folder: name });
    }
    let num = next_track_num(&folder);
    let mut fname = format!("{:03} - {}.mp3", num, sanitize_win(&name_title));
    let mut final_path = folder.join(&fname);
    let mut extra = 0u32;
    while final_path.exists() {
        extra += 1;
        fname = format!("{:03}.{} - {}.mp3", num, extra, sanitize_win(&title));
        final_path = folder.join(&fname);
    }
    yt_stage(tx, format!("[3/3] saving to {}", final_path.display()));
    yt_stage_bar(tx, STAGE_SAVE, "saving to Music folder");
    std::fs::rename(&mp3, &final_path).map_err(|e| {
        let m = format!("could not save to Music folder: {}", e);
        yt_stage(tx, format!("[3/3] FAILED: {}", m));
        m
    })?;
    yt_stage(tx, format!("done: {}", final_path.display()));
    // No "done" stage message: the bar reaching 100% is driven by `YtDone`
    // once the file is actually in place. Announcing it here filled the bar
    // before the job had finished, which then read as a bar that overshoots
    // and falls back.
    Ok(final_path)
}

pub fn artist_album_folder(root: &Path, artist: &str, album: &str) -> PathBuf {
    // No trustworthy artist (YouTube gave none and the title didn't look like
    // "Artist - Song"). One flat folder is better than a folder named after
    // the uploading channel, which is usually unrelated to the music.
    if artist.trim().is_empty() {
        let flat = root.join("YouTube");
        let _ = std::fs::create_dir_all(&flat);
        return flat;
    }
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

#[cfg(test)]
mod yt_tests {
    use super::*;

    // yt-dlp prints the literal string "NA" for metadata a video doesn't
    // supply. The artist/album folder only tested for empty, so untagged
    // downloads landed in a literal "NA - NA" directory.
    #[test]
    fn absent_metadata_is_treated_as_missing() {
        assert_eq!(clean_field("NA"), "");
        assert_eq!(clean_field(" na "), "");
        assert_eq!(clean_field("None"), "");
        assert_eq!(clean_field("null"), "");
        assert_eq!(clean_field(""), "");
        assert_eq!(clean_field("   "), "");
    }

    // This is the exact bug that produced "The Pulse Music - 2299" and
    // "001 - untitled.mp3": the six fields were read one position early, so the
    // *uploader* became the artist and the *artist* became the title.
    // The line below is verbatim real output from a full-album upload.
    #[test]
    fn lookup_fields_are_not_shifted() {
        let line = "CD-E-LDc384|Green Day - Dookie [Full Album]|NA|The Pulse Music|NA|2299";
        let (id, title, artist, uploader, album, secs) =
            parse_lookup_line(line).expect("should parse");
        assert_eq!(id, "CD-E-LDc384");
        assert_eq!(title, "Green Day - Dookie [Full Album]");
        // The channel must never end up here.
        assert_eq!(artist, "", "uploader leaked into the artist slot");
        assert_eq!(uploader, "The Pulse Music");
        assert_eq!(album, "");
        assert_eq!(secs, 2299.0);

        // And with a real artist present, that is what must be used.
        let (id, title, artist, uploader, _, secs) = parse_lookup_line(
            "fJ9rUzIMcZQ|Queen - Bohemian Rhapsody|Queen|Queen - Topic|NA|331",
        )
        .expect("should parse");
        assert_eq!(id, "fJ9rUzIMcZQ");
        assert_eq!(title, "Queen - Bohemian Rhapsody");
        assert_eq!(artist, "Queen");
        assert_eq!(uploader, "Queen - Topic");
        assert_eq!(secs, 331.0);
    }

    #[test]
    fn a_parsed_full_album_keeps_its_title_and_never_the_channel() {
        // End-to-end over the parsed tuple, the way the download path uses it.
        let (_id, title, yt_artist, uploader, _yt_album, secs) = parse_lookup_line(
            "CD-E-LDc384|Green Day - Dookie [Full Album]|NA|The Pulse Music|NA|2299",
        )
        .unwrap();
        let artist = if !yt_artist.is_empty() {
            yt_artist
        } else {
            artist_from_title(&title).unwrap_or_default()
        };
        assert_eq!(artist, "Green Day");
        let album = detect_album(&title, secs).unwrap();
        assert_eq!(album, "Dookie");
        assert_eq!(group_folder_name(&artist, &album), "Green Day - Dookie");
        // The channel appears nowhere in the folder or the file name.
        let fname = format!("001 - {}.mp3", sanitize_win(&title));
        assert!(!fname.contains("Pulse"), "channel leaked into filename: {}", fname);
        assert!(fname.contains("Dookie"));
        // Sanity: the real YouTube tag really is the channel, so this is the
        // field that must stay out of naming.
        assert_eq!(uploader, "The Pulse Music");
    }

    #[test]
    fn an_unusable_line_reports_nothing_rather_than_guessing() {
        // Too few fields: must not silently produce empty strings that later
        // turn into "untitled".
        assert!(parse_lookup_line("only|two|fields").is_none());
        assert!(parse_lookup_line("").is_none());
    }

    #[test]
    fn real_metadata_survives_cleaning() {
        assert_eq!(clean_field("Green Day"), "Green Day");
        assert_eq!(clean_field(" Dookie "), "Dookie");
        // A real artist/album must not be mistaken for a missing one.
        assert_eq!(clean_field("Nana"), "Nana");
        assert_eq!(clean_field("NONE"), "");
    }

    // The artist comes from the title, never from the uploading channel.
    #[test]
    fn artist_is_read_out_of_the_title() {
        assert_eq!(
            artist_from_title("Green Day - Dookie [Full Album]").as_deref(),
            Some("Green Day")
        );
        assert_eq!(
            artist_from_title("Metallica : Enter Sandman").as_deref(),
            Some("Metallica")
        );
        assert_eq!(
            artist_from_title("Rick Astley - Never Gonna Give You Up").as_deref(),
            Some("Rick Astley")
        );
        // Trailing noise after the song name is fine; we only read the left side.
        assert_eq!(
            artist_from_title("Madonna - Vogue (Official Video)").as_deref(),
            Some("Madonna")
        );
    }

    // Real titles captured from YouTube, all of which report artist=NA.
    #[test]
    fn handles_real_youtube_titles() {
        let cases: [(&str, Option<&str>); 5] = [
            ("Metallica: Enter Sandman (Official Music Video)", Some("Metallica")),
            ("Luis Fonsi - Despacito ft. Daddy Yankee", Some("Luis Fonsi")),
            // U+00FB multiplication sign, as the official upload actually uses.
            ("Queen \u{00FB} Bohemian Rhapsody (Official Video Remastered)", Some("Queen")),
            ("Ed Sheeran - Shape of You (Official Music Video)", Some("Ed Sheeran")),
            ("Mark Ronson - Uptown Funk (Official Video) ft. Bruno Mars", Some("Mark Ronson")),
        ];
        for (title, want) in cases {
            assert_eq!(artist_from_title(title).as_deref(), want, "title was {:?}", title);
        }
    }

    #[test]
    fn gives_up_rather_than_inventing_an_artist() {
        // No separator: this is the stock-footage clip that used to produce a
        // folder named after its "Media Canvas" channel.
        assert_eq!(
            artist_from_title("Breaking News Opener | News Intro | News Sting"),
            None
        );
        assert_eq!(artist_from_title(""), None);
        assert_eq!(artist_from_title("   "), None);
        // Structural labels are not artists.
        assert_eq!(artist_from_title("Episode 1: Pilot"), None);
        assert_eq!(artist_from_title("Part 2 - The Return"), None);
        assert_eq!(artist_from_title("Track 3 - Interlude"), None);
        // Degenerate left-hand sides.
        assert_eq!(artist_from_title(" - Song Name"), None);
        assert_eq!(artist_from_title("123 - Song Name"), None);
    }

    #[test]
    fn artist_album_folder_falls_back_to_one_flat_folder() {
        let tmp = std::env::temp_dir().join("mpd_folder_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let got = artist_album_folder(&tmp, "", "");
        assert_eq!(got.file_name().unwrap().to_string_lossy(), "YouTube");
        assert!(got.is_dir(), "flat fallback folder should be created");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn artist_album_folder_uses_the_artist_when_known() {
        let tmp = std::env::temp_dir().join("mpd_folder_test2");
        let _ = std::fs::remove_dir_all(&tmp);
        let got = artist_album_folder(&tmp, "Green Day", "");
        assert_eq!(got.file_name().unwrap().to_string_lossy(), "Green Day - Singles");
        // Reuses the same folder rather than creating "green day - singles".
        let again = artist_album_folder(&tmp, "green day", "");
        assert_eq!(again, got);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // A queue of one album's worth of videos must land in one folder.
    #[test]
    fn same_album_shares_a_group() {
        let a = album_group_key("Green Day", "Dookie");
        let b = album_group_key("green day", "dookie");
        assert_eq!(a, b, "album identity should ignore case");
        // A missing album is "Singles" on both sides.
        assert_eq!(
            album_group_key("Some Band", ""),
            album_group_key("Some Band", "Singles")
        );
    }

    #[test]
    fn different_albums_do_not_share_a_group() {
        assert_ne!(album_group_key("Green Day", "Dookie"), album_group_key("Green Day", "Kerplunk"));
        assert_ne!(album_group_key("Green Day", "Dookie"), album_group_key("Metallica", "Dookie"));
        assert_ne!(album_group_key("", ""), album_group_key("Metallica", ""));
    }

    #[test]
    fn folder_names_never_use_the_channel() {
        assert_eq!(group_folder_name("Green Day", "Dookie"), "Green Day - Dookie");
        assert_eq!(group_folder_name("Green Day", ""), "Green Day - Singles");
        // No artist -> the flat folder, not something invented.
        assert_eq!(group_folder_name("", ""), "YouTube");
        assert_eq!(group_folder_name("  ", "Dookie"), "YouTube");
    }

    // The naming bug: video packaging was reaching the filename and the tag,
    // producing files like "001 - Green Day - American Idiot [Official Music
    // Video] [4K Upgrade].mp3".
    #[test]
    fn saved_name_drops_video_packaging() {
        // These are the actual titles that produced the bad filenames.
        assert_eq!(
            strip_bracketed_tail("Green Day - American Idiot [Official Music Video] [4K Upgrade]"),
            "Green Day - American Idiot"
        );
        assert_eq!(
            strip_bracketed_tail("Green Day - Dookie [Full Album]"),
            "Green Day - Dookie"
        );
        assert_eq!(
            strip_bracketed_tail("Madonna - Vogue (Official Video)"),
            "Madonna - Vogue"
        );
        assert_eq!(
            strip_bracketed_tail("Rick Astley - Never Gonna Give You Up (Official Music Video) [HD]"),
            "Rick Astley - Never Gonna Give You Up"
        );
        // A title with no packaging is left exactly as it was.
        assert_eq!(
            strip_bracketed_tail("Metallica - Enter Sandman"),
            "Metallica - Enter Sandman"
        );
        // Brackets in the middle of a name are real content, not packaging, and
        // must survive - only the trailing run is removed.
        assert_eq!(
            strip_bracketed_tail("Blur - Song 2 [Original Mix] feat. Someone"),
            "Blur - Song 2 [Original Mix] feat. Someone"
        );
    }

    #[test]
    fn saved_name_never_becomes_empty() {
        // A title made only of packaging must not blank the name out; the
        // caller falls back to the original title when this returns empty.
        assert!(strip_bracketed_tail("[Official Music Video]").is_empty());
        assert!(strip_bracketed_tail("NA").is_empty());
    }

    // The bar used to show only yt-dlp's raw download percentage, so it sat at
    // 0% through the ~3s lookup and froze at 100% through the ~4.5s transcode.
    // These pin the three rules the bar now has to obey.
    #[test]
    fn stages_span_the_whole_bar_without_gaps() {
        assert_eq!(STAGES[0].lo, 0.0, "the bar must start at zero");
        assert_eq!(STAGES[STAGES.len() - 1].hi, 1.0, "the bar must end at one");
        for w in STAGES.windows(2) {
            assert_eq!(w[0].hi, w[1].lo, "gap or overlap between stages");
        }
        for s in STAGES.iter() {
            assert!(s.hi > s.lo, "stage with no width: {:?}", s);
        }
    }

    #[test]
    fn the_bar_never_reaches_100_before_the_item_is_finished() {
        // This is the reported bug: the bar filled to 100% and then dropped
        // back to a lower number. No stage before the last may report 1.0.
        for stage in [STAGE_LOOKUP, STAGE_DOWNLOAD, STAGE_TRANSCODE] {
            // Long past any plausible duration, and with a 100% real reading.
            for secs in [0.0f32, 1.0, 5.0, 60.0, 3600.0] {
                for pct in [None, Some(0.0), Some(50.0), Some(100.0)] {
                    let f = pipeline_frac(stage, secs, pct);
                    assert!(
                        f < 1.0,
                        "stage {} hit 100% at {}s with {:?}",
                        stage,
                        secs,
                        pct
                    );
                }
            }
        }
    }
    #[test]
    fn each_stage_advances_continuously_over_time() {
        // The stages with no percentage of their own must keep filling rather
        // than freezing - that was "the bar doesn't follow the download".
        for stage in [STAGE_LOOKUP, STAGE_TRANSCODE] {
            let mut prev = -1.0;
            for i in 0..=40 {
                let f = pipeline_frac(stage, i as f32 * 0.25, None);
                assert!(f >= prev, "stage {} went backwards at {}s", stage, i as f32 * 0.25);
                prev = f;
            }
            assert!(prev > STAGES[stage as usize].lo, "stage {} never moved", stage);
        }
    }

    #[test]
    fn the_download_uses_the_real_percentage() {
        let lo = STAGES[STAGE_DOWNLOAD as usize].lo;
        let hi = STAGES[STAGE_DOWNLOAD as usize].hi;
        assert!(pipeline_frac(STAGE_DOWNLOAD, 0.0, Some(0.0)) >= lo);
        // A finished download tops out its own stage and no further - the
        // transcode and save still have to run.
        assert!((pipeline_frac(STAGE_DOWNLOAD, 0.0, Some(100.0)) - hi).abs() < 0.001);
        assert!(pipeline_frac(STAGE_DOWNLOAD, 0.0, Some(100.0)) < 1.0);
        // Monotonic in the real percentage.
        let mut prev = -1.0;
        for p in [0.0f32, 20.0, 40.0, 60.0, 80.0, 100.0] {
            let f = pipeline_frac(STAGE_DOWNLOAD, 1.0, Some(p));
            assert!(f > prev, "not monotonic at {}%", p);
            prev = f;
        }
    }

    #[test]
    fn stage_changes_never_move_the_bar_backwards() {
        // The UI keeps the running maximum, so walking the stages in order can
        // only ever climb. A finished download followed by a transcode is the
        // exact sequence that used to drop from 100% to a lower number.
        let mut bar = 0.0f32;
        let seq: [(u8, f32, Option<f32>); 6] = [
            (STAGE_LOOKUP, 9.0, None),
            (STAGE_DOWNLOAD, 0.0, Some(100.0)),
            (STAGE_TRANSCODE, 0.0, None),
            (STAGE_TRANSCODE, 4.5, None),
            (STAGE_SAVE, 0.3, None),
            (STAGE_SAVE, 0.3, None),
        ];
        for (stage, secs, pct) in seq {
            bar = bar.max(pipeline_frac(stage, secs, pct)).clamp(0.0, 0.999);
        }
        assert!(bar > 0.95, "bar should be nearly done, was {}", bar);
        assert!(bar < 1.0, "bar must not read 100% before YtDone, was {}", bar);
    }

    // A full CD encode should be filed under the album, not under a folder
    // named after what is actually the album-as-a-title.
    #[test]
    fn long_videos_are_albums() {
        // Real-world shape: 45 minute upload, album named in the title.
        assert_eq!(
            detect_album("Green Day - Dookie [Full Album]", 2700.0).as_deref(),
            Some("Dookie")
        );
        // Long even without the word "album" in the title.
        assert_eq!(
            detect_album("Pink Floyd - The Wall", 4200.0).as_deref(),
            Some("The Wall")
        );
        // Parenthetical form.
        assert_eq!(
            detect_album("Metallica - Master of Puppets (Full Album)", 3600.0).as_deref(),
            Some("Master of Puppets")
        );
    }

    #[test]
    fn title_words_alone_mark_an_album() {
        // Short runtime but says so.
        assert_eq!(
            detect_album("Some Band - The Record (Complete Album)", 600.0).as_deref(),
            Some("The Record")
        );
        // No separator: the whole title, minus the bracketed noise.
        assert_eq!(
            detect_album("Abbey Road [Full Album]", 1800.0).as_deref(),
            Some("Abbey Road")
        );
    }

    #[test]
    fn short_songs_are_not_albums() {
        // The awkward case this avoids: a single song must NOT become a folder
        // named after the song.
        assert_eq!(detect_album("Metallica: Enter Sandman (Official Music Video)", 339.0), None);
        assert_eq!(detect_album("Ed Sheeran - Shape of You (Official Music Video)", 234.0), None);
        assert_eq!(detect_album("Rick Astley - Never Gonna Give You Up", 213.0), None);
    }

    // Matching a download against albums already in the local library.
    fn cache_with(entries: &[(&str, &str, &str)]) -> HashMap<String, crate::settings::MetaCacheEntry> {
        let mut m = HashMap::new();
        for (i, (t, a, al)) in entries.iter().enumerate() {
            m.insert(
                format!("song{}.mp3", i),
                crate::settings::MetaCacheEntry {
                    title: t.to_string(),
                    artist: a.to_string(),
                    album: al.to_string(),
                    secs: 200.0,
                },
            );
        }
        m
    }

    #[test]
    fn finds_the_album_in_the_local_library() {
        let idx = build_album_index(&cache_with(&[
            ("Enter Sandman", "Metallica", "Master of Puppets"),
            ("Shape of You", "Ed Sheeran", "Divide"),
        ]));
        assert_eq!(
            lookup_album(&idx, "Metallica", "Enter Sandman").as_deref(),
            Some("Master of Puppets")
        );
        // Case and bracketed YouTube noise must not stop a match.
        assert_eq!(
            lookup_album(&idx, "metallica", "Enter Sandman (Official Music Video)").as_deref(),
            Some("Master of Puppets")
        );
        assert_eq!(lookup_album(&idx, "Nobody", "Nothing").as_deref(), None);
    }

    #[test]
    fn matches_a_youtube_title_that_carries_the_artist_prefix() {
        // The library tags it "Thunderstruck"; YouTube calls it
        // "AC/DC - Thunderstruck". Without stripping the prefix this never
        // matches and the download lands in Singles.
        let idx = build_album_index(&cache_with(&[("Thunderstruck", "AC/DC", "The Razors Edge")]));
        assert_eq!(
            lookup_album(&idx, "AC/DC", "AC/DC - Thunderstruck").as_deref(),
            Some("The Razors Edge")
        );
    }

    #[test]
    fn ambiguous_titles_are_not_matched_on_title_alone() {
        // Two different albums share the song title, so a title-only lookup
        // would be a coin flip. It must be refused.
        let idx = build_album_index(&cache_with(&[
            ("Yesterday", "The Beatles", "Help!"),
            ("Yesterday", "Guns N' Roses", "Use Your Illusion I"),
        ]));
        // Artist still resolves it unambiguously.
        assert_eq!(
            lookup_album(&idx, "The Beatles", "Yesterday").as_deref(),
            Some("Help!")
        );
        // No artist, shared title: refuse rather than guess.
        assert_eq!(lookup_album(&idx, "", "Yesterday"), None);
    }

    #[test]
    fn ignores_library_entries_with_no_album() {
        let idx = build_album_index(&cache_with(&[
            ("Mystery Track", "Some Band", ""),
            ("Other", "Some Band", "NA"),
        ]));
        assert!(idx.is_empty(), "entries without a real album should not be indexed");
        assert_eq!(lookup_album(&idx, "Some Band", "Mystery Track"), None);
    }

    #[test]
    fn match_keys_drop_bracketed_noise() {
        assert_eq!(norm_match_key("Enter Sandman (Official Music Video)"), "entersandman");
        assert_eq!(norm_match_key("Dookie [Full Album]"), "dookie");
        assert_eq!(norm_match_key("Thunderstruck"), "thunderstruck");
        assert_eq!(norm_match_key("Hello, World!"), "helloworld");
    }

    // Real `yt-dlp --newline` lines. These arrive on STDOUT; yt_grab used to
    // throw stdout away and read only stderr, so nothing ever showed up.
    #[test]
    fn parses_single_stream_progress() {
        let (pct, speed, eta) =
            parse_yt_progress("[download]  42.3% of  3.45MiB at  1.20MiB/s ETA 00:02").unwrap();
        assert!((pct - 42.3).abs() < 0.01, "pct was {}", pct);
        assert_eq!(speed, "1.20MiB/s");
        assert_eq!(eta, "00:02");
    }

    #[test]
    fn parses_completed_progress() {
        let (pct, speed, eta) =
            parse_yt_progress("[download] 100.0% of 3.45MiB in 00:03").unwrap();
        assert!((pct - 100.0).abs() < 0.01);
        assert!(speed.is_empty(), "speed was {:?}", speed);
        assert!(eta.is_empty(), "eta was {:?}", eta);
    }

    #[test]
    fn ignores_non_progress_lines() {
        assert!(parse_yt_progress("[download] Destination: abc.webm").is_none());
        assert!(parse_yt_progress("[youtube] abc: Downloading webpage").is_none());
        assert!(parse_yt_progress("").is_none());
    }

    #[test]
    fn keeps_the_lines_the_ui_shows() {
        for line in [
            "[download]  42.3% of 3.45MiB at 1.20MiB/s ETA 00:02",
            "[download] Destination: abc.webm",
            "[ExtractAudio] Destination: abc.mp3",
            "[Merger] Merging formats into \"abc.m4a\"",
            "[MetadataParser] Adding metadata to abc.m4a",
            "[EmbedThumbnail] Embedding thumbnail in file",
            "Deleting original file abc.webm",
            "ERROR: unable to download video data",
        ] {
            assert!(is_yt_log_line(line), "should be kept: {}", line);
        }
        assert!(!is_yt_log_line("just some noise"));
    }
}

