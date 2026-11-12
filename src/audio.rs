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

type AutoDecoder = Decoder<BufReader<File>>;

/// Playback source for one queued item. Files use the normal rodio decoder;
/// CD tracks stream raw PCM from cdparanoia, so nothing is written to disk.
pub enum PlaySource {
    File(AutoDecoder),
    #[cfg(target_os = "linux")]
    Cdda(crate::cd::CddaSource),
}

impl Iterator for PlaySource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            PlaySource::File(s) => s.next(),
            #[cfg(target_os = "linux")]
            PlaySource::Cdda(s) => s.next(),
        }
    }
}

impl Source for PlaySource {
    fn current_frame_len(&self) -> Option<usize> {
        match self {
            PlaySource::File(s) => s.current_frame_len(),
            #[cfg(target_os = "linux")]
            PlaySource::Cdda(s) => s.current_frame_len(),
        }
    }

    fn channels(&self) -> u16 {
        match self {
            PlaySource::File(s) => s.channels(),
            #[cfg(target_os = "linux")]
            PlaySource::Cdda(s) => s.channels(),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            PlaySource::File(s) => s.sample_rate(),
            #[cfg(target_os = "linux")]
            PlaySource::Cdda(s) => s.sample_rate(),
        }
    }

    fn total_duration(&self) -> Option<Duration> {
        match self {
            PlaySource::File(s) => s.total_duration(),
            #[cfg(target_os = "linux")]
            PlaySource::Cdda(s) => s.total_duration(),
        }
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        match self {
            PlaySource::File(s) => s.try_seek(pos),
            #[cfg(target_os = "linux")]
            PlaySource::Cdda(s) => s.try_seek(pos),
        }
    }
}

type AutoSource = NotifySource<VizTap<EqSource<PlaySource>>>;

/// Wrapper that records the path when its source actually starts playing, so
/// the UI can catch up to a track that advanced while it was hidden.
pub struct NotifySource<S> {
    inner: S,
    path: String,
    now: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    current: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    history: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    fired: bool,
}

impl<S> NotifySource<S> {
    fn new(
        inner: S,
        path: String,
        now: std::sync::Arc<std::sync::Mutex<Option<String>>>,
        current: std::sync::Arc<std::sync::Mutex<Option<String>>>,
        history: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        NotifySource { inner, path, now, current, history, fired: false }
    }
}

impl<S: Source> Iterator for NotifySource<S>
where
    S::Item: rodio::Sample,
{
    type Item = S::Item;
    fn next(&mut self) -> Option<Self::Item> {
        if !self.fired {
            self.fired = true;
            av_log(&format!("auto-advance: started {}", self.path));
            if let Ok(mut slot) = self.now.lock() {
                *slot = Some(self.path.clone());
            }
            if let Ok(mut cur) = self.current.lock() {
                *cur = Some(self.path.clone());
            }
            if let Ok(mut h) = self.history.lock() {
                if h.last() != Some(&self.path) {
                    h.push(self.path.clone());
                    if h.len() > 200 {
                        h.remove(0);
                    }
                }
            }
        }
        self.inner.next()
    }
}

impl<S: Source> Source for NotifySource<S>
where
    S::Item: rodio::Sample,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.inner.try_seek(pos)
    }
}

fn open_play_source(path: &str) -> Option<(PlaySource, Option<PathBuf>)> {
    #[cfg(target_os = "linux")]
    if let Some((drive, track, duration_ms)) = crate::cd::cdda_parts(path) {
        let src = crate::cd::cdda_source(drive, track, duration_ms)?;
        return Some((PlaySource::Cdda(src), None));
    }
    let (decoder, wav) = match open_decoder(path) {
        Some(d) => (d, None),
        None => {
            let wav = transcode_with_ffmpeg(path, &ffmpeg_path()).ok()?;
            let d = open_decoder(&wav.to_string_lossy())?;
            (d, Some(wav))
        }
    };
    Some((PlaySource::File(decoder), wav))
}

