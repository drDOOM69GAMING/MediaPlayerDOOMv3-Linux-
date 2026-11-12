#![allow(unused_imports)]
#![allow(dead_code)]
use eframe::egui;
use eframe::egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use rand::Rng;
use rodio::{Decoder, OutputStreamHandle, Sink, Source};
use crate::app::{PlayerApp, PlayerState};
use crate::{
    consts::*, deck_widgets::*, eq::*, ffmpeg::*, files::*, library::*,
    lib_thread::*, log::*, network::*, pipe::*, settings::*, theme::*,
    tools::*, types::*, util::*, viz::*, windows::*, ytdlp::*,
};

// Video playback: open/seek/window, video queue, bounds/cut analysis and video-audio attach.

/// Sample the movie audio sink at a few milliseconds instead of once per egui
/// paint. The frame reader uses this clock to decide when the next picture is
/// due; a 30 Hz clock made 24/25/30 fps video wait in paint-sized steps, which
/// looked like judder even though audio stayed in sync.
fn spawn_video_clock(
    sink: std::sync::Arc<Sink>,
    clock: std::sync::Arc<std::sync::atomic::AtomicU64>,
    gen: u64,
) {
    let weak = std::sync::Arc::downgrade(&sink);
    std::thread::spawn(move || loop {
        use std::sync::atomic::Ordering;
        // The app drops its Arc when the video closes or seeks. The weak
        // reference is the stop signal for this poller.
        let Some(s) = weak.upgrade() else { break };
        if s.is_paused() {
            clock.store(0, Ordering::Relaxed);
        } else {
            let pos = s.get_pos().as_millis() as u64 + 1;
            let tagged = (((gen & 0xFFFF) << 48) | pos.min((1u64 << 48) - 1)) as u64;
            clock.store(tagged, Ordering::Relaxed);
        }
        drop(s);
        std::thread::sleep(Duration::from_millis(2));
    });
}

/// Find a local poster/cover image for a disc folder, ISO or mounted drive.
fn find_disc_image(source: &str) -> Option<Vec<u8>> {
    let p = Path::new(source);
    let mut roots: Vec<PathBuf> = Vec::new();
    if p.is_dir() {
        roots.push(p.to_path_buf());
    } else if p.is_file() {
        if let Some(parent) = p.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if source.starts_with("/dev/") {
        if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
            for line in mounts.lines() {
                let mut it = line.split_whitespace();
                if let (Some(dev), Some(mnt)) = (it.next(), it.next()) {
                    if dev == source {
                        roots.push(PathBuf::from(mnt.replace("\\040", " ")));
                    }
                }
            }
        }
    }
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let exts = ["jpg", "jpeg", "png", "webp"];
    for root in roots {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for name in ["cover", "folder", "poster", "front", "dvd", "bluray"] {
            for ext in exts {
                candidates.push(root.join(format!("{}.{}", name, ext)));
            }
        }
        if !stem.is_empty() {
            for ext in exts {
                candidates.push(root.join(format!("{}.{}", stem, ext)));
            }
        }
        // Blu-ray metadata folders often carry a thumbnail/cover.
        if let Ok(rd) = std::fs::read_dir(root.join("BDMV/META/DL")) {
            let mut extra: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp")
                })
                .collect();
            extra.sort();
            candidates.extend(extra);
        }
        for c in candidates {
            if !c.is_file() {
                continue;
            }
            if std::fs::metadata(&c).map(|m| m.len() > 25 * 1024 * 1024).unwrap_or(true) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&c) {
                if image::load_from_memory(&bytes).is_ok() {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

fn audio_track_label(t: &crate::types::AudioStreamInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !t.lang.is_empty() {
        parts.push(t.lang.to_ascii_uppercase());
    }
    if t.channels > 0 {
        parts.push(format!("{}ch", t.channels));
    }
    if !t.title.is_empty() {
        parts.push(t.title.clone());
    }
    if parts.is_empty() {
        format!("Track {}", t.ordinal + 1)
    } else {
        parts.join(" · ")
    }
}

/// Best human title for a disc: label for a device, folder name for a folder,
/// file stem for an ISO. Used for the poster search and the sidebar.
fn disc_title(source: &str, kind: &str) -> String {
    let p = Path::new(source);
    let mut raw = String::new();
    if p.is_dir() {
        raw = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
    } else if p.is_file() {
        raw = p
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
    } else if source.starts_with("/dev/") {
        if let Ok(out) = Command::new("lsblk")
            .args(["-no", "LABEL", source])
            .output()
        {
            raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        if raw.is_empty() {
            if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
                for line in mounts.lines() {
                    let mut it = line.split_whitespace();
                    if let (Some(dev), Some(mnt)) = (it.next(), it.next()) {
                        if dev == source {
                            raw = Path::new(&mnt.replace("\\040", " "))
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            break;
                        }
                    }
                }
            }
        }
    }
    let cleaned = raw
        .replace('_', " ")
        .replace('.', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty()
        || cleaned.eq_ignore_ascii_case("video_ts")
        || cleaned.eq_ignore_ascii_case("bdmv")
        || cleaned.eq_ignore_ascii_case("dvd video")
    {
        format!("{} disc", kind)
    } else {
        cleaned
    }
}

impl PlayerApp {
    /// Load a local disc poster, a cached movie poster, or ask iTunes for one.
    pub(crate) fn set_video_poster_bytes(&mut self, bytes: Vec<u8>) {
        if let Ok(img) = image::load_from_memory(&bytes) {
            let rgba = img.to_rgba8();
            let thumb = image::imageops::thumbnail(&rgba, 360, 540);
            let size = [thumb.width() as usize, thumb.height() as usize];
            let ci = ColorImage::from_rgba_unmultiplied(size, thumb.as_raw());
            self.video_poster = Some(
                self.ctx
                    .load_texture("disc_poster", ci, TextureOptions::LINEAR),
            );
        }
    }

    pub(crate) fn load_disc_poster(&mut self, source: &str, kind: &str, title: &str) {
        self.video_poster = None;
        self.video_poster_kind = kind.to_string();
        self.video_poster_title = title.to_string();
        let key = format!("discv2:{}", title.to_lowercase());
        self.video_poster_key = key.clone();

        // 1. A poster fetched previously for this title.
        if let Some(bytes) = read_art_cache(&key) {
            self.set_video_poster_bytes(bytes);
            return;
        }
        // 2. A local cover/folder/poster image or Blu-ray metadata thumbnail.
        if let Some(bytes) = find_disc_image(source) {
            self.set_video_poster_bytes(bytes.clone());
            let _ = write_art_cache(&key, &bytes);
            return;
        }
        // 3. Ask iTunes Movies in the background.
        if title.chars().filter(|c| c.is_alphanumeric()).count() >= 3 {
            let _ = self.net_tx.send(NetCmd::DiscPoster {
                key,
                title: title.to_string(),
            });
        }
    }

    /// Draw the disc cover on pause/loading. Falls back to a generic disc
    /// badge when no local image exists.
    fn draw_disc_poster(&self, ui: &egui::Ui, area: egui::Rect) {
        if !(self.video_tex.is_none() || self.video_paused) {
            return;
        }
        if self.video_poster.is_none() && self.video_poster_title.is_empty() {
            return;
        }
        let p = ui.painter().with_clip_rect(area);
        let max_h = if self.video_fs { 430.0 } else { 220.0 };
        let panel_h = area.height().min(max_h);
        let panel_w = (panel_h * 0.70).min(area.width());
        let panel = egui::Rect::from_center_size(area.center(), egui::vec2(panel_w, panel_h));
        p.rect_filled(panel, 10.0, Color32::from_rgba_unmultiplied(0, 0, 0, 210));
        p.rect_stroke(
            panel,
            10.0,
            egui::Stroke::new(1.0, Color32::from_gray(90)),
            egui::StrokeKind::Inside,
        );
        let art = egui::Rect::from_min_size(
            panel.min + egui::vec2(14.0, 14.0),
            egui::vec2(panel.width() - 28.0, panel.height() - 62.0),
        );
        if let Some(tex) = &self.video_poster {
            let ts = tex.size();
            let (tw, th) = (ts[0].max(1) as f32, ts[1].max(1) as f32);
            let scale = (art.width() / tw).min(art.height() / th);
            let dst = egui::Rect::from_center_size(
                art.center(),
                egui::vec2(tw * scale, th * scale),
            );
            p.image(
                tex.id(),
                dst,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            let c = art.center();
            let r = (art.width().min(art.height()) * 0.36).max(24.0);
            p.circle_filled(c, r, Color32::from_gray(32));
            p.circle_stroke(c, r, egui::Stroke::new(2.0, Color32::from_gray(130)));
            p.circle_stroke(c, r * 0.62, egui::Stroke::new(1.0, Color32::from_gray(80)));
            p.circle_filled(c, r * 0.12, Color32::from_gray(15));
            p.text(
                c,
                egui::Align2::CENTER_CENTER,
                if self.video_poster_kind.is_empty() {
                    "DISC"
                } else {
                    &self.video_poster_kind
                },
                egui::FontId::monospace((r * 0.25).max(11.0)),
                Color32::from_gray(220),
            );
        }
        p.text(
            egui::pos2(panel.center().x, panel.max.y - 46.0),
            egui::Align2::CENTER_TOP,
            &self.video_poster_title,
            egui::FontId::monospace(if self.video_fs { 15.0 } else { 11.0 }),
            Color32::WHITE,
        );
        let info = if self
            .video_current
            .as_deref()
            .map(crate::dvd::is_dvd)
            .unwrap_or(false)
        {
            format!(
                "DVD {}x{}  {}",
                self.video_dims.0,
                self.video_dims.1,
                if self.dvd_upscale { "Lanczos" } else { "standard" }
            )
        } else {
            "PAUSED / LOADING".to_string()
        };
        p.text(
            egui::pos2(panel.center().x, panel.max.y - 24.0),
            egui::Align2::CENTER_TOP,
            info,
            egui::FontId::monospace(9.0),
            Color32::from_gray(140),
        );
    }

    pub(crate) fn play_dvd_source(&mut self, source: &str) {
        let source = source.trim();
        if source.is_empty() {
            self.set_error("No DVD source");
            return;
        }
        let wrapped = crate::dvd::wrap(source);
        // Optical discs must never resume from a stale timestamp: an input
        // seek on a real DVD can land in a bad cell and produce persistent
        // macroblocking. Also drop cached DVD probes so a new disc is probed.
        self.video_positions
            .retain(|k, _| !k.starts_with(crate::dvd::PREFIX));
        crate::ffmpeg::clear_dvd_meta_cache();
        crate::ffmpeg::clear_audio_map_cache();
        self.video_queue = vec![wrapped.clone()];
        self.video_queue_idx = 0;
        self.load_disc_poster(source, "DVD", &disc_title(source, "DVD"));
        self.play_video_item(wrapped);
        self.set_status(format!("DVD: {}", crate::dvd::display_name(&crate::dvd::wrap(source))));
    }

    pub(crate) fn play_bluray_source(&mut self, source: &str) {
        let source = source.trim();
        if source.is_empty() {
            self.set_error("No Blu-ray source");
            return;
        }
        // MakeMKV may have left a drive-holding guiserver behind.
        crate::bluray::release_stale_makemkv();
        let source_owned = source.to_string();
        self.request_bluray_protection(&source_owned, true);
        let info = crate::bluray::main_title_info(source);
        let playlist = info.as_ref().map(|i| i.playlist);
        let wrapped = crate::bluray::wrap_playlist(source, playlist);
        // Never resume a Blu-ray from a saved timestamp: the engine restarts
        // at the playlist start, and old bad timestamps used to make launch
        // look like it was stuck on the intro.
        self.video_positions.remove(&wrapped);
        self.video_queue = vec![wrapped.clone()];
        self.video_queue_idx = 0;
        self.load_disc_poster(source, "BLU-RAY", &disc_title(source, "BLU-RAY"));
        self.play_video_item(wrapped);
        if let Some(i) = &info {
            self.video_dur_secs = i.secs;
        }
        self.set_status(format!("BLU-RAY: {}", crate::bluray::display_name(&crate::bluray::wrap_playlist(source, playlist))));
    }

    pub(crate) fn play_bluray_drive(&mut self) {
        match crate::bluray::first_drive() {
            Some(d) => self.play_bluray_source(&d),
            None => self.set_error("No optical drive found"),
        }
    }

    pub(crate) fn open_bluray_iso(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("Blu-ray image or folder", &["iso", "img"])
            .pick_file()
        {
            self.play_bluray_source(&p.to_string_lossy());
        }
    }

    pub(crate) fn eject_bluray(&mut self) {
        let current = self.video_current.clone().filter(|p| crate::bluray::is_bluray(p));
        if let Some(cur) = current {
            crate::bluray::eject(&cur);
            self.close_video();
            self.set_status("Blu-ray tray ejected");
        } else if let Some(d) = crate::bluray::first_drive() {
            crate::bluray::eject(&crate::bluray::wrap(&d));
        }
    }

    pub(crate) fn play_dvd_drive(&mut self) {
        match crate::dvd::first_drive() {
            Some(d) => self.play_dvd_source(&d),
            None => self.set_error("No optical drive found"),
        }
    }

    pub(crate) fn open_dvd_iso(&mut self) {
        if let Some(p) = rfd::FileDialog::new()
            .add_filter("DVD image", &["iso", "img"])
            .pick_file()
        {
            self.play_dvd_source(&p.to_string_lossy());
        }
    }

    pub(crate) fn eject_dvd(&mut self) {
        let current = self.video_current.clone().filter(|p| crate::dvd::is_dvd(p));
        if let Some(cur) = current {
            crate::dvd::eject(&cur);
            self.close_video();
            self.set_status("DVD tray ejected");
        } else if let Some(d) = crate::dvd::first_drive() {
            crate::dvd::eject(&crate::dvd::wrap(&d));
        }
    }

    pub(crate) fn set_video_fs(&mut self, on: bool) {
        if on && !self.video_fs {
            if !self.fullscreen {
                self.video_fs_restore_app = true;
                self.set_fullscreen(true);
            } else {
                self.video_fs_restore_app = false;
            }
        }
        if !on && self.video_fs && self.video_fs_restore_app {
            self.video_fs_restore_app = false;
            self.set_fullscreen(false);
        }
        self.video_fs = on;
        if !on {
            self.video_pos = egui::pos2(60.0, 60.0);
        } else {
            self.video_last_mouse = 0.0;
        }
    }

    pub(crate) fn play_video(&mut self) {
        if let Some(song) = self.state.current_song.clone() {
            let song_path = PathBuf::from(&song);
            let dir = song_path.parent().map(|p| p.to_path_buf());
            let name = song_path.file_stem().map(|s| s.to_string_lossy().to_string());
            if let (Some(dir), Some(name)) = (dir, name) {
                for ext in ["mp4", "mkv", "avi", "webm"] {
                    let vp = dir.join(format!("{}.{}", name, ext));
                    if vp.is_file() {
                        self.enqueue_videos(vec![vp.to_string_lossy().to_string()]);
                        return;
                    }
                }
            }
        }
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Video", &["mp4", "mkv", "avi", "webm", "mov"])
            .pick_files()
        {
            let vids: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
            self.enqueue_videos(vids);
        }
    }

    pub(crate) fn attach_video_audio(&mut self, path: &str, seek: f32) {
        self.video_clock.store(0, std::sync::atomic::Ordering::Relaxed);
        self.video_seek_base = seek.max(0.0);
        av_log(&format!("attach_audio path={} seek={:.3} paused={} tex={}", path, seek, self.video_paused, self.video_tex.is_some()));
        if (crate::dvd::is_dvd(path) || crate::bluray::is_bluray(path))
            && self.video_audio_tracks.is_empty()
        {
            self.video_audio_tracks = crate::ffmpeg::disc_audio_tracks(path);
            self.video_audio_sel = self
                .video_audio_tracks
                .iter()
                .position(|t| crate::ffmpeg::is_english(&t.lang))
                .unwrap_or(0);
            if let Some(ord) = self.video_audio_choice.get(path).copied() {
                if let Some(i) = self
                    .video_audio_tracks
                    .iter()
                    .position(|t| t.ordinal == ord)
                {
                    self.video_audio_sel = i;
                }
            }
        }
        let audio_map = self.video_audio_map();
        match spawn_video_audio_mapped(path, &ffmpeg_path(), seek, audio_map.as_deref()) {
            Ok((mut child, pipe)) => {
                if let Ok(sink) = Sink::try_new(&self._stream_handle) {
                    sink.set_volume(self.video_volume as f32 / 100.0 * 0.9);
                    sink.pause();
                    sink.append(EqSource::new(pipe, self.eq_shared.clone()));
                    if !self.video_paused {
                        // Always wait for the first frame of this new open,
                        // including seeks and audio-track changes. Starting
                        // audio against the old texture made the new audio run
                        // ahead while the video reader was still opening.
                        self.video_audio_pending = true;
                        av_log("audio pending first frame");
                    }
                    let sink = std::sync::Arc::new(sink);
                    // Poll the audio clock off the UI thread. egui only paints
                    // every few milliseconds, and sampling the sink there made
                    // the video reader quantise frame waits to the paint rate.
                    spawn_video_clock(
                        sink.clone(),
                        self.video_clock.clone(),
                        self.video_gen,
                    );
                    self.video_open_at = self.ctx.input(|i| i.time);
                    self.video_audio_proc = Some(child);
                    self.video_sink = Some(sink);
                } else {
                    let _ = child.kill();
                    let _ = child.wait();
                    av_log("audio sink creation FAILED");
                }
            }
            Err(e) => {
                self.video_audio_pending = false;
                av_log(&format!("audio spawn FAILED (video only): {}", e));
                self.set_status(format!("MOVIE (video only): {}", e));
            }
        }
    }

    /// Apply the saved audio/subtitle choice (and the auto-English setting)
    /// after the stream probe arrives. Returns true when playback should be
    /// re-opened to apply a non-default choice.
    pub(crate) fn restore_video_choices(&mut self) -> bool {
        let Some(path) = self.video_current.clone() else {
            return false;
        };
        let mut changed = false;

        let desired_audio = self
            .video_audio_choice
            .get(&path)
            .and_then(|ord| self.video_audio_tracks.iter().position(|t| t.ordinal == *ord))
            .unwrap_or(0);
        if desired_audio != self.video_audio_sel {
            self.video_audio_sel = desired_audio;
            changed = true;
        }

        let mut sub_sel: Option<usize> = None;
        if let Some(key) = self.video_sub_choice.get(&path) {
            if key != "off" {
                if let Some(n) = key.strip_prefix("embedded:") {
                    if let Ok(ord) = n.parse::<usize>() {
                        sub_sel = self
                            .video_subs
                            .iter()
                            .position(|s| !s.external && s.ordinal == ord);
                    }
                } else if let Some(p) = key.strip_prefix("external:") {
                    sub_sel = self
                        .video_subs
                        .iter()
                        .position(|s| s.external && s.path == p);
                }
            }
        } else if self.video_auto_subs {
            sub_sel = self.video_subs.iter().position(|s| {
                crate::ffmpeg::is_english(&s.lang)
                    || s.title.to_ascii_lowercase().contains("eng")
            });
        }
        if sub_sel != self.video_sub_sel {
            self.video_sub_sel = sub_sel;
            changed = true;
        }
        self.video_subtitle = self.video_sub_sel.map(|i| {
            let s = &self.video_subs[i];
            if s.external {
                crate::types::VideoSubtitle::External {
                    path: s.path.clone(),
                }
            } else {
                crate::types::VideoSubtitle::Embedded {
                    path: path.clone(),
                    ordinal: s.ordinal,
                }
            }
        });
        changed
    }

    pub(crate) fn video_sub_style(&self) -> crate::types::SubtitleStyle {
        crate::types::SubtitleStyle {
            size: self.video_sub_size,
            margin: self.video_sub_margin,
            outline: self.video_sub_outline,
        }
    }

    /// ffmpeg map for the selected local audio track. Discs keep their own
    /// English/preferred mapping because their stream indexes are different.
    /// Current position from the audio clock, which is more precise than the
    /// once-per-second VideoPos message. Used when restarting for an audio or
    /// upscale change so the picture does not jump back.
    pub(crate) fn video_position_now(&self) -> f32 {
        if let Some(s) = &self.video_sink {
            if !self.video_ended {
                return self.video_seek_base + s.get_pos().as_secs_f32();
            }
        }
        self.video_cur_secs
    }

    pub(crate) fn video_audio_map(&self) -> Option<String> {
        let _path = self.video_current.as_deref()?;
        if self.video_audio_tracks.is_empty() {
            return None;
        }
        let i = self.video_audio_sel.min(self.video_audio_tracks.len() - 1);
        self.video_audio_tracks
            .get(i)
            .map(|t| format!("0:a:{}", t.ordinal))
    }

    pub(crate) fn video_audio_label(&self) -> String {
        let n = self.video_audio_tracks.len();
        if n == 0 {
            return "AUDIO --".to_string();
        }
        let i = self.video_audio_sel.min(n - 1);
        let t = &self.video_audio_tracks[i];
        let name = if !t.lang.is_empty() {
            t.lang.to_ascii_uppercase()
        } else if !t.title.is_empty() {
            t.title.clone()
        } else {
            format!("TRACK {}", t.ordinal + 1)
        };
        format!("AUDIO {}/{} {}", i + 1, n, name)
    }

    pub(crate) fn video_sub_label(&self) -> String {
        match self.video_sub_sel {
            Some(i) if i < self.video_subs.len() => {
                let s = &self.video_subs[i];
                let name = if !s.lang.is_empty() {
                    s.lang.to_ascii_uppercase()
                } else if !s.title.is_empty() {
                    s.title.clone()
                } else if s.external {
                    "EXT".to_string()
                } else {
                    format!("#{}", s.ordinal + 1)
                };
                format!("SUBS {}", name)
            }
            _ => "SUBS OFF".to_string(),
        }
    }

    pub(crate) fn cycle_video_audio(&mut self) {
        if self.video_audio_tracks.len() <= 1 {
            self.set_status("No other audio tracks in this video");
            return;
        }
        self.video_audio_sel = (self.video_audio_sel + 1) % self.video_audio_tracks.len();
        if let Some(p) = self.video_current.clone() {
            if let Some(t) = self.video_audio_tracks.get(self.video_audio_sel) {
                self.video_audio_choice.insert(p, t.ordinal);
                self.save_settings();
            }
        }
        let label = self.video_audio_label();
        let pos = self.video_position_now();
        self.seek_video(pos);
        self.set_status(label);
    }

    pub(crate) fn cycle_video_subs(&mut self) {
        if self.video_subs.is_empty() {
            self.set_status("No subtitles found; use OPEN SUBS to add one");
            return;
        }
        let next = match self.video_sub_sel {
            None => Some(0),
            Some(i) if i + 1 < self.video_subs.len() => Some(i + 1),
            _ => None,
        };
        self.video_sub_sel = next;
        self.video_subtitle = next.map(|i| {
            let s = &self.video_subs[i];
            if s.external {
                crate::types::VideoSubtitle::External { path: s.path.clone() }
            } else {
                crate::types::VideoSubtitle::Embedded {
                    path: self.video_current.clone().unwrap_or_default(),
                    ordinal: s.ordinal,
                }
            }
        });
        if let Some(p) = self.video_current.clone() {
            let key = match next {
                None => "off".to_string(),
                Some(i) => {
                    let s = &self.video_subs[i];
                    if s.external {
                        format!("external:{}", s.path)
                    } else {
                        format!("embedded:{}", s.ordinal)
                    }
                }
            };
            self.video_sub_choice.insert(p, key);
            self.save_settings();
        }
        let label = self.video_sub_label();
        let pos = self.video_position_now();
        self.seek_video(pos);
        self.set_status(label);
    }

    /// Add a subtitle file the user picked, then turn it on immediately.
    pub(crate) fn open_external_subs(&mut self) {
        if self.video_current.is_none() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Subtitles", &["srt", "ass", "ssa", "vtt"])
            .pick_file()
        else {
            return;
        };
        let path = path.to_string_lossy().to_string();
        let title = Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        self.video_subs.push(crate::types::SubtitleStreamInfo {
            ordinal: 0,
            lang: String::new(),
            title,
            path,
            external: true,
        });
        self.video_sub_sel = Some(self.video_subs.len() - 1);
        self.video_subtitle = Some(crate::types::VideoSubtitle::External {
            path: self.video_subs[self.video_sub_sel.unwrap()].path.clone(),
        });
        if let Some(p) = self.video_current.clone() {
            let ext = self.video_subs[self.video_sub_sel.unwrap()].path.clone();
            self.video_sub_choice.insert(p, format!("external:{}", ext));
            self.save_settings();
        }
        let pos = self.video_position_now();
        self.seek_video(pos);
        self.set_status("External subtitles enabled");
    }

    /// Explicit audio and subtitle picker for the current movie.
    pub(crate) fn show_video_tracks_window(&mut self, ctx: &egui::Context) {
        if !self.video_tracks_open {
            return;
        }
        let th = self.theme_colors();
        let path = self.video_current.clone();
        let disc = path
            .as_deref()
            .map(|p| crate::dvd::is_dvd(p) || crate::bluray::is_bluray(p))
            .unwrap_or(false);
        let is_dvd = path.as_deref().map(crate::dvd::is_dvd).unwrap_or(false);
        let mut open = self.video_tracks_open;
        let mut close_clicked = false;
        let mut pick_audio: Option<usize> = None;
        let mut pick_sub: Option<Option<usize>> = None;
        let mut open_subs = false;
        let mut rescan = false;
        let mut scale_toggle = false;
        egui::Window::new("MOVIE TRACKS")
            .resizable(false)
            .collapsible(false)
            .default_width(390.0)
            .order(egui::Order::Foreground)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(RichText::new("AUDIO").color(th.accent).monospace().strong().size(10.0));
                if self.video_audio_tracks.is_empty() {
                    ui.label(
                        RichText::new("No audio tracks detected")
                            .color(Color32::from_gray(150))
                            .monospace()
                            .size(10.0),
                    );
                } else {
                    for (i, t) in self.video_audio_tracks.iter().enumerate() {
                        let label = format!("{}. {}", i + 1, audio_track_label(t));
                        if ui
                            .selectable_label(self.video_audio_sel == i, label)
                            .clicked()
                        {
                            pick_audio = Some(i);
                        }
                    }
                }
                ui.horizontal(|ui| {
                    if retro_btn(ui, "RESCAN", th.btn_fg, th.btn_bg).clicked() {
                        rescan = true;
                    }
                    ui.label(
                        RichText::new("Discs report language tracks; local files include the title")
                            .color(Color32::from_gray(120))
                            .monospace()
                            .size(8.0),
                    );
                });
                ui.separator();
                ui.label(
                    RichText::new("SUBTITLES")
                        .color(th.accent)
                        .monospace()
                        .strong()
                        .size(10.0),
                );
                if ui
                    .selectable_label(self.video_sub_sel.is_none(), "Off")
                    .clicked()
                {
                    pick_sub = Some(None);
                }
                for (i, sub) in self.video_subs.iter().enumerate() {
                    let label = if sub.external {
                        format!("External: {}", sub.title)
                    } else {
                        let mut s = format!("Embedded: {}", sub.lang.to_ascii_uppercase());
                        if !sub.title.is_empty() {
                            s.push_str(" · ");
                            s.push_str(&sub.title);
                        }
                        s
                    };
                    if ui
                        .selectable_label(self.video_sub_sel == Some(i), label)
                        .clicked()
                    {
                        pick_sub = Some(Some(i));
                    }
                }
                if retro_btn(ui, "OPEN SUBS...", th.btn_fg, th.btn_bg).clicked() {
                    open_subs = true;
                }
                if disc {
                    ui.label(
                        RichText::new(
                            "Disc bitmap subtitles (VobSub/PGS) are not listed; use OPEN SUBS for a text file",
                        )
                        .color(Color32::from_gray(120))
                        .monospace()
                        .size(8.0),
                    );
                }
                if is_dvd {
                    ui.separator();
                    if ui.checkbox(&mut self.dvd_upscale, "DVD Lanczos upscale").changed() {
                        scale_toggle = true;
                    }
                }
                ui.separator();
                if retro_btn(ui, "CLOSE", th.btn_fg, th.btn_bg).clicked() {
                    close_clicked = true;
                }
            });
        if close_clicked {
            open = false;
        }

        if let Some(i) = pick_audio {
            if i < self.video_audio_tracks.len() {
                self.video_audio_sel = i;
                if let Some(p) = path.clone() {
                    if let Some(t) = self.video_audio_tracks.get(i) {
                        self.video_audio_choice.insert(p, t.ordinal);
                        self.save_settings();
                    }
                }
                let pos = self.video_position_now();
                self.seek_video(pos);
            }
        }
        if let Some(sel) = pick_sub {
            self.video_sub_sel = sel;
            self.video_subtitle = sel.map(|i| {
                let s = &self.video_subs[i];
                if s.external {
                    crate::types::VideoSubtitle::External { path: s.path.clone() }
                } else {
                    crate::types::VideoSubtitle::Embedded {
                        path: path.clone().unwrap_or_default(),
                        ordinal: s.ordinal,
                    }
                }
            });
            if let Some(p) = path.clone() {
                let key = match sel {
                    None => "off".to_string(),
                    Some(i) => {
                        let s = &self.video_subs[i];
                        if s.external {
                            format!("external:{}", s.path)
                        } else {
                            format!("embedded:{}", s.ordinal)
                        }
                    }
                };
                self.video_sub_choice.insert(p, key);
                self.save_settings();
            }
            let pos = self.video_position_now();
            self.seek_video(pos);
        }
        if open_subs {
            self.open_external_subs();
        }
        if scale_toggle {
            self.save_settings();
            let pos = self.video_position_now();
            self.seek_video(pos);
        }
        if rescan {
            if disc {
                if let Some(p) = path.clone() {
                    self.video_audio_tracks = crate::ffmpeg::disc_audio_tracks(&p);
                    self.video_audio_sel = self
                        .video_audio_tracks
                        .iter()
                        .position(|t| crate::ffmpeg::is_english(&t.lang))
                        .unwrap_or(0);
                }
            } else if let Some(p) = path.clone() {
                self.video_audio_tracks.clear();
                let gen = self.video_gen;
                let seek = self.video_position_now();
                let _ = self.lib_tx.send(LibCmd::VideoOpen {
                    path: p,
                    gen,
                    seek,
                    subtitle: self.video_subtitle.clone(),
                    sub_style: self.video_sub_style(),
                    dvd_upscale: self.dvd_upscale,
                    probe: true,
                });
            }
            self.set_status("Rescanning movie tracks...");
        }
        self.video_tracks_open = open;
    }

    /// Small window for subtitle size, bottom margin and outline.
    pub(crate) fn show_subtitle_style_window(&mut self, ctx: &egui::Context) {
        if !self.video_sub_style_open {
            return;
        }
        let th = self.theme_colors();
        let mut open = self.video_sub_style_open;
        let mut apply = false;
        let mut close_clicked = false;
        egui::Window::new("SUBTITLE STYLE")
            .resizable(false)
            .collapsible(false)
            .default_width(330.0)
            .order(egui::Order::Foreground)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("SIZE")
                        .color(th.accent)
                        .monospace()
                        .size(10.0),
                );
                ui.add(
                    egui::Slider::new(&mut self.video_sub_size, 16..=80)
                        .text("pt")
                        .clamping(egui::SliderClamping::Always),
                );
                ui.label(
                    RichText::new("BOTTOM MARGIN")
                        .color(th.accent)
                        .monospace()
                        .size(10.0),
                );
                ui.add(
                    egui::Slider::new(&mut self.video_sub_margin, 10..=180)
                        .text("px")
                        .clamping(egui::SliderClamping::Always),
                );
                ui.label(
                    RichText::new("OUTLINE")
                        .color(th.accent)
                        .monospace()
                        .size(10.0),
                );
                ui.add(
                    egui::Slider::new(&mut self.video_sub_outline, 0..=4)
                        .text("px")
                        .clamping(egui::SliderClamping::Always),
                );
                ui.add_space(4.0);
                if ui
                    .checkbox(&mut self.video_auto_subs, "Auto-select English subtitles")
                    .changed()
                {
                    self.save_settings();
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if retro_btn(ui, "APPLY", th.playing_fg, th.btn_bg).clicked() {
                        apply = true;
                    }
                    if retro_btn(ui, "RESET", th.btn_fg, th.btn_bg).clicked() {
                        self.video_sub_size = 32;
                        self.video_sub_margin = 40;
                        self.video_sub_outline = 2;
                        apply = true;
                    }
                    if retro_btn(ui, "CLOSE", th.btn_fg, th.btn_bg).clicked() {
                        close_clicked = true;
                    }
                });
            });
        if close_clicked {
            open = false;
        }
        if apply {
            self.save_settings();
            if self.video_sub_sel.is_some() {
                let pos = self.video_position_now();
                self.seek_video(pos);
            }
            self.set_status("Subtitle style updated");
        }
        self.video_sub_style_open = open;
    }

    pub(crate) fn seek_video(&mut self, tsecs: f32) {
        let Some(path) = self.video_current.clone() else { return };
        av_log(&format!("seek_video target={:.3}s (dur={:.3}) path={}", tsecs, self.video_dur_secs, path));
        self.video_time_offset = 0.0;
        let was_paused = self.video_paused;
        let mut tsecs = tsecs.max(0.0);
        if self.video_dur_secs > 0.0 {
            tsecs = tsecs.min((self.video_dur_secs - 1.0).max(0.0));
        }
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
        self.video_gen += 1;
        let gen = self.video_gen;
        let _ = self.lib_tx.send(LibCmd::VideoOpen { path: path.clone(), gen, seek: tsecs, subtitle: self.video_subtitle.clone(), sub_style: self.video_sub_style(), dvd_upscale: self.dvd_upscale, probe: false });
        self.video_closing = false;
        self.video_paused = false;
        self.video_ended = false;
        self.video_cur_secs = tsecs;
        // Deliberately keep the current texture: a seek re-opens ffmpeg, and
        // blanking here meant every seek flashed a full black "LOADING..."
        // screen for as long as the seek took. The stale frame stays up until
        // the first frame of the new position replaces it.
        self.video_loading_since = Some(Instant::now());
        self.video_load_timed_out = false;
        self.video_decode_error.clear();
        self.attach_video_audio(&path, tsecs);
        // Keep the paused state across a seek: seek while paused should
        // show the frame at the new position and stay paused, not resume.
        if was_paused {
            self.video_paused = true;
            let _ = self.lib_tx.send(LibCmd::VideoPause(true));
            if let Some(s) = &self.video_sink {
                s.pause();
            }
            self.video_audio_pending = false;
        }
        let frac = if self.video_dur_secs > 0.0 { (tsecs / self.video_dur_secs).clamp(0.0, 1.0) } else { 0.0 };
        self.set_status(format!("MOVIE seek {:.0}%", frac * 100.0));
    }

    pub(crate) fn play_video_item(&mut self, path: String) {
        self.video_closing = false;
        self.video_time_offset = 0.0;
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
        if self.deck_mode == DeckMode::Tape {
            if let Some(cur) = self.state.current_song.clone() {
                if Path::new(&cur).is_file() {
                    self.deck_resume = Some((cur, self.pos.max(0.0)));
                }
            }
        }
        self.stop_radio();
        self.stop_music();
        self.video_gen += 1;
        let gen = self.video_gen;
        let saved_pos = self.video_positions.get(&path).copied().unwrap_or(0.0);
        av_log(&format!("play_video_item path={} saved_pos={:.3}", path, saved_pos));
        if !crate::dvd::is_dvd(&path) && !crate::bluray::is_bluray(&path) {
            self.video_poster = None;
            self.video_poster_key.clear();
            self.video_poster_kind.clear();
            self.video_poster_title.clear();
        }
        self.video_audio_tracks.clear();
        self.video_audio_sel = 0;
        self.video_subs.clear();
        self.video_sub_sel = None;
        self.video_subtitle = None;
        let _ = self.lib_tx.send(LibCmd::VideoOpen { path: path.clone(), gen, seek: saved_pos, subtitle: None, sub_style: self.video_sub_style(), dvd_upscale: self.dvd_upscale, probe: true });
        self.video_aspect = self.video_aspects.get(&path).copied().unwrap_or(self.video_aspect_default);
        self.video_on = true;
        self.video_paused = false;
        self.video_ended = false;
        self.video_pos = egui::pos2(60.0, 60.0);
        self.video_tex = None;
        self.video_bar_visible = false;
        self.video_dur_secs = 0.0;
        self.video_cur_secs = 0.0;
        self.video_seek_t = None;
        self.video_current = Some(path.clone());
        self.video_loading_since = Some(Instant::now());
        self.video_load_timed_out = false;
        self.video_decode_error.clear();
        if !path.ends_with("_cut.mkv") {
            let cut_dir = work_dir().join("cuts");
            if let Ok(rd) = std::fs::read_dir(&cut_dir) {
                for e in rd.flatten() {
                    let _ = std::fs::remove_file(e.path());
                }
            }
            self.video_cut_cache.clear();
            self.video_cut_inflight.clear();
        }
        self.attach_video_audio(&path, saved_pos);
        self.video_cur_secs = saved_pos;
        self.auto_bounds.remove(&path);
        if self.intro_skip_enabled
            && !path.ends_with("_cut.mkv")
            && !crate::dvd::is_dvd(&path)
            && !crate::bluray::is_bluray(&path)
        {
            // No manual and no learned markers? Analyse this episode's audio
            // in the worker thread while it plays. Analysis is advisory: it
            // only seek-skips, never cuts and never writes settings.
            let (ie, cs) = self.video_bound_secs();
            if ie <= 0.0 && cs <= 0.0 {
                let _ = self.lib_tx.send(LibCmd::VideoAnalyze { path: path.clone(), gen });
            }
        }
        self.set_status(format!(
            "MOVIE {}/{}: {}",
            self.video_queue_idx + 1,
            self.video_queue.len(),
            if crate::dvd::is_dvd(&path) {
                crate::dvd::display_name(&path)
            } else if crate::bluray::is_bluray(&path) {
                crate::bluray::display_name(&path)
            } else {
                stem(&path)
            }
        ));
        self.video_deferred_open = true;
    }

    pub(crate) fn enqueue_videos(&mut self, paths: Vec<String>) {
        if paths.is_empty() {
            return;
        }
        if self.video_on {
            let n = self.video_queue.len();
            self.video_queue.extend(paths);
            self.set_status(format!("Queued {} video(s) -> {}", self.video_queue.len() - n, self.video_queue.len()));
        } else {
            let s = self.video_queue.len();
            self.video_queue.extend(paths);
            let first = self.video_queue[s].clone();
            self.video_queue_idx = s;
            self.play_video_item(first);
        }
    }

    pub(crate) fn play_queue_idx(&mut self, idx: usize) {
        if idx < self.video_queue.len() {
            self.video_queue_idx = idx;
            self.play_video_item(self.video_queue[idx].clone());
        }
    }

    /// Advance to the next queued video. A single-item queue has no "next", so
    /// this is a no-op there; see `next_queue_idx`.
    pub(crate) fn queue_next(&mut self) -> bool {
        let Some(ni) = next_queue_idx(self.video_queue_idx, self.video_queue.len()) else {
            return false;
        };
        self.play_queue_idx(ni);
        true
    }

    pub(crate) fn queue_prev(&mut self) {
        if self.video_queue.is_empty() {
            return;
        }
        let pi = (self.video_queue_idx + self.video_queue.len() - 1) % self.video_queue.len();
        self.play_queue_idx(pi);
    }

    pub(crate) fn remove_queue_item(&mut self, idx: usize) {
        if idx < self.video_queue.len() {
            self.video_queue.remove(idx);
            if self.video_queue.is_empty() {
                return;
            }
            if idx < self.video_queue_idx {
                self.video_queue_idx -= 1;
            }
            if self.video_queue_idx >= self.video_queue.len() {
                self.video_queue_idx = self.video_queue.len() - 1;
            }
        }
    }

    /// Returns the (intro_end, credits_start) markers for the current video.
    ///
    /// Manual markers only - see `resolve_bounds`.
    pub(crate) fn video_bound_secs(&self) -> (f32, f32) {
        let path = self.video_current.as_deref();
        let (intro, credits) = resolve_bounds(&self.video_bounds, &self.show_bounds, path);
        if intro > 0.0 || credits > 0.0 {
            return (intro, credits);
        }
        // Second layer: learn from other episodes in the same folder. Marking
        // one episode therefore makes every other episode skip automatically.
        if let Some(p) = path {
            if let Some(b) = learned_folder_bounds(&self.video_bounds, p) {
                return b;
            }
            if let Some(&b) = self.auto_bounds.get(p) {
                return b;
            }
        }
        (0.0, 0.0)
    }

    /// Shift-clicking INTRO / CREDS clears that marker, for this file and for
    /// the show preset. Without this a single mis-click could only be undone
    /// by overwriting it, and because a marker is also stored per folder it
    /// would keep applying to every other episode.
    pub(crate) fn clear_video_markers(&mut self, intro: bool, credits: bool) {
        let Some(p) = self.video_current.clone() else { return };
        let dir = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string());
        let zero = |b: (f32, f32)| (if intro { 0.0 } else { b.0 }, if credits { 0.0 } else { b.1 });
        if let Some(b) = self.video_bounds.get(&p).copied() {
            self.video_bounds.insert(p.clone(), zero(b));
        }
        if let Some(d) = &dir {
            if let Some(b) = self.show_bounds.get(d).copied() {
                self.show_bounds.insert(d.clone(), zero(b));
            }
        }
        // A cut file was built from the old markers; drop it so a stale cut is
        // not replayed against the markers that are now gone.
        let stale: Vec<String> = self
            .video_cut_cache
            .iter()
            .filter(|(k, _)| k.as_str() == p.as_str())
            .map(|(_, v)| v.clone())
            .collect();
        for c in stale {
            self.video_cut_cache.remove(&p);
            let _ = std::fs::remove_file(&c);
        }
        self.video_positions.remove(&p);
        self.auto_bounds.remove(&p);
        self.save_settings();
        let what = match (intro, credits) {
            (true, true) => "INTRO+CREDS",
            (true, false) => "INTRO",
            _ => "CREDS",
        };
        self.set_status(format!("Cleared {} markers for this file and folder", what));
    }

    pub(crate) fn try_cut_current_video(&mut self, path: String) {
        if !self.intro_skip_enabled {
            return;
        }
        if path.ends_with("_cut.mkv") || self.video_cut_inflight.contains(&path) || self.video_cut_cache.contains_key(&path) {
            return;
        }
        if crate::dvd::is_dvd(&path) || crate::bluray::is_bluray(&path) {
            return;
        }
        let (ie, cs) = self.video_bounds.get(&path).copied().unwrap_or((0.0, 0.0));
        if ie < 2.0 || cs < 2.0 {
            return;
        }
        if !mkvmerge_path().is_file() {
            return;
        }
        self.video_cut_inflight.insert(path.clone());
        let gen = self.video_gen;
        av_log(&format!("cut START {} ie={:.2} cs={:.2}", path, ie, cs));
        let _ = self.lib_tx.send(LibCmd::VideoCut { path, gen, intro_end: ie, credits_start: cs });
    }

    pub(crate) fn start_video(&mut self, path: String) {
        if self.video_on {
            let n = self.video_queue.len();
            self.video_queue.push(path);
            self.set_status(format!("Queued -> {}", self.video_queue.len()));
            let _ = n;
        } else {
            self.video_queue.push(path);
            let last = self.video_queue.len() - 1;
            self.video_queue_idx = last;
            self.play_video_item(self.video_queue[last].clone());
        }
    }

    pub(crate) fn close_video(&mut self) {
        if let Some(path) = self.video_current.clone() {
            if self.video_cur_secs > 3.0 && self.video_dur_secs > 0.0 && self.video_cur_secs < self.video_dur_secs - 5.0 {
                self.video_positions.insert(path, self.video_cur_secs);
                self.save_settings();
            }
        }
        self.video_closing = true;
        let _ = self.lib_tx.send(LibCmd::VideoClose);
        self.video_on = false;
        self.video_ended = false;
        self.video_paused = false;
        self.video_ended = false;
        self.video_current = None;
        self.video_queue.clear();
        self.video_queue_idx = 0;
        self.video_tex = None;
        self.video_poster = None;
        self.video_poster_key.clear();
        self.video_poster_kind.clear();
        self.video_poster_title.clear();
        self.video_loading_since = None;
        self.video_load_timed_out = false;
        self.video_decode_error.clear();
        self.video_audio_pending = false;
        self.video_time_offset = 0.0;
        self.video_cur_secs = 0.0;
        self.video_dur_secs = 0.0;
        self.set_video_fs(false);
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
    }

    pub(crate) fn toggle_video_play(&mut self) {
        if self.video_ended {
            self.video_ended = false;
            self.video_paused = false;
            if self.video_queue_idx < self.video_queue.len() {
                self.play_video_item(self.video_queue[self.video_queue_idx].clone());
                self.set_status("MOVIE: replay");
            }
            return;
        }
        self.video_paused = !self.video_paused;
        let _ = self.lib_tx.send(LibCmd::VideoPause(self.video_paused));
        if let Some(s) = &self.video_sink {
            if self.video_paused { s.pause(); } else { s.play(); }
        }
        self.set_status(if self.video_paused { "MOVIE: paused" } else { "MOVIE: playing" });
    }

    /// Text shown while no frame is available yet. Showing how long it has
    /// been waiting turns an unexplained black screen into something the user
    /// can judge (and PlayerApp gives up with a real error after 20s).
    pub(crate) fn video_loading_text(&self) -> String {
        let name = self
            .video_current
            .as_deref()
            .map(stem)
            .unwrap_or_else(|| "video".into());
        match self.video_loading_since {
            Some(s) => format!("LOADING {}  {:.0}s", name, s.elapsed().as_secs_f32()),
            None => format!("LOADING {}", name),
        }
    }

    pub(crate) fn video_window(&mut self, ctx: &egui::Context) {
        let (w, h) = self.video_dims;
        if self.video_fs {
            let scr = ctx.viewport_rect();
            let sw = scr.width();
            let sh = scr.height();
            let (iw0, ih0) = if w > 0 && h > 0 { (w as f32, h as f32) } else { (16.0, 9.0) };
            let (iw, ih) = match self.video_aspect {
                VideoAspect::Original => (iw0, ih0),
                VideoAspect::R43 => (4.0, 3.0),
                VideoAspect::R169 => (16.0, 9.0),
            };
            let scale = (sw / iw).min(sh / ih);
            let vw = iw * scale;
            let vh = ih * scale;
            let img = egui::Rect::from_center_size(scr.center(), egui::vec2(vw, vh));
            let mouse = ctx.input(|i| i.pointer.latest_pos());
            let t_now = ctx.input(|i| i.time);
            let mouse_moved = ctx.input(|i| i.pointer.delta().length() > 0.0 || i.pointer.any_pressed());
            if self.video_last_mouse == 0.0 {
                self.video_last_mouse = t_now;
            }
            if mouse_moved {
                self.video_last_mouse = t_now;
            }
            let queue_region = egui::Rect::from_min_max(
                egui::pos2(scr.max.x - 300.0, scr.min.y + 32.0),
                egui::pos2(scr.max.x, scr.max.y),
            );
            let hovering = mouse.map_or(false, |m| {
                (m.y <= scr.min.y + 60.0 && m.x >= scr.min.x && m.x <= scr.max.x) || queue_region.contains(m)
            });
            self.video_bar_visible = hovering;
            let mut fs_off = false;
            let mut togg = false;
            let mut next = false;
            let mut prev = false;
            let mut add = false;
            let mut aspect_cycle = false;
            let mut stop_video = false;
            let mut play_idx: Option<usize> = None;
            let mut del_idx: Option<usize> = None;
            let mut clearq = false;
            let mut audio_cycle = false;
            let mut subs_cycle = false;
            let mut open_subs = false;
            let mut open_sub_style = false;
            let mut open_tracks = false;
            let mut dvd_scale_toggle = false;
            // Shift-clicking INTRO / CREDS clears that marker instead of
            // setting it (egui 0.36 has no secondary-click sense).
            let shift = ctx.input(|i| i.modifiers.shift);
            egui::Area::new(egui::Id::new("movie_win"))
                .fixed_pos(scr.min)
                .constrain(false)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let resp = ui.allocate_response(scr.size(), egui::Sense::click_and_drag());
                    let rect = resp.rect;
                    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                    if let Some(tex) = &self.video_tex {
                        ui.painter().image(tex.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    } else {
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, self.video_loading_text(), egui::FontId::monospace(14.0), Color32::from_gray(140));
                    }
                    if self.video_paused {
                        ui.painter().rect_stroke(img, 0.0, egui::Stroke::new(3.0, Color32::from_rgb(255, 220, 80)), egui::StrokeKind::Inside);
                    }
                    self.draw_disc_poster(ui, img);
                    if self.video_bar_visible {
                        let bar = egui::Rect::from_min_size(scr.min, egui::vec2(sw, 30.0));
                        ui.painter().rect_filled(bar, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 170));
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(8.0, 4.0), egui::vec2(76.0, 22.0)), egui::Button::new("EXIT FS")).clicked() {
                            fs_off = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(92.0, 4.0), egui::vec2(80.0, 22.0)), egui::Button::new(if self.video_paused { "PLAY" } else { "PAUSE" })).clicked() {
                            togg = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(180.0, 4.0), egui::vec2(52.0, 22.0)), egui::Button::new("<<")).clicked() {
                            prev = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(240.0, 4.0), egui::vec2(52.0, 22.0)), egui::Button::new(">>")).clicked() {
                            next = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(300.0, 4.0), egui::vec2(80.0, 22.0)), egui::Button::new("+ ADD")).clicked() {
                            add = true;
                        }
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(388.0, 4.0), egui::vec2(150.0, 22.0)), egui::Button::new(format!("ASPECT {}", self.video_aspect.label()))).clicked() {
                            aspect_cycle = true;
                        }
                        let skip_lbl = if self.intro_skip_enabled { "SKIP: ON" } else { "SKIP: OFF" };
                        let skip_color = if self.intro_skip_enabled { Color32::from_rgb(100, 255, 100) } else { Color32::from_gray(160) };
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(546.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new(egui::RichText::new(skip_lbl).color(skip_color))).clicked() {
                            self.intro_skip_enabled = !self.intro_skip_enabled;
                            self.save_settings();
                            self.set_status(if self.intro_skip_enabled {
                                "SKIP: on - markers you set will be used"
                            } else {
                                "SKIP: off - no intro/credits skipping"
                            });
                        }
                        let (bie, bcs) = self.video_bound_secs();
                        let bnd_lbl = if bie > 0.0 {
                            format!("INTRO {}", fmt_time(bie))
                        } else {
                            "INTRO --".to_string()
                        };
                        let intro_btn = ui.put(
                            egui::Rect::from_min_size(scr.min + egui::vec2(640.0, 4.0), egui::vec2(88.0, 22.0)),
                            egui::Button::new(bnd_lbl),
                        );
                        let mut clear_intro = false;
                        if intro_btn.clicked() {
                            if shift {
                                clear_intro = true;
                            } else if let Some(p) = self.video_current.clone() {
                                // Keep the other auto-detected marker instead of
                                // dropping it just because it was not manual yet.
                                let (_, ccs) = self.video_bound_secs();
                                self.video_bounds.insert(p.clone(), (self.video_cur_secs, ccs));
                                if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                                    self.show_bounds.insert(dir, (self.video_cur_secs, ccs));
                                }
                                self.save_settings();
                                self.set_status(format!("Intro END set at {} - applies to this whole folder", fmt_time(self.video_cur_secs)));
                                self.try_cut_current_video(p);
                            }
                        }
                        let crd_lbl = if bcs > 0.0 {
                            format!("CREDS {}", fmt_time(bcs))
                        } else {
                            "CREDS --".to_string()
                        };
                        let creds_btn = ui.put(
                            egui::Rect::from_min_size(scr.min + egui::vec2(734.0, 4.0), egui::vec2(86.0, 22.0)),
                            egui::Button::new(crd_lbl),
                        );
                        let mut clear_creds = false;
                        if creds_btn.clicked() {
                            if shift {
                                clear_creds = true;
                            } else if let Some(p) = self.video_current.clone() {
                                let (iie, _) = self.video_bound_secs();
                                self.video_bounds.insert(p.clone(), (iie, self.video_cur_secs));
                                if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                                    self.show_bounds.insert(dir, (iie, self.video_cur_secs));
                                }
                                self.save_settings();
                                self.set_status(format!("Credits START set at {} - applies to this whole folder", fmt_time(self.video_cur_secs)));
                                self.try_cut_current_video(p);
                            }
                        }
                        if clear_intro || clear_creds {
                            self.clear_video_markers(clear_intro, clear_creds);
                        }
                        let mut vv = self.video_volume;
                        let vsl = ui.put(
                            egui::Rect::from_min_size(scr.min + egui::vec2(890.0, 8.0), egui::vec2(170.0, 14.0)),
                            egui::Slider::new(&mut vv, 0..=200).text("VOL"),
                        );
                        if vsl.changed() {
                            self.video_volume = vv;
                            if let Some(s) = &self.video_sink {
                                s.set_volume(vv as f32 / 100.0 * 0.9);
                            }
                            self.save_settings();
                        }
                        let bar2 = egui::Rect::from_min_size(
                            scr.min + egui::vec2(0.0, 30.0),
                            egui::vec2(sw.min(930.0), 26.0),
                        );
                        ui.painter().rect_filled(
                            bar2,
                            0.0,
                            Color32::from_rgba_unmultiplied(0, 0, 0, 140),
                        );
                        if ui
                            .put(
                                egui::Rect::from_min_size(
                                    scr.min + egui::vec2(8.0, 33.0),
                                    egui::vec2(190.0, 20.0),
                                ),
                                egui::Button::new(self.video_audio_label()),
                            )
                            .on_hover_text("Cycle the audio track")
                            .clicked()
                        {
                            audio_cycle = true;
                        }
                        if ui
                            .put(
                                egui::Rect::from_min_size(
                                    scr.min + egui::vec2(206.0, 33.0),
                                    egui::vec2(180.0, 20.0),
                                ),
                                egui::Button::new(self.video_sub_label()),
                            )
                            .on_hover_text("Cycle subtitles: off, embedded tracks, sidecar files")
                            .clicked()
                        {
                            subs_cycle = true;
                        }
                        if ui
                            .put(
                                egui::Rect::from_min_size(
                                    scr.min + egui::vec2(394.0, 33.0),
                                    egui::vec2(110.0, 20.0),
                                ),
                                egui::Button::new("OPEN SUBS"),
                            )
                            .on_hover_text("Load a .srt, .ass, .ssa or .vtt file")
                            .clicked()
                        {
                            open_subs = true;
                        }
                        if ui
                            .put(
                                egui::Rect::from_min_size(
                                    scr.min + egui::vec2(510.0, 33.0),
                                    egui::vec2(120.0, 20.0),
                                ),
                                egui::Button::new("SUB STYLE"),
                            )
                            .on_hover_text("Change subtitle size, position and outline")
                            .clicked()
                        {
                            open_sub_style = true;
                        }
                        if ui
                            .put(
                                egui::Rect::from_min_size(
                                    scr.min + egui::vec2(796.0, 33.0),
                                    egui::vec2(110.0, 20.0),
                                ),
                                egui::Button::new("TRACKS"),
                            )
                            .on_hover_text("Choose audio and subtitle tracks")
                            .clicked()
                        {
                            open_tracks = true;
                        }
                        if self
                            .video_current
                            .as_deref()
                            .map(crate::dvd::is_dvd)
                            .unwrap_or(false)
                        {
                            let lbl = if self.dvd_upscale {
                                if self.video_dims.0 > 0 {
                                    format!(
                                        "DVD SCALE: ON {}x{}",
                                        self.video_dims.0, self.video_dims.1
                                    )
                                } else {
                                    "DVD SCALE: ON".to_string()
                                }
                            } else {
                                "DVD SCALE: OFF".to_string()
                            };
                            let col = if self.dvd_upscale {
                                Color32::from_rgb(120, 255, 120)
                            } else {
                                Color32::from_gray(170)
                            };
                            if ui
                                .put(
                                    egui::Rect::from_min_size(
                                        scr.min + egui::vec2(638.0, 33.0),
                                        egui::vec2(150.0, 20.0),
                                    ),
                                    egui::Button::new(egui::RichText::new(lbl).color(col)),
                                )
                                .on_hover_text("Lanczos upscale and mild sharpening for DVD")
                                .clicked()
                            {
                                dvd_scale_toggle = true;
                            }
                        }
                    }
                    if self.video_bar_visible && !self.video_queue.is_empty() {
                        let qw = 300.0f32;
                        let qrect = egui::Rect::from_min_max(
                            egui::pos2(scr.max.x - qw, scr.min.y + 32.0),
                            egui::pos2(scr.max.x, scr.max.y),
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(qrect), |ui| {
                            ui.painter().rect_filled(qrect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 190));
                            ui.painter().rect_stroke(qrect, 0.0, egui::Stroke::new(1.0, Color32::from_gray(60)), egui::StrokeKind::Inside);
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("QUEUE").color(Color32::from_rgb(120, 255, 120)).monospace().strong().size(12.0));
                                ui.label(RichText::new(format!(" {} ", self.video_queue.len())).color(Color32::from_gray(170)).monospace().size(11.0));
                                if ui.small_button("CLEAR").clicked() { clearq = true; }
                            });
                            ui.separator();
                            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                                let mut remove_here: Option<usize> = None;
                                let mut play_here: Option<usize> = None;
                                for (i, p) in self.video_queue.iter().enumerate() {
                                    let is_cur = i == self.video_queue_idx && self.video_on;
                                    ui.horizontal(|ui| {
                                        let col = if is_cur { Color32::from_rgb(255, 220, 80) } else { Color32::from_gray(200) };
                                        let mark = if is_cur { "▶" } else { " " };
                                        let txt = format!("{} {:02}  {}", mark, i + 1, stem(p));
                                        let label = RichText::new(txt).color(col).monospace().size(11.0);
                                        if is_cur {
                                            let row_rect = egui::Rect::from_min_size(
                                                ui.cursor().min,
                                                egui::vec2(ui.available_width(), 16.0),
                                            );
                                            ui.painter().rect_filled(
                                                egui::Rect::from_min_max(
                                                    egui::pos2(qrect.min.x, row_rect.min.y),
                                                    egui::pos2(qrect.max.x, row_rect.max.y),
                                                ),
                                                0.0,
                                                Color32::from_rgba_unmultiplied(255, 220, 80, 25),
                                            );
                                        }
                                        let resp = ui.add(egui::Label::new(label).truncate());
                                        if resp.clicked() {
                                            play_here = Some(i);
                                        }
                                        if ui.small_button("✕").clicked() {
                                            remove_here = Some(i);
                                        }
                                    });
                                }
                                if let Some(i) = play_here { play_idx = Some(i); }
                                if let Some(i) = remove_here { del_idx = Some(i); }
                            });
                        });
                    }
                    let bottom_hover = mouse.map_or(false, |m| m.y >= sh - 64.0 && m.y <= sh && m.x >= scr.min.x && m.x <= scr.max.x);
                    let show_seek = bottom_hover || self.video_seek_t.is_some();
                    if show_seek {
                        let by = sh - 66.0;
                        let lx = 40.0;
                        let rx = sw - 40.0;
                        let track = egui::Rect::from_min_max(egui::pos2(lx, by + 24.0), egui::pos2(rx, by + 30.0));
                        let yt = if let Some(t) = self.video_seek_t { t } else if self.video_dur_secs > 0.0 { (self.video_cur_secs / self.video_dur_secs).clamp(0.0, 1.0) } else { 0.0 };
                        let p = ui.painter();
                        p.rect_filled(egui::Rect::from_min_max(scr.min + egui::vec2(0.0, by), egui::pos2(scr.max.x, scr.max.y)), 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 170));
                        p.rect_filled(track, 3.0, Color32::from_gray(60));
                        let fw = track.width() * yt;
                        if fw > 0.0 {
                            p.rect_filled(egui::Rect::from_min_size(track.min, egui::vec2(fw, track.height())), 3.0, Color32::from_rgb(255, 200, 60));
                        }
                        let tcol = Color32::from_rgb(225, 225, 225);
                        let shown_cur = if let Some(t) = self.video_seek_t { t * self.video_dur_secs } else { self.video_cur_secs };
                        p.text(egui::pos2(lx, by + 10.0), egui::Align2::LEFT_CENTER, fmt_time(shown_cur), egui::FontId::monospace(13.0), tcol);
                        p.text(egui::pos2(rx, by + 10.0), egui::Align2::RIGHT_CENTER, fmt_time(self.video_dur_secs), egui::FontId::monospace(13.0), tcol);
                        if self.video_dur_secs > 0.0 {
                            let mut t = yt;
                            let slip = ui.put(
                                egui::Rect::from_min_max(egui::pos2(lx, by), egui::pos2(rx, by + 34.0)),
                                egui::Slider::new(&mut t, 0.0..=1.0).show_value(false),
                            );
                            if slip.drag_started() || slip.dragged() {
                                self.video_seek_t = Some(t.clamp(0.0, 1.0));
                            }
                            if slip.drag_stopped() {
                                av_log(&format!(
                                    "seek slider t={:.3} rect={:?} pointer={:?} viewport={:?} ppp={:.2}",
                                    t,
                                    slip.rect,
                                    ctx.input(|i| i.pointer.latest_pos()),
                                    ctx.viewport_rect(),
                                    ctx.input(|i| i.pixels_per_point())
                                ));
                                self.video_seek_t = None;
                                self.seek_video(t.clamp(0.0, 1.0) * self.video_dur_secs);
                            }
                        }
                    }
                    if resp.double_clicked() {
                        togg = true;
                    }
                });
            if stop_video {
                self.close_video();
                return;
            }
            if fs_off { self.set_video_fs(false); }
            if let Some(i) = play_idx { self.play_queue_idx(i); }
            if let Some(i) = del_idx { self.remove_queue_item(i); }
            if clearq { self.video_queue.clear(); self.video_queue_idx = 0; }
            if add {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Video", &["mp4", "mkv", "avi", "webm", "mov"])
                    .pick_files()
                {
                    let vids: Vec<String> = paths.iter().map(|p| p.to_string_lossy().to_string()).collect();
                    self.enqueue_videos(vids);
                }
            }
            if aspect_cycle {
                self.video_aspect = self.video_aspect.next();
                self.video_aspect_default = self.video_aspect;
                if let Some(p) = self.video_current.clone() {
                    self.video_aspects.insert(p, self.video_aspect);
                }
                self.save_settings();
                self.set_status(format!("MOVIE aspect {}: {}", self.video_aspect.label(), stem(self.video_current.as_deref().unwrap_or("-"))));
            }
            if audio_cycle { self.cycle_video_audio(); }
            if subs_cycle { self.cycle_video_subs(); }
            if open_subs { self.open_external_subs(); }
            if open_sub_style { self.video_sub_style_open = true; }
            if open_tracks { self.video_tracks_open = true; }
            if dvd_scale_toggle {
                self.dvd_upscale = !self.dvd_upscale;
                self.save_settings();
                let pos = self.video_position_now();
                self.seek_video(pos);
                self.set_status(if self.dvd_upscale {
                    "DVD scale: Lanczos + sharpening"
                } else {
                    "DVD scale: standard"
                });
            }
            if prev { self.queue_prev(); }
            if next { self.queue_next(); }
            if togg {
                self.toggle_video_play();
            }
            self.video_hide_cursor(ctx, t_now);
            return;
        }
        let vw = 480.0f32;
        let vh = match self.video_aspect {
            VideoAspect::Original => {
                if w > 0 && h > 0 { vw * h as f32 / w as f32 } else { 270.0 }
            }
            VideoAspect::R43 => vw * 3.0 / 4.0,
            VideoAspect::R169 => vw * 9.0 / 16.0,
        };
        let mut close = false;
        let mut togg = false;
        let mut aspect_cycle = false;
        let mut audio_cycle = false;
        let mut subs_cycle = false;
        let mut open_subs = false;
        let mut open_sub_style = false;
        let mut open_tracks = false;
        let mut dvd_scale_toggle = false;
        egui::Area::new(egui::Id::new("movie_win"))
            .fixed_pos(self.video_pos)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(Color32::from_rgb(4, 4, 6))
                    .stroke(egui::Stroke::new(2.0, Color32::from_rgb(96, 96, 108)))
                    .inner_margin(4.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("MOVIE").color(self.theme_colors().accent).monospace().size(10.0));
                            ui.label(
                                RichText::new(if self.video_paused { "PAUSED - double click to resume" } else { "double click to pause" })
                                    .color(if self.video_paused { Color32::from_rgb(255, 220, 80) } else { Color32::from_rgb(120, 255, 120) })
                                    .monospace()
                                    .size(9.0),
                            );
                        });
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(vw, vh), egui::Sense::click_and_drag());
                        if let Some(tex) = &self.video_tex {
                            ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        } else {
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, self.video_loading_text(), egui::FontId::monospace(12.0), Color32::from_gray(120));
                        }
                        if self.video_paused {
                            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(3.0, Color32::from_rgb(255, 220, 80)), egui::StrokeKind::Inside);
                        }
                        self.draw_disc_poster(ui, rect);
                        if resp.dragged() {
                            self.video_pos += resp.drag_delta();
                        }
                        if resp.double_clicked() {
                            togg = true;
                        }
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("VOL").color(self.theme_colors().accent).monospace().size(10.0));
                            let mut vv = self.video_volume;
                            if ui.add(egui::Slider::new(&mut vv, 0..=200).show_value(false)).changed() {
                                self.video_volume = vv;
                                if let Some(s) = &self.video_sink {
                                    s.set_volume(vv as f32 / 100.0 * 0.9);
                                }
                                self.save_settings();
                            }
                            if ui.button("Close").clicked() { close = true; }
                            if ui.button(if self.video_paused { "Play" } else { "Pause" }).clicked() { togg = true; }
                            if ui.button(format!("Aspect {}", self.video_aspect.label())).clicked() { aspect_cycle = true; }
                            if ui.button("Fullscreen [Alt+Enter]").clicked() { self.set_video_fs(true); }
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button(self.video_audio_label()).on_hover_text("Cycle the audio track").clicked() {
                                audio_cycle = true;
                            }
                            if ui.button(self.video_sub_label()).on_hover_text("Cycle subtitles").clicked() {
                                subs_cycle = true;
                            }
                            if ui.button("Open subs").on_hover_text("Load a subtitle file").clicked() {
                                open_subs = true;
                            }
                            if ui.button("Sub style").on_hover_text("Subtitle size, position and outline").clicked() {
                                open_sub_style = true;
                            }
                            if ui.button("Tracks").on_hover_text("Choose audio and subtitle tracks").clicked() {
                                open_tracks = true;
                            }
                            if self.video_current.as_deref().map(crate::dvd::is_dvd).unwrap_or(false)
                                && ui.button(if self.dvd_upscale { "DVD scale: on" } else { "DVD scale: off" })
                                    .on_hover_text("Lanczos upscale and mild sharpening")
                                    .clicked()
                            {
                                dvd_scale_toggle = true;
                            }
                        });
                    });
            });
        if togg {
            self.toggle_video_play();
        }
        if close {
            self.close_video();
        }
        if audio_cycle { self.cycle_video_audio(); }
        if subs_cycle { self.cycle_video_subs(); }
        if open_subs { self.open_external_subs(); }
        if dvd_scale_toggle {
            self.dvd_upscale = !self.dvd_upscale;
            self.save_settings();
            let pos = self.video_position_now();
            self.seek_video(pos);
            self.set_status(if self.dvd_upscale {
                "DVD scale: Lanczos + sharpening"
            } else {
                "DVD scale: standard"
            });
        }
        if aspect_cycle {
            self.video_aspect = self.video_aspect.next();
            self.video_aspect_default = self.video_aspect;
            if let Some(p) = self.video_current.clone() {
                self.video_aspects.insert(p, self.video_aspect);
            }
            self.save_settings();
            self.set_status(format!("MOVIE aspect {}: {}", self.video_aspect.label(), stem(self.video_current.as_deref().unwrap_or("-"))));
        }
    }

    pub(crate) fn video_hide_cursor(&mut self, ctx: &egui::Context, t_now: f64) {
        if !self.video_fs || self.video_paused || self.video_ended || self.video_bar_visible {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
            return;
        }
        if t_now - self.video_last_mouse > 5.0 {
            ctx.set_cursor_icon(egui::CursorIcon::None);
        } else {
            ctx.set_cursor_icon(egui::CursorIcon::Default);
        }
    }
}

