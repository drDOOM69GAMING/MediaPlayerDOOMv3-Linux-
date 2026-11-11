#![allow(unused_imports)]
#![allow(dead_code)]
// Net thread: yt-dlp + web art/lyrics lookup.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use crate::consts::{APP_NAME, APP_VERSION};
use crate::files::normalize_art;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
use crate::ytdlp::*;
pub fn net_loop(rx: Receiver<NetCmd>, tx: Sender<Msg>) {
    while let Ok(cmd) = rx.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match cmd {
                NetCmd::WebArt(artist, album) => {
                    let bytes = fetch_web_art(&artist, &album).and_then(|b| normalize_art(&b));
                    let _ = tx.send(Msg::ArtWeb { bytes });
                }
                NetCmd::WebArtExt(artist, album, title) => {
                    let bytes = fetch_web_art(&artist, &album)
                        .or_else(|| fetch_song_art(&artist, &title))
                        .and_then(|b| normalize_art(&b));
                    let _ = tx.send(Msg::ArtWeb { bytes });
                }
                NetCmd::WebArtFor { .. } => {
                    // "Meta All" web lookups are handled by the parallel
                    // web_art_loop pool, not the single net thread.
                }
                NetCmd::WebArtGroup { .. } => {
                    // Album-group lookups also run in the parallel pool.
                }
                NetCmd::WebArtSong { .. } => {
                    // Song fallback lookups also run in the parallel pool.
                }
                NetCmd::Lyrics(artist, title) => {
                    let text = fetch_lyrics(&artist, &title);
                    let _ = tx.send(Msg::Lyrics { artist, title, text });
                }
                NetCmd::CheckUpdate { current } => {
                    if let Some((version, url)) = check_latest_release(&current) {
                        let _ = tx.send(Msg::UpdateAvailable { version, url });
                    }
                }
            }
        }));
    }
}

/// Dedicated worker pool for "Meta All" web art. Runs on its own channel so
/// several artwork lookups happen in parallel instead of one slow sequential
/// stream on the single net thread - a big library would otherwise sit at
/// "N web art left" for hours and never finish.
pub fn web_art_loop(
    rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<NetCmd>>>,
    tx: Sender<Msg>,
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    loop {
        let cmd = {
            let guard = rx.lock().unwrap();
            guard.recv()
        };
        let Ok(cmd) = cmd else { break };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match cmd {
                NetCmd::WebArtFor { path, artist, album } => {
                    let bytes = fetch_web_art(&artist, &album).and_then(|b| normalize_art(&b));
                    let _ = tx.send(Msg::ArtWebFor { path, bytes });
                }
                NetCmd::WebArtGroup { gen, paths, artist, album } => {
                    if gen == generation.load(Ordering::Relaxed) {
                        let bytes = fetch_web_art(&artist, &album).and_then(|b| normalize_art(&b));
                        if gen == generation.load(Ordering::Relaxed) {
                            let _ = tx.send(Msg::ArtWebGroup { paths, bytes });
                        }
                    }
                }
                NetCmd::WebArtSong { gen, paths, artist, title } => {
                    if gen == generation.load(Ordering::Relaxed) {
                        let bytes = fetch_song_art(&artist, &title).and_then(|b| normalize_art(&b));
                        if gen == generation.load(Ordering::Relaxed) {
                            let _ = tx.send(Msg::ArtWebGroup { paths, bytes });
                        }
                    }
                }
                _ => {}
            }
        }));
    }
}

/// Sequential MusicBrainz ratings worker. MusicBrainz allows ~1 request per
/// second, so running more threads would not help. Each album takes two
/// requests (release search + release lookup with recordings/ratings).
pub fn ratings_loop(
    rx: Receiver<RatingCmd>,
    tx: Sender<Msg>,
    cancel: std::sync::Arc<AtomicBool>,
    generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
) {
    while let Ok(cmd) = rx.recv() {
        let RatingCmd::Album { gen, key, artist, album, paths } = cmd;
        if cancel.load(Ordering::Relaxed)
            || gen != generation.load(std::sync::atomic::Ordering::Relaxed)
        {
            continue;
        }
        let result = fetch_album_ratings(&artist, &album);
        if cancel.load(Ordering::Relaxed)
            || gen != generation.load(std::sync::atomic::Ordering::Relaxed)
        {
            continue;
        }
        let (album_stars, tracks) = result.unwrap_or((0, Vec::new()));
        let _ = tx.send(Msg::RatingsAlbum { gen, key, paths, album_stars, tracks });
    }
}

pub fn yt_loop(
    rx: Receiver<YtCmd>,
    tx: Sender<Msg>,
    album_index: std::sync::Arc<HashMap<String, String>>,
) {
    while let Ok(cmd) = rx.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match cmd {
                YtCmd::Download { query, auto_play, chunks, group } => {
                    let r = yt_download_and_store(&query, chunks, group, &album_index, &tx);
                    match r {
                        Ok(path) => { let _ = tx.send(Msg::YtDone { path, auto_play }); }
                        Err(e) => { let _ = tx.send(Msg::YtFail { err: e, auto_play }); }
                    }
                }
                YtCmd::Resolve { idx, query } => {
                    let display = yt_resolve_title(&query);
                    let _ = tx.send(Msg::YtResolved { idx, query, display });
                }
            }
        }));
    }
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_' || *b == b'.' || *b == b'~' {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn http_client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent(format!("{}/{} (album art lookup)", APP_NAME, APP_VERSION))
        .build()
        .ok()
}

// ---------------------------------------------------------------------------
// Artwork sources
//
// iTunes is fast and has excellent mainstream coverage; Deezer fills in a lot
// of electronic/indie/European releases; MusicBrainz + Cover Art Archive covers
// plenty of obscure and classical albums. The first usable image wins.
// ---------------------------------------------------------------------------