fn build_auto_source(
    path: &str,
    eq_shared: &std::sync::Arc<std::sync::Mutex<EqShared>>,
    viz_lock: &std::sync::Arc<std::sync::Mutex<VizState>>,
    now: &std::sync::Arc<std::sync::Mutex<Option<String>>>,
    current: &std::sync::Arc<std::sync::Mutex<Option<String>>>,
    history: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> Option<(AutoSource, Option<PathBuf>)> {
    let (source, wav) = open_play_source(path)?;
    let eq = EqSource::new(source, eq_shared.clone());
    let tapped = VizTap::new(eq, viz_lock.clone());
    Some((
        NotifySource::new(
            tapped,
            path.to_string(),
            now.clone(),
            current.clone(),
            history.clone(),
        ),
        wav,
    ))
}

/// Prepare and queue one upcoming track while the current one plays. This is
/// what keeps playback moving when egui gets no repaint events (minimized or
/// fully obscured window). The sink holds at most one queued source.
pub(crate) fn spawn_auto_advance(
    sink: std::sync::Arc<Sink>,
    eq_shared: std::sync::Arc<std::sync::Mutex<EqShared>>,
    viz_lock: std::sync::Arc<std::sync::Mutex<VizState>>,
    queue: std::sync::Arc<std::sync::Mutex<VecDeque<String>>>,
    gen: std::sync::Arc<std::sync::atomic::AtomicU64>,
    now: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    active: std::sync::Arc<std::sync::atomic::AtomicBool>,
    hidden: std::sync::Arc<std::sync::atomic::AtomicBool>,
    control: std::sync::Arc<std::sync::Mutex<VecDeque<String>>>,
    paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stopped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    current: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    history: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    tx: Sender<Msg>,
) {
    {
        let now = now.clone();
        let hidden = hidden.clone();
        std::thread::spawn(move || {
            use std::sync::atomic::Ordering;
            let mut last = String::new();
            let mut last_at = std::time::Instant::now() - Duration::from_secs(30);
            loop {
                std::thread::sleep(Duration::from_millis(350));
                if !hidden.load(Ordering::Relaxed) {
                    continue;
                }
                let current = now
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone());
                if let Some(path) = current {
                    if path != last && last_at.elapsed() >= Duration::from_secs(4) {
                        crate::windows::notify_song_path(&path);
                        last = path;
                        last_at = std::time::Instant::now();
                    }
                }
            }
        });
    }
    std::thread::spawn(move || {
        use std::sync::atomic::Ordering;
        let mut seen_playing = false;
        let mut seen_generation = 0u64;
        let mut startup_grace = true;
        loop {
        // Background tray transport. These commands arrive while the UI may be
        // frozen in Hide mode, so they must act on the sink directly.
        let cmd = control.lock().ok().and_then(|mut q| q.pop_front());
        if let Some(cmd) = cmd {
            av_log(&format!("tray-control: {}", cmd));
            let mut start = |path: String| -> bool {
                let Some((source, wav)) =
                    build_auto_source(&path, &eq_shared, &viz_lock, &now, &current, &history)
                else {
                    return false;
                };
                if let Some(w) = wav {
                    let _ = tx.send(Msg::Transcoded { display: path.clone(), wav: w });
                }
                sink.stop();
                sink.append(source);
                sink.play();
                av_log(&format!("tray-control: started {}", path));
                paused.store(false, Ordering::Relaxed);
                stopped.store(false, Ordering::Relaxed);
                active.store(true, Ordering::Relaxed);
                true
            };
            match cmd.as_str() {
                "play_pause" => {
                    if stopped.load(Ordering::Relaxed) || sink.empty() {
                        let path = current
                            .lock()
                            .ok()
                            .and_then(|c| c.clone())
                            .or_else(|| queue.lock().ok().and_then(|mut q| q.pop_front()));
                        if let Some(path) = path {
                            let _ = start(path);
                        }
                    } else if paused.load(Ordering::Relaxed) {
                        sink.play();
                        paused.store(false, Ordering::Relaxed);
                        active.store(true, Ordering::Relaxed);
                    } else {
                        sink.pause();
                        paused.store(true, Ordering::Relaxed);
                        active.store(false, Ordering::Relaxed);
                    }
                }
                "stop" => {
                    sink.stop();
                    paused.store(false, Ordering::Relaxed);
                    stopped.store(true, Ordering::Relaxed);
                    active.store(false, Ordering::Relaxed);
                    if let Ok(mut n) = now.lock() {
                        *n = None;
                    }
                }
                "skip" => {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    if let Some(path) = next {
                        let _ = start(path);
                    }
                }
                "prev" => {
                    let cur = current.lock().ok().and_then(|c| c.clone());
                    let prev = {
                        let mut h = match history.lock() {
                            Ok(h) => h,
                            Err(p) => p.into_inner(),
                        };
                        if h.last().map(|p| Some(p.as_str()) == cur.as_deref()).unwrap_or(false) {
                            h.pop();
                        }
                        h.pop()
                    };
                    if let Some(path) = prev {
                        let _ = start(path);
                    }
                }
                _ => {}
            }
            continue;
        }
        if !active.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(150));
            continue;
        }
        // 0 = current track has not reached the sink yet; >1 = a next track is
        // already queued. Only prepare when exactly the current one is playing.
        let generation = gen.load(Ordering::Relaxed);
        if generation != seen_generation {
            seen_generation = generation;
            seen_playing = false;
        }
        let len = sink.len();
        if len == 1 {
            seen_playing = true;
        }
        #[cfg(target_os = "linux")]
        let next_cdda = queue
            .lock()
            .ok()
            .and_then(|q| q.front().cloned())
            .map(|p| crate::cd::is_cdda(&p))
            .unwrap_or(false);
        #[cfg(not(target_os = "linux"))]
        let next_cdda = false;
        if next_cdda {
            // A CD can only read one place at a time. Do not pre-open the next
            // track while the current one is still playing; poll tightly so
            // the next track starts as soon as the sink drains.
            if len > 0 {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
        } else if len > 1 {
            std::thread::sleep(Duration::from_millis(80));
            continue;
        } else if len == 0 && !seen_playing {
            // The current track has not reached the sink yet (for example an
            // ffmpeg transcode is still being prepared). Never skip it.
            std::thread::sleep(Duration::from_millis(80));
            continue;
        } else if startup_grace && len == 1 && sink.get_pos() < Duration::from_secs(5) {
            // Give only the first song after launch a quiet read window. Later
            // songs pre-open immediately so track changes stay seamless.
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        let path = queue
            .lock()
            .ok()
            .and_then(|mut q| q.pop_front());
        let Some(path) = path else {
            std::thread::sleep(Duration::from_millis(120));
            continue;
        };
        if !active.load(Ordering::Relaxed) || gen.load(Ordering::Relaxed) != generation {
            continue;
        }
        let Some((source, wav)) =
            build_auto_source(&path, &eq_shared, &viz_lock, &now, &current, &history)
        else {
            av_log(&format!("auto-advance: could not open {}", path));
            continue;
        };
        if !active.load(Ordering::Relaxed) || gen.load(Ordering::Relaxed) != generation {
            continue;
        }
        if let Some(w) = wav {
            // Cache the WAV for seeking when the UI comes back. The handler
            // ignores this if a user-started transcode is already pending.
            let _ = tx.send(Msg::Transcoded { display: path.clone(), wav: w });
        }
        av_log(&format!("auto-advance: queued {}", path));
        sink.append(source);
        startup_grace = false;
        }
    });
}

