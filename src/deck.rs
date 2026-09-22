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

// The tape deck: music + radio + disc transport, band/library ingest, tape winding and deck widgets.

impl PlayerApp {
    pub(crate) fn resume_tape(&mut self, path: &str, pos: f32) {
        let pos = pos.max(0.0);
        self.eq_resume = Some(Duration::from_secs_f32(pos));
        self.state.is_paused = false;
        self.state.current_song = Some(path.to_string());
        self.playing_pl_idx = self.state.playlist.iter().position(|p| p == path);
        self.playing = Self::playing_display(path);
        self.do_play(path, path);
        self.set_status(format!("TAPE resumed: {}", stem(path)));
    }

    pub(crate) fn set_deck_mode(&mut self, m: DeckMode) {
        if self.deck_mode == m {
            if m != DeckMode::Tape {
                return;
            }
            if self.play_started.is_none() && !self.radio_on {
                self.stop_radio();
                if let Some((path, pos)) = self.deck_resume.take() {
                    if Path::new(&path).is_file() {
                        self.resume_tape(&path, pos);
                        return;
                    }
                }
                if let Some(cur) = self.state.current_song.clone() {
                    if Path::new(&cur).is_file() {
                        self.resume_tape(&cur, 0.0);
                    }
                }
            }
            return;
        }
        if m == DeckMode::Tape {
            let from_disc = self.deck_mode == DeckMode::Disc;
            self.stop_radio();
            self.deck_mode = m;
            if from_disc {
                self.disc_in = false;
                self.disc_label = String::new();
                self.state.playlist = self.all_songs_ordered();
                self.playing_pl_idx = None;
                self.rebuild_display_cache();
                self.save_settings();
            }
            if let Some((path, pos)) = self.deck_resume.take() {
                if Path::new(&path).is_file() {
                    self.resume_tape(&path, pos);
                }
            }
            return;
        }
        if self.deck_mode == DeckMode::Tape {
            if let Some(cur) = self.state.current_song.clone() {
                if Path::new(&cur).is_file() {
                    self.deck_resume = Some((cur, self.pos.max(0.0)));
                }
            }
        }
        self.stop_radio();
        self.deck_mode = m;
    }

    pub(crate) fn radio_band_range(&self) -> (f32, f32) {
        if self.radio_band == "AM" {
            (530.0, 1700.0)
        } else {
            (87.5, 108.0)
        }
    }

    pub(crate) fn radio_tuned_at(&self) -> Option<usize> {
        let (lo, hi) = self.radio_band_range();
        let mut best: Option<(usize, f32)> = None;
        for (i, s) in self.radio_presets.iter().enumerate() {
            if !s.band.eq_ignore_ascii_case(&self.radio_band) {
                continue;
            }
            if s.freq < lo || s.freq > hi {
                continue;
            }
            let d = (s.freq - self.radio_freq).abs();
            if best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }

    pub(crate) fn start_radio(&mut self, idx: usize) {
        if idx >= self.radio_presets.len() {
            return;
        }
        if self.radio_last_start.elapsed() < Duration::from_millis(1200) && self.radio_on {
            return;
        }
        let st = self.radio_presets[idx].clone();
        self.set_winding(None);
        if let Some(mut c) = self.radio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.radio_loading = true;
        match spawn_radio_stream(&st.url, &ffmpeg_path()) {
            Ok((proc, src)) => {
                self.sink.append(self.tap_viz(EqSource::new(src, self.eq_shared.clone())));
                self.sink.play();
                self.state.is_paused = false;
                self.radio_proc = Some(proc);
                self.radio_playing_idx = Some(idx);
                self.radio_on = true;
                self.radio_loading = false;
                self.radio_last_start = Instant::now();
                self.play_started = None;
                self.state.current_song = None;
                self.set_status(format!("♪ {} · {}", fmt_freq(st.freq), st.name));
            }
            Err(e) => {
                self.radio_loading = false;
                self.radio_on = false;
                self.radio_playing_idx = None;
                self.set_error(format!("Radio: {}", e));
            }
        }
    }

    pub(crate) fn stop_radio(&mut self) {
        if let Some(mut c) = self.radio_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.radio_proc = None;
        self.radio_on = false;
        self.radio_playing_idx = None;
        self.radio_loading = false;
        self.radio_last_moved = 0.0;
        self.radio_tuned = None;
        self.sink.stop();
        self.play_started = None;
        self.state.is_paused = false;
    }

    pub(crate) fn tune_radio_to(&mut self, freq: f32, now: f64) {
        let (lo, hi) = self.radio_band_range();
        self.radio_freq = freq.clamp(lo, hi);
        self.radio_last_moved = now;
    }

    pub(crate) fn tick_radio(&mut self, now: f64) {
        let ever_tuned = self.radio_last_moved > 0.0;
        if !ever_tuned && !self.radio_on {
            return;
        }
        let target = self.radio_tuned_at();
        self.radio_tuned = target;
        if now - self.radio_last_moved > 0.7 {
            if let Some(t) = target {
                if self.radio_playing_idx != Some(t) || (self.radio_playing_idx.is_none() && !self.radio_on) {
                    self.start_radio(t);
                }
            }
        }
        if self.radio_on && !self.radio_loading {
            if self.sink.empty() && self.play_started.is_none() {
                if let Some(idx) = self.radio_playing_idx {
                    let patience = if self.play_started.is_none() {
                        Duration::from_millis(12000)
                    } else {
                        Duration::from_millis(2500)
                    };
                    if self.radio_last_start.elapsed() > patience {
                        self.radio_proc = None;
                        self.start_radio(idx);
                    }
                }
            }
        }
    }

    pub(crate) fn scan_radio(&mut self, now: f64) {
        let (lo, hi) = self.radio_band_range();
        let mut cands: Vec<usize> = self
            .radio_presets
            .iter()
            .enumerate()
            .filter(|(_, s)| s.band.eq_ignore_ascii_case(&self.radio_band) && s.freq >= lo && s.freq <= hi)
            .map(|(i, _)| i)
            .collect();
        cands.sort_by(|&a, &b| {
            let fa = self.radio_presets[a].freq;
            let fb = self.radio_presets[b].freq;
            let ka = if fa <= self.radio_freq { 1 } else { 0 };
            let kb = if fb <= self.radio_freq { 1 } else { 0 };
            ka.cmp(&kb).then_with(|| fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal))
        });
        if let Some(&i) = cands.first() {
            self.tune_radio_to(self.radio_presets[i].freq, now);
            self.start_radio(i);
        }
    }