pub fn fetch_web_art(artist: &str, album: &str) -> Option<Vec<u8>> {
    let artist = artist.trim();
    let album = album.trim();
    if is_placeholder_meta(album) {
        return None;
    }
    // A known album with an unknown/Various-Artists artist is still worth an
    // album-only search (compilations and untagged rips).
    let search_artist = if is_placeholder_meta(artist) || artist.eq_ignore_ascii_case("various artists") {
        ""
    } else {
        artist
    };
    let client = http_client()?;
    let mut urls: Vec<String> = Vec::new();
    if let Some(u) = itunes_art_url(&client, search_artist, album) {
        urls.push(u);
    }
    if let Some(u) = deezer_art_url(&client, search_artist, album) {
        urls.push(u);
    }
    for u in cover_art_archive_urls(&client, search_artist, album) {
        urls.push(u);
    }
    if let Some(u) = theaudiodb_art_url(&client, search_artist, album) {
        urls.push(u);
    }
    if let Some(info) = discogs_info(search_artist, album) {
        if let Some(u) = info.art_url {
            urls.push(u);
        }
    }
    for url in urls {
        if let Some(bytes) = download_art(&client, &url) {
            return Some(bytes);
        }
    }
    None
}

fn download_art(client: &reqwest::blocking::Client, url: &str) -> Option<Vec<u8>> {
    let res = client.get(url).send().ok()?;
    if !res.status().is_success() {
        return None;
    }
    let bytes = res.bytes().ok()?.to_vec();
    if art_bytes_ok(&bytes, 100) {
        Some(bytes)
    } else {
        None
    }
}

/// Last-resort cover lookup for a single song whose album metadata is
/// missing. It searches by artist + track title instead of album.
pub fn fetch_song_art(artist: &str, title: &str) -> Option<Vec<u8>> {
    let artist = artist.trim();
    let title = title.trim();
    if is_placeholder_meta(title) {
        return None;
    }
    let client = http_client()?;
    let mut urls: Vec<String> = Vec::new();
    if let Some(u) = itunes_song_art_url(&client, artist, title) {
        urls.push(u);
    }
    if let Some(u) = deezer_song_art_url(&client, artist, title) {
        urls.push(u);
    }
    for u in mb_song_art_urls(&client, artist, title) {
        urls.push(u);
    }
    for url in urls {
        if let Some(bytes) = download_art(&client, &url) {
            return Some(bytes);
        }
    }
    None
}

/// MusicBrainz recording search, then the release/release-group Cover Art
/// Archive image. Useful for singles and live tracks whose album tag is
/// missing or too generic for the album search.
fn mb_song_art_urls(
    client: &reqwest::blocking::Client,
    artist: &str,
    title: &str,
) -> Vec<String> {
    let query = if is_placeholder_meta(artist) {
        format!("recording:\"{}\"", mb_escape(title))
    } else {
        format!(
            "recording:\"{}\" AND artist:\"{}\"",
            mb_escape(title),
            mb_escape(artist)
        )
    };
    let Some(json) = mb_get_json(
        client,
        "https://musicbrainz.org/ws/2/recording/",
        &[
            ("query", query),
            ("fmt", "json".to_string()),
            ("limit", "5".to_string()),
            ("inc", "releases+release-groups".to_string()),
        ],
        1,
    ) else {
        return Vec::new();
    };
    let Some((release, group)) = parse_mb_song_ids(&json, artist, title) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(id) = release {
        out.push(format!("https://coverartarchive.org/release/{}/front-500", id));
    }
    if let Some(id) = group {
        out.push(format!("https://coverartarchive.org/release-group/{}/front-500", id));
    }
    out
}

/// Best release/release-group for a recording match.
pub fn parse_mb_song_ids(
    json: &serde_json::Value,
    artist: &str,
    title: &str,
) -> Option<(Option<String>, Option<String>)> {
    let recordings = json.get("recordings")?.as_array()?;
    let mut best: Option<(i32, Option<String>, Option<String>)> = None;
    for r in recordings {
        let rt = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let ra = r
            .get("artist-credit")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let score = match_score(artist, title, ra, rt);
        if score < 3 {
            continue;
        }
        let Some(releases) = r.get("releases").and_then(|v| v.as_array()) else {
            continue;
        };
        for rel in releases {
            let id = rel.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
            let group = rel
                .get("release-group")
                .and_then(|g| g.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if id.is_none() && group.is_none() {
                continue;
            }
            let mut s = score;
            if rel.get("status").and_then(|v| v.as_str()) == Some("Official") {
                s += 1;
            }
            if best.as_ref().map_or(true, |(bs, _, _)| s > *bs) {
                best = Some((s, id, group));
            }
        }
    }
    best.map(|(_, id, group)| (id, group))
}

fn itunes_song_art_url(client: &reqwest::blocking::Client, artist: &str, title: &str) -> Option<String> {
    let term = if is_placeholder_meta(artist) {
        title.to_string()
    } else {
        format!("{} {}", artist, title)
    };
    let res = client
        .get("https://itunes.apple.com/search")
        .query(&[("term", term), ("entity", "song".to_string()), ("limit", "8".to_string())])
        .send()
        .ok()?;
    let json: serde_json::Value = res.json().ok()?;
    parse_itunes_song_art(&json, artist, title)
}

pub fn parse_itunes_song_art(json: &serde_json::Value, artist: &str, title: &str) -> Option<String> {
    let results = json.get("results")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in results {
        let ca = r.get("artistName").and_then(|v| v.as_str()).unwrap_or("");
        let tr = r.get("trackName").and_then(|v| v.as_str()).unwrap_or("");
        let score = match_score(artist, title, ca, tr);
        if score < 3 {
            continue;
        }
        if let Some(url) = r.get("artworkUrl100").and_then(|v| v.as_str()) {
            let url = url.replace("100x100bb", "600x600bb");
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, url));
            }
        }
    }
    best.map(|(_, u)| u)
}

