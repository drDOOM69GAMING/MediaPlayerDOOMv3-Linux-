#![allow(unused_imports)]
#![allow(dead_code)]
//! Player application: state, core helpers and the event loop.
//!
//! The app is split across feature modules that each extend
//! `impl PlayerApp`: audio (music engine), deck (tape/radio/disc UI),
//! download (yt-dlp queue + lyrics), handlers (input/actions),
//! panels (main UI) and video (video playback).
use crate::{
    consts::*, deck_widgets::*, eq::*, ffmpeg::*, files::*, library::*,
    lib_thread::*, log::*, network::*, pipe::*, settings::*, theme::*,
    tools::*, types::*, util::*, viz::*, windows::*, ytdlp::*,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use rand::Rng;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use lofty::prelude::*;

pub(crate) struct PlayerState {
    pub(crate) volume: u32,
    pub(crate) current_song: Option<String>,
    pub(crate) playlist: Vec<String>,
    pub(crate) prev_songs: Vec<String>,
    pub(crate) repeat_enabled: bool,
    pub(crate) playlist_only_mode: bool,
    pub(crate) playlist_only_sequential: bool,
    pub(crate) dir_sequential: bool,
    pub(crate) is_paused: bool,
    pub(crate) song_count: u64,
    pub(crate) skip_count: u64,
    pub(crate) start_time: Option<Instant>,
    pub(crate) current_dir: Option<String>,
    pub(crate) theme: String,
    pub(crate) song_length: Duration,
    pub(crate) play_history: HashMap<String, u32>,
    pub(crate) song_weights: HashMap<String, f32>,
    pub(crate) eq_preset: String,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            volume: 50,
            theme: "Winamp".to_string(),
            eq_preset: "Flat".to_string(),
            current_song: None,
            playlist: Vec::new(),
            prev_songs: Vec::new(),
            repeat_enabled: false,
            playlist_only_mode: false,
            playlist_only_sequential: false,
            dir_sequential: false,
            is_paused: false,
            song_count: 0,
            skip_count: 0,
            start_time: None,
            current_dir: None,
            song_length: Duration::ZERO,
            play_history: HashMap::new(),
            song_weights: HashMap::new(),
        }
    }
}

pub(crate) struct PlayerApp {
    pub(crate) ctx: egui::Context,
    pub(crate) _stream: OutputStream,
    pub(crate) _stream_handle: OutputStreamHandle,
    pub(crate) sink: Sink,
    pub(crate) state: PlayerState,
    pub(crate) settings_path: PathBuf,
    pub(crate) history_path: PathBuf,
    pub(crate) lib_tx: Sender<LibCmd>,
    pub(crate) net_tx: Sender<NetCmd>,
    pub(crate) web_tx: Sender<NetCmd>,
    pub(crate) yt_tx: Sender<YtCmd>,
    pub(crate) msg_rx: Receiver<Msg>,
    pub(crate) status: String,
    pub(crate) error: String,
    pub(crate) status_until: Option<Instant>,
    pub(crate) error_until: Option<Instant>,
    pub(crate) meta: (String, String, String),
    pub(crate) playing: String,
    pub(crate) art_tex: Option<TextureHandle>,
    pub(crate) art_state: u8,
    pub(crate) art_local_valid: bool,
    pub(crate) display_cache: Vec<String>,
    pub(crate) tag_names: HashMap<String, String>,
    pub(crate) tags_pending: HashSet<String>,
    pub(crate) meta_cache: HashMap<String, MetaCacheEntry>,
    pub(crate) meta_cache_dirty: bool,
    pub(crate) meta_cache_last_save: Instant,
    pub(crate) meta_all_pending: HashSet<String>,
    pub(crate) art_all_pending: HashSet<String>,
    pub(crate) web_all_pending: HashSet<String>,
    pub(crate) meta_all_active: bool,
    pub(crate) no_art: HashSet<String>,
    pub(crate) no_art_dirty: bool,
    pub(crate) pl_view_h: f32,
    pub(crate) playing_pl_idx: Option<usize>,
    pub(crate) pl_last_cur: Option<usize>,
    pub(crate) search_query: String,
    pub(crate) yt_query: String,
    pub(crate) yt_queue_mode: bool,
    pub(crate) yt_queue: Vec<(String, String)>,
    pub(crate) yt_queue_processing: bool,
    pub(crate) yt_queue_active: Option<String>,
    pub(crate) yt_queue_show: bool,
    pub(crate) eq_shared: std::sync::Arc<std::sync::Mutex<EqShared>>,
    pub(crate) eq_resume: Option<Duration>,
    pub(crate) yt_log: Vec<String>,
    pub(crate) yt_pct: f32,
    pub(crate) yt_speed: String,
    pub(crate) yt_eta: String,
    pub(crate) last_skip: Instant,
    pub(crate) play_started: Option<Instant>,
    pub(crate) pos: f32,
    pub(crate) len_secs: f32,
    pub(crate) seek_base: f32,
    pub(crate) dragging: bool,
    pub(crate) running: String,
    pub(crate) scanning: bool,
    pub(crate) scan_found: usize,
    pub(crate) scan_tick: std::time::Instant,
    pub(crate) fullscreen: bool,
    pub(crate) fs_asserted: bool,
    pub(crate) video_fs_restore_app: bool,
    pub(crate) tape_anim: f32,
    pub(crate) tape_out: bool,
    pub(crate) tape_reversing: bool,
    pub(crate) tape_timer: f32,
    pub(crate) tape_side: char,
    pub(crate) tape_songs: u32,
    pub(crate) tape_cap: u32,
    pub(crate) winding: Option<f32>,
    pub(crate) wind_arm: Option<(f32, f64)>,
    pub(crate) wind_playing: bool,
    pub(crate) recording: bool,
    pub(crate) last_time: f64,
    pub(crate) pending_transcode: Option<(String, String)>,
    pub(crate) transcodes: HashMap<String, String>,
    pub(crate) sleep_minutes: u32,
    pub(crate) sleep_input: String,
    pub(crate) sleep_deadline: Option<Instant>,
    pub(crate) fading: bool,
    pub(crate) fade_start: Instant,
    pub(crate) fade_from_vol: u32,
    pub(crate) show_help: bool,
    pub(crate) search_focus: bool,
    pub(crate) yt_focus: bool,
    pub(crate) eq_on: bool,
    pub(crate) eq_custom: Option<[f32; 10]>,
    pub(crate) lyrics: Option<String>,
    pub(crate) lyrics_title: String,
    pub(crate) lyrics_for: Option<String>,
    pub(crate) web_art_for: Option<String>,
    pub(crate) viz_bars: Vec<f32>,
    pub(crate) viz_peaks: Vec<f32>,
    pub(crate) viz_lock: std::sync::Arc<std::sync::Mutex<VizState>>,
    pub(crate) viz_rms: f32,
    pub(crate) viz_rms_prev: f32,
    pub(crate) viz_pop: f32,
    pub(crate) viz_bpm: f32,
    pub(crate) want_web: bool,
    pub(crate) last_drop: Vec<PathBuf>,
    pub(crate) drag_from: Option<usize>,
    pub(crate) drag_hover: Option<usize>,
    pub(crate) drag_active: bool,
    pub(crate) _tray: Option<TrayIcon>,
    pub(crate) _hkm: Option<GlobalHotKeyManager>,
    pub(crate) show_id: u32,
    pub(crate) quit_id: u32,
    pub(crate) radio_presets: Vec<RadioStation>,
    pub(crate) show_radio: bool,
    pub(crate) deck_mode: DeckMode,
    pub(crate) radio_freq: f32,
    pub(crate) radio_band: String,
    pub(crate) radio_tuned: Option<usize>,
    pub(crate) radio_on: bool,
    pub(crate) radio_playing_idx: Option<usize>,
    pub(crate) radio_loading: bool,
    pub(crate) radio_proc: Option<Child>,
    pub(crate) radio_last_moved: f64,
    pub(crate) radio_last_start: Instant,
    pub(crate) show_add_radio: bool,
    pub(crate) add_name: String,
    pub(crate) add_band: String,
    pub(crate) add_freq: String,
    pub(crate) add_url: String,
    pub(crate) disc_in: bool,
    pub(crate) disc_label: String,
    pub(crate) last_cd_dir: Option<String>,
    pub(crate) disc_saved: Vec<String>,
    pub(crate) band_map: HashMap<String, Vec<String>>,
    pub(crate) full_library: Vec<String>,
    pub(crate) band_sel: String,
    pub(crate) bands_path: PathBuf,
    pub(crate) video_on: bool,
    pub(crate) video_paused: bool,
    pub(crate) video_dims: (u32, u32),
    pub(crate) video_tex: Option<egui::TextureHandle>,
    pub(crate) video_pos: egui::Pos2,
    pub(crate) video_fs: bool,
    pub(crate) video_sink: Option<Sink>,
    pub(crate) video_audio_proc: Option<Child>,
    pub(crate) video_audio_pending: bool,
    pub(crate) video_open_at: f64,
    pub(crate) video_clock: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub(crate) video_seek_base: f32,
    pub(crate) video_deferred_open: bool,
    pub(crate) video_last_mouse: f64,
    pub(crate) video_bar_visible: bool,
    pub(crate) video_queue: Vec<String>,
    pub(crate) video_queue_idx: usize,
    pub(crate) video_gen: u64,
    pub(crate) video_closing: bool,
    pub(crate) video_aspect: VideoAspect,
    pub(crate) video_dur_secs: f32,
    pub(crate) video_cur_secs: f32,
    pub(crate) video_seek_t: Option<f32>,
    pub(crate) video_current: Option<String>,
    pub(crate) video_ended: bool,
    pub(crate) video_aspects: HashMap<String, VideoAspect>,
    pub(crate) video_aspect_default: VideoAspect,
    pub(crate) video_volume: u32,
    pub(crate) deck_resume: Option<(String, f32)>,
    pub(crate) video_positions: HashMap<String, f32>,
    pub(crate) intro_skip_enabled: bool,
    pub(crate) intro_skip_secs: f32,
    pub(crate) credits_skip_secs: f32,
    pub(crate) video_bounds: HashMap<String, (f32, f32)>,
    pub(crate) show_bounds: HashMap<String, (f32, f32)>,
    pub(crate) analyzing_video: HashSet<String>,
    pub(crate) video_cut_cache: HashMap<String, String>,
    pub(crate) video_cut_inflight: HashSet<String>,
    pub(crate) random_on: bool,
    pub(crate) switch_accum: f32,
    pub(crate) ratings: HashMap<String, u8>,
    pub(crate) rating_filter: u8,
}