impl PlayerApp {
    pub(crate) fn play_song(&mut self, path: &str) {
        let cdda = crate::cd::is_cdda(path);
        if path.is_empty() || (!cdda && !crate::cue::exists(path)) {
            self.set_error("File not found");
            return;
        }
        self.reset_auto_advance();
        if let Ok(mut c) = self.tray_control.lock() {
            c.clear();
        }
        self.song_starting = true;
        self.tray_paused
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.tray_stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.tray_note_current(path);
        self.state.current_song = Some(path.to_string());
        self.playing_pl_idx = self.state.playlist.iter().position(|p| p == path);
        self.eq_resume = None;
        if !cdda {
            self.state.prev_songs.push(path.to_string());
            *self.state.play_history.entry(path.to_string()).or_insert(0) += 1;
        }
        self.state.song_count += 1;
        self.save_history();
        self.save_settings();
        self.playing = self
            .cd_display
            .get(path)
            .cloned()
            .unwrap_or_else(|| Self::playing_display(path));
        self.set_status(format!("Loading: {}...", stem(path)));
        self.do_play(path, path);
        self.auto_active.store(true, std::sync::atomic::Ordering::Relaxed);
        self.refill_auto_advance();
    }

    /// Abandon any queued next track and invalidate an in-flight preparation.
    /// Called whenever the user or the UI starts/stops/changes playback.
    pub(crate) fn reset_auto_advance(&mut self) {
        self.auto_gen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut q) = self.auto_queue.lock() {
            q.clear();
        }
        if let Ok(mut now) = self.auto_now.lock() {
            *now = None;
        }
        self.auto_active
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Remember a path for the background tray controller, so Skip and
    /// Previous still work while the UI is hidden.
    pub(crate) fn tray_note_current(&self, path: &str) {
        if let Ok(mut cur) = self.tray_current.lock() {
            *cur = Some(path.to_string());
        }
        if let Ok(mut h) = self.tray_history.lock() {
            if h.last().map(|p| p.as_str()) != Some(path) {
                h.push(path.to_string());
                if h.len() > 200 {
                    h.remove(0);
                }
            }
        }
    }