fn deezer_song_art_url(client: &reqwest::blocking::Client, artist: &str, title: &str) -> Option<String> {
    let q = if is_placeholder_meta(artist) {
        title.to_string()
    } else {
        format!("{} {}", artist, title)
    };
    let res = client
        .get("https://api.deezer.com/search")
        .query(&[("q", q), ("limit", "8".to_string())])
        .send()
        .ok()?;
    let json: serde_json::Value = res.json().ok()?;
    parse_deezer_song_art(&json, artist, title)
}

pub fn parse_deezer_song_art(json: &serde_json::Value, artist: &str, title: &str) -> Option<String> {
    let results = json.get("data")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in results {
        let ca = r.get("artist").and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or("");
        let tr = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let score = match_score(artist, title, ca, tr);
        if score < 3 {
            continue;
        }
        let url = r
            .get("album")
            .and_then(|a| a.get("cover_xl"))
            .and_then(|v| v.as_str())
            .or_else(|| r.get("album").and_then(|a| a.get("cover_big")).and_then(|v| v.as_str()));
        if let Some(url) = url {
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, url.to_string()));
            }
        }
    }
    best.map(|(_, u)| u)
}

fn itunes_search(client: &reqwest::blocking::Client, term: &str) -> Option<serde_json::Value> {
    let res = client
        .get("https://itunes.apple.com/search")
        .query(&[("term", term.to_string()), ("entity", "album".to_string()), ("limit", "8".to_string())])
        .send()
        .ok()?;
    res.json().ok()
}

fn itunes_art_url(client: &reqwest::blocking::Client, artist: &str, album: &str) -> Option<String> {
    let json = itunes_search(client, &format!("{} {}", artist, album))?;
    parse_itunes_art(&json, artist, album)
}

pub fn parse_itunes_art(json: &serde_json::Value, artist: &str, album: &str) -> Option<String> {
    let results = json.get("results")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in results {
        let ca = r.get("artistName").and_then(|v| v.as_str()).unwrap_or("");
        let al = r.get("collectionName").and_then(|v| v.as_str()).unwrap_or("");
        let score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        if let Some(url) = r.get("artworkUrl100").and_then(|v| v.as_str()) {
            let url = url.replace("100x100bb", "600x600bb");
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, url));
            }
        }
    }
    best.map(|(_, u)| u)
}

fn deezer_art_url(client: &reqwest::blocking::Client, artist: &str, album: &str) -> Option<String> {
    let res = client
        .get("https://api.deezer.com/search/album")
        .query(&[("q", format!("{} {}", artist, album)), ("limit", "8".to_string())])
        .send()
        .ok()?;
    let json: serde_json::Value = res.json().ok()?;
    parse_deezer_art(&json, artist, album)
}

pub fn parse_deezer_art(json: &serde_json::Value, artist: &str, album: &str) -> Option<String> {
    let results = json.get("data")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in results {
        let ca = r.get("artist").and_then(|a| a.get("name")).and_then(|v| v.as_str()).unwrap_or("");
        let al = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        let url = r
            .get("cover_xl")
            .and_then(|v| v.as_str())
            .or_else(|| r.get("cover_big").and_then(|v| v.as_str()));
        if let Some(url) = url {
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, url.to_string()));
            }
        }
    }
    best.map(|(_, u)| u)
}

/// MusicBrainz's API asks for at most one request per second per client.
static MB_LAST: std::sync::OnceLock<std::sync::Mutex<Instant>> = std::sync::OnceLock::new();

fn mb_throttle() {
    let lock = MB_LAST.get_or_init(|| std::sync::Mutex::new(Instant::now() - Duration::from_secs(2)));
    if let Ok(mut last) = lock.lock() {
        let since = last.elapsed();
        if since < Duration::from_millis(1100) {
            std::thread::sleep(Duration::from_millis(1100) - since);
        }
        *last = Instant::now();
    }
}

/// MusicBrainz occasionally answers a burst with 503/429. Retry with backoff
/// (honouring Retry-After when present) so a large Meta All run does not lose
/// albums to a transient throttle.
fn mb_get_json(
    client: &reqwest::blocking::Client,
    url: &str,
    params: &[(&str, String)],
    retries: u64,
) -> Option<serde_json::Value> {
    for attempt in 0..retries {
        mb_throttle();
        let res = client.get(url).query(params).send().ok()?;
        if res.status().is_success() {
            return res.json().ok();
        }
        let code = res.status().as_u16();
        if code == 429 || code == 503 {
            let wait = res
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(2 + attempt * 3)
                .clamp(1, 15);
            std::thread::sleep(Duration::from_secs(wait));
            continue;
        }
        return None;
    }
    None
}

/// Cover Art Archive URLs for the best matching release. The release-specific
/// front image is tried first; when that release has no art, the release-group
/// front often does. Both come from the same MusicBrainz search.
fn cover_art_archive_urls(
    client: &reqwest::blocking::Client,
    artist: &str,
    album: &str,
) -> Vec<String> {
    let query = if is_placeholder_meta(artist) {
        format!("release:\"{}\"", album)
    } else {
        format!("release:\"{}\" AND artist:\"{}\"", album, artist)
    };
    let Some(json) = mb_get_json(
        client,
        "https://musicbrainz.org/ws/2/release/",
        &[
            ("query", query),
            ("fmt", "json".to_string()),
            ("limit", "8".to_string()),
        ],
        1,
    ) else {
        return Vec::new();
    };
    let Some((release, group)) = parse_musicbrainz_ids(&json, artist, album) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(id) = release {
        out.push(format!("https://coverartarchive.org/release/{}/front-500", id));
    }
    if let Some(id) = group {
        out.push(format!("https://coverartarchive.org/release-group/{}/front-500", id));
    }
    out
}