impl PlayerApp {
    fn load_meta_cache() -> HashMap<String, MetaCacheEntry> {
        std::fs::read(meta_cache_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn load_no_art() -> HashSet<String> {
        std::fs::read(missing_art_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        #[cfg(target_os = "windows")]
        apply_taskbar_icon(cc);
        let ctx = cc.egui_ctx.clone();

        let (stream, handle) = OutputStream::try_default()?;
        let sink = Sink::try_new(&handle)?;
        sink.pause();

        let settings_path = data_dir().join("settings.json");
        let history_path = data_dir().join("history.json");

        let mut state = PlayerState::default();
        let mut video_aspects: HashMap<String, VideoAspect> = HashMap::new();
        let mut video_aspect_default = VideoAspect::Original;
        let mut video_volume = 100;
        let mut ratings: HashMap<String, u8> = HashMap::new();
        let mut rating_filter = 0u8;
        let mut last_cd_dir: Option<String> = None;
        let mut video_positions: HashMap<String, f32> = HashMap::new();
        let mut intro_skip_enabled = false;
        let mut intro_skip_secs: f32 = 90.0;
        let mut eq_on = false;
        let mut eq_custom: Option<[f32; 10]> = None;
        let mut credits_skip_secs: f32 = 90.0;
        let mut video_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        let mut show_bounds: HashMap<String, (f32, f32)> = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(&settings_path) {
            if let Ok(s) = serde_json::from_str::<Settings>(&text) {
                state.volume = s.volume.clamp(0, 100);
                state.current_song = s.last_played.filter(|p| Path::new(p).is_file());
                state.playlist = s.playlist;
                state.playlist.sort_by_key(|p| song_sort_key(p));
                if !s.theme.is_empty() { state.theme = s.theme; }
                video_aspects = s.video_aspects;
                video_aspect_default = s.video_aspect_default;
                video_volume = s.video_volume.clamp(0, 200);
                last_cd_dir = s.last_cd_dir.filter(|d| Path::new(d).is_dir());
                video_positions = s.video_positions;
                intro_skip_enabled = s.intro_skip_enabled;
                intro_skip_secs = s.intro_skip_secs;
                credits_skip_secs = s.credits_skip_secs;
                video_bounds = s.video_bounds;
                show_bounds = s.show_bounds;
                eq_on = s.eq_on;
                if !s.eq_preset.is_empty() { state.eq_preset = s.eq_preset; }
                eq_custom = s.eq_custom;
                ratings = s.ratings;
                rating_filter = s.rating_filter.min(5);
            }
        }
        if let Ok(text) = std::fs::read_to_string(&history_path) {
            if let Ok(h) = serde_json::from_str::<HashMap<String, u32>>(&text) {
                state.play_history = h;
            }
        }
        sink.set_volume(state.volume as f32 / 100.0);

        let video_clock = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let (lib_tx, lib_rx) = channel::<LibCmd>();
        let (net_tx, net_rx) = channel::<NetCmd>();
        let (web_tx, web_rx) = channel::<NetCmd>();
        let (yt_tx, yt_rx) = channel::<YtCmd>();
        let (msg_tx, msg_rx) = channel::<Msg>();
        {
            let tx = msg_tx.clone();
            let clock = std::sync::Arc::clone(&video_clock);
            thread::spawn(move || lib_loop(lib_rx, tx, clock));
        }
        {
            let tx = msg_tx.clone();
            thread::spawn(move || net_loop(net_rx, tx));
        }
        let web_rx = std::sync::Arc::new(std::sync::Mutex::new(web_rx));
        for _ in 0..6 {
            let rx = std::sync::Arc::clone(&web_rx);
            let tx = msg_tx.clone();
            thread::spawn(move || web_art_loop(rx, tx));
        }
        {
            let tx = msg_tx.clone();
            thread::spawn(move || yt_loop(yt_rx, tx));
        }

        let display_cache: Vec<String> = state.playlist.iter().map(|p| make_display(p)).collect();

        let old_bands = data_dir().join("bands.json");
        if old_bands.exists() {
            let _ = std::fs::remove_file(&old_bands);
        }
        let old2 = data_dir().join("bands2.json");
        if old2.exists() {
            let _ = std::fs::remove_file(&old2);
        }
        let bands_path = data_dir().join("bands3.json");
        let band_map = load_bands(&bands_path);
        let full_library: Vec<String> = state.playlist.clone();

        let tray_config = make_tray().ok();
        let hotkeys = make_hotkeys();
        let (show_id, quit_id) = hotkeys.as_ref().map(|(_, s, q)| (*s, *q)).unwrap_or((0, 0));

        let mut app = Self {
            ctx: ctx.clone(),
            _stream: stream,
            _stream_handle: handle,
            sink,
            state,
            settings_path,
            history_path,
            lib_tx,
            net_tx,
            web_tx,
            yt_tx,
            msg_rx,
            status: format!("{} v{} - ready", APP_NAME, APP_VERSION),
            error: String::new(),
            status_until: None,
            error_until: None,
            meta: (String::new(), "Unknown".to_string(), "Unknown".to_string()),
            playing: String::new(),
            art_tex: None,
            art_state: 0,
            art_local_valid: false,
            display_cache,
            tag_names: HashMap::new(),
            tags_pending: HashSet::new(),
            meta_cache: Self::load_meta_cache(),
            meta_cache_dirty: false,
            meta_cache_last_save: Instant::now(),
            meta_all_pending: HashSet::new(),
            art_all_pending: HashSet::new(),
            web_all_pending: HashSet::new(),
            meta_all_active: false,
            no_art: Self::load_no_art(),
            no_art_dirty: false,
            pl_view_h: 300.0,
            playing_pl_idx: None,
            pl_last_cur: None,
            search_query: String::new(),
            yt_query: String::new(),
            yt_queue_mode: false,
            yt_queue: Vec::new(),
            yt_queue_processing: false,
            yt_queue_active: None,
            yt_queue_show: false,
            eq_shared: std::sync::Arc::new(std::sync::Mutex::new(EqShared { gains: [0.0; 10], dirty: false })),
            eq_resume: None,
            yt_log: Vec::new(),
            yt_pct: -1.0,
            yt_speed: String::new(),
            yt_eta: String::new(),
            last_skip: Instant::now(),
            play_started: None,
            pos: 0.0,
            len_secs: 0.0,
            seek_base: 0.0,
            dragging: false,
            running: String::new(),
            scanning: true,
            scan_found: 0,
            scan_tick: std::time::Instant::now(),
            fullscreen: true,
            fs_asserted: false,
            video_fs_restore_app: false,
            tape_anim: 0.0,
            tape_out: false,
            tape_reversing: false,
            tape_timer: 0.0,
            tape_side: 'A',
            tape_songs: 0,
            tape_cap: 5,
            winding: None,
            wind_arm: None,
            wind_playing: false,
            recording: false,
            radio_presets: load_radio(),
            show_radio: false,
            deck_mode: DeckMode::Tape,
            radio_freq: 88.1,
            radio_band: "FM".to_string(),
            radio_tuned: None,
            radio_on: false,
            radio_playing_idx: None,
            radio_loading: false,
            radio_proc: None,
            radio_last_moved: 0.0,
            radio_last_start: Instant::now(),
            show_add_radio: false,
            add_name: String::new(),
            add_band: "FM".to_string(),
            add_freq: String::new(),
            add_url: String::new(),
            disc_in: false,
            disc_label: String::new(),
            last_cd_dir,
            disc_saved: Vec::new(),
            band_map,
            full_library,
            band_sel: String::new(),
            video_on: false,
            video_paused: false,
            video_dims: (640, 360),
            video_tex: None,
            video_pos: egui::pos2(60.0, 60.0),
            video_fs: false,
            video_sink: None,
            video_audio_proc: None,
            video_audio_pending: false,
            video_open_at: 0.0,
            video_clock,
            video_seek_base: 0.0,
            video_deferred_open: false,
            video_last_mouse: 0.0,
            video_bar_visible: false,
            video_queue: Vec::new(),
            video_queue_idx: 0,
            video_gen: 0,
            video_closing: false,
            video_aspect: VideoAspect::Original,
            video_dur_secs: 0.0,
            video_cur_secs: 0.0,
            video_seek_t: None,
            video_current: None,
video_ended: false,
        video_aspects,
        video_aspect_default,
        video_volume,
        deck_resume: None,
        video_positions: video_positions,
        intro_skip_enabled: intro_skip_enabled,
        intro_skip_secs: intro_skip_secs,
        credits_skip_secs: credits_skip_secs,
        video_bounds: video_bounds,
        video_cut_cache: HashMap::new(),
        video_cut_inflight: HashSet::new(),
        show_bounds: show_bounds,
            analyzing_video: HashSet::new(),
            random_on: true,
            ratings,
            rating_filter,
            switch_accum: 0.0,
            bands_path,
            last_time: 0.0,
            pending_transcode: None,
            transcodes: HashMap::new(),
            sleep_minutes: 0,
            sleep_input: "0".to_string(),
            sleep_deadline: None,
            fading: false,
            fade_start: Instant::now(),
            fade_from_vol: 50,
            show_help: false,
            search_focus: false,
            yt_focus: false,
            eq_on,
            eq_custom,
            lyrics: None,
            lyrics_title: String::new(),
            lyrics_for: None,
            web_art_for: None,
            viz_bars: vec![2.0; 12],
            viz_peaks: vec![0.0; 12],
            viz_lock: std::sync::Arc::new(std::sync::Mutex::new(VizState::default())),
            viz_rms: 0.0,
            viz_rms_prev: 0.0,
            viz_pop: 0.0,
            viz_bpm: 120.0,
            want_web: false,
            last_drop: Vec::new(),
            drag_from: None,
            drag_hover: None,
            drag_active: false,
            _tray: tray_config,
            _hkm: hotkeys.map(|(h, _, _)| h),
            show_id,
            quit_id,
        };
        app.apply_eq_live();
        let _ = app.lib_tx.send(LibCmd::FindMusicFolder);
        app.request_playlist_tags();
        Ok(app)
    }

    pub(crate) fn save_settings(&self) {
        let s = Settings {
            volume: self.state.volume,
            last_played: self.state.current_song.clone(),
            playlist: self.state.playlist.clone(),
            theme: self.state.theme.clone(),
            video_aspects: self.video_aspects.clone(),
            video_aspect_default: self.video_aspect_default,
            video_volume: self.video_volume,
            last_cd_dir: self.last_cd_dir.clone(),
            video_positions: self.video_positions.clone(),
            intro_skip_enabled: self.intro_skip_enabled,
            intro_skip_secs: self.intro_skip_secs,
            credits_skip_secs: self.credits_skip_secs,
            video_bounds: self.video_bounds.clone(),
            show_bounds: self.show_bounds.clone(),
            eq_on: self.eq_on,
            eq_preset: self.eq_name().to_string(),
            eq_custom: self.eq_custom,
            video_cut_cache: self.video_cut_cache.clone(),
            video_cut_inflight: self.video_cut_inflight.clone(),
            ratings: self.ratings.clone(),
            rating_filter: self.rating_filter,
        };
        if let Ok(text) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write(&self.settings_path, text);
        }
    }

    pub(crate) fn save_history(&self) {
        if let Ok(text) = serde_json::to_string_pretty(&self.state.play_history) {
            let _ = std::fs::write(&self.history_path, text);
        }
    }

    pub(crate) fn palette(&self) -> ThemePalette {
        palette(&self.state.theme)
    }

    pub(crate) fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
        self.status_until = Some(Instant::now() + Duration::from_secs(5));
    }

    pub(crate) fn set_error(&mut self, s: impl Into<String>) {
        self.error = s.into();
        self.error_until = Some(Instant::now() + Duration::from_secs(5));
    }

    pub(crate) fn theme_colors(&self) -> ThemePalette {
        self.palette()
    }

    pub(crate) fn apply_visuals(&self, ctx: &egui::Context) {
        let th = self.palette();
        let mut vis = egui::Visuals::dark();
        vis.panel_fill = th.bg;
        vis.window_fill = th.bg;
        vis.extreme_bg_color = th.highlight;
        vis.faint_bg_color = th.highlight;
        vis.hyperlink_color = th.accent;
        vis.override_text_color = Some(th.fg);
        vis.selection.bg_fill = th.accent;
        vis.selection.stroke = egui::Stroke::new(1.0, th.fg);
        vis.slider_trailing_fill = true;
        let set = |w: &mut egui::style::WidgetVisuals| {
            w.bg_fill = th.btn_bg;
            w.weak_bg_fill = th.highlight;
            w.fg_stroke = egui::Stroke::new(1.0, th.btn_fg);
            w.bg_stroke = egui::Stroke::new(1.0, th.highlight);
            w.corner_radius = egui::CornerRadius::same(2);
        };
        set(&mut vis.widgets.noninteractive);
        set(&mut vis.widgets.inactive);
        set(&mut vis.widgets.hovered);
        set(&mut vis.widgets.active);
        set(&mut vis.widgets.open);
        vis.widgets.hovered.bg_fill = th.highlight;
        vis.widgets.active.bg_fill = th.highlight;
        ctx.set_visuals(vis);
        let mut style = egui::Style::default();
        style.spacing.slider_width = 160.0;
        style.spacing.item_spacing = egui::vec2(5.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        ctx.set_style_of(egui::Theme::Dark, style);
    }

    pub(crate) fn rebuild_display_cache(&mut self) {
        let pl = self.state.playlist.clone();
        let mut seed: Vec<(String, String)> = Vec::new();
        let mut cache: Vec<String> = Vec::with_capacity(pl.len());
        for p in &pl {
            if let Some(n) = self.tag_names.get(p) {
                cache.push(n.clone());
            } else if let Some(e) = self.meta_cache.get(p) {
                // Local cache hit: real name with no lofty re-read needed.
                let n = playlist_display(&e.artist, &e.title, p);
                seed.push((p.clone(), n.clone()));
                cache.push(n);
            } else {
                cache.push(make_display(p));
            }
        }
        for (p, n) in seed {
            self.tag_names.insert(p, n);
        }
        self.display_cache = cache;
        self.request_playlist_tags();
    }

    pub(crate) fn save_meta_cache(&mut self) {
        if !self.meta_cache_dirty {
            return;
        }
        if let Ok(json) = serde_json::to_string(&self.meta_cache) {
            if std::fs::write(meta_cache_path(), json).is_ok() {
                self.meta_cache_dirty = false;
                self.meta_cache_last_save = Instant::now();
            }
        }
    }

    pub(crate) fn save_no_art(&mut self) {
        if !self.no_art_dirty {
            return;
        }
        if let Ok(json) = serde_json::to_string(&self.no_art) {
            if std::fs::write(missing_art_path(), json).is_ok() {
                self.no_art_dirty = false;
            }
        }
    }

    /// "Meta All": batch-fetch tags + cover art for every track in the current
    /// playlist into the local %APPDATA% cache. Tags stream back via the tag
    /// worker, covers are scanned locally by the art worker and any track still
    /// missing art with known artist/album gets a web cover fetch.
    pub(crate) fn download_all_meta(&mut self) {
        let pl = self.state.playlist.clone();
        if pl.is_empty() {
            self.set_status("No playlist loaded - load or scan a library first");
            return;
        }
        if self.meta_all_active {
            let n = self.meta_all_pending.len() + self.art_all_pending.len() + self.web_all_pending.len();
            self.set_status(format!("Meta All still running: {} items queued...", n));
            return;
        }
        let need_meta: Vec<String> = pl
            .iter()
            .filter(|p| !self.meta_cache.contains_key(*p) && !self.meta_all_pending.contains(*p))
            .cloned()
            .collect();
        let have_meta = pl.len() - need_meta.len();
        let need_art: Vec<String> = pl
            .iter()
            .filter(|p| !art_cache_path(p).is_file() && !self.no_art.contains(*p) && !self.art_all_pending.contains(*p))
            .cloned()
            .collect();
        let have_art = pl.len() - need_art.len();
        if need_meta.is_empty() && need_art.is_empty() {
            self.set_status(format!(
                "All done here! Everything is already cached locally ({} tracks) - add more music and hit Meta All again to download more.",
                pl.len()
            ));
            return;
        }
        self.meta_all_active = true;
        const CHUNK: usize = 200;
        for chunk in need_meta.chunks(CHUNK) {
            self.meta_all_pending.extend(chunk.iter().cloned());
            let _ = self.lib_tx.send(LibCmd::BatchMeta(chunk.to_vec()));
        }
        for chunk in need_art.chunks(CHUNK) {
            self.art_all_pending.extend(chunk.iter().cloned());
            let _ = self.lib_tx.send(LibCmd::ArtAll(chunk.to_vec()));
        }
        self.set_status(format!(
            "Meta All (8 parallel workers): resuming - {} tags + {} covers already cached; fetching {} tags and {} covers...",
            have_meta, have_art, need_meta.len(), need_art.len()
        ));
    }

    fn meta_all_tick(&mut self, msg: String) {
        if !self.meta_all_active {
            return;
        }
        self.set_status(msg);
        if self.meta_all_pending.is_empty() && self.art_all_pending.is_empty() && self.web_all_pending.is_empty() {
            self.meta_all_active = false;
            let total = self.state.playlist.len();
            let no_art_cnt = self.state.playlist
                .iter()
                .filter(|p| self.no_art.contains(*p) && !art_cache_path(p).is_file())
                .count();
            let tail = if no_art_cnt > 0 {
                format!(" ({} found no art - remembered so they're skipped next time)", no_art_cnt)
            } else {
                String::new()
            };
            self.set_status(format!(
                "All done here! Tags + covers cached for {} tracks{} - add more music and hit Meta All again to download more meta.",
                total, tail
            ));
            self.save_meta_cache();
            self.save_no_art();
        }
    }

    /// Ask the library thread for tag metadata of playlist entries we do not
    /// have yet (async, so the list keeps working with fallback names meanwhile).
    pub(crate) fn request_playlist_tags(&mut self) {
        let need: Vec<String> = self
            .state
            .playlist
            .iter()
            .filter(|p| !self.tag_names.contains_key(*p) && !self.tags_pending.contains(*p))
            .cloned()
            .collect();
        if !need.is_empty() {
            for p in &need {
                self.tags_pending.insert(p.clone());
            }
            // Split into small batches so tag results stream back progressively
            // and the lib loop never carries one huge blocking request.
            const CHUNK: usize = 200;
            for chunk in need.chunks(CHUNK) {
                let _ = self.lib_tx.send(LibCmd::TagBatch(chunk.to_vec()));
            }
        }
    }

    pub(crate) fn playing_display(path: &str) -> String {
        let p = Path::new(path);
        let folder = p.parent().and_then(|x| x.file_name()).map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| "?".to_string());
        format!("{} > {}", folder, beautify_name(&stem(path)))
    }

    pub(crate) fn eq_cur(&self) -> [f32; 10] {
        if let Some(c) = self.eq_custom {
            return c;
        }
        EQ_PRESETS.iter().find(|p| p.name == self.state.eq_preset).map(|p| p.bands).unwrap_or(EQ_PRESETS[0].bands)
    }

    pub(crate) fn eq_name(&self) -> &str {
        if self.eq_custom.is_some() { "Custom" } else { self.state.eq_preset.as_str() }
    }

    pub(crate) fn drain_msg(&mut self) {
        while let Ok(m) = self.msg_rx.try_recv() {
            match m {
                Msg::Error(s) => self.set_error(s),
                Msg::DirScanned { dir, files } => {
                    self.scanning = false;
                    self.state.current_dir = Some(dir.clone());
                    self.ingest_dir(&dir, &files);
                    self.state.playlist = files;
                    self.state.playlist.sort_by_key(|p| song_sort_key(p));
                    self.state.song_count = 0;
                    self.state.skip_count = 0;
                    self.state.start_time = Some(Instant::now());
                    self.playing_pl_idx = None;
                    self.rebuild_display_cache();
                    if self.state.playlist.is_empty() {
                        self.set_status("No songs found");
                    } else {
                        self.set_status(format!("Loaded {} songs", self.state.playlist.len()));
                    }
                    self.save_settings();
                    if !self.state.playlist.is_empty() && !self.state.is_paused {
                        self.skip_song();
                    }
                }
                Msg::PlaylistSaved => self.set_status("Playlist saved"),
                Msg::PlaylistLoaded { files } => {
                    self.ingest_bands(&files, None);
                    self.state.playlist = files;
                    self.state.playlist.sort_by_key(|p| song_sort_key(p));
                    self.rebuild_display_cache();
                    self.set_status("Playlist loaded");
                    self.save_settings();
                }
                Msg::Added { files } => {
                    self.scanning = false;
                    self.ingest_bands(&files, None);
                    let mut n = 0;
                    for f in files {
                        if !self.state.playlist.contains(&f) {
                            self.state.playlist.push(f);
                            n += 1;
                        }
                    }
                    self.state.playlist.sort_by_key(|p| song_sort_key(p));
                    self.rebuild_display_cache();
                    self.set_status(format!("Added {} files", n));
                    self.save_settings();
                }
                Msg::ScanProgress { found } => {
                    self.scanning = true;
                    self.scan_found = found;
                    self.scan_tick = std::time::Instant::now();
                }
                Msg::FolderFound { folder } => {
                    if let Some(f) = folder {
                        self.set_status(format!("Auto-loaded: {}", f));
                        let _ = self.lib_tx.send(LibCmd::Scan(f));
                    } else {
                        self.scanning = false;
                        self.set_status("No Music folder found — use ADD DIR to pick one");
                    }
                }
                Msg::Meta { path, title, artist, album, duration } => {
                    if self.state.current_song.as_deref() == Some(&path) {
                        let mut artist = artist;
                        let mut album = album;
                        if artist == "Unknown" || album == "Unknown" {
                            if let Some((a, al)) = folder_artist_album(&path) {
                                if artist == "Unknown" {
                                    artist = a;
                                }
                                if album == "Unknown" {
                                    album = al;
                                }
                            }
                        }
                        self.meta = (title.clone(), artist.clone(), album.clone());
                        self.state.song_length = duration;
                        let d = duration.as_secs_f32();
                        if d > 0.0 {
                            self.len_secs = d;
                        }
                        self.want_web = artist != "Unknown" && album != "Unknown";
                        self.ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("♪ {} - {}", artist, title)));
                        self.meta_cache.insert(path.clone(), MetaCacheEntry { title, artist, album, secs: duration.as_secs_f32() });
                        self.meta_cache_dirty = true;
                    }
                }
                Msg::Tags { entries } => {
                    let mut changed = false;
                    let in_pl: HashSet<String> = self.state.playlist.iter().cloned().collect();
                    for (path, artist, title) in entries {
                        self.tags_pending.remove(&path);
                        if self.tag_names.contains_key(&path) {
                            continue;
                        }
                        self.tag_names.insert(path.clone(), playlist_display(&artist, &title, &path));
                        if in_pl.contains(&path) {
                            changed = true;
                        }
                    }
                    if changed {
                        self.rebuild_display_cache();
                    }
                }
                Msg::BatchMeta { entries } => {
                    let mut changed = false;
                    for (path, title, artist, album, secs) in entries {
                        self.meta_all_pending.remove(&path);
                        self.meta_cache.insert(path.clone(), MetaCacheEntry { title, artist, album, secs });
                        self.meta_cache_dirty = true;
                        if !self.tag_names.contains_key(&path) {
                            let e = self.meta_cache.get(&path).unwrap();
                            let n = playlist_display(&e.artist, &e.title, &path);
                            self.tag_names.insert(path.clone(), n);
                            changed = true;
                        }
                    }
                    if changed {
                        self.rebuild_display_cache();
                    }
                    self.meta_all_tick(format!("Meta All: {} tag jobs, {} covers, {} web left", self.meta_all_pending.len(), self.art_all_pending.len(), self.web_all_pending.len()));
                }
                Msg::ArtLocal { bytes } => {
                    if let Some(b) = bytes {
                        self.art_local_valid = true;
                        self.load_art_texture(b.clone());
                        if let Some(cur) = self.state.current_song.clone() {
                            if let Some(nb) = normalize_art(&b) {
                                let _ = write_art_cache(&cur, &nb);
                            }
                        }
                    } else if self.want_web {
                        let (a, al) = (self.meta.1.clone(), self.meta.2.clone());
                        self.web_art_for = self.state.current_song.clone();
                        let _ = self.net_tx.send(NetCmd::WebArt(a, al));
                    }
                }
                Msg::ArtWeb { bytes } => {
                    if self.web_art_for.as_deref() == self.state.current_song.as_deref() {
                        if let Some(b) = bytes {
                            if self.want_web && !self.art_local_valid {
                                self.load_art_texture(b.clone());
                            }
                            if let Some(cur) = self.state.current_song.clone() {
                                if let Some(nb) = normalize_art(&b) {
                                    let _ = write_art_cache(&cur, &nb);
                                }
                            }
                        }
                    }
                }
                Msg::ArtAll { missing } => {
                    let mut web: Vec<(String, String, String)> = Vec::new();
                    for p in missing {
                        self.art_all_pending.remove(&p);
                        if art_cache_path(&p).is_file() {
                            continue;
                        }
                        if let Some(e) = self.meta_cache.get(&p) {
                            let mut artist = e.artist.clone();
                            let mut album = e.album.clone();
                            if artist == "Unknown" || album == "Unknown" {
                                if let Some((a, al)) = folder_artist_album(&p) {
                                    if artist == "Unknown" {
                                        artist = a;
                                    }
                                    if album == "Unknown" {
                                        album = al;
                                    }
                                }
                            }
                            if artist != "Unknown" && album != "Unknown" && !self.web_all_pending.contains(&p) {
                                self.web_all_pending.insert(p.clone());
                                web.push((p, artist, album));
                            } else if artist == "Unknown" || album == "Unknown" {
                                // Tags are known but there's nothing to web-search
                                // with - and no local art was found. Remember this
                                // track as art-less so Meta All skips it next run.
                                if self.no_art.insert(p.clone()) {
                                    self.no_art_dirty = true;
                                }
                            }
                        }
                    }
                    for (p, a, al) in web {
                        let _ = self.web_tx.send(NetCmd::WebArtFor { path: p, artist: a, album: al });
                    }
                    self.meta_all_tick(format!("Meta All: {} covers, {} web left", self.art_all_pending.len(), self.web_all_pending.len()));
                }
                Msg::ArtWebFor { path, bytes } => {
                    self.web_all_pending.remove(&path);
                    if let Some(b) = bytes {
                        if let Some(nb) = normalize_art(&b) {
                            if write_art_cache(&path, &nb) {
                                if self.no_art.remove(&path) {
                                    self.no_art_dirty = true;
                                }
                            } else if self.no_art.insert(path.clone()) {
                                self.no_art_dirty = true;
                            }
                        } else if self.no_art.insert(path.clone()) {
                            self.no_art_dirty = true;
                        }
                    } else if self.no_art.insert(path.clone()) {
                        self.no_art_dirty = true;
                    }
                    self.meta_all_tick(format!("Meta All: {} web art left", self.web_all_pending.len()));
                }
                Msg::Lyrics { artist, title, text } => {
                    if self.lyrics_for.as_deref() == self.state.current_song.as_deref() {
                        if let Some(t) = text {
                            self.lyrics = Some(t);
                            self.lyrics_title = format!("{} - {}", artist, title);
                        } else {
                            self.set_error("Lyrics not found");
                        }
                    }
                }
                Msg::YtStatus(s) => {
                    self.status = s;
                    self.status_until = Some(Instant::now() + Duration::from_secs(8));
                }
                Msg::YtLog(s) => {
                    if let Some((p, sp, et)) = parse_yt_progress(&s) {
                        self.yt_pct = p;
                        self.yt_speed = sp;
                        self.yt_eta = et;
                    }
                    self.yt_log.push(s);
                    const MAX_LOG: usize = 60;
                    if self.yt_log.len() > MAX_LOG {
                        let over = self.yt_log.len() - MAX_LOG;
                        self.yt_log.drain(0..over);
                    }
                }
                Msg::YtDone { path, auto_play } => {
                    self.yt_pct = -1.0;
                    self.yt_speed.clear();
                    self.yt_eta.clear();
                    let s = path.to_string_lossy().to_string();
                    if !self.state.playlist.contains(&s) {
                        self.state.playlist.push(s.clone());
                        self.state.playlist.sort_by_key(|p| song_sort_key(p));
                        self.rebuild_display_cache();
                        self.save_settings();
                        self.ingest_bands(&[s.clone()], None);
                    }
                    if auto_play {
                        self.play_song(&s);
                    }
                    self.set_status(format!("Downloaded: {}", stem(&s)));
                    if !auto_play && self.yt_queue_processing {
                        self.yt_queue_advance();
                    }
                }
                Msg::YtFail { err, auto_play } => {
                    self.yt_pct = -1.0;
                    self.yt_speed.clear();
                    self.yt_eta.clear();
                    self.set_error(err);
                    if !auto_play && self.yt_queue_processing {
                        self.yt_queue_advance();
                    }
                }
                Msg::YtResolved { idx, query, display } => {
                    if let Some(e) = self.yt_queue.get_mut(idx) {
                        if e.0 == query {
                            e.1 = display;
                        }
                    }
                }
                Msg::Transcoded { display, wav } => {
                    let wavs = wav.to_string_lossy().to_string();
                    if let Some((pd, pa)) = self.pending_transcode.take() {
                        self.transcodes.insert(pa.clone(), wavs.clone());
                        if pd == display {
                            self.do_play(&display, &pa);
                        }
                    } else if !self.transcodes.contains_key(&display) {
                        self.transcodes.insert(display.clone(), wavs);
                    }
                }
                Msg::TranscodeFail { display, err } => {
                    if let Some((pd, _)) = self.pending_transcode.take() {
                        if pd == display {
                            self.state.current_song = None;
                            self.playing = String::new();
                            self.pos = 0.0;
                            self.len_secs = 0.0;
                            self.play_started = None;
                            self.set_error(format!("{}: {}", stem(&display), err));
                        }
                    }
                }
                Msg::Recorded { display, path } => {
                    self.recording = false;
                    let p = Path::new(&path);
                    let dir = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
                    self.set_status(format!("Recorded: {} → {}", stem(&display), dir));
                }
                Msg::RecordFail { display, err } => {
                    self.recording = false;
                    self.set_error(format!("{}: {}", stem(&display), err));
                }
                Msg::VideoFrame { w, h, rgba, gen } => {
                    if gen == self.video_gen {
                        self.video_dims = (w, h);
                        let size = [w as usize, h as usize];
                        let img = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                        self.video_tex = Some(self.ctx.load_texture("video", img, egui::TextureOptions::LINEAR));
                        if self.video_audio_pending && !self.video_paused {
                            self.video_audio_pending = false;
                            if let Some(s) = &self.video_sink {
                                s.play();
                                av_log("audio started on first frame");
                            }
                        }
                        if self.video_deferred_open {
                            self.video_deferred_open = false;
                            av_log(&format!("FIRST FRAME {:.3}s after open", self.ctx.input(|i| i.time) - self.video_open_at));
                            if let Some(p) = self.video_current.clone() {
                                av_log(&format!("first frame shown, deferred analyze/cut for {}", p));
                                self.maybe_analyze_video(&p);
                                self.try_cut_current_video(p);
                            }
                        }
                    }
                }
                Msg::VideoClosed { gen } => {
                    let was_eof = gen == self.video_gen && !self.video_closing;
                    if was_eof {
                        if let Some(path) = self.video_current.clone() {
                            self.video_positions.remove(&path);
                            self.save_settings();
                        }
                        if !self.video_queue.is_empty() {
                            self.queue_next();
                        } else {
                            self.video_ended = true;
                            self.video_paused = true;
                            self.set_status("MOVIE: end - PLAY to replay");
                        }
                    }
                }
                Msg::VideoPos { gen, secs } => {
                    if gen == self.video_gen {
                        self.video_cur_secs = secs;
                        let apos = self.video_sink.as_ref().map(|s| s.get_pos().as_secs_f32()).unwrap_or(-1.0);
                        if apos >= 0.0 && self.video_seek_base <= secs {
                            let audio_file = self.video_seek_base + apos;
                            av_log(&format!("tick video={:.3} audio={:.3} drift={:+.0}ms", secs, audio_file, (audio_file - secs) * 1000.0));
                        }
                        if self.intro_skip_enabled && self.video_dur_secs > 0.0 {
                            let cur_path = self.video_current.clone().unwrap_or_default();
                            if !cur_path.ends_with("_cut.mkv") {
                                let (ie, cs) = self.video_bound_secs();
                                if secs < ie && ie > 2.0 {
                                    if let Some(cut) = self.video_cut_cache.get(&cur_path).cloned() {
                                        av_log(&format!("SKIP intro at {:.3}s -> cut file {}", secs, cut));
                                        self.play_video_item(cut);
                                        self.set_status("Auto-cut skip → cut file");
                                    } else {
                                        av_log(&format!("SKIP intro at {:.3}s -> seek {:.3}s (no cut file)", secs, ie + 1.0));
                                        self.seek_video(ie + 1.0);
                                        self.set_status(format!("Skipped intro → {:.0}s ({})", ie, fmt_time(ie)));
                                    }
                                } else if cs > 2.0 && secs >= cs {
                                    if !self.video_queue.is_empty() {
                                        av_log(&format!("SKIP credits at {:.3}s (cs={:.3}) -> next", secs, cs));
                                        self.queue_next();
                                        self.set_status("Skipped credits → next video");
                                    }
                                }
                            }
                        }
                    }
                }
                Msg::VideoMeta { gen, dur } => {
                    if gen == self.video_gen {
                        self.video_dur_secs = dur;
                    }
                }
                Msg::VideoBounds { gen, path, intro_end, credits_start } => {
                    if gen == self.video_gen {
                        self.analyzing_video.remove(&path);
                        let had = self.video_bounds.get(&path).copied();
                        if (intro_end > 2.0 || credits_start > 2.0) && had != Some((intro_end, credits_start)) {
                            self.video_bounds.insert(path.clone(), (intro_end, credits_start));
                            if let Some(dir) = Path::new(&path).parent().map(|p| p.to_string_lossy().to_string()) {
                                let merged = self.show_bounds.get(&dir).copied();
                                match merged {
                                    Some((pi, pc)) => {
                                        let ni = if intro_end > 2.0 { intro_end } else { pi };
                                        let nc = if credits_start > 2.0 { credits_start } else { pc };
                                        self.show_bounds.insert(dir, (ni, nc));
                                    }
                                    None => {
                                        self.show_bounds.insert(dir, (intro_end, credits_start));
                                    }
                                }
                            }
                            self.save_settings();
                            if self.intro_skip_enabled && !path.ends_with("_cut.mkv") && !self.video_cut_inflight.contains(&path) {
                                let (i2, c2) = self.video_bounds.get(&path).copied().unwrap_or((0.0, 0.0));
                                if i2 > 2.0 && c2 > 2.0 && mkvmerge_path().is_file() && !self.video_cut_cache.contains_key(&path) {
                                    self.video_cut_inflight.insert(path.clone());
                                    let _ = self.lib_tx.send(LibCmd::VideoCut { path: path.clone(), gen, intro_end: i2, credits_start: c2 });
                                }
                            }
                        }
                    }
                }
                Msg::VideoCutDone { gen, path, cut, err } => {
                    if gen == self.video_gen {
                        self.video_cut_inflight.remove(&path);
                        if let Some(cp) = cut {
                            self.video_cut_cache.insert(path.clone(), cp.clone());
                            if let Some(&(ie, cs)) = self.video_bounds.get(&path) {
                                self.video_bounds.insert(cp.clone(), (ie, cs));
                            }
                            self.save_settings();
                            if self.video_current.as_deref() == Some(path.as_str()) {
                                self.set_status(format!("Auto-cut done → playing {}", stem(&cp)));
                                self.play_video_item(cp);
                            }
                        } else if let Some(e) = err {
                            self.set_status(format!("Auto-cut failed: {}; using seek skip", e));
                        }
                    }
                }
            }
        }

