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
    /// Bumped whenever a field needs a one-time repair on load. See
    /// `SETTINGS_VERSION` in app.rs for what each version migrates.
    #[serde(default)]
    pub settings_version: u32,
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
    /// Songs whose rating was set by the user (so Meta All never overwrites
    /// them with a community rating).
    #[serde(default)]
    pub manual_ratings: HashSet<String>,
    /// 0 = old schema where every existing rating was manual; 1 = manual vs
    /// fetched ratings are tracked separately.
    #[serde(default)]
    pub ratings_schema: u32,
    #[serde(default)]
    pub rating_filter: u8,
    /// Library root that the startup scan settled on.
    ///
    /// `find_music_folder()` recursively walks `Music` on *every* logical drive
    /// on every launch, which on a large library is minutes of I/O before a
    /// single song plays. Remembering the answer makes that a one-time cost;
    /// a stale entry (drive unplugged, folder moved) is filtered on load and
    /// the search simply runs again.
    #[serde(default)]
    pub music_root: Option<String>,
    /// Video subtitle appearance: font size, bottom margin and outline width.
    #[serde(default = "default_subtitle_size")]
    pub subtitle_size: u32,
    #[serde(default = "default_subtitle_margin")]
    pub subtitle_margin: u32,
    #[serde(default = "default_subtitle_outline")]
    pub subtitle_outline: u32,
    /// Automatically turn on the first English subtitle track.
    #[serde(default)]
    pub video_auto_subs: bool,
    /// Remembered audio track ordinal per video path.
    #[serde(default)]
    pub video_audio_choice: HashMap<String, usize>,
    /// Remembered subtitle choice per video path ("off", "embedded:N", "external:/path").
    #[serde(default)]
    pub video_sub_choice: HashMap<String, String>,
    /// Lanczos upscale plus mild sharpening for DVDs.
    #[serde(default = "default_dvd_upscale")]
    pub dvd_upscale: bool,
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

pub fn default_subtitle_size() -> u32 {
    32
}

pub fn default_subtitle_margin() -> u32 {
    40
}

pub fn default_subtitle_outline() -> u32 {
    2
}

pub fn default_dvd_upscale() -> bool {
    true
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

