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

// Music playback engine: song/decoder pipeline, transport, volume, seek and playback-state updates.

/// Discards the first `skip` seconds of a source. Used to implement seeking for
/// decoders that cannot seek themselves (e.g. symphonia's FLAC reader reports
/// `Unseekable`, which makes `Sink::try_seek` fail and the slider "snap back").
pub struct SkipFirst<S> {
    inner: S,
    remaining: u64,
    skip: Duration,
    channels: u16,
}

impl<S: rodio::Source> SkipFirst<S>
where
    S::Item: rodio::Sample,
{
    pub fn new(inner: S, skip: Duration) -> Self {
        let rate = inner.sample_rate().max(1) as f64;
        let ch = inner.channels().max(1) as f64;
        let remaining = (skip.as_secs_f64() * rate * ch) as u64;
        let channels = inner.channels().max(1);
        SkipFirst { inner, remaining, skip, channels }
    }
}

impl<S: rodio::Source> Iterator for SkipFirst<S>
where
    S::Item: rodio::Sample,
{
    type Item = S::Item;
    fn next(&mut self) -> Option<Self::Item> {
        while self.remaining > 0 {
            let item = self.inner.next();
            if item.is_none() {
                return None;
            }
            self.remaining -= 1;
        }
        self.inner.next()
    }
}

impl<S: rodio::Source> rodio::Source for SkipFirst<S>
where
    S::Item: rodio::Sample,
    f32: cpal::FromSample<S::Item>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration().map(|d| d.saturating_sub(self.skip))
    }
}

impl PlayerApp {
    pub(crate) fn play_song(&mut self, path: &str) {
        if path.is_empty() || !Path::new(path).is_file() {
            self.set_error("File not found");
            return;
        }
        self.state.current_song = Some(path.to_string());
        self.playing_pl_idx = self.state.playlist.iter().position(|p| p == path);
        self.eq_resume = None;
        self.state.prev_songs.push(path.to_string());
        self.state.song_count += 1;
        *self.state.play_history.entry(path.to_string()).or_insert(0) += 1;
        self.save_history();
        self.save_settings();
        self.playing = Self::playing_display(path);
        self.set_status(format!("Loading: {}...", stem(path)));
        self.do_play(path, path);
    }

    pub(crate) fn send_meta_art(&mut self, path: String) {
        self.meta = (stem(&path), "Unknown".to_string(), "Unknown".to_string());
        self.art_state = 1;
        self.art_local_valid = false;
        self.want_web = false;
        self.lyrics_for = None;
        self.web_art_for = None;
        // Local cache fast path: tags + cover already fetched into appdata?
        let cached_meta = self.meta_cache.get(&path).cloned();
        if let Some(e) = &cached_meta {
            self.meta = (e.title.clone(), e.artist.clone(), e.album.clone());
            self.state.song_length = Duration::from_secs_f32(e.secs.max(0.0));
            if e.secs > 0.0 {
                self.len_secs = e.secs;
            }
            self.want_web = e.artist != "Unknown" && e.album != "Unknown";
        }
        if let Some(b) = read_art_cache(&path) {
            self.art_local_valid = true;
            self.load_art_texture(b);
        } else {
            self.art_tex = None;
            self.art_state = 1;
            let _ = self.lib_tx.send(LibCmd::ArtLocal(path.clone()));
        }
        // Live refresh keeps cached tags fresh (and backfills the cache on the
        // first play of an uncached track).
        let _ = self.lib_tx.send(LibCmd::Meta(path));
    }