    pub(crate) fn record_radio(&mut self) {
        if self.recording {
            return;
        }
        let Some(idx) = self.radio_playing_idx else {
            self.set_error("Tune a station first");
            return;
        };
        let st = self.radio_presets[idx].clone();
        let rec_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Recordings");
        let safe: String = st
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let mut dest = rec_dir.join(format!("{}_RADIO.wav", safe));
        let mut n = 2;
        while dest.exists() {
            dest = rec_dir.join(format!("{}_RADIO ({}).wav", safe, n));
            n += 1;
        }
        let _ = self.lib_tx.send(LibCmd::RecordRadio {
            display: format!("{} {}", st.band, st.name),
            url: st.url,
            dest: dest.to_string_lossy().to_string(),
        });
        self.recording = true;
        self.set_status(format!("Recording 30s of {}...", st.name));
    }

    pub(crate) fn insert_disc(&mut self) {
        let mut dialog = rfd::FileDialog::new();
        if let Some(d) = self.last_cd_dir.clone() {
            if Path::new(&d).is_dir() {
                dialog = dialog.set_directory(d);
            }
        }
        if let Some(dir) = dialog.pick_folder() {
            let files = collect_audio(&dir.to_string_lossy());
            if files.is_empty() {
                self.set_error("No audio in that folder");
                return;
            }
            self.last_cd_dir = Some(dir.to_string_lossy().to_string());
            self.disc_saved = self.state.playlist.clone();
            self.state.playlist = files;
            self.rebuild_display_cache();
            self.disc_in = true;
            self.disc_label = dir
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| dir.to_string_lossy().to_string());
            self.save_settings();
            self.ingest_bands(&self.state.playlist.clone(), Some(self.disc_label.clone()));
            self.set_status(format!("DISC IN: {}", self.disc_label));
            if !self.state.playlist.is_empty() {
                let first = self.state.playlist[0].clone();
                if self.state.is_paused { self.unpause_music(); }
                self.play_song(&first);
            }
        }
    }

    pub(crate) fn eject_disc(&mut self) {
        if !self.disc_in {
            return;
        }
        self.disc_in = false;
        if !self.disc_saved.is_empty() {
            self.state.playlist = self.disc_saved.clone();
        }
        self.state.current_song = None;
        self.playing_pl_idx = None;
        self.stop_music();
        if self.radio_on {
            self.stop_radio();
        }
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status("DISC EJECTED");
    }

    pub(crate) fn ingest_bands(&mut self, files: &[String], group: Option<String>) {
        let mut changed = false;
        for f in files {
            let band = group.clone().unwrap_or_else(|| band_of(f));
            if band.is_empty() { continue; }
            if !self.full_library.contains(f) {
                self.full_library.push(f.clone());
                changed = true;
            }
            let list = self.band_map.entry(band.clone()).or_default();
            if !list.contains(f) {
                list.push(f.clone());
                changed = true;
            }
        }
        if changed {
            self.sort_bands_library();
            self.save_bands();
        }
    }

    pub(crate) fn sort_bands_library(&mut self) {
        self.full_library.sort_by_key(|p| song_sort_key(p));
        for list in self.band_map.values_mut() {
            list.sort_by_key(|p| song_sort_key(p));
        }
    }

    pub(crate) fn ingest_dir(&mut self, dir: &str, files: &[String]) {
        let root = Path::new(dir);
        let root_name = root.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let nested_cnt = files.iter().filter(|f| Path::new(f).parent() != Some(root)).count();
        let library_mode = nested_cnt > 0 && nested_cnt > files.len() - nested_cnt;
        let mut changed = false;
        for f in files {
            let p = Path::new(f);
            let band = if library_mode {
                if p.parent() == Some(root) {
                    band_of_file(f)
                } else {
                    p.strip_prefix(root)
                        .ok()
                        .and_then(|rel| rel.components().next())
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                        .unwrap_or_else(|| root_name.clone())
                }
            } else {
                root_name.clone()
            };
            if band.is_empty() { continue; }
            if !self.full_library.contains(f) {
                self.full_library.push(f.clone());
                changed = true;
            }
            let list = self.band_map.entry(band.clone()).or_default();
            if !list.contains(f) {
                list.push(f.clone());
                changed = true;
            }
        }
        if changed {
            self.sort_bands_library();
            self.save_bands();
        }
    }

    pub(crate) fn save_bands(&self) {
        if let Ok(text) = serde_json::to_string_pretty(&self.band_map) {
            let _ = std::fs::write(&self.bands_path, text);
        }
    }

    pub(crate) fn draw_play_switch(&mut self, ui: &mut egui::Ui, th: &ThemePalette) {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("PLAY ORDER").color(darken(th.accent, 0.35)).monospace().size(8.0));
        });
        let size = egui::vec2(64.0, 118.0);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        painter.rect_filled(rect, 9.0, lighten(th.btn_bg, 0.12));
        painter.rect_stroke(rect, 9.0, egui::Stroke::new(2.0, th.btn_fg), egui::StrokeKind::Outside);
        let corners = [
            rect.min + egui::vec2(6.0, 6.0),
            rect.min + egui::vec2(rect.width() - 6.0, 6.0),
            rect.min + egui::vec2(6.0, rect.height() - 6.0),
            rect.max - egui::vec2(6.0, 6.0),
        ];
        for s in corners {
            painter.circle_filled(s, 2.6, darken(th.btn_bg, 0.5));
            if darken(th.btn_fg, 0.3) != Color32::TRANSPARENT {
                painter.circle_stroke(s, 2.6, egui::Stroke::new(1.0, darken(th.btn_fg, 0.3)));
            }
        }
        let groove = rect.shrink2(egui::vec2(19.0, 18.0));
        painter.rect_filled(groove, 5.0, Color32::from_rgb(2, 2, 5));
        painter.rect_stroke(groove, 5.0, egui::Stroke::new(1.5, darken(th.btn_fg, 0.25)), egui::StrokeKind::Inside);
        painter.text(
            egui::pos2(groove.center().x, groove.min.y + 10.0),
            egui::Align2::CENTER_CENTER,
            "ON",
            egui::FontId::monospace(8.0),
            if self.random_on { th.playing_fg } else { Color32::from_gray(60) },
        );
        painter.text(
            egui::pos2(groove.center().x, groove.max.y - 10.0),
            egui::Align2::CENTER_CENTER,
            "OFF",
            egui::FontId::monospace(8.0),
            if !self.random_on { th.playing_fg } else { Color32::from_gray(60) },
        );
        let thr = 26.0;
        if resp.dragged() {
            self.switch_accum = (self.switch_accum - resp.drag_delta().y).clamp(-thr, thr);
        } else if resp.clicked() {
            self.random_on = !self.random_on;
            self.switch_accum = 0.0;
        }
        if resp.drag_stopped() {
            if self.switch_accum.abs() >= thr * 0.5 {
                self.random_on = self.switch_accum > 0.0;
            }
            self.switch_accum = 0.0;
            if self.random_on {
                self.set_status("Random shuffle ON — skip picks a random track");
            } else {
                self.set_status("Normal order ON — tracks play in playlist order");
            }
        }
        let base = if self.random_on { 1.0 } else { 0.0 };
        let kf = (base + self.switch_accum / thr).clamp(0.0, 1.0);
        let knw = 42.0;
        let travel = groove.height() - knw - 8.0;
        let kx = groove.min.x + 4.0;
        let ky = groove.min.y + 4.0 + (1.0 - kf) * (travel.max(0.0));
        let knob = egui::Rect::from_min_size(egui::pos2(kx, ky), egui::vec2(groove.width() - 8.0, knw));
        let knob_col = if self.random_on { mix(th.btn_bg, th.accent, 0.4) } else { darken(th.btn_bg, 0.2) };
        painter.rect_filled(knob, 6.0, knob_col);
        painter.rect_stroke(
            knob,
            6.0,
            egui::Stroke::new(1.5, if self.random_on { lighten(th.accent, 0.15) } else { th.btn_fg }),
            egui::StrokeKind::Inside,
        );
        painter.hline(knob.min.x + 6.0..=knob.max.x - 6.0, knob.center().y, egui::Stroke::new(2.0, darken(knob_col, 0.35)));
        ui.add_space(2.0);
        ui.vertical_centered(|ui| {
            let (txt, col) = if self.random_on { ("RANDOM SHUFFLE", th.accent) } else { ("NORMAL ORDER", th.playing_fg) };
            ui.label(RichText::new(txt).color(col).monospace().size(9.0));
        });
    }

    pub(crate) fn all_songs_ordered(&self) -> Vec<String> {
        let mut band_names: Vec<String> = self.band_map.keys().cloned().collect();
        band_names.sort_by_key(|b| b.to_lowercase());
        let mut seen: HashSet<String> = HashSet::new();
        let mut ordered: Vec<(String, String)> = Vec::new();
        for b in band_names {
            if let Some(v) = self.band_map.get(&b) {
                let mut songs = v.clone();
                songs.sort_by_key(|p| song_sort_key(p));
                for s in songs {
                    if seen.insert(s.clone()) {
                        ordered.push((b.clone(), s));
                    }
                }
            }
        }
        for f in self.full_library.iter() {
            if seen.insert(f.clone()) {
                ordered.push((band_of(f), f.clone()));
            }
        }
        ordered.sort_by_key(|(b, _)| b.to_lowercase());
        ordered.into_iter().map(|(_, s)| s).collect()
    }

    pub(crate) fn choose_band(&mut self, band: String) {
        self.band_sel = band.clone();
        self.disc_in = false;
        self.state.playlist = if band.is_empty() {
            self.all_songs_ordered()
        } else if let Some(v) = self.band_map.get(&band) {
            let mut songs = v.clone();
            songs.sort_by_key(|p| song_sort_key(p));
            songs
        } else {
            Vec::new()
        };
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status(
            if band.is_empty() {
                format!("All tracks: {}", self.state.playlist.len())
            } else {
                format!("Band {}: {} songs", band, self.state.playlist.len())
            },
        );
        if self.radio_on {
            self.stop_radio();
        }
        if !self.state.playlist.is_empty() && !self.state.is_paused {
            self.skip_song();
        }
    }

    pub(crate) fn clear_bands(&mut self) {
        self.band_map.clear();
        self.full_library.clear();
        self.band_sel = String::new();
        self.save_bands();
        self.set_status("Band library cleared");
    }

    pub(crate) fn step_tape(&mut self, dt: f32) {
        if self.tape_reversing {
            const OUT: f32 = 0.55;
            const HOLD: f32 = 0.5;
            self.tape_timer += dt;
            if self.tape_timer < OUT {
                self.tape_anim = self.tape_timer / OUT;
            } else if self.tape_timer < OUT + HOLD {
                self.tape_anim = 1.0;
            } else if self.tape_timer < OUT * 2.0 + HOLD {
                self.tape_anim = 1.0 - (self.tape_timer - OUT - HOLD) / OUT;
            } else {
                self.tape_reversing = false;
                self.tape_anim = 0.0;
            }
        } else if self.tape_out {
            self.tape_anim = (self.tape_anim + dt / 0.5).min(1.0);
        } else {
            self.tape_anim = (self.tape_anim - dt / 0.4).max(0.0);
        }
    }

    pub(crate) fn set_winding(&mut self, w: Option<f32>) {
        let prev = self.winding;
        self.winding = w;
        self.wind_arm = None;
        if self.state.current_song.is_none() {
            return;
        }
        if prev == w {
            return;
        }
        // Disengage previous
        if prev == Some(1.0) {
            self.sink.set_speed(1.0);
        } else if prev == Some(-1.0) && self.wind_playing {
            self.sink.play();
            self.seek_to(self.pos);
        }
        // Engage new
        match w {
            Some(1.0) => {
                if !self.state.is_paused && self.play_started.is_some() {
                    self.sink.set_speed(6.0);
                }
            }
            Some(-1.0) => {
                self.wind_playing = !self.state.is_paused && self.play_started.is_some();
                if self.wind_playing {
                    self.sink.pause();
                }
            }
            _ => {}
        }
    }

    pub(crate) fn step_wind(&mut self, dt: f32) {
        let Some(dir) = self.winding else { return };
        if self.state.current_song.is_none() {
            return;
        }
        let speed = (self.len_secs / 25.0).max(3.0);
        if dir > 0.0 && !self.state.is_paused && self.play_started.is_some() {
            let cur = self.sink.get_pos().as_secs_f32();
            self.pos = cur.clamp(0.0, self.len_secs);
            if self.pos >= self.len_secs && self.len_secs > 0.0 {
                self.set_winding(None);
            }
        } else {
            let np = (self.pos + dir * speed * dt).clamp(0.0, self.len_secs);
            self.pos = np;
        }
    }

    pub(crate) fn req_record(&mut self) {
        if self.recording {
            return;
        }
        let Some(s) = self.state.current_song.clone() else { return };
        if !Path::new(&s).is_file() {
            self.set_error("Nothing to record");
            return;
        }
        let base = stem(&s);
        let rec_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Recordings");
        let mut dest = rec_dir.join(format!("{}_REC.wav", base));
        let mut n = 2;
        while dest.exists() {
            dest = rec_dir.join(format!("{}_REC ({}).wav", base, n));
            n += 1;
        }
        let display = self
            .state
            .playlist
            .iter()
            .position(|p| *p == s)
            .and_then(|i| self.display_cache.get(i).cloned())
            .unwrap_or_else(|| s.clone());
        let _ = self.lib_tx.send(LibCmd::Record {
            display,
            audio: s,
            dest: dest.to_string_lossy().to_string(),
        });
        self.recording = true;
        self.set_status("Recording...");
    }

    pub(crate) fn marquee_text(&self) -> String {
        if let Some(s) = self.state.current_song.clone() {
            if self.meta.1 != "Unknown" {
                format!("♪ {} - {}", self.meta.1, self.meta.0)
            } else {
                format!("♪ {}", stem(&s))
            }
        } else {
            self.status.clone()
        }
    }

    pub(crate) fn update_viz(&mut self, dt: f32) {
        let vh = 76.0;
        let (bars, rms, onset, beat_pulse, bpm) = match self.viz_lock.lock() {
            Ok(st) => (st.bars, st.rms, st.onset, st.beat_pulse, st.bpm),
            Err(p) => {
                let st = p.into_inner();
                (st.bars, st.rms, st.onset, st.beat_pulse, st.bpm)
            }
        };
        let active = !self.state.is_paused && (self.state.current_song.is_some() || self.radio_on);
        self.viz_bpm = if active { bpm } else { 0.0 };
        for i in 0..self.viz_bars.len() {
            let target = if active { (bars[i] * vh).max(2.0) } else { 2.0 };
            let cur = self.viz_bars[i];
            let k = if target > cur {
                1.0 - (-dt * 55.0).exp()
            } else {
                1.0 - (-dt * (14.0 + onset * 70.0)).exp()
            };
            let next = cur + (target - cur) * k;
            self.viz_bars[i] = next.max(2.0).min(vh);
            let peak = self.viz_peaks[i];
            if next > peak {
                self.viz_peaks[i] = next;
            } else {
                let fall = if active { 12.0 + onset * 60.0 } else { 8.0 };
                self.viz_peaks[i] = (peak - dt * fall).max(0.0);
            }
        }
        let dr = if active { (rms - self.viz_rms_prev).clamp(-1.0, 1.0) } else { 0.0 };
        self.viz_rms_prev = rms;
        // Envelope "kick" from a sudden RMS rise, plus the beat-grid pulse from
        // the tempo tracker, so the woofers thump on sustained 4-on-the-floor.
        let env_kick = if active { (dr * 3.0).max(0.0).min(1.0) } else { 0.0 };
        let beat_kick = if active { beat_pulse } else { 0.0 };
        let kick = env_kick.max(beat_kick * 0.9);
        self.viz_pop = kick.max(self.viz_pop * (1.0 - (-dt * 14.0).exp()));
        let tr = if active { rms } else { 0.0 };
        self.viz_rms += (tr - self.viz_rms) * (1.0 - (-dt * 15.0).exp());
    }
}
