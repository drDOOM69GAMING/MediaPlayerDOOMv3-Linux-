#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Random Shuffle Player — entry point and module root.
//!
//! The build script (`build.rs`) generates `embedded_tools.rs` with the
//! bundled ffmpeg/ffprobe/yt-dlp/mkvmerge binaries as `pub const` byte slices.
//! It is included at crate root so every module can reach the constants.

include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));

pub(crate) mod app;
pub(crate) mod audio;
pub(crate) mod consts;
pub(crate) mod deck;
pub(crate) mod deck_widgets;
pub(crate) mod download;
pub(crate) mod eq;
pub(crate) mod ffmpeg;
pub(crate) mod files;
pub(crate) mod handlers;
pub(crate) mod library;
pub(crate) mod lib_thread;
pub(crate) mod log;
pub(crate) mod network;
pub(crate) mod panels;
pub(crate) mod pipe;
pub(crate) mod settings;
pub(crate) mod theme;
pub(crate) mod tools;
pub(crate) mod types;
pub(crate) mod util;
pub(crate) mod video;
pub(crate) mod viz;
pub(crate) mod windows;
pub(crate) mod ytdlp;

fn main() -> eframe::Result {
    app::run()
}