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

// Input/action handlers: keyboard, file drops, hotkeys, fullscreen, eq/theme cycling, playlist and sleep-timer actions.

impl PlayerApp {
    pub(crate) fn cycle_theme(&mut self) {
        let idx = THEME_NAMES.iter().position(|t| *t == self.state.theme).unwrap_or(0);
        self.state.theme = THEME_NAMES[(idx + 1) % THEME_NAMES.len()].to_string();
        self.save_settings();
        self.set_status(format!("Theme: {}", self.state.theme));
    }

    pub(crate) fn cycle_eq(&mut self) {
        let names: Vec<&'static str> = EQ_PRESETS.iter().map(|p| p.name).collect();
        let cur = names.iter().position(|n| *n == self.state.eq_preset).unwrap_or(0);
        let next = (cur + 1) % names.len();
        self.state.eq_preset = names[next].to_string();
        self.eq_custom = None;
        self.apply_eq_live();
        self.save_settings();
        self.set_status(format!("EQ: {}", self.eq_name()));
    }

    pub(crate) fn cycle_eq_back(&mut self) {
        let names: Vec<&'static str> = EQ_PRESETS.iter().map(|p| p.name).collect();
        let cur = names.iter().position(|n| *n == self.state.eq_preset).unwrap_or(0);
        let prev = (cur + names.len() - 1) % names.len();
        self.state.eq_preset = names[prev].to_string();
        self.eq_custom = None;
        self.apply_eq_live();
        self.save_settings();
        self.set_status(format!("EQ: {}", self.eq_name()));
    }

    pub(crate) fn eq_set(&mut self, i: usize, v: f32) {
        if i >= 10 {
            return;
        }
        if self.eq_custom.is_none() {
            self.eq_custom = Some(self.eq_cur());
            self.state.eq_preset = "Flat".to_string();
        }
        if let Some(c) = &mut self.eq_custom {
            c[i] = v;
        }
        self.apply_eq_live();
        self.save_settings();
    }

    pub(crate) fn apply_eq_live(&mut self) {
        let gains = if self.eq_on { self.eq_cur() } else { [0.0; 10] };
        if let Ok(mut st) = self.eq_shared.lock() {
            st.gains = gains;
            st.dirty = true;
        }
    }

    pub(crate) fn eq_toggle(&mut self) {
        self.eq_on = !self.eq_on;
        self.apply_eq_live();
        self.save_settings();
        self.set_status(format!("EQ {}", if self.eq_on { format!("{}", self.eq_name()) } else { "off".into() }));
    }

    pub(crate) fn toggle_repeat(&mut self) {
        self.state.repeat_enabled = !self.state.repeat_enabled;
        self.set_status(format!("Repeat {}", if self.state.repeat_enabled { "On" } else { "Off" }));
    }

    pub(crate) fn smart_shuffle(&mut self) {
        if self.state.playlist.len() < 3 {
            self.set_error("Need more songs in playlist");
            return;
        }
        let weight: f32 = self.state.playlist.iter().map(|s| self.state.song_weights.get(s).copied().unwrap_or(1.0)).sum();
        if weight <= 0.0 {
            self.set_error("No weighted songs yet, play more!");
            return;
        }
        self.set_status(format!("Smart shuffle: {} songs", self.state.playlist.len()));
    }

    pub(crate) fn save_song(&mut self) {
        if let Some(s) = self.state.current_song.clone() {
            if !self.state.playlist.contains(&s) {
                self.state.playlist.push(s);
                self.rebuild_display_cache();
                self.save_settings();
                self.set_status("Song saved to playlist");
            }
        }
    }

    pub(crate) fn clear_playlist(&mut self) {
        self.state.playlist.clear();
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status("Playlist cleared");
    }

    pub(crate) fn shuffle_playlist(&mut self) {
        let mut rng = rand::rng();
        let mut pl = std::mem::take(&mut self.state.playlist);
        let n = pl.len();
        for i in (1..n).rev() {
            let j = rng.random_range(0..=i);
            pl.swap(i, j);
        }
        self.state.playlist = pl;
        self.rebuild_display_cache();
        self.save_settings();
        self.set_status("Playlist shuffled");
    }

    pub(crate) fn move_playlist(&mut self, from: usize, to: usize) {
        if from >= self.state.playlist.len() || to > self.state.playlist.len() || from == to {
            return;
        }
        let mut pl = std::mem::take(&mut self.state.playlist);
        let s = pl.remove(from);
        pl.insert(to, s);
        self.state.playlist = pl;
        self.rebuild_display_cache();
        self.save_settings();
    }

    pub(crate) fn set_sleep_timer(&mut self, minutes: i64) {
        if minutes > 0 {
            self.sleep_minutes = minutes as u32;
            self.sleep_deadline = Some(Instant::now() + Duration::from_secs(minutes as u64 * 60));
            self.set_status(format!("Sleep timer: {} min", minutes));
        } else {
            self.sleep_minutes = 0;
            self.sleep_deadline = None;
            self.set_status("Sleep timer off");
        }
    }

