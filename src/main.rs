#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Random Shuffle Player — entry point and module root.
//!
//! The build script (`build.rs`) generates `embedded_tools.rs` with the
//! bundled ffmpeg/ffprobe/yt-dlp/mkvmerge binaries as `pub const` byte slices.
//! It is included at crate root so every module can reach the constants.

include!(concat!(env!("OUT_DIR"), "/embedded_tools.rs"));

pub(crate) mod app;
pub(crate) mod audio;
pub(crate) mod bdstream;
pub(crate) mod consts;
pub(crate) mod convert;
pub(crate) mod bluray;
pub(crate) mod disc;
pub(crate) mod dvd;
#[cfg(target_os = "linux")]
pub(crate) mod cd;
#[cfg(not(target_os = "linux"))]
pub(crate) mod cd {
    use crate::types::{CdInfo, Msg};
    use std::path::PathBuf;
    use std::sync::mpsc::Sender;
    pub fn monitor(_tx: Sender<Msg>) {}
    pub fn eject(_drive: &str) -> bool { false }
    pub fn rip(_info: &CdInfo, _tx: &Sender<Msg>) -> Result<(PathBuf, Vec<String>, String), String> {
        Err("CD ripping is only supported on Linux".into())
    }
}
#[cfg(target_os = "linux")]
pub(crate) mod desktop;
#[cfg(not(target_os = "linux"))]
pub(crate) mod desktop {
    /// Desktop shortcuts are a freedesktop/Linux concept; other platforms
    /// provide their own integration (and Windows embeds the icon in the exe).
    pub fn ensure_desktop_shortcut() -> bool {
        false
    }
}
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
    // Optional startup video: `mediaplayerofdoom --video /path/episode.mkv`
    // (a bare path also works). Handy for testing and for opening an episode
    // straight from a file manager.
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Internal helper: stream a Blu-ray title (decrypted) to stdout so ffmpeg
    // can seek by restarting this process at a timestamp. Not user-facing.
    if args.first().map(|a| a.as_str()) == Some("--bd-stream") {
        std::process::exit(crate::bdstream::run_cli(&args[1..]));
    }
    if args.iter().any(|a| a == "--check-dvd") {
        let deps = crate::tools::dvd_deps();
        println!("DVD / Blu-ray support check");
        println!(
            "  dvdbackup .... {}",
            crate::tools::dvdbackup_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "MISSING".to_string())
        );
        println!(
            "  genisoimage .. {}",
            crate::tools::genisoimage_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "MISSING".to_string())
        );
        for (name, ok) in [
            ("libdvdread", deps.libdvdread),
            ("libdvdnav", deps.libdvdnav),
            ("libdvdcss", deps.libdvdcss),
        ] {
            println!("  {:<13} {}", name, if ok { "ok" } else { "MISSING" });
        }
        let hint = deps.package_hint();
        if hint.is_empty() {
            println!("All DVD tools are present.");
        } else {
            println!("{}", hint);
        }
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{} v{}", crate::consts::APP_NAME, crate::consts::APP_VERSION);
        return Ok(());
    }
    if args.iter().any(|a| a == "--create-shortcut") {
        let made = crate::desktop::ensure_desktop_shortcut();
        println!(
            "{}",
            if made { "desktop shortcut created" } else { "desktop shortcut already exists (or could not be created; see desktop.log)" }
        );
        return Ok(());
    }
    let mut video: Option<String> = None;
    let mut dvd: Option<String> = None;
    let mut bluray: Option<String> = None;
    let mut windowed = false;
    let mut mode: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--windowed" || args[i] == "-w" {
            windowed = true;
            i += 1;
            continue;
        }
        if args[i] == "--dvd" {
            if let Some(p) = args.get(i + 1) {
                dvd = Some(p.clone());
                i += 2;
                continue;
            }
        }
        if args[i] == "--bluray" || args[i] == "--bd" {
            if let Some(p) = args.get(i + 1) {
                bluray = Some(p.clone());
                i += 2;
                continue;
            }
        }
        if args[i] == "--mode" {
            if let Some(m) = args.get(i + 1) {
                mode = Some(m.to_ascii_lowercase());
                i += 2;
                continue;
            }
        }
        if args[i] == "--video" {
            if let Some(p) = args.get(i + 1) {
                video = Some(p.clone());
                i += 2;
                continue;
            }
        } else if !args[i].starts_with('-') && video.is_none() {
            video = Some(args[i].clone());
        }
        i += 1;
    }
    app::run(video, windowed, mode, dvd, bluray)
}