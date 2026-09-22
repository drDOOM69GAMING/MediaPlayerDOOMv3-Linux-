#![allow(unused_imports)]
#![allow(dead_code)]
use eframe::egui;
use eframe::egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use rand::Rng;
use rodio::{Decoder, OutputStreamHandle, Sink, Source};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::Foundation::HWND;
use crate::app::{PlayerApp, PlayerState};
use crate::{
    consts::*, deck_widgets::*, eq::*, ffmpeg::*, files::*, library::*,
    lib_thread::*, log::*, network::*, pipe::*, settings::*, theme::*,
    tools::*, types::*, util::*, viz::*, windows::*, ytdlp::*,
};

// Video playback: open/seek/window, video queue, bounds/cut analysis and video-audio attach.

impl PlayerApp {
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
        match spawn_video_audio(path, &ffmpeg_path(), seek) {
            Ok((mut child, pipe)) => {
                if let Ok(sink) = Sink::try_new(&self._stream_handle) {
                    sink.set_volume(self.video_volume as f32 / 100.0 * 0.9);
                    sink.pause();
                    sink.append(EqSource::new(pipe, self.eq_shared.clone()));
                    if !self.video_paused {
                        if self.video_tex.is_some() {
                            sink.play();
                            av_log("audio started immediately (tex present)");
                        } else {
                            self.video_audio_pending = true;
                            av_log("audio pending first frame");
                        }
                    }
                    self.video_open_at = self.ctx.input(|i| i.time);
                    self.video_audio_proc = Some(child);
                    self.video_sink = Some(sink);
                } else {
                    let _ = child.kill();
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

    pub(crate) fn seek_video(&mut self, tsecs: f32) {
        let Some(path) = self.video_current.clone() else { return };
        let was_paused = self.video_paused;
        let mut tsecs = tsecs.max(0.0);
        if self.video_dur_secs > 0.0 {
            tsecs = tsecs.min((self.video_dur_secs - 1.0).max(0.0));
        }
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
        }
        if let Some(s) = self.video_sink.take() {
            s.stop();
        }
        self.video_gen += 1;
        let gen = self.video_gen;
        let _ = self.lib_tx.send(LibCmd::VideoOpen { path: path.clone(), gen, seek: tsecs });
        self.video_closing = false;
        self.video_paused = false;
        self.video_ended = false;
        self.video_cur_secs = tsecs;
        self.video_tex = None;
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
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
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
        let _ = self.lib_tx.send(LibCmd::VideoOpen { path: path.clone(), gen, seek: saved_pos });
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
        self.set_status(format!("MOVIE {}/{}: {}", self.video_queue_idx + 1, self.video_queue.len(), stem(&path)));
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

    pub(crate) fn queue_next(&mut self) {
        if self.video_queue.is_empty() {
            return;
        }
        let ni = (self.video_queue_idx + 1) % self.video_queue.len();
        self.play_queue_idx(ni);
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

    pub(crate) fn video_bound_secs(&self) -> (f32, f32) {
        let mut intro = if self.intro_skip_secs > 2.0 { self.intro_skip_secs } else { 90.0 };
        let mut credits = if self.video_dur_secs > 0.0 {
            (self.video_dur_secs - self.credits_skip_secs).max(0.0)
        } else {
            0.0
        };
        if let Some(p) = self.video_current.clone() {
            if let Some(&(ie, cs)) = self.video_bounds.get(&p) {
                if ie > 2.0 {
                    intro = ie;
                }
                if cs > 2.0 {
                    credits = cs;
                }
            } else if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                if let Some(&(ie, cs)) = self.show_bounds.get(&dir) {
                    if ie > 2.0 {
                        intro = ie;
                    }
                    if cs > 2.0 {
                        credits = cs;
                    }
                }
            }
        }
        if credits < 10.0 && self.video_dur_secs > 0.0 {
            credits = (self.video_dur_secs - self.credits_skip_secs).max(0.0);
        }
        (intro, credits)
    }

        pub(crate) fn maybe_analyze_video(&mut self, path: &str) {
        if !self.intro_skip_enabled {
            return;
        }
        let known = self
            .video_bounds
            .get(path)
            .map(|b| b.0 > 2.0 || b.1 > 2.0)
            .unwrap_or(false);
        if known || self.analyzing_video.contains(path) {
            return;
        }
        self.analyzing_video.insert(path.to_string());
        let gen = self.video_gen;
        av_log(&format!("analyze START {}", path));
        let _ = self.lib_tx.send(LibCmd::VideoAnalyze { path: path.to_string(), gen });
        self.set_status(format!("ANALYZE intro/credits: {}", stem(path)));
    }

    pub(crate) fn try_cut_current_video(&mut self, path: String) {
        if !self.intro_skip_enabled {
            return;
        }
        if path.ends_with("_cut.mkv") || self.video_cut_inflight.contains(&path) || self.video_cut_cache.contains_key(&path) {
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
        self.video_tex = None;
        self.set_video_fs(false);
        if let Some(mut c) = self.video_audio_proc.take() {
            let _ = c.kill();
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
                (m.y <= scr.min.y + 34.0 && m.x >= scr.min.x && m.x <= scr.max.x) || queue_region.contains(m)
            });
            self.video_bar_visible = hovering;
            let mut fs_off = false;
            let mut togg = false;
            let mut next = false;
            let mut prev = false;
            let mut add = false;
            let mut aspect_cycle = false;
            let mut play_idx: Option<usize> = None;
            let mut del_idx: Option<usize> = None;
            let mut clearq = false;
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
                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "LOADING...", egui::FontId::monospace(14.0), Color32::from_gray(140));
                    }
                    if self.video_paused {
                        ui.painter().rect_stroke(img, 0.0, egui::Stroke::new(3.0, Color32::from_rgb(255, 220, 80)), egui::StrokeKind::Inside);
                    }
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
                            if self.intro_skip_enabled {
                                if let Some(p) = self.video_current.clone() {
                                    self.analyzing_video.remove(&p);
                                    self.maybe_analyze_video(&p);
                                }
                            }
                        }
                        let (bie, bcs) = self.video_bound_secs();
                        let bnd_lbl = if bie > 0.0 || bcs > 0.0 {
                            format!("INTRO {}", fmt_time(bie))
                        } else {
                            "INTRO --".to_string()
                        };
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(640.0, 4.0), egui::vec2(88.0, 22.0)), egui::Button::new(bnd_lbl)).clicked() {
                            if let Some(p) = self.video_current.clone() {
                                let (_, ccs) = self.video_bounds.get(&p).copied().unwrap_or((0.0, bcs));
                                self.video_bounds.insert(p.clone(), (self.video_cur_secs, ccs));
                                if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                                    let (_, pc) = self.show_bounds.get(&dir).copied().unwrap_or((0.0, 0.0));
                                    self.show_bounds.insert(dir, (self.video_cur_secs, pc));
                                }
                                self.save_settings();
                                self.set_status(format!("Intro END set at {} (preset)", fmt_time(self.video_cur_secs)));
                                self.try_cut_current_video(p);
                            }
                        }
                        let crd_lbl = if bcs > 0.0 {
                            format!("CREDS {}", fmt_time(bcs))
                        } else {
                            "CREDS --".to_string()
                        };
                        if ui.put(egui::Rect::from_min_size(scr.min + egui::vec2(734.0, 4.0), egui::vec2(86.0, 22.0)), egui::Button::new(crd_lbl)).clicked() {
                            if let Some(p) = self.video_current.clone() {
                                let (iie, _) = self.video_bounds.get(&p).copied().unwrap_or((0.0, 0.0));
                                self.video_bounds.insert(p.clone(), (iie, self.video_cur_secs));
                                if let Some(dir) = Path::new(&p).parent().map(|x| x.to_string_lossy().to_string()) {
                                    let (pi, _) = self.show_bounds.get(&dir).copied().unwrap_or((0.0, 0.0));
                                    self.show_bounds.insert(dir, (pi, self.video_cur_secs));
                                }
                                self.save_settings();
                                self.set_status(format!("Credits START set at {} (preset)", fmt_time(self.video_cur_secs)));
                            }
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
                                self.video_seek_t = None;
                                self.seek_video(t.clamp(0.0, 1.0) * self.video_dur_secs);
                            }
                        }
                    }
                    if resp.double_clicked() {
                        togg = true;
                    }
                });
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
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "LOADING...", egui::FontId::monospace(12.0), Color32::from_gray(120));
                        }
                        if self.video_paused {
                            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(3.0, Color32::from_rgb(255, 220, 80)), egui::StrokeKind::Inside);
                        }
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
                    });
            });
        if togg {
            self.toggle_video_play();
        }
        if close {
            self.close_video();
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
