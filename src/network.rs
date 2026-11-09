#![allow(unused_imports)]
#![allow(dead_code)]
// Net thread: yt-dlp + web art/lyrics lookup.
use std::collections::HashMap;
use std::path::PathBuf;
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
                NetCmd::WebArtFor { .. } => {
                    // "Meta All" web lookups are handled by the parallel
                    // web_art_loop pool, not the single net thread.
                }
                NetCmd::Lyrics(artist, title) => {
                    let text = fetch_lyrics(&artist, &title);
                    let _ = tx.send(Msg::Lyrics { artist, title, text });
                }
            }
        }));
    }
}

/// Dedicated worker pool for "Meta All" web art. Runs on its own channel so
/// several artwork lookups happen in parallel instead of one slow sequential
/// stream on the single net thread - a big library would otherwise sit at
/// "N web art left" for hours and never finish.
pub fn web_art_loop(rx: std::sync::Arc<std::sync::Mutex<std::sync::mpsc::Receiver<NetCmd>>>, tx: Sender<Msg>) {
    loop {
        let cmd = {
            let guard = rx.lock().unwrap();
            guard.recv()
        };
        let Ok(cmd) = cmd else { break };
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let NetCmd::WebArtFor { path, artist, album } = cmd {
                let bytes = fetch_web_art(&artist, &album).and_then(|b| normalize_art(&b));
                let _ = tx.send(Msg::ArtWebFor { path, bytes });
            }
        }));
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
    if is_placeholder_meta(artist) || is_placeholder_meta(album) {
        return None;
    }
    let client = http_client()?;
    let mut urls: Vec<String> = Vec::new();
    if let Some(u) = itunes_art_url(&client, artist, album) {
        urls.push(u);
    }
    if let Some(u) = deezer_art_url(&client, artist, album) {
        urls.push(u);
    }
    if let Some(u) = cover_art_archive_url(&client, artist, album) {
        urls.push(u);
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

fn cover_art_archive_url(client: &reqwest::blocking::Client, artist: &str, album: &str) -> Option<String> {
    mb_throttle();
    let query = format!("release:\"{}\" AND artist:\"{}\"", album, artist);
    let res = client
        .get("https://musicbrainz.org/ws/2/release/")
        .query(&[("query", query), ("fmt", "json".to_string()), ("limit", "8".to_string())])
        .send()
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().ok()?;
    let id = parse_musicbrainz_id(&json, artist, album)?;
    Some(format!("https://coverartarchive.org/release/{}/front-500", id))
}

pub fn parse_musicbrainz_id(json: &serde_json::Value, artist: &str, album: &str) -> Option<String> {
    let releases = json.get("releases")?.as_array()?;
    let mut best: Option<(i32, String)> = None;
    for r in releases {
        let al = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let ca = r
            .get("artist-credit")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|a| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let score = match_score(artist, album, ca, al);
        if score < 3 {
            continue;
        }
        if let Some(id) = r.get("id").and_then(|v| v.as_str()) {
            if best.as_ref().map_or(true, |(s, _)| score > *s) {
                best = Some((score, id.to_string()));
            }
        }
    }
    best.map(|(_, id)| id)
}

fn normalize_name(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 0 means "not the same album". Anything >= 3 is a plausible match; a higher
/// score means a better one.
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
            eprintln!("  caa:    {:?}", cover_art_archive_url(&c, artist, album));
            let bytes = fetch_web_art(artist, album);
            eprintln!("  result: {}", bytes.as_ref().map(|b| b.len()).unwrap_or(0));
            assert!(bytes.is_some(), "no art found for {artist} / {album}");
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
