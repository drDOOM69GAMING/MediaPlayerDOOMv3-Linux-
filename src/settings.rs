#![allow(unused_imports)]
#![allow(dead_code)]
// Persisted settings + radio station list.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use crate::types::VideoAspect;
use crate::util::data_dir;

/// One cached entry of a playlist track's tags (title/artist/album/duration),
/// persisted in %APPDATA%\MediaPlayerOFDOOM\metacache.json so the playlist
/// needs no lofty re-scan every session and the NOW panel shows instantly.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
pub struct MetaCacheEntry {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub secs: f32,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Settings {
    pub volume: u32,
    pub last_played: Option<String>,
    pub playlist: Vec<String>,
    pub theme: String,
    #[serde(default)]
    pub video_aspects: HashMap<String, VideoAspect>,
    #[serde(default)]
    pub video_aspect_default: VideoAspect,
    #[serde(default = "default_video_volume")]
    pub video_volume: u32,
    #[serde(default)]
    pub last_cd_dir: Option<String>,
    #[serde(default)]
    pub video_positions: HashMap<String, f32>,
    #[serde(default)]
    pub intro_skip_enabled: bool,
    #[serde(default)]
    pub video_cut_cache: HashMap<String, String>,
    #[serde(skip)]
    pub video_cut_inflight: HashSet<String>,
    #[serde(default = "default_intro_skip")]
    pub intro_skip_secs: f32,
    #[serde(default = "default_credits_skip")]
    pub credits_skip_secs: f32,
    #[serde(default)]
    pub video_bounds: HashMap<String, (f32, f32)>,
    #[serde(default)]
    pub show_bounds: HashMap<String, (f32, f32)>,
    #[serde(default)]
    pub eq_on: bool,
    #[serde(default)]
    pub eq_preset: String,
    #[serde(default)]
    pub eq_custom: Option<[f32; 10]>,
    #[serde(default)]
    pub ratings: HashMap<String, u8>,
    #[serde(default)]
    pub rating_filter: u8,
}

pub fn default_video_volume() -> u32 {
    100
}

pub fn default_intro_skip() -> f32 {
    90.0
}

pub fn default_credits_skip() -> f32 {
    90.0
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct RadioStation {
    pub name: String,
    pub band: String,
    pub freq: f32,
    pub url: String,
}

pub fn default_radio() -> Vec<RadioStation> {
    vec![
        RadioStation { name: "KEXP Indie/Rock".into(), band: "FM".into(), freq: 90.3, url: "http://live-mp3-128.kexp.org/kexp128.mp3".into() },
        RadioStation { name: "Radio Paradise".into(), band: "FM".into(), freq: 88.1, url: "https://stream.radioparadise.com/mp3-192".into() },
        RadioStation { name: "SomaFM Groove Salad".into(), band: "FM".into(), freq: 93.7, url: "https://ice1.somafm.com/groovesalad-128-mp3".into() },
        RadioStation { name: "SomaFM Indie Pop Rocks".into(), band: "FM".into(), freq: 94.5, url: "https://ice1.somafm.com/indiepop-128-mp3".into() },
        RadioStation { name: "SomaFM Sonic Universe".into(), band: "FM".into(), freq: 96.3, url: "https://ice1.somafm.com/sonicuniverse-128-mp3".into() },
        RadioStation { name: "SomaFM Underground 80s".into(), band: "FM".into(), freq: 101.1, url: "https://ice1.somafm.com/u80s-128-mp3".into() },
        RadioStation { name: "Kissin 92.5 (Joplin)".into(), band: "FM".into(), freq: 92.5, url: "https://playerservices.streamtheworld.com/api/livestream-redirect/KSYNFM.mp3".into() },
        RadioStation { name: "Big Dog 97.9 (Joplin)".into(), band: "FM".into(), freq: 97.9, url: "https://playerservices.streamtheworld.com/api/livestream-redirect/KXDGFM.mp3".into() },
        RadioStation { name: "Rock 107.1 (Joplin)".into(), band: "FM".into(), freq: 107.1, url: "https://ice42.securenetsystems.net/KJML".into() },
    ]
}

pub fn load_radio() -> Vec<RadioStation> {
    let p = data_dir().join("radio.json");
    if let Ok(text) = std::fs::read_to_string(&p) {
        if let Ok(list) = serde_json::from_str::<Vec<RadioStation>>(&text) {
            if !list.is_empty() {
                return list;
            }
        }
    }
    let list = default_radio();
    save_radio(&list);
    list
}

pub fn save_radio(list: &[RadioStation]) {
    if let Ok(text) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(data_dir().join("radio.json"), text);
    }
}