/// Release and release-group IDs for the best matching MusicBrainz release.
pub fn parse_musicbrainz_ids(
    json: &serde_json::Value,
    artist: &str,
    album: &str,
) -> Option<(Option<String>, Option<String>)> {
    let releases = json.get("releases")?.as_array()?;
    let mut best: Option<(i32, Option<String>, Option<String>)> = None;
    for r in releases {
        let al = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let ca = r
            .get("artist-credit")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        if r.get("status").and_then(|v| v.as_str()) == Some("Official") {
            score += 1;
        }
        if r.get("country").and_then(|v| v.as_str()).is_some() {
            score += 1;
        }
        let id = r.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let group = r
            .get("release-group")
            .and_then(|g| g.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if id.is_none() && group.is_none() {
            continue;
        }
        if best.as_ref().map_or(true, |(s, _, _)| score > *s) {
            best = Some((score, id, group));
        }
    }
    best.map(|(_, id, group)| (id, group))
}

pub fn parse_musicbrainz_id(json: &serde_json::Value, artist: &str, album: &str) -> Option<String> {
    parse_musicbrainz_ids(json, artist, album).and_then(|(id, _)| id)
}

fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 0 means "not the same album". Anything >= 3 is a plausible match; a higher
/// score means a better one.
/// TheAudioDB is the last-resort cover source: a free community database that
/// often has artwork for releases the big services miss. The public test key
/// (`2`) is rate-limited, so it is tried only after iTunes/Deezer/CAA.
fn theaudiodb_art_url(client: &reqwest::blocking::Client, artist: &str, album: &str) -> Option<String> {
    let mut req = client
        .get("https://www.theaudiodb.com/api/v1/json/2/searchalbum.php")
        .query(&[("a", album)]);
    if !is_placeholder_meta(artist) {
        req = req.query(&[("s", artist)]);
    }
    let res = req.send().ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().ok()?;
    parse_theaudiodb_art(&json, artist, album)
}

pub fn parse_theaudiodb_art(json: &serde_json::Value, artist: &str, album: &str) -> Option<String> {
    let list = json.get("album")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in list {
        let ca = r.get("strArtist").and_then(|v| v.as_str()).unwrap_or("");
        let al = r.get("strAlbum").and_then(|v| v.as_str()).unwrap_or("");
        let score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        let url = r
            .get("strAlbumThumbHQ")
            .and_then(|v| v.as_str())
            .filter(|u| !u.is_empty())
            .or_else(|| {
                r.get("strAlbumThumb")
                    .and_then(|v| v.as_str())
                    .filter(|u| !u.is_empty())
            });
        if let Some(url) = url {
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, url.to_string()));
            }
        }
    }
    best.map(|(_, u)| u)
}

// ---------------------------------------------------------------------------
// Discogs artwork/rating fallback
//
// Discogs has broad physical-release coverage, which fills a lot of the gaps
// left by iTunes/Deezer/MusicBrainz. The public API allows 25 requests/minute
// without a token, so every request is throttled and results are cached per
// album for the session.
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct DiscogsInfo {
    art_url: Option<String>,
    rating: Option<(f32, u32)>,
}

fn discogs_client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!(
            "{}/{} +https://github.com/drDOOM69GAMING/MediaPlayerDOOMv3-Linux-",
            APP_NAME, APP_VERSION
        ))
        .build()
        .ok()
}

static DISCOGS_LAST: std::sync::OnceLock<std::sync::Mutex<Instant>> =
    std::sync::OnceLock::new();

fn discogs_throttle() {
    let lock = DISCOGS_LAST
        .get_or_init(|| std::sync::Mutex::new(Instant::now() - Duration::from_secs(10)));
    if let Ok(mut last) = lock.lock() {
        let since = last.elapsed();
        if since < Duration::from_millis(2500) {
            std::thread::sleep(Duration::from_millis(2500) - since);
        }
        *last = Instant::now();
    }
}

static DISCOGS_CACHE: std::sync::OnceLock<
    std::sync::Mutex<HashMap<String, Option<DiscogsInfo>>>,
> = std::sync::OnceLock::new();

fn discogs_info(artist: &str, album: &str) -> Option<DiscogsInfo> {
    let key = format!("{}|{}", normalize_name(artist), normalize_name(album));
    if key.trim_matches('|').is_empty() {
        return None;
    }
    let cache = DISCOGS_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Ok(c) = cache.lock() {
        if let Some(v) = c.get(&key) {
            return v.clone();
        }
    }
    let info = fetch_discogs_info(artist, album);
    if let Ok(mut c) = cache.lock() {
        c.insert(key, info.clone());
    }
    info
}

fn discogs_get_json(
    client: &reqwest::blocking::Client,
    url: &str,
    params: &[(&str, String)],
) -> Option<serde_json::Value> {
    for attempt in 0..3 {
        discogs_throttle();
        let res = client.get(url).query(params).send().ok()?;
        if res.status().is_success() {
            return res.json().ok();
        }
        let code = res.status().as_u16();
        if code == 429 || code == 503 {
            let wait = res
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(3 + attempt * 3)
                .clamp(1, 20);
            std::thread::sleep(Duration::from_secs(wait));
            continue;
        }
        return None;
    }
    None
}

fn fetch_discogs_info(artist: &str, album: &str) -> Option<DiscogsInfo> {
    let client = discogs_client()?;
    let mut params: Vec<(&str, String)> = vec![
        ("release_title", album.to_string()),
        ("type", "release".to_string()),
        ("per_page", "8".to_string()),
    ];
    if !is_placeholder_meta(artist) {
        params.push(("artist", artist.to_string()));
    }
    let json = discogs_get_json(&client, "https://api.discogs.com/database/search", &params)?;
    let (resource, cover) = parse_discogs_search(&json, artist, album)?;
    let mut info = DiscogsInfo {
        art_url: cover,
        rating: None,
    };
    if !resource.is_empty() {
        if let Some(detail) = discogs_get_json(&client, &resource, &[]) {
            let (art, rating) = parse_discogs_release(&detail);
            if art.is_some() {
                info.art_url = art;
            }
            info.rating = rating;
        }
    }
    Some(info)
}

