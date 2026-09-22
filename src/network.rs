#![allow(unused_imports)]
#![allow(dead_code)]
// Net thread: yt-dlp + web art/lyrics lookup.
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;
use crate::tools::*;
use crate::types::*;
use crate::util::*;
use crate::ytdlp::*;
pub fn net_loop(rx: Receiver<NetCmd>, tx: Sender<Msg>) {
    while let Ok(cmd) = rx.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match cmd {
                NetCmd::WebArt(artist, album) => {
                    let bytes = fetch_web_art(&artist, &album);
                    let _ = tx.send(Msg::ArtWeb { bytes });
                }
                NetCmd::WebArtFor { path, artist, album } => {
                    let bytes = fetch_web_art(&artist, &album);
                    let _ = tx.send(Msg::ArtWebFor { path, bytes });
                }
                NetCmd::Lyrics(artist, title) => {
                    let text = fetch_lyrics(&artist, &title);
                    let _ = tx.send(Msg::Lyrics { artist, title, text });
                }
            }
        }));
    }
}

pub fn yt_loop(rx: Receiver<YtCmd>, tx: Sender<Msg>) {
    while let Ok(cmd) = rx.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match cmd {
                YtCmd::Download { query, auto_play, chunks } => {
                    let r = yt_download_and_store(&query, chunks, &tx);
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

pub fn fetch_web_art(artist: &str, album: &str) -> Option<Vec<u8>> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(6)).build().ok()?;
    let res = client
        .get("https://itunes.apple.com/search")
        .query(&[("term", format!("{} {}", artist, album)), ("entity", "album".to_string()), ("limit", "1".to_string())])
        .send()
        .ok()?;
    let json: serde_json::Value = res.json().ok()?;
    let n = json.get("resultCount")?.as_u64()?;
    if n == 0 { return None; }
    let url = json["results"][0]["artworkUrl100"].as_str()?;
    let url = url.replace("100x100bb", "500x500bb");
    client.get(&url).send().ok()?.bytes().ok().map(|b| b.to_vec())
}

pub fn fetch_lyrics(artist: &str, title: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().ok()?;
    let url = format!("https://api.lyrics.ovh/v1/{}/{}", percent_encode(artist), percent_encode(title));
    let res = client.get(&url).send().ok()?;
    if !res.status().is_success() { return None; }
    let json: serde_json::Value = res.json().ok()?;
    json.get("lyrics").and_then(|l| l.as_str()).map(|s| s.to_string())
}

