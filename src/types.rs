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
    Dvd,
    BluRay,
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

/// A physical audio CD found in an optical drive.
#[derive(Clone, Debug)]
pub struct CdInfo {
    pub drive: String,
    /// Audio track numbers, in disc order.
    pub tracks: Vec<u8>,
    /// Absolute frame offsets (LBA + 150) matching `tracks`.
    pub offsets: Vec<u32>,
    pub leadout: u32,
    pub disc_id: String,
    pub total_secs: f32,
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
    ArtAll { done: Vec<String>, missing: Vec<String> },
    ArtWebFor { path: String, bytes: Option<Vec<u8>> },
    ArtWebGroup { paths: Vec<String>, bytes: Option<Vec<u8>> },
    /// MusicBrainz community ratings for one album. `tracks` are
    /// (normalized title, stars) and `album_stars` is the release-group fallback.
    RatingsAlbum { gen: u64, key: String, paths: Vec<String>, album_stars: u8, tracks: Vec<(String, u8)> },
    /// On-demand rating for the currently playing album (no Meta All run).
    CurrentRating { path: String, album_stars: u8, tracks: Vec<(String, u8)> },
    Lyrics { artist: String, title: String, text: Option<String> },
    YtStatus(String),
    YtLog(String),
    /// Progress of the whole job: which stage is running, and a label for it.
    /// The stage index selects a slice of one continuous bar - see
    /// `ytdlp::pipeline_frac`. The download stage fills from yt-dlp's own
    /// percentage; the stages either side have no percentage, so they are paced
    /// by the clock.
    YtStage { stage: u8, label: String },
    YtDone { path: PathBuf, auto_play: bool },
    /// A queue download opened a new destination folder. Carries the album
    /// identity and the folder name so the rest of the queue can share it.
    YtGroupEstablished { key: String, folder: String },
    YtFail { err: String, auto_play: bool },
    YtResolved { idx: usize, query: String, display: String },
    Transcoded { display: String, wav: PathBuf },
    TranscodeFail { display: String, err: String },
    Recorded { display: String, path: String },
    RecordFail { display: String, err: String },
    VideoFrame { w: u32, h: u32, rgba: Vec<u8>, gen: u64 },
    VideoClosed { gen: u64 },
    /// ffmpeg's stderr for the current open, so a failed decode can report why.
    VideoDecodeErr { gen: u64, err: String },
    VideoPos { gen: u64, secs: f32 },
    VideoMeta { gen: u64, dur: f32 },
    VideoBounds { gen: u64, path: String, intro_end: f32, credits_start: f32 },
    VideoCutDone { gen: u64, path: String, cut: Option<String>, err: Option<String> },
    CdDetected(Option<CdInfo>),
    MetaAllArtScan { need_art: Vec<String>, total: usize },
    CdProgress { track: usize, total: usize, text: String },
    CdDone { folder: String, files: Vec<String>, label: String },
    CdFail(String),
    DiscDumpProgress { done: u64, total: u64, text: String },
    DiscDumpDone(String),
    DiscDumpFail(String),
    DiscDumpCancelled,
    ConvertProgress { done: usize, total: usize, text: String },
    ConvertDone { folder: String, converted: usize, failed: usize },
    ConvertFail(String),
    ConvertCancelled,
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
    /// One lookup shared by every track on an album; the image is written to
    /// all `paths` so a 12-track album needs one request, not twelve.
    WebArtGroup { gen: u64, paths: Vec<String>, artist: String, album: String },
    /// Fallback for a track with no usable album: search by song title.
    WebArtSong { gen: u64, paths: Vec<String>, artist: String, title: String },
    Lyrics(String, String),
}

/// One album's worth of ratings to look up on MusicBrainz.
pub enum RatingCmd {
    Album { gen: u64, key: String, artist: String, album: String, paths: Vec<String> },
}

pub enum YtCmd {
    /// `group` carries `(album identity, folder name)` established by an earlier
    /// item in the same download queue, so a run of tracks from one album all
    /// land in one folder. `None` for a single download.
    Download { query: String, auto_play: bool, chunks: u32, group: Option<(String, String)> },
    Resolve { idx: usize, query: String },
}