    /// Pull paused/stopped changes made by the tray controller into the UI
    /// state after the window is shown again.
    pub(crate) fn sync_tray_state(&mut self) {
        if self
            .tray_stopped
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            self.state.current_song = None;
            self.playing.clear();
            self.state.is_paused = false;
            self.pos = 0.0;
            self.seek_base = 0.0;
            self.len_secs = 0.0;
            self.play_started = None;
            self.meta = (String::new(), "Unknown".to_string(), "Unknown".to_string());
            self.art_tex = None;
            self.art_state = 1;
            self.set_status("Stopped");
            return;
        }
        let paused = self
            .tray_paused
            .load(std::sync::atomic::Ordering::Relaxed);
        if self.state.is_paused != paused {
            self.state.is_paused = paused;
            self.set_status(if paused { "Paused" } else { "Playing" });
        }
    }

    /// Fill the background queue with upcoming paths. The worker only needs
    /// one ahead at a time; a generous prefill means a minimized player keeps
    /// advancing for hours without a UI frame.
    pub(crate) fn refill_auto_advance(&mut self) {
        if !self.auto_active.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        // A loaded disc always plays in track order: no shuffle, no folder
        // filter and no star filter. This is the whole point of the CD deck.
        let disc_order = self.disc_in && self.deck_mode == DeckMode::Disc;
        let pl = self.state.playlist.clone();
        if pl.is_empty() {
            return;
        }
        let limit = if self.random_on && !disc_order { 200 } else { pl.len().min(200) };
        let mut q = match self.auto_queue.lock() {
            Ok(q) => q,
            Err(p) => p.into_inner(),
        };
        if q.len() >= limit {
            return;
        }
        let weights = self.state.song_weights.clone();
        let mut current = self.state.current_song.clone();
        let mut idx = current
            .as_ref()
            .and_then(|c| pl.iter().position(|p| p == c))
            .unwrap_or(0);
        while q.len() < limit {
            let next = if self.random_on && !disc_order {
                random_next(current.as_deref(), &pl, &weights)
            } else {
                idx = (idx + 1) % pl.len();
                pl[idx].clone()
            };
            q.push_back(next.clone());
            current = Some(next);
        }
    }

    /// Catch the UI up when a background-prepared track starts. The audio is
    /// already playing; this only refreshes song state, tags, art and history.
    pub(crate) fn sync_auto_track(&mut self) {
        let path = match self.auto_now.lock() {
            Ok(mut now) => now.take(),
            Err(p) => p.into_inner().take(),
        };
        let Some(path) = path else { return };
        if self.radio_on || self.video_on {
            return;
        }
        if self.state.current_song.as_deref() == Some(path.as_str()) {
            return;
        }
        let cdda = crate::cd::is_cdda(&path);
        if !cdda && !crate::cue::exists(&path) {
            return;
        }
        self.tray_note_current(&path);
        self.tray_paused
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.tray_stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.state.current_song = Some(path.clone());
        self.playing_pl_idx = self.state.playlist.iter().position(|p| p == &path);
        if !cdda {
            self.state.prev_songs.push(path.clone());
            *self.state.play_history.entry(path.clone()).or_insert(0) += 1;
        }
        self.state.song_count += 1;
        self.save_history();
        self.playing = self
            .cd_display
            .get(&path)
            .cloned()
            .unwrap_or_else(|| Self::playing_display(&path));
        self.pos = 0.0;
        self.seek_base = 0.0;
        self.len_secs = crate::cd::cdda_parts(&path)
            .map(|(_, _, ms)| ms as f32 / 1000.0)
            .unwrap_or(0.0);
        self.state.song_length = Duration::from_secs_f32(self.len_secs);
        self.play_started = Some(Instant::now());
        self.send_meta_art(path.clone());
        self.save_settings();
        self.refill_auto_advance();
    }

    pub(crate) fn send_meta_art(&mut self, path: String) {
        if let Some((_, track, duration_ms)) = crate::cd::cdda_parts(&path) {
            let title = self
                .cd_titles
                .get(&path)
                .cloned()
                .unwrap_or_else(|| format!("Track {:02}", track));
            let artist = if self.cd_artist.trim().is_empty() {
                "Audio CD".to_string()
            } else {
                self.cd_artist.clone()
            };
            let album = if self.cd_album.trim().is_empty() {
                self.disc_label.clone()
            } else {
                self.cd_album.clone()
            };
            self.meta = (title, artist, album);
            if !self.cd_art_loaded {
                self.art_state = 0;
                self.art_tex = None;
            }
            self.art_local_valid = false;
            self.want_web = false;
            self.web_art_for = None;
            self.lyrics = None;
            self.lyrics_for = None;
            self.len_secs = duration_ms as f32 / 1000.0;
            self.state.song_length = Duration::from_millis(duration_ms);
            return;
        }
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
        }
        // Fill gaps from the folder name immediately, before the async tag
        // probe answers, so the artwork lookup still has something to search
        // with on a track whose tags are missing.
        merge_folder_meta(&mut self.meta.1, &mut self.meta.2, &path);
        self.want_web = !is_placeholder_meta(&self.meta.1)
            && (!is_placeholder_meta(&self.meta.2) || !is_placeholder_meta(&self.meta.0));
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
        #[cfg(target_os = "linux")]
        if let Some((drive, track, duration_ms)) = crate::cd::cdda_parts(audio) {
            match crate::cd::cdda_source(drive, track, duration_ms) {
                Some(src) => self.play_source(display, PlaySource::Cdda(src), true),
                None => self.set_error(
                    "cdparanoia is not installed (install it to play audio CDs)",
                ),
            }
            return;
        }
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
                self.set_error(format!("Cannot decode {} (ffmpeg not found - install it or add it to the tools folder)", stem(display)));
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
                    self.set_error(format!("Cannot decode {} (ffmpeg not found - install it or add it to the tools folder)", stem(display)));
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
        self.play_source(display, PlaySource::File(src), forward_track);
    }

    pub(crate) fn play_source(&mut self, display: &str, src: PlaySource, forward_track: bool) {
        self.song_starting = false;
        self.set_winding(None);
        self.seek_base = 0.0;
        if self.radio_on {
            self.stop_radio();
        }
        let new_track = forward_track && self.state.current_song.as_deref() != Some(display);
        if new_track && self.tape_out {
            self.tape_out = false;
        }
        self.kill_audio_pipe();
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        let dur = src.total_duration().unwrap_or_default().as_secs_f32();
        if crate::cd::is_cdda(display) {
            if dur > 0.0 {
                self.len_secs = dur;
                self.state.song_length = Duration::from_secs_f32(dur);
            }
        } else if self.len_secs <= 0.0 && dur > 0.0 {
            self.len_secs = dur;
        }
        self.sink.append(self.tap_viz(EqSource::new(src, self.eq_shared.clone())));
        self.sink.play();
        self.state.current_song = Some(display.to_string());
        if !crate::cd::is_cdda(display) {
            self.state.prev_songs.push(display.to_string());
            *self.state.play_history.entry(display.to_string()).or_insert(0) += 1;
        }
        self.state.song_count += 1;
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
        self.playing = self
            .cd_display
            .get(display)
            .cloned()
            .unwrap_or_else(|| Self::playing_display(display));
        self.set_status(format!("Playing: {}", stem(display)));
        self.send_meta_art(display.to_string());
        self.request_current_rating();
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
            self.manual_ratings.remove(path);
            self.set_status(format!("{}: rating cleared", stem(path)));
        } else {
            self.ratings.insert(path.to_string(), stars);
            self.manual_ratings.insert(path.to_string());
            self.set_status(format!("{}: {} {}", stem(path), "★".repeat(stars as usize), if stars == 1 { "star" } else { "stars" }));
        }
        self.save_settings();
    }

    /// Remove every star rating, including manual ones, and turn the star
    /// filter off so the playlist is not left filtered to nothing.
    pub(crate) fn clear_all_ratings(&mut self) {
        let n = self.ratings.len();
        self.ratings.clear();
        self.manual_ratings.clear();
        self.rating_filter = 0;
        self.save_settings();
        self.set_status(format!(
            "Cleared {} star rating{}",
            n,
            if n == 1 { "" } else { "s" }
        ));
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
        av_log(&format!(
            "skip_song current={:?} elapsed={:?} sink_empty={} paused={}",
            self.state.current_song,
            self.play_started.map(|t| t.elapsed()),
            self.sink.empty(),
            self.state.is_paused
        ));
        if let Some(cur) = self.state.current_song.clone() {
            if self.pos < 10.0 {
                let w = self.state.song_weights.entry(cur.clone()).or_insert(1.0);
                *w *= 0.8;
            }
        }
        if self.state.playlist.is_empty() || self.state.is_paused {
            return;
        }
        if self.disc_in && self.deck_mode == DeckMode::Disc {
            let pl = self.state.playlist.clone();
            let i = self
                .state
                .current_song
                .as_deref()
                .and_then(|c| pl.iter().position(|p| p == c))
                .unwrap_or(0);
            let ni = (i + 1) % pl.len();
            let song = pl[ni].clone();
            self.state.skip_count += 1;
            self.playing_pl_idx = Some(ni);
            self.play_song(&song);
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
        } else if !self.random_on {
            let i = self.state.current_song.as_deref().map(cur_pos).unwrap_or(0);
            let ni = (i + 1) % slots.len();
            (slots[ni].1.clone(), Some(slots[ni].0))
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
        if self.disc_in && self.deck_mode == DeckMode::Disc {
            let pl = self.state.playlist.clone();
            let i = self
                .state
                .current_song
                .as_deref()
                .and_then(|c| pl.iter().position(|p| p == c))
                .unwrap_or(0);
            let pi = (i + pl.len() - 1) % pl.len();
            let prev = pl[pi].clone();
            self.playing_pl_idx = Some(pi);
            self.play_song(&prev);
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
        // The CD deck's PLAY button doubles as "play the disc in the tray"
        // when no disc folder is loaded yet.
        if self.deck_mode == DeckMode::Disc
            && !self.disc_in
            && self.cd_physical.is_some()
        {
            self.start_cd_play();
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
                if crate::cd::is_cdda(&path) {
                    self.play_song(&path);
                    return;
                }
                if crate::cue::exists(&path) {
                    self.resume_tape(&path, pos);
                    return;
                }
            }
        }
        if let Some(cur) = self.state.current_song.clone() {
            if crate::cd::is_cdda(&cur) {
                self.play_song(&cur);
                return;
            }
            if crate::cue::exists(&cur) {
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
        self.auto_active
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.tray_paused
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.tray_stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.set_status("Paused");
    }

    pub(crate) fn unpause_music(&mut self) {
        self.state.is_paused = false;
        self.sink.play();
        self.auto_active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.tray_paused
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.tray_stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.refill_auto_advance();
        self.set_status("Playing");
    }

    pub(crate) fn stop_music(&mut self) {
        self.song_starting = false;
        self.reset_auto_advance();
        if let Ok(mut c) = self.tray_control.lock() {
            c.clear();
        }
        self.tray_paused
            .store(false, std::sync::atomic::Ordering::Relaxed);
        self.tray_stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut cur) = self.tray_current.lock() {
            *cur = None;
        }
        self.kill_audio_pipe();
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
        self.viz_rms_prev = 0.0;
        self.viz_pop = 0.0;
        self.viz_bpm = 0.0;
        // The shared spectrum state is read by the deck; clearing it stops the
        // "frozen bars" look immediately instead of waiting for the decay.
        if let Ok(mut st) = self.viz_lock.lock() {
            *st = crate::viz::VizState::default();
        }
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

    /// M toggles between the current volume and silence, remembering the level
    /// so unmuting restores exactly what the user had.
    pub(crate) fn toggle_mute(&mut self) {
        if self.state.volume == 0 {
            let restore = if self.muted_volume == 0 { 50 } else { self.muted_volume };
            self.set_volume(restore);
            self.set_status("Unmuted");
        } else {
            self.muted_volume = self.state.volume;
            self.set_volume(0);
            self.set_status("Muted");
        }
    }

    /// Seek the current song. Tries the decoder's native seek first; if the
    /// decoder cannot seek (e.g. symphonia's FLAC reader reports Unseekable, so
    /// `Sink::try_seek` fails) the song is reopened and skipped ahead by samples.
    pub(crate) fn seek_to(&mut self, secs: f32) {
        if self.state.current_song.is_none() {
            return;
        }
        if self
            .state
            .current_song
            .as_deref()
            .map(crate::cd::is_cdda)
            .unwrap_or(false)
        {
            self.seek_cdda(secs);
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

    /// In-track CD seek. cdparanoia cannot seek inside a running stream, so
    /// restart it at the requested track offset and keep the same display path.
    fn seek_cdda(&mut self, secs: f32) {
        let Some(path) = self.state.current_song.clone() else {
            return;
        };
        let Some((drive, track, duration_ms)) = crate::cd::cdda_parts(&path) else {
            return;
        };
        let secs = secs.max(0.0);
        let secs = if self.len_secs > 0.0 {
            secs.min(self.len_secs)
        } else {
            secs
        };
        let start_ms = (secs * 1000.0).round() as u64;
        let Some(src) = crate::cd::cdda_source_at(drive, track, duration_ms, start_ms) else {
            self.set_error("CD seek failed: cannot start cdparanoia");
            return;
        };
        let was_paused = self.state.is_paused;
        self.reset_auto_advance();
        self.kill_audio_pipe();
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.sink.append(self.tap_viz(EqSource::new(
            PlaySource::Cdda(src),
            self.eq_shared.clone(),
        )));
        if was_paused {
            self.sink.pause();
        } else {
            self.sink.play();
        }
        self.seek_base = secs;
        self.pos = secs;
        self.state.is_paused = was_paused;
        self.play_started = Some(Instant::now());
        self.song_starting = false;
        self.dragging = false;
        self.auto_active
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.refill_auto_advance();
        self.set_status(format!(
            "CD seek {:02}:{:02}",
            (secs as u32) / 60,
            (secs as u32) % 60
        ));
    }

    pub(crate) fn kill_audio_pipe(&mut self) {
        if let Some(mut c) = self.audio_pipe_proc.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Fast seek for formats whose built-in decoder is unseekable (notably
    /// FLAC). Asking ffmpeg to start at the target avoids decoding every
    /// sample from the file's beginning, which on a spinning HDD could take
    /// longer than the track itself.
    fn seek_via_pipe(&mut self, secs: f32) -> bool {
        let Some(path) = self.state.current_song.clone() else { return false };
        if !crate::cue::exists(&path) {
            return false;
        }
        let src = self.transcodes.get(&path).cloned().unwrap_or(path);
        let ff = ffmpeg_path();
        if !ff.is_file() {
            return false;
        }
        let (child, pipe) = match spawn_video_audio(&src, &ff, secs) {
            Ok(v) => v,
            Err(_) => return false,
        };
        self.kill_audio_pipe();
        self.sink.stop();
        self.sink.set_volume(self.state.volume as f32 / 100.0);
        self.sink.append(self.tap_viz(EqSource::new(pipe, self.eq_shared.clone())));
        if self.state.is_paused {
            self.sink.pause();
        } else {
            self.sink.play();
        }
        self.audio_pipe_proc = Some(child);
        self.seek_base = secs;
        self.pos = secs;
        self.play_started = Some(Instant::now());
        true
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
        if !crate::cue::exists(&path) {
            return;
        }
        let src_path = self.transcodes.get(&path).cloned().unwrap_or_else(|| path.clone());
        let Some(dec) = open_decoder(&src_path) else {
            self.set_error(format!("Seek failed: cannot reopen {}", stem(&path)));
            return;
        };
        // For a real seek, let ffmpeg jump to the position instead of decoding
        // and throwing away every sample before it.
        if secs >= 2.0 && self.seek_via_pipe(secs) {
            return;
        }
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
                // A cold 4 TB HDD can take several seconds to open and buffer
                // the first samples of a FLAC. The old 600 ms patience made
                // the player skip perfectly good tracks in a loop, opening a
                // new file (and its art) every couple of seconds.
                let cdda = self
                    .state
                    .current_song
                    .as_deref()
                    .map(crate::cd::is_cdda)
                    .unwrap_or(false);
                // Between CD tracks the sink is briefly empty while the drive
                // starts the next track. Do not treat that gap as a stall.
                if !cdda && start.elapsed() > Duration::from_secs(5) && self.sink.empty() && self.state.current_song.is_some() {
                    av_log(&format!("watchdog: sink empty after {:?}, skipping", start.elapsed()));
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
            // An optical drive can take a while to deliver the first samples.
            // Say what is happening instead of looking stuck at 0:00.
            if let Some(cur) = &self.state.current_song {
                if crate::cd::is_cdda(cur)
                    && self.play_started.is_some()
                    && self.sink.get_pos().as_secs_f32() < 0.05
                {
                    self.set_status("CD: reading track from the drive...");
                }
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
