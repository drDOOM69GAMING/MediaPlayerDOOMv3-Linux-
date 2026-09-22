#![allow(unused_imports)]
#![allow(dead_code)]
// Shared message/enum types.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeckMode {
    Tape,
    Disc,
    Radio,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum VideoAspect {
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "4:3")]
    R43,
    #[serde(rename = "16:9")]
    R169,
}

impl VideoAspect {
    pub fn label(self) -> &'static str {
        match self {
            VideoAspect::Original => "ORIG",
            VideoAspect::R43 => "4:3",
            VideoAspect::R169 => "16:9",
        }
    }
    pub fn next(self) -> VideoAspect {
        match self {
            VideoAspect::Original => VideoAspect::R43,
            VideoAspect::R43 => VideoAspect::R169,
            VideoAspect::R169 => VideoAspect::Original,
        }
    }
}

impl Default for VideoAspect {
    fn default() -> Self {
        VideoAspect::Original
    }
}

pub enum Msg {
    Error(String),
    DirScanned { dir: String, files: Vec<String> },
    ScanProgress { found: usize },
    PlaylistSaved,
    PlaylistLoaded { files: Vec<String> },
    Added { files: Vec<String> },
    FolderFound { folder: Option<String> },
    Meta { path: String, title: String, artist: String, album: String, duration: Duration },
    Tags { entries: Vec<(String, String, String)> },
    ArtLocal { bytes: Option<Vec<u8>> },
    ArtWeb { bytes: Option<Vec<u8>> },
    BatchMeta { entries: Vec<(String, String, String, String, f32)> },
    ArtAll { missing: Vec<String> },
    ArtWebFor { path: String, bytes: Option<Vec<u8>> },
    Lyrics { artist: String, title: String, text: Option<String> },
    YtStatus(String),
    YtLog(String),
    YtDone { path: PathBuf, auto_play: bool },
    YtFail { err: String, auto_play: bool },
    YtResolved { idx: usize, query: String, display: String },
    Transcoded { display: String, wav: PathBuf },
    TranscodeFail { display: String, err: String },
    Recorded { display: String, path: String },
    RecordFail { display: String, err: String },
    VideoFrame { w: u32, h: u32, rgba: Vec<u8>, gen: u64 },
    VideoClosed { gen: u64 },
    VideoPos { gen: u64, secs: f32 },
    VideoMeta { gen: u64, dur: f32 },
    VideoBounds { gen: u64, path: String, intro_end: f32, credits_start: f32 },
    VideoCutDone { gen: u64, path: String, cut: Option<String>, err: Option<String> },
}

pub enum LibCmd {
    Scan(String),
    AddMany(Vec<String>),
    LoadPlaylist(String),
    SavePlaylist(String, Vec<String>),
    Meta(String),
    TagBatch(Vec<String>),
    ArtLocal(String),
    BatchMeta(Vec<String>),
    ArtAll(Vec<String>),
    Transcode { display: String, audio: String },
    Record { display: String, audio: String, dest: String },
    RecordRadio { display: String, url: String, dest: String },
    FindMusicFolder,
    VideoOpen { path: String, gen: u64, seek: f32 },
    VideoClose,
    VideoPause(bool),
    VideoAnalyze { path: String, gen: u64 },
    VideoCut { path: String, gen: u64, intro_end: f32, credits_start: f32 },
}

pub enum NetCmd {
    WebArt(String, String),
    WebArtFor { path: String, artist: String, album: String },
    Lyrics(String, String),
}

pub enum YtCmd {
    Download { query: String, auto_play: bool, chunks: u32 },
    Resolve { idx: usize, query: String },
}