    pub(crate) fn request_quit(&mut self, ctx: &egui::Context) {
        self.save_settings();
        self.save_history();
        let _ = ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub(crate) fn handle_drops(&mut self) {
        let dropped: Vec<PathBuf> = self
            .ctx
            .input(|i| i.raw.dropped_files.iter().map(|d| d.path().to_path_buf()).collect());
        if !dropped.is_empty() && dropped != self.last_drop {
            self.last_drop = dropped.clone();
            let entries: Vec<String> = dropped.iter().map(|p| p.to_string_lossy().to_string()).collect();
            self.set_status("Adding files...");
            let _ = self.lib_tx.send(LibCmd::AddMany(entries));
        }
    }

    /// Remove the main window from the taskbar but keep the tray icon and
    /// audio/background auto-advance alive.
    pub(crate) fn hide_to_tray(&mut self) {
        self.hidden_to_tray
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let real_hide = crate::windows::hide_window(&self.ctx);
        self.notify_current_song();
        self.set_status(if real_hide {
            "Hidden to tray — right-click the icon to show"
        } else {
            "Minimized — taskbar hide is not available on this desktop"
        });
    }

    /// Restore the window from Hide mode.
    pub(crate) fn show_from_tray(&mut self) {
        self.hidden_to_tray
            .store(false, std::sync::atomic::Ordering::Relaxed);
        crate::windows::show_window(&self.ctx);
        self.ctx
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        self.set_status("Player shown");
    }

    pub(crate) fn notify_current_song(&self) {
        if let Some(p) = &self.state.current_song {
            notify_song_path(p);
        } else {
            notify_song(
                "",
                "Hidden to tray",
                "Right-click the tray icon and choose Show",
            );
        }
    }

    pub(crate) fn poll_ext(&mut self) {
        while let Some(id) = poll_tray_menu() {
            match id.as_str() {
                "show" => self.show_from_tray(),
                "hide" => self.hide_to_tray(),
                "play_pause" => self.toggle_pause(),
                "skip" => self.skip_song(),
                "prev" => self.prev_song(),
                "stop" => self.stop_music(),
                "quit" => {
                    let ctx = self.ctx.clone();
                    self.request_quit(&ctx);
                }
                _ => {}
            }
        }
        if let Ok(e) = GlobalHotKeyEvent::receiver().try_recv() {
            if e.state == HotKeyState::Pressed {
                match hotkey_action(e.id) {
                    Some("show") => {
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                    Some("quit") => {
                        let ctx = self.ctx.clone();
                        self.request_quit(&ctx);
                    }
                    Some("play_pause") => self.toggle_pause(),
                    Some("skip") => self.skip_song(),
                    Some("prev") => self.prev_song(),
                    Some("stop") => self.stop_music(),
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn handle_keys(&mut self) {
        let focused = self.ctx.memory(|m| m.focused().is_some());
        let (space, left, right, up, down, f, f10, esc, shift, alt_enter) = self.ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::F10),
                i.key_pressed(egui::Key::Escape),
                i.modifiers.shift,
                i.modifiers.alt && i.key_pressed(egui::Key::Enter),
            )
        });
        if !focused {
            if self.video_on {
                if space { self.toggle_video_play(); }
                if left { self.seek_video((self.video_cur_secs - 5.0).max(0.0)); }
                if right { self.seek_video((self.video_cur_secs + 5.0).min((self.video_dur_secs - 1.0).max(0.0))); }
            } else {
                if space { self.toggle_pause(); }
                if right {
                    if shift { self.skip_forward(); } else { self.skip_song(); }
                }
                if left {
                    if shift { self.skip_backward(); } else { self.prev_song(); }
                }
                if up { self.increase_volume(); }
                if down { self.decrease_volume(); }
            }
        }
        if f { self.toggle_fullscreen(); }
        if alt_enter {
            if self.video_on {
                self.set_video_fs(!self.video_fs);
            } else {
                self.toggle_fullscreen();
            }
        }
        if esc && self.fullscreen { self.set_fullscreen(false); }
        if esc && self.video_fs { self.set_video_fs(false); }
        if f10 {
            let ctx = self.ctx.clone();
            self.request_quit(&ctx);
        }

        let chars: Vec<char> = self.ctx.input(|i| {
            i.events.iter().filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.chars().collect::<Vec<char>>()),
                _ => None,
            }).flatten().collect()
        });
        if !focused && !self.video_on {
            for c in chars {
                match c {
                    '+' | '=' => self.increase_volume(),
                    '-' | '_' => self.decrease_volume(),
                    // Documented shortcuts (README) first, then the older
                    // single-letter ones kept as aliases.
                    'p' | 'P' | 'x' | 'X' => self.toggle_pause(),
                    's' | 'S' | 'n' | 'N' => self.skip_song(),
                    'z' | 'Z' => self.prev_song(),
                    'm' | 'M' => self.toggle_mute(),
                    'd' | 'D' => self.cycle_deck_mode(),
                    'r' | 'R' => {
                        if self.radio_on { self.record_radio() } else { self.req_record() }
                    }
                    'v' | 'V' => self.play_video(),
                    'l' | 'L' => self.fetch_lyrics(),
                    't' | 'T' => self.cycle_theme(),
                    'c' | 'C' => self.stop_music(),
                    'b' | 'B' => self.toggle_repeat(),
                    'j' | 'J' => self.search_focus = true,
                    _ => {}
                }
            }
        }
    }

    pub(crate) fn toggle_fullscreen(&mut self) {
        let next = !self.fullscreen;
        self.set_fullscreen(next);
    }

    pub(crate) fn set_fullscreen(&mut self, on: bool) {
        self.fullscreen = on;
        if on {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        } else {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
    }
}