        // Debounced persist of the local tag cache while a big Meta All or
        // automatic tag batch streams in.
        if self.meta_cache_dirty && self.meta_cache_last_save.elapsed() > Duration::from_secs(3) {
            self.save_meta_cache();
        }
        if self.no_art_dirty && self.meta_cache_last_save.elapsed() > Duration::from_secs(3) {
            self.save_no_art();
        }
    }

    pub(crate) fn load_art_texture(&mut self, bytes: Vec<u8>) {
        match image::load_from_memory(&bytes) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let thumb = image::imageops::thumbnail(&rgba, 220, 220);
                let size = [thumb.width() as usize, thumb.height() as usize];
                let ci = ColorImage::from_rgba_unmultiplied(size, thumb.as_raw());
                self.art_tex = Some(self.ctx.load_texture("album_art", ci, TextureOptions::LINEAR));
                self.art_state = 2;
            }
            Err(_) => { self.art_state = 1; }
        }
    }
}

impl std::ops::Drop for PlayerApp {
    fn drop(&mut self) {
        self.close_video();
        self.stop_radio();
        self.save_meta_cache();
        self.save_no_art();
        self.save_settings();
        self.save_history();
        self.sink.stop();
    }
}

impl eframe::App for PlayerApp {
    fn on_exit(&mut self) {
        self.close_video();
        self.stop_radio();
        self.save_settings();
        self.save_history();
        if let Ok(rd) = std::fs::read_dir(work_dir().join("cuts")) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.ctx = ctx.clone();
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
        if !self.fs_asserted {
            self.fs_asserted = true;
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        let now = ctx.input(|i| i.time);
        let dt = ((now - self.last_time) as f32).clamp(0.0, 0.1);
        self.last_time = now;
        self.step_tape(dt);
        self.step_wind(dt);
        self.apply_visuals(&self.ctx);
        ui.set_min_size(ui.available_size());
        let th = self.theme_colors();
        ui.painter().rect_filled(ui.max_rect(), 0.0, th.bg);

        self.drain_msg();
        self.update_playback_state();
        if let Some(s) = &self.video_sink {
            if self.video_audio_pending || self.video_paused {
                self.video_clock.store(0, std::sync::atomic::Ordering::Relaxed);
            } else {
                let pos = s.get_pos().as_millis() as u64 + 1;
                let tagged = (((self.video_gen as u64) & 0xFFFF) << 48) | pos.min((1u64 << 48) - 1);
                self.video_clock.store(tagged, std::sync::atomic::Ordering::Relaxed);
            }
        } else {
            self.video_clock.store(0, std::sync::atomic::Ordering::Relaxed);
        }
        if self.video_audio_pending && !self.video_paused && now - self.video_open_at > 2.5 {
            self.video_audio_pending = false;
            if let Some(s) = &self.video_sink {
                s.play();
            }
        }
        self.tick_radio(now);
        self.poll_ext();
        self.handle_keys();
        self.handle_drops();
        self.draw_sleep_timer(&self.ctx.clone());

        self.draw_title_bar(ui);
        self.draw_status_bar(ui);
        self.draw_side_panel(ui);
        self.draw_central(ui);

        if self.show_help {
            self.show_help_window(&ctx);
        }
        if self.show_add_radio {
            self.show_add_radio_window(&ctx);
        }
        if self.show_radio {
            self.show_radio_window(&ctx);
        }
        if self.video_on {
            self.video_window(&ctx);
        }
        if self.lyrics.is_some() {
            self.ui_lyrics();
        }
        if self.yt_queue_show {
            self.ui_queue_window(&ctx);
        }
    }
}

pub fn run() -> eframe::Result {
std::panic::set_hook(Box::new(|info| {
        // Expected, caught decode panics (rodio symphonia init-seek bug) are
        // muted here; open_decoder logs a clean decode-skip line instead.
        if crate::library::EXPECTING_DECODE_PANIC.with(|fl| fl.get()) {
            return;
        }
        use std::io::Write as _;
        let bt = std::backtrace::Backtrace::force_capture();
        let log = data_dir().join("crash.log");
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            let _ = writeln!(f, "\n===== panic at {} =====", ts);
            let _ = writeln!(f, "{}", info);
            let _ = writeln!(f, "{}", bt);
        }
        eprintln!("{}", info);
    }));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_fullscreen(true)
            .with_icon(window_icon_data().unwrap_or_else(|| egui::IconData {
                rgba: build_icon_fallback_rgba(),
                width: 32,
                height: 32,
            }))
            .with_title(format!("{} v{}", APP_NAME, APP_VERSION)),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            let app = PlayerApp::new(cc)?;
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
}