pub fn parse_discogs_search(
    json: &serde_json::Value,
    artist: &str,
    album: &str,
) -> Option<(String, Option<String>)> {
    let results = json.get("results")?.as_array()?;
    let mut best: Option<(i32, String, Option<String>)> = None;
    for r in results {
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let (ca, al) = title.split_once(" - ").unwrap_or(("", title));
        let ca = ca.split(" (").next().unwrap_or(ca).trim();
        let mut score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        // A picture-disc release often has the disc photo as its primary
        // image. Prefer a normal release when both match, and use the
        // community have-count as a tie-breaker.
        let picture_disc = r
            .get("format")
            .and_then(|v| v.as_array())
            .map(|formats| {
                formats.iter().any(|f| {
                    f.as_str()
                        .map(|s| s.eq_ignore_ascii_case("Picture Disc"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if picture_disc {
            score -= 2;
        }
        let have = r
            .get("community")
            .and_then(|c| c.get("have"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if have > 5000 {
            score += 1;
        }
        let resource = r
            .get("resource_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cover = r
            .get("cover_image")
            .and_then(|v| v.as_str())
            .filter(|u| !u.is_empty())
            .map(|s| s.to_string());
        if best.as_ref().map_or(true, |(s, _, _)| score > *s) {
            best = Some((score, resource, cover));
        }
    }
    best.map(|(_, resource, cover)| (resource, cover))
}

pub fn parse_discogs_release(json: &serde_json::Value) -> (Option<String>, Option<(f32, u32)>) {
    let art = json
        .get("images")
        .and_then(|v| v.as_array())
        .and_then(|images| {
            images
                .iter()
                .find(|i| i.get("type").and_then(|v| v.as_str()) == Some("primary"))
                .or_else(|| images.first())
        })
        .and_then(|img| img.get("uri"))
        .and_then(|v| v.as_str())
        .filter(|u| !u.is_empty())
        .map(|s| s.to_string());
    let rating = json
        .get("community")
        .and_then(|c| c.get("rating"))
        .and_then(|r| {
            let avg = r.get("average").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let count = r.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            if avg > 0.0 && count > 0 {
                Some((avg, count))
            } else {
                None
            }
        });
    (art, rating)
}

fn discogs_album_rating(artist: &str, album: &str) -> Option<u8> {
    let info = discogs_info(artist, album)?;
    let (average, count) = info.rating?;
    if count == 0 {
        return None;
    }
    Some((average.round() as u8).clamp(1, 5))
}

fn theaudiodb_rating(artist: &str, album: &str) -> Option<u8> {
    let client = http_client()?;
    let mut req = client
        .get("https://www.theaudiodb.com/api/v1/json/2/searchalbum.php")
        .query(&[("a", album)]);
    if !is_placeholder_meta(artist) {
        req = req.query(&[("s", artist)]);
    }
    let res = req.send().ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().ok()?;
    parse_theaudiodb_rating(&json, artist, album)
}

pub fn parse_theaudiodb_rating(
    json: &serde_json::Value,
    artist: &str,
    album: &str,
) -> Option<u8> {
    let list = json.get("album")?.as_array()?;
    let mut best: Option<(i32, u8)> = None;
    for r in list {
        let ca = r.get("strArtist").and_then(|v| v.as_str()).unwrap_or("");
        let al = r.get("strAlbum").and_then(|v| v.as_str()).unwrap_or("");
        let score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        let raw = r.get("intScore").and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_f64().map(|f| f.to_string()))
        });
        let Some(score10) = raw.and_then(|s| s.parse::<f32>().ok()) else {
            continue;
        };
        if score10 <= 0.0 {
            continue;
        }
        let stars = (score10 / 2.0).round().clamp(1.0, 5.0) as u8;
        if best.as_ref().map_or(true, |(s, _)| score > *s) {
            best = Some((score, stars));
        }
    }
    best.map(|(_, stars)| stars)
}

// ---------------------------------------------------------------------------
// MusicBrainz community ratings (1-5 stars)
//
// MusicBrainz exposes public community ratings for release groups (albums) and
// recordings (songs). One release lookup returns both, so an album costs two
// requests: a release search and the release lookup.
// ---------------------------------------------------------------------------

fn mb_escape(s: &str) -> String {
    s.replace('\\', " ").replace('"', " ").trim().to_string()
}

fn mb_stars(value: f64) -> u8 {
    if value <= 0.0 {
        0
    } else {
        (value.round() as i64).clamp(1, 5) as u8
    }
}

/// Normalized key used to match a local track title to a MusicBrainz track.
pub fn rating_title_key(title: &str) -> String {
    let mut s = normalize_name(title);
    // Common suffixes that differ between local rips and MusicBrainz.
    for junk in ["remastered", "remaster", "deluxe edition", "deluxe", "bonus track", "edition", "explicit"] {
        s = s.replace(junk, "");
    }
    s
}

fn mb_release_search(client: &reqwest::blocking::Client, artist: &str, album: &str) -> Option<String> {
    let query = if is_placeholder_meta(artist) {
        format!("release:\"{}\"", mb_escape(album))
    } else {
        format!("release:\"{}\" AND artist:\"{}\"", mb_escape(album), mb_escape(artist))
    };
    let json = mb_get_json(
        client,
        "https://musicbrainz.org/ws/2/release/",
        &[
            ("query", query),
            ("fmt", "json".to_string()),
            ("limit", "8".to_string()),
        ],
        4,
    )?;
    if let Some(id) = parse_mb_release_id(&json, artist, album) {
        return Some(id);
    }
    // Compilations and "Various Artists" folders often have a track artist
    // that is not the release artist. Retry album-only before giving up.
    if !is_placeholder_meta(artist) {
        let query = format!("release:\"{}\"", mb_escape(album));
        let json = mb_get_json(
            client,
            "https://musicbrainz.org/ws/2/release/",
            &[
                ("query", query),
                ("fmt", "json".to_string()),
                ("limit", "8".to_string()),
            ],
            4,
        )?;
        return parse_mb_release_id(&json, "", album);
    }
    None
}

pub fn parse_mb_release_id(json: &serde_json::Value, artist: &str, album: &str) -> Option<String> {
    let releases = json.get("releases")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in releases {
        let al = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let ca = r
            .get("artist-credit")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        if r.get("status").and_then(|v| v.as_str()) == Some("Official") {
            score += 1;
        }
        if r.get("country").and_then(|v| v.as_str()).is_some() {
            score += 1;
        }
        if let Some(id) = r.get("id").and_then(|v| v.as_str()) {
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, id.to_string()));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// Fetch per-song and album community ratings for one album. Returns
/// `(album_stars, [(normalized track title, stars)])`, or `None` when no
/// release matched. A `(0, [])` result means "matched but nobody rated it".
pub fn fetch_album_ratings(artist: &str, album: &str) -> Option<(u8, Vec<(String, u8)>)> {
    let author = artist.trim();
    let album = album.trim();
    if album.is_empty() || is_placeholder_meta(album) {
        return None;
    }
    let client = http_client()?;
    let mb = mb_release_search(&client, author, album).and_then(|id| {
        mb_get_json(
            &client,
            &format!(
                "https://musicbrainz.org/ws/2/release/{}?inc=recordings+ratings+release-groups&fmt=json",
                id
            ),
            &[],
            4,
        )
    });
    let (mut album_stars, tracks) = mb
        .as_ref()
        .map(parse_album_ratings)
        .unwrap_or((0, Vec::new()));
    // MusicBrainz ratings are sparse. TheAudioDB and Discogs both expose a
    // community score; try them before giving the album up as unrated.
    if album_stars == 0 {
        if let Some(stars) =
            theaudiodb_rating(author, album).or_else(|| discogs_album_rating(author, album))
        {
            album_stars = stars;
        }
    }
    if album_stars == 0 && tracks.is_empty() {
        None
    } else {
        Some((album_stars, tracks))
    }
}

pub fn parse_album_ratings(json: &serde_json::Value) -> (u8, Vec<(String, u8)>) {
    let album_stars = json
        .get("release-group")
        .and_then(|g| g.get("rating"))
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_f64())
        .map(mb_stars)
        .unwrap_or(0);
    let mut tracks: Vec<(String, u8)> = Vec::new();
    if let Some(media) = json.get("media").and_then(|v| v.as_array()) {
        for m in media {
            let Some(list) = m.get("tracks").and_then(|v| v.as_array()) else { continue };
            for t in list {
                let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("");
                if title.is_empty() {
                    continue;
                }
                if let Some(value) = t
                    .get("recording")
                    .and_then(|r| r.get("rating"))
                    .and_then(|r| r.get("value"))
                    .and_then(|v| v.as_f64())
                {
                    let stars = mb_stars(value);
                    if stars > 0 {
                        tracks.push((rating_title_key(title), stars));
                    }
                }
            }
        }
    }
    (album_stars, tracks)
}

pub fn match_score(artist: &str, album: &str, cand_artist: &str, cand_album: &str) -> i32 {
    let a = normalize_name(album);
    let ca = normalize_name(cand_album);
    if a.is_empty() || ca.is_empty() {
        return 0;
    }
    let mut score = if ca == a {
        6
    } else if ca.contains(&a) || a.contains(&ca) {
        3
    } else {
        return 0;
    };
    let ar = normalize_name(artist);
    let car = normalize_name(cand_artist);
    if !ar.is_empty() && !car.is_empty() {
        if car == ar {
            score += 4;
        } else if car.contains(&ar) || ar.contains(&car) {
            score += 2;
        } else {
            score -= 2;
        }
    }
    score
}

pub fn fetch_lyrics(artist: &str, title: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().ok()?;
    let url = format!("https://api.lyrics.ovh/v1/{}/{}", percent_encode(artist), percent_encode(title));
    let res = client.get(&url).send().ok()?;
    if !res.status().is_success() { return None; }
    let json: serde_json::Value = res.json().ok()?;
    json.get("lyrics").and_then(|l| l.as_str()).map(|s| s.to_string())
}

/// Numeric parts of a version string. A leading `v` and any suffix such as
/// `-beta` are ignored, so `v3.2.41` and `3.2.41-rc1` both read as 3.2.41.
fn version_parts(s: &str) -> Vec<u32> {
    s.trim()
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().unwrap_or(0))
        .collect()
}

/// True when `latest` is a later release than `current`.
pub fn version_newer(latest: &str, current: &str) -> bool {
    let a = version_parts(latest);
    let b = version_parts(current);
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// Ask GitHub for the latest release. Returns `(version, release page)` only
/// when it is newer than the running build; a rate-limited or offline request
/// simply returns `None` and the app carries on.
pub fn check_latest_release(current: &str) -> Option<(String, String)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("{}/{} (update check)", APP_NAME, APP_VERSION))
        .build()
        .ok()?;
    let res = client
        .get("https://api.github.com/repos/drDOOM69GAMING/MediaPlayerDOOMv3-Linux-/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().ok()?;
    let tag = json.get("tag_name").and_then(|v| v.as_str())?;
    let url = json
        .get("html_url")
        .and_then(|v| v.as_str())
        .unwrap_or("https://github.com/drDOOM69GAMING/MediaPlayerDOOMv3-Linux-/releases/latest");
    if version_newer(tag, current) {
        Some((tag.trim_start_matches('v').to_string(), url.to_string()))
    } else {
        None
    }
}

#[cfg(test)]
mod art_tests {
    use super::*;

    #[test]
    fn album_matching_rejects_unrelated_results() {
        assert_eq!(match_score("Tool", "Lateralus", "Tool", "Lateralus"), 10);
        assert!(match_score("Tool", "Lateralus", "Tool", "Lateralus (Deluxe)") >= 3);
        assert_eq!(match_score("Tool", "Lateralus", "A Perfect Circle", "Mer de Noms"), 0);
        // Same album title with a different artist is only a weak match.
        assert!(match_score("Tool", "Greatest Hits", "Queen", "Greatest Hits") > 0);
        assert_eq!(match_score("", "", "Tool", "Lateralus"), 0);
    }

    #[test]
    fn discogs_picks_matching_release_and_reads_rating() {
        let search = serde_json::json!({
          "results": [
            {"title":"A Perfect Circle - Mer de Noms",
             "resource_url":"https://api.discogs.com/releases/1",
             "cover_image":"https://example/wrong.jpg"},
            {"title":"Tool (2) - Lateralus",
             "resource_url":"https://api.discogs.com/releases/513594",
             "cover_image":""}
          ]
        });
        let (resource, cover) = parse_discogs_search(&search, "Tool", "Lateralus").unwrap();
        assert_eq!(resource, "https://api.discogs.com/releases/513594");
        assert!(cover.is_none());

        let release = serde_json::json!({
          "images": [
            {"type":"secondary","uri":"https://example/back.jpg"},
            {"type":"primary","uri":"https://example/front.jpg"}
          ],
          "community": {"rating": {"average": 4.6, "count": 3272}}
        });
        let (art, rating) = parse_discogs_release(&release);
        assert_eq!(art.as_deref(), Some("https://example/front.jpg"));
        let (average, count) = rating.unwrap();
        assert!((average - 4.6).abs() < 0.01);
        assert_eq!(count, 3272);
    }

    #[test]
    fn musicbrainz_extracts_release_group_id() {
        let json = serde_json::json!({
          "releases": [
            {"id":"rel-1","title":"Lateralus","status":"Official","country":"US",
             "artist-credit":[{"name":"Tool"}],
             "release-group":{"id":"rg-1"}}
          ]
        });
        let (release, group) = parse_musicbrainz_ids(&json, "Tool", "Lateralus").unwrap();
        assert_eq!(release.as_deref(), Some("rel-1"));
        assert_eq!(group.as_deref(), Some("rg-1"));
        assert_eq!(
            parse_musicbrainz_id(&json, "Tool", "Lateralus").as_deref(),
            Some("rel-1")
        );
    }

    #[test]
    #[ignore]
    fn live_missing_art_sample() {
        let path = crate::util::data_dir().join("metacache.json");
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("no metacache");
            return;
        };
        let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&text) else {
            eprintln!("bad metacache");
            return;
        };
        let mut seen = std::collections::HashSet::new();
        let mut checked = 0;
        for (p, e) in map {
            if crate::library::art_cache_ok(&p) {
                continue;
            }
            let artist = e.get("artist").and_then(|v| v.as_str()).unwrap_or("");
            let album = e.get("album").and_then(|v| v.as_str()).unwrap_or("");
            if album.is_empty() {
                continue;
            }
            let key = format!("{}|{}", artist, album);
            if !seen.insert(key) {
                continue;
            }
            eprintln!("--- {artist} / {album}");
            let art = fetch_web_art(artist, album);
            eprintln!("  album art: {}", art.as_ref().map(|b| b.len()).unwrap_or(0));
            let title = e.get("title").and_then(|v| v.as_str()).unwrap_or("");
            if art.is_none() && !title.is_empty() {
                let song = fetch_song_art(artist, title);
                eprintln!("  song art: {}", song.as_ref().map(|b| b.len()).unwrap_or(0));
            }
            checked += 1;
            if checked >= 8 {
                break;
            }
        }
        eprintln!("checked {checked} missing albums");
    }

    #[test]
    fn musicbrainz_song_fallback_extracts_release_ids() {
        let json = serde_json::json!({
          "recordings": [
            {"title":"Lateralus","artist-credit":[{"name":"Tool"}],
             "releases":[
               {"id":"rel-1","status":"Official","release-group":{"id":"rg-1"}}
             ]}
          ]
        });
        let (release, group) = parse_mb_song_ids(&json, "Tool", "Lateralus").unwrap();
        assert_eq!(release.as_deref(), Some("rel-1"));
        assert_eq!(group.as_deref(), Some("rg-1"));
    }

    #[test]
    fn theaudiodb_parses_score_out_of_ten() {
        let json = serde_json::json!({
          "album": [
            {"strArtist":"Tool","strAlbum":"Lateralus","intScore":"8.4"}
          ]
        });
        assert_eq!(
            parse_theaudiodb_rating(&json, "Tool", "Lateralus"),
            Some(4)
        );
    }

    #[test]
    fn itunes_picks_the_matching_album_not_the_first_result() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "resultCount": 2,
          "results": [
            {"artistName":"A Perfect Circle","collectionName":"Mer de Noms",
             "artworkUrl100":"https://example/wrong/100x100bb.jpg"},
            {"artistName":"Tool","collectionName":"Lateralus",
             "artworkUrl100":"https://example/right/100x100bb.jpg"}
          ]}"#).unwrap();
        let url = parse_itunes_art(&json, "Tool", "Lateralus").unwrap();
        assert!(url.contains("/right/"));
        assert!(url.contains("600x600bb"));
    }

    #[test]
    fn deezer_parses_high_resolution_cover() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "data": [
            {"title":"Mer de Noms","artist":{"name":"A Perfect Circle"},
             "cover_xl":"https://example/wrong.jpg"},
            {"title":"Lateralus","artist":{"name":"Tool"},
             "cover_xl":"https://example/right-1000.jpg","cover_big":"https://example/right-500.jpg"}
          ]}"#).unwrap();
        assert_eq!(parse_deezer_art(&json, "Tool", "Lateralus").unwrap(), "https://example/right-1000.jpg");
    }

    #[test]
    fn musicbrainz_parses_release_id() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "releases": [
            {"id":"wrong-id","title":"Mer de Noms","artist-credit":[{"name":"A Perfect Circle"}]},
            {"id":"right-id","title":"Lateralus","artist-credit":[{"name":"Tool"}]}
          ]}"#).unwrap();
        assert_eq!(parse_musicbrainz_id(&json, "Tool", "Lateralus").unwrap(), "right-id");
    }

    /// Live network check, run manually:
    /// `cargo test live_art_sources -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_art_sources() {
        let c = http_client().expect("client");
        for (artist, album) in [
            ("Tool", "Lateralus"),
            ("Perturbator", "Dangerous Days"),
            ("Godspeed You! Black Emperor", "Lift Your Skinny Fists Like Antennas to Heaven"),
            ("Type O Negative", "Bloody Kisses"),
            ("Carpenter Brut", "Trilogy"),
        ] {
            eprintln!("--- {artist} / {album}");
            eprintln!("  itunes: {:?}", itunes_art_url(&c, artist, album));
            eprintln!("  deezer: {:?}", deezer_art_url(&c, artist, album));
            eprintln!("  caa:    {:?}", cover_art_archive_urls(&c, artist, album));
            let bytes = fetch_web_art(artist, album);
            eprintln!("  result: {}", bytes.as_ref().map(|b| b.len()).unwrap_or(0));
            assert!(bytes.is_some(), "no art found for {artist} / {album}");
        }
    }

    #[test]
    fn itunes_song_art_uses_the_matching_track() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "results": [
            {"trackName":"Wrong Song","artistName":"Other","artworkUrl100":"https://example/wrong.jpg"},
            {"trackName":"Bedrock Anthem","artistName":"Weird Al Yankovic",
             "artworkUrl100":"https://example/right/100x100bb.jpg"}
          ]}"#).unwrap();
        assert_eq!(
            parse_itunes_song_art(&json, "Weird Al Yankovic", "Bedrock Anthem").unwrap(),
            "https://example/right/600x600bb.jpg"
        );
    }

    #[test]
    fn deezer_song_art_uses_the_matching_track() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "data": [
            {"title":"Wrong Song","artist":{"name":"Other"},"album":{"cover_xl":"https://example/wrong.jpg"}},
            {"title":"Bedrock Anthem","artist":{"name":"Weird Al Yankovic"},
             "album":{"cover_xl":"https://example/right.jpg"}}
          ]}"#).unwrap();
        assert_eq!(
            parse_deezer_song_art(&json, "Weird Al Yankovic", "Bedrock Anthem").unwrap(),
            "https://example/right.jpg"
        );
    }

    #[test]
    fn musicbrainz_album_and_track_ratings() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "release-group": {"rating": {"value": 4.45, "votes-count": 72}},
          "media": [{"tracks": [
            {"title":"Smells Like Teen Spirit","recording":{"rating":{"value":4.25,"votes-count":41}}},
            {"title":"In Bloom (Remastered)","recording":{"rating":{"value":3.95,"votes-count":22}}},
            {"title":"Unrated Song","recording":{}}
          ]}]}"#).unwrap();
        let (album, tracks) = parse_album_ratings(&json);
        assert_eq!(album, 4);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].1, 4);
        assert_eq!(tracks[1].0, rating_title_key("In Bloom"));
        assert_eq!(tracks[1].1, 4);
    }

    #[test]
    fn theaudiodb_parses_album_thumb() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "album": [
            {"strArtist":"A Perfect Circle","strAlbum":"Mer de Noms","strAlbumThumb":"https://example/wrong.jpg"},
            {"strArtist":"Tool","strAlbum":"Lateralus","strAlbumThumb":"https://example/right.jpg"}
          ]}"#).unwrap();
        assert_eq!(parse_theaudiodb_art(&json, "Tool", "Lateralus").unwrap(), "https://example/right.jpg");
    }

    /// Live network check, run manually:
    /// `cargo test live_album_ratings -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_album_ratings() {
        for (artist, album) in [
            ("Nirvana", "Nevermind"),
            ("Tool", "Lateralus"),
            ("Pink Floyd", "The Dark Side of the Moon"),
        ] {
            let r = fetch_album_ratings(artist, album);
            eprintln!("{artist} / {album}: {r:?}");
            assert!(r.is_some(), "no MusicBrainz release for {artist} / {album}");
        }
    }

    #[test]
    fn blank_and_tiny_images_are_rejected() {
        // 1x1 white PNG
        let tiny = include_bytes!("../assets/app_icon.jpg");
        assert!(art_bytes_ok(tiny, 32), "the real icon is a valid image");
        // A solid 200x200 white PNG is a common "no cover" placeholder.
        let img = image::RgbaImage::from_pixel(200, 200, image::Rgba([255, 255, 255, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(!art_bytes_ok(&png.into_inner(), 32), "solid placeholder must be rejected");
        assert!(!art_bytes_ok(b"not an image at all", 1));
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn newer_versions_win_and_equal_versions_do_not() {
        assert!(version_newer("v3.2.41", "3.2.40"));
        assert!(!version_newer("v3.2.40", "3.2.40"));
        assert!(!version_newer("3.2.39", "3.2.40"));
        assert!(version_newer("3.10.0", "3.9.9"));
        assert!(version_newer("3.2.41-rc1", "3.2.40"));
        assert!(!version_newer("3.2.41", "3.2.41-rc1"));
        assert!(!version_newer("", "3.2.41"));
    }
}