    pub(crate) fn do_play(&mut self, display: &str, audio: &str) {
        let src_path = if let Some(w) = self.transcodes.get(audio) {
            w.clone()
        } else {
            audio.to_string()
        };
        // rodio 0.19's symphonia decoder panics (unreachable! SeekError) when
        // opening MPEG-4 audio (M4A/M4B/M4P) instead of erroring. Route those
        // straight to the cached ffmpeg transcode (work\tc) - the path they
        // were forced through anyway after the panic.
        let needs_ffmpeg = src_path == audio
            && matches!(
                Path::new(&src_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .as_deref(),
                Some("m4a" | "m4b" | "m4p")
            );
        if needs_ffmpeg {
            if !ffmpeg_path().is_file() {
                self.set_error(format!("Cannot decode {} (ffmpeg missing from tools folder)", stem(display)));
                return;
            }
            self.begin_transcode(display, audio);
            return;
        }
        let src = match open_decoder(&src_path) {
            Some(s) => s,
            None => {
                if src_path != audio {
                    self.set_error(format!("Cached transcode unreadable: {}", stem(display)));
                    return;
                }
                if !ffmpeg_path().is_file() {
                    self.set_error(format!("Cannot decode {} (ffmpeg missing from tools folder)", stem(display)));
                    return;
                }
                self.begin_transcode(display, audio);
                return;
            }
        };
        self.play_decoded(display, src, true);
    }

    pub(crate) fn begin_transcode(&mut self, display: &str, audio: &str) {
        self.pending_transcode = Some((display.to_string(), audio.to_string()));
        self.set_status(format!("Converting {} with ffmpeg...", stem(display)));
        let _ = self.lib_tx.send(LibCmd::Transcode {
            display: display.to_string(),
            audio: audio.to_string(),
        });
    }

    pub(crate) fn tap_viz<S: rodio::Source>(&self, src: S) -> VizTap<S>
    where
        S::Item: rodio::Sample,
        f32: cpal::FromSample<S::Item>,
    {
        VizTap::new(src, self.viz_lock.clone())
    }

    pub(crate) fn play_decoded(&mut self, display: &str, src: Decoder<BufReader<File>>, forward_track: bool) {
        self.set_winding(None);
        self.seek_base = 0.0;
        if self.radio_on {
            self.stop_radio();
        }
        let new_track = forward_track && self.state.current_song.as_deref() != Some(display);
        if new_track && self.tape_out {
            self.tape_out = false;
        }
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        let dur = src.total_duration().unwrap_or_default().as_secs_f32();
        if self.len_secs <= 0.0 && dur > 0.0 {
            self.len_secs = dur;
        }
        self.sink.append(self.tap_viz(EqSource::new(src, self.eq_shared.clone())));
        self.sink.play();
        self.state.current_song = Some(display.to_string());
        self.state.prev_songs.push(display.to_string());
        self.state.song_count += 1;
        *self.state.play_history.entry(display.to_string()).or_insert(0) += 1;
        self.state.is_paused = false;
        if new_track {
            self.tape_songs += 1;
            if self.tape_songs > self.tape_cap {
                self.tape_songs = 1;
                self.tape_side = if self.tape_side == 'A' { 'B' } else { 'A' };
                self.tape_reversing = true;
                self.tape_timer = 0.0;
                self.set_status(format!("Auto-reverse  SIDE {}", self.tape_side));
            }
        }
        self.play_started = Some(Instant::now());
        let resume = self.eq_resume.take();
        if let Some(d) = resume {
            if self.sink.try_seek(d).is_ok() {
                self.pos = self.sink.get_pos().as_secs_f32();
            } else {
                // Decoder cannot seek (e.g. FLAC reports Unseekable): restart the
                // source and skip ahead by samples so tape-resume works everywhere.
                self.seek_base = d.as_secs_f32();
                self.reopen_at(self.seek_base);
            }
        } else {
            self.pos = 0.0;
        }
        self.dragging = false;
        self.save_history();
        self.save_settings();
        self.playing = Self::playing_display(display);
        self.set_status(format!("Playing: {}", stem(display)));
        self.send_meta_art(display.to_string());
    }

    /// Star rating (1-5) for a path, 0 when unrated.
    pub(crate) fn song_rating(&self, path: &str) -> u8 {
        self.ratings.get(path).copied().unwrap_or(0).min(5)
    }

    /// Playlist entries that pass the active star filter, as (real index, path).
    /// With the filter off, every entry qualifies.
    pub(crate) fn rated_slots(&self) -> Vec<(usize, String)> {
        let min = self.rating_filter;
        self.state
            .playlist
            .iter()
            .enumerate()
            .filter(|(_, p)| min == 0 || self.song_rating(p) >= min)
            .map(|(i, p)| (i, p.clone()))
            .collect()
    }

    /// Set (1-5) or clear (0) the star rating for a song. Persisted to appdata.
    pub(crate) fn rate_song(&mut self, path: &str, stars: u8) {
        let stars = stars.min(5);
        if stars == 0 {
            self.ratings.remove(path);
            self.set_status(format!("{}: rating cleared", stem(path)));
        } else {
            self.ratings.insert(path.to_string(), stars);
            self.set_status(format!("{}: {} {}", stem(path), "★".repeat(stars as usize), if stars == 1 { "star" } else { "stars" }));
        }
        self.save_settings();
    }

    /// Cycle the star filter: All -> 1+ -> 2+ -> ... -> 5+ -> All.
    pub(crate) fn cycle_rating_filter(&mut self) {
        self.rating_filter = (self.rating_filter + 1) % 6;
        self.save_settings();
        if self.rating_filter == 0 {
            self.set_status("Star filter: All songs");
        } else {
            self.set_status(format!("Star filter: only {}+ star songs play", self.rating_filter));
        }
    }

    pub(crate) fn skip_song(&mut self) {
        if let Some(cur) = self.state.current_song.clone() {
            if self.pos < 10.0 {
                let w = self.state.song_weights.entry(cur.clone()).or_insert(1.0);
                *w *= 0.8;
            }
        }
        if self.state.playlist.is_empty() || self.state.is_paused {
            return;
        }
        let pl = self.state.playlist.clone();
        let min = self.rating_filter;
        let slots = self.rated_slots();
        if slots.is_empty() {
            if min > 0 {
                self.set_status(format!("★{} filter: no songs match", min));
            }
            return;
        }
        let slot_paths: Vec<String> = slots.iter().map(|(_, p)| p.clone()).collect();
        let weights: HashMap<String, f32> = self.state.song_weights.clone();
        let cur_pos = |c: &str| -> usize {
            pl.iter()
                .position(|p| p == c)
                .and_then(|ri| slots.iter().position(|(si, _)| *si == ri))
                .unwrap_or(0)
        };
        let (song, mut idx) = if self.disc_in {
            let i = self.state.current_song.as_deref().map(cur_pos).unwrap_or(0);
            let ni = (i + 1) % slots.len();
            (slots[ni].1.clone(), Some(slots[ni].0))
        } else if self.state.playlist_only_mode && self.state.playlist_only_sequential {
            let ni = self.state.song_count as usize % slots.len();
            (slots[ni].1.clone(), Some(slots[ni].0))
        } else if !self.random_on {
            let i = self.state.current_song.as_deref().map(cur_pos).unwrap_or(0);
            let ni = (i + 1) % slots.len();
            (slots[ni].1.clone(), Some(slots[ni].0))
        } else if self.state.playlist_only_mode {
            (random_next(self.state.current_song.as_deref(), &slot_paths, &weights), None)
        } else if self.state.dir_sequential {
            if let Some(dir) = self.state.current_dir.clone() {
                let files = collect_audio(&dir);
                let rated: Vec<String> = files.iter().filter(|f| min == 0 || self.song_rating(f) >= min).cloned().collect();
                if !rated.is_empty() {
                    let picked = rated[self.state.song_count as usize % rated.len()].clone();
                    let pi = pl.iter().position(|p| *p == picked);
                    (picked, pi)
                } else {
                    (random_next(self.state.current_song.as_deref(), &slot_paths, &weights), None)
                }
            } else {
                (random_next(self.state.current_song.as_deref(), &slot_paths, &weights), None)
            }
        } else {
            (random_next(self.state.current_song.as_deref(), &slot_paths, &weights), None)
        };
        if idx.is_none() {
            idx = pl.iter().position(|p| *p == song);
        }
        self.state.skip_count += 1;
        self.playing_pl_idx = idx;
        self.play_song(&song);
    }

    pub(crate) fn prev_song(&mut self) {
        if self.state.playlist.is_empty() || self.state.is_paused {
            return;
        }
        let pl = self.state.playlist.clone();
        let min = self.rating_filter;
        let slots = self.rated_slots();
        if slots.is_empty() {
            if min > 0 {
                self.set_status(format!("★{} filter: no songs match", min));
            }
            return;
        }
        let i = self
            .state
            .current_song
            .as_deref()
            .and_then(|c| pl.iter().position(|p| p == c))
            .and_then(|ri| slots.iter().position(|(si, _)| *si == ri))
            .unwrap_or(0);
        let pi = (i + slots.len() - 1) % slots.len();
        let (prev, real_i) = (slots[pi].1.clone(), slots[pi].0);
        self.playing_pl_idx = Some(real_i);
        self.play_song(&prev);
    }

    pub(crate) fn toggle_pause(&mut self) {
        self.play_press();
    }

    pub(crate) fn play_press(&mut self) {
        if self.radio_on {
            if self.state.is_paused {
                self.unpause_music();
            } else {
                self.pause_music();
            }
            return;
        }
        let loaded = self.state.current_song.is_some();
        let active = self.play_started.is_some() && !self.sink.empty();
        if loaded && active {
            if self.state.is_paused {
                self.unpause_music();
            } else {
                self.pause_music();
            }
            return;
        }
        if self.state.current_song.is_none() {
            if let Some((path, pos)) = self.deck_resume.take() {
                if Path::new(&path).is_file() {
                    self.resume_tape(&path, pos);
                    return;
                }
            }
        }
        if let Some(cur) = self.state.current_song.clone() {
            if Path::new(&cur).is_file() {
                self.resume_tape(&cur, self.pos.max(0.0));
                return;
            }
        }
        if !self.state.playlist.is_empty() {
            let pl = self.state.playlist.clone();
            let idx = self.playing_pl_idx.filter(|i| *i < pl.len()).unwrap_or(0);
            self.playing_pl_idx = Some(idx);
            let song = pl[idx].clone();
            self.play_song(&song);
            return;
        }
        self.set_status("Nothing to play - add music first");
    }

    pub(crate) fn pause_music(&mut self) {
        self.state.is_paused = true;
        self.sink.pause();
        self.set_status("Paused");
    }

    pub(crate) fn unpause_music(&mut self) {
        self.state.is_paused = false;
        self.sink.play();
        self.set_status("Playing");
    }

    pub(crate) fn stop_music(&mut self) {
        self.sink.stop();
        self.sink.set_speed(1.0);
        self.play_started = None;
        self.winding = None;
        self.wind_arm = None;
        self.wind_playing = false;
        self.state.current_song = None;
        self.state.is_paused = false;
        self.playing.clear();
        self.pos = 0.0;
        self.seek_base = 0.0;
        self.len_secs = 0.0;
        self.meta = (String::new(), "Unknown".to_string(), "Unknown".to_string());
        self.lyrics = None;
        self.lyrics_title = String::new();
        self.art_local_valid = false;
        self.tape_anim = 0.0;
        self.tape_timer = 0.0;
        for i in 0..self.viz_bars.len() {
            self.viz_bars[i] = 2.0;
            self.viz_peaks[i] = 0.0;
        }
        self.viz_rms = 0.0;
        self.viz_pop = 0.0;
    }

    pub(crate) fn set_volume(&mut self, level: u32) {
        self.state.volume = level.clamp(0, 100);
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.save_settings();
    }

    pub(crate) fn increase_volume(&mut self) {
        self.set_volume(self.state.volume + 5);
    }

    pub(crate) fn decrease_volume(&mut self) {
        self.set_volume(self.state.volume.saturating_sub(5));
    }

    /// Seek the current song. Tries the decoder's native seek first; if the
    /// decoder cannot seek (e.g. symphonia's FLAC reader reports Unseekable, so
    /// `Sink::try_seek` fails) the song is reopened and skipped ahead by samples.
    pub(crate) fn seek_to(&mut self, secs: f32) {
        if self.state.current_song.is_none() {
            return;
        }
        let secs = secs.max(0.0);
        if self.sink.try_seek(Duration::from_secs_f32(secs)).is_ok() {
            self.pos = secs;
            self.seek_base = 0.0;
            return;
        }
        self.seek_base = secs;
        self.reopen_at(secs);
    }

    pub(crate) fn seek_skip(&mut self, d: f32) {
        if self.state.current_song.is_none() {
            return;
        }
        let upper = if self.len_secs > 0.0 { self.len_secs } else { f32::INFINITY };
        let np = (self.pos + d).clamp(0.0, upper);
        self.seek_to(np);
    }

    /// Restart the current song and drop samples up to `secs` so seeking works
    /// even for decoders without native seek support. Keeps pause/volume and
    /// the EQ + visualizer pipeline intact.
    fn reopen_at(&mut self, secs: f32) {
        let Some(path) = self.state.current_song.clone() else { return };
        if !Path::new(&path).is_file() {
            return;
        }
        let src_path = self.transcodes.get(&path).cloned().unwrap_or_else(|| path.clone());
        let Some(dec) = open_decoder(&src_path) else {
            self.set_error(format!("Seek failed: cannot reopen {}", stem(&path)));
            return;
        };
        let was_paused = self.state.is_paused;
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        let skip = Duration::from_secs_f32(secs.max(0.0));
        self.sink
            .append(self.tap_viz(EqSource::new(SkipFirst::new(dec, skip), self.eq_shared.clone())));
        self.sink.play();
        if was_paused {
            self.sink.pause();
        }
        self.pos = secs;
        self.play_started = Some(Instant::now());
    }

    pub(crate) fn skip_forward(&mut self) {
        self.seek_skip(10.0);
    }

    pub(crate) fn skip_backward(&mut self) {
        self.seek_skip(-10.0);
    }

    pub(crate) fn update_playback_state(&mut self) {
        if !self.state.is_paused {
            if let Some(start) = self.play_started {
                if start.elapsed() > Duration::from_millis(600) && self.sink.empty() && self.state.current_song.is_some() {
                    let now = Instant::now();
                    if now.duration_since(self.last_skip) > Duration::from_millis(1500) {
                        self.last_skip = now;
                        if self.state.repeat_enabled {
                            if let Some(s) = self.state.current_song.clone() {
                                self.play_song(&s);
                            }
                        } else {
                            self.skip_song();
                        }
                    }
                }
            }
        }
        if !self.state.is_paused && self.state.current_song.is_some() {
            if self.winding.is_none() && !self.dragging {
                // After a reopen-seek the sink position restarts at 0 within the
                // freshly appended source, so add the reopen offset back on top.
                self.pos = self.seek_base + self.sink.get_pos().as_secs_f32();
            }
        }
        if self.scanning {
            let d = self.scan_tick.elapsed().as_millis() / 300 % 3;
            let dots = ".".repeat(d as usize + 1);
            self.running = format!("SCANNING{}  {} songs found", dots, self.scan_found);
        } else if self.state.start_time.is_some() {
            let d = self.state.start_time.as_ref().map(|t| t.elapsed()).unwrap_or_default();
            self.running = format!("Songs: {} | Skips: {} | {}", self.state.song_count, self.state.skip_count, fmt_run(d));
        }
        if let Some(until) = self.status_until {
            if Instant::now() > until {
                self.status = format!("{} v{}", APP_NAME, APP_VERSION);
                self.status_until = None;
            }
        }
        if let Some(until) = self.error_until {
            if Instant::now() > until {
                self.error.clear();
                self.error_until = None;
            }
        }
    }
}