#[cfg(test)]
mod eq_dsp_tests {
    use super::*;

    struct SineSource {
        rate: u32,
        t: f64,
        n: usize,
        f0: f64,
        f1: f64,
        amp0: f64,
        amp1: f64,
    }

    impl SineSource {
        fn new(rate: u32, secs: f64, f0: f64, amp0: f64, f1: f64, amp1: f64) -> Self {
            SineSource { rate, t: 0.0, n: (rate as f64 * secs) as usize, f0, f1, amp0, amp1 }
        }
    }

    impl Iterator for SineSource {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if self.n == 0 {
                return None;
            }
            self.n -= 1;
            let s = self.t;
            let v = self.amp0 * (2.0 * std::f64::consts::PI * self.f0 * s).sin()
                + self.amp1 * (2.0 * std::f64::consts::PI * self.f1 * s).sin();
            self.t += 1.0 / self.rate as f64;
            Some(v as f32)
        }
    }

    impl rodio::Source for SineSource {
        fn current_frame_len(&self) -> Option<usize> { None }
        fn channels(&self) -> u16 { 1 }
        fn sample_rate(&self) -> u32 { self.rate }
        fn total_duration(&self) -> Option<std::time::Duration> { None }
    }

    fn drain_eq(gains: [f32; 10], src: impl rodio::Source<Item = f32>) -> f64 {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(EqShared { gains, dirty: true }));
        let mut eq = EqSource::new(src, shared);
        eq.preamp = 1.0;
        let mut sum = 0.0f64;
        let mut n = 0usize;
        while let Some(s) = eq.next() {
            sum += (s as f64).powi(2);
            n += 1;
        }
        if n == 0 { 0.0 } else { (sum / n as f64).sqrt() }
    }

    #[test]
    fn eq_boost_raises_rms() {
        let base = SineSource::new(44100, 2.0, 60.0, 0.3, 1500.0, 0.3);
        let boosted = drain_eq([8.0, 6.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], base);
        let base = SineSource::new(44100, 2.0, 60.0, 0.3, 1500.0, 0.3);
        let flat = drain_eq([0.0; 10], base);
        assert!(boosted > flat * 1.5, "boost should raise RMS: flat={} eq={}", flat, boosted);
    }

    #[test]
    fn eq_cut_lowers_rms() {
        let base = SineSource::new(44100, 2.0, 60.0, 0.3, 1500.0, 0.3);
        let cut = drain_eq([-8.0, -6.0, -4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], base);
        let base = SineSource::new(44100, 2.0, 60.0, 0.3, 1500.0, 0.3);
        let flat = drain_eq([0.0; 10], base);
        assert!(cut < flat * 0.95, "cut should lower RMS: flat={} eq={}", flat, cut);
    }

    #[test]
    fn eq_flat_is_passthrough() {
        let base = SineSource::new(44100, 1.0, 440.0, 0.5, 880.0, 0.2);
        let a = drain_eq([0.0; 10], base);
        let base = SineSource::new(44100, 1.0, 440.0, 0.5, 880.0, 0.2);
        let mut src = base;
        let mut sum = 0.0f64;
        let mut n = 0usize;
        while let Some(s) = src.next() {
            sum += (s as f64).powi(2);
            n += 1;
        }
        let raw = if n == 0 { 0.0 } else { (sum / n as f64).sqrt() };
        assert!((a - raw).abs() < 1e-6, "flat EQ equals input: raw={} eq={}", raw, a);
    }

    #[test]
    fn eq_curve_reaches_boost_at_center() {
        let mut gains = [0.0f32; 10];
        gains[0] = 8.0;
        let boosted = eq_response_linear(EQ_BANDS[0], &gains, 44100.0);
        let flat = eq_response_linear(EQ_BANDS[0], &[0.0; 10], 44100.0);
        assert!(boosted > flat * 1.9, "boost dB reached at band center: flat={} eq={}", flat, boosted);
    }

    #[test]
    fn eq_curve_reaches_cut_at_center() {
        let mut gains = [0.0f32; 10];
        gains[0] = -8.0;
        let cut = eq_response_linear(EQ_BANDS[0], &gains, 44100.0);
        let flat = eq_response_linear(EQ_BANDS[0], &[0.0; 10], 44100.0);
        assert!(cut < flat * 0.5, "cut dB reached at band center: flat={} eq={}", flat, cut);
    }

    #[test]
    fn eq_auto_preamp_limits_clipping() {
        let gains = [12.0; 10];
        let pa = eq_auto_preamp(&gains, 44100.0);
        assert!(pa <= 1.0, "heavy boost should not raise gain: {}", pa);
        assert!(pa > 0.001, "preamp sane: {}", pa);
        assert!(pa < 1.0, "heavy boost should auto-gain down: {}", pa);
        let peak = (22u32..=20000u32)
            .step_by(17)
            .map(|f| eq_response_linear(f as f64, &gains, 44100.0))
            .fold(0.0f64, f64::max);
        assert!(peak * pa as f64 <= 1.05, "preamped curve peak ~0dB: peak={} pa={}", peak, pa);
    }
}