/// Median intro/credits markers already set on other files in the same folder.
///
/// This is the safe "mark one episode, skip the whole series" path: it only
/// uses timestamps the user actually set, never guesses, so it cannot cut
/// content on its own.
pub fn learned_folder_bounds(
    bounds: &HashMap<String, (f32, f32)>,
    path: &str,
) -> Option<(f32, f32)> {
    let dir = Path::new(path).parent()?;
    let mut intros: Vec<f32> = Vec::new();
    let mut credits: Vec<f32> = Vec::new();
    for (other, &(i, c)) in bounds {
        if other == path {
            continue;
        }
        if Path::new(other).parent() != Some(dir) {
            continue;
        }
        if i > 1.0 {
            intros.push(i);
        }
        if c > 1.0 {
            credits.push(c);
        }
    }
    if intros.is_empty() && credits.is_empty() {
        return None;
    }
    let median = |v: &mut Vec<f32>| -> f32 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v[v.len() / 2]
    };
    Some((median(&mut intros), median(&mut credits)))
}

/// Resolve the manual intro/credits markers that apply to `path`.
///
/// Markers are only ever written by the user pressing INTRO / CREDS, which
/// stores both a per-file entry and a per-show ("preset") entry. There are no
/// invented fallbacks: an unmarked video resolves to (0.0, 0.0), which every
/// caller reads as "no marker". This is what stops the player from skipping or
/// cutting a video on its own.
pub fn resolve_bounds(
    video_bounds: &HashMap<String, (f32, f32)>,
    show_bounds: &HashMap<String, (f32, f32)>,
    path: Option<&str>,
) -> (f32, f32) {
    let Some(p) = path else { return (0.0, 0.0) };
    if let Some(&b) = video_bounds.get(p) {
        return b;
    }
    if let Some(dir) = Path::new(p).parent().map(|x| x.to_string_lossy().to_string()) {
        if let Some(&b) = show_bounds.get(&dir) {
            return b;
        }
    }
    (0.0, 0.0)
}

/// The index `queue_next` should move to, or `None` when there is no distinct
/// next video.
///
/// A plain modulo would hand back the current index for a single-video queue,
/// which made the player reopen the same file forever - and every reopen is a
/// fresh full-screen decode, i.e. an endless black "LOADING..." screen.
pub fn next_queue_idx(idx: usize, len: usize) -> Option<usize> {
    if len < 2 {
        return None;
    }
    let ni = (idx + 1) % len;
    if ni == idx {
        None
    } else {
        Some(ni)
    }
}

#[cfg(test)]
mod video_tests {
    use super::*;

    fn maps() -> (HashMap<String, (f32, f32)>, HashMap<String, (f32, f32)>) {
        (HashMap::new(), HashMap::new())
    }

    /// Build a platform-native path so these tests exercise the OS separator
    /// logic that `resolve_bounds` actually uses (a hard-coded `E:\...` only
    /// works on Windows and silently broke on Linux).
    fn p(parts: &[&str]) -> String {
        let mut b = std::path::PathBuf::new();
        for part in parts {
            b.push(part);
        }
        b.to_string_lossy().to_string()
    }

    /// The reported bug: with `intro_skip_enabled` on, every video was being
    /// skipped 90s in and cut, because bounds were invented for any video that
    /// had no stored marker.
    #[test]
    fn unmarked_video_has_no_bounds() {
        let (v, s) = maps();
        let (i, c) = resolve_bounds(&v, &s, Some(&p(&["DRAGONBALL", "Dragon Ball.118.mkv"])));
        assert_eq!((i, c), (0.0, 0.0), "must not invent an intro bound");
        assert_eq!((i, c), (0.0, 0.0), "must not invent a credits bound");
    }

    #[test]
    fn no_current_video_has_no_bounds() {
        let (v, s) = maps();
        assert_eq!(resolve_bounds(&v, &s, None), (0.0, 0.0));
    }

    #[test]
    fn manual_per_file_marker_is_used() {
        let (mut v, s) = maps();
        let file = p(&["SHOW", "ep01.mkv"]);
        v.insert(file.clone(), (81.0, 1349.5));
        assert_eq!(resolve_bounds(&v, &s, Some(&file)), (81.0, 1349.5));
    }

    /// The INTRO/CREDS buttons also record a per-show preset, which is what
    /// carries a marker to the rest of an episode run.
    #[test]
    fn show_preset_applies_to_unmarked_episodes() {
        let (v, mut s) = maps();
        s.insert(p(&["SHOW"]), (81.0, 1349.5));
        assert_eq!(
            resolve_bounds(&v, &s, Some(&p(&["SHOW", "ep02.mkv"]))),
            (81.0, 1349.5)
        );
    }

    #[test]
    fn per_file_marker_wins_over_show_preset() {
        let (mut v, mut s) = maps();
        s.insert(p(&["SHOW"]), (81.0, 1349.5));
        let file = p(&["SHOW", "ep02.mkv"]);
        v.insert(file.clone(), (12.0, 0.0));
        assert_eq!(resolve_bounds(&v, &s, Some(&file)), (12.0, 0.0));
    }

    #[test]
    fn learns_markers_from_other_episodes_in_the_folder() {
        let mut v = HashMap::new();
        v.insert(p(&["SHOW", "ep01.mkv"]), (82.0, 1340.0));
        v.insert(p(&["SHOW", "ep02.mkv"]), (84.0, 1344.0));
        v.insert(p(&["SHOW", "ep03.mkv"]), (83.0, 1342.0));
        v.insert(p(&["OTHER", "ep01.mkv"]), (5.0, 50.0));
        let learned = learned_folder_bounds(&v, &p(&["SHOW", "ep99.mkv"])).unwrap();
        assert_eq!(learned, (83.0, 1342.0));
    }

    #[test]
    fn learning_ignores_the_current_file_and_empty_folders() {
        let mut v = HashMap::new();
        v.insert(p(&["SHOW", "ep01.mkv"]), (82.0, 1340.0));
        assert_eq!(learned_folder_bounds(&v, &p(&["SHOW", "ep01.mkv"])), None);
        assert_eq!(learned_folder_bounds(&v, &p(&["EMPTY", "ep01.mkv"])), None);
        assert_eq!(learned_folder_bounds(&HashMap::new(), &p(&["SHOW", "ep02.mkv"])), None);
    }

    #[test]
    fn unrelated_directory_has_no_bounds() {
        let (v, mut s) = maps();
        s.insert(p(&["SHOW"]), (81.0, 1349.5));
        assert_eq!(resolve_bounds(&v, &s, Some(&p(&["Other", "clip.mkv"]))), (0.0, 0.0));
    }

    /// The reported bug: reaching the credits marker with one video queued
    /// replayed that same video forever.
    #[test]
    fn single_video_queue_has_no_next() {
        assert_eq!(next_queue_idx(0, 0), None);
        assert_eq!(next_queue_idx(0, 1), None);
    }

    #[test]
    fn multi_video_queue_advances_and_wraps() {
        assert_eq!(next_queue_idx(0, 2), Some(1));
        assert_eq!(next_queue_idx(1, 2), Some(0));
        assert_eq!(next_queue_idx(0, 3), Some(1));
        assert_eq!(next_queue_idx(2, 3), Some(0));
    }
}

#[cfg(test)]
mod poster_tests {
    use super::*;

    #[test]
    fn finds_folder_cover_and_bluray_metadata_thumbnail() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mpdoom-poster-{}", stamp));
        let meta = dir.join("BDMV/META/DL");
        std::fs::create_dir_all(&meta).unwrap();
        let img = image::RgbaImage::from_pixel(32, 48, image::Rgba([200, 40, 40, 255]));
        let cover = dir.join("cover.jpg");
        image::DynamicImage::ImageRgba8(img.clone())
            .save(&cover)
            .unwrap();
        assert!(find_disc_image(&dir.to_string_lossy()).is_some());

        let thumb = meta.join("bdmt_eng.jpg");
        image::DynamicImage::ImageRgba8(img)
            .save(&thumb)
            .unwrap();
        assert!(find_disc_image(&dir.to_string_lossy()).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
