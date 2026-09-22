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

// Download side: yt-dlp queue management/window, queue start/advance and lyrics fetch.

impl PlayerApp {
    pub(crate) fn yt_queue_add(&mut self, q: &str) {
        let q = q.trim().to_string();
        if q.is_empty() {
            self.set_error("Enter a YouTube URL or search term");
            return;
        }
        let idx = self.yt_queue.len();
        self.yt_queue.push((q.clone(), q.clone()));
        self.yt_query.clear();
        self.yt_queue_show = true;
        self.set_status(format!("Queued: {}", truncate_mid(&q, 50)));
        let _ = self.yt_tx.send(YtCmd::Resolve { idx, query: q });
    }

    pub(crate) fn yt_queue_start(&mut self) {
        if self.yt_queue_processing || self.yt_queue.is_empty() {
            return;
        }
        self.yt_queue_processing = true;
        self.yt_queue_show = true;
        self.yt_queue_send_next();
    }

    pub(crate) fn yt_queue_send_next(&mut self) {
        if !self.yt_queue_processing {
            return;
        }
        if self.yt_queue.is_empty() {
            self.yt_queue_processing = false;
            self.yt_queue_active = None;
            self.set_status("Queue finished");
            return;
        }
        let (q, disp) = self.yt_queue.remove(0);
        let shown = if disp.is_empty() { q.clone() } else { disp };
        self.yt_queue_active = Some(shown.clone());
        self.set_status(format!("Queue: downloading \"{}\"...", truncate_mid(&shown, 60)));
        let _ = self.yt_tx.send(YtCmd::Download { query: q, auto_play: false, chunks: 8 });
    }

    pub(crate) fn yt_queue_advance(&mut self) {
        self.yt_queue_active = None;
        if self.yt_queue.is_empty() {
            self.yt_queue_processing = false;
            self.set_status("Queue finished");
        } else {
            self.yt_queue_send_next();
        }
    }

    pub(crate) fn ui_queue_window(&mut self, ctx: &egui::Context) {
        let mut open = self.yt_queue_show;
        let mut closed = false;
        let max_h: f32 = (ctx.viewport_rect().height() * 0.6).clamp(120.0, 900.0);
        egui::Window::new("YT DOWNLOAD QUEUE")
            .collapsible(false)
            .resizable(true)
            .default_size([480.0, 320.0])
            .open(&mut open)
            .show(ctx, |ui| {
                let th = self.theme_colors();
                ui.horizontal(|ui| {
                    if retro_btn(ui, "Start", Color32::from_rgb(0, 255, 0), th.btn_bg).clicked() {
                        self.yt_queue_start();
                    }
                    let clear_d = self.yt_queue_processing;
                    if retro_btn(ui, "Clear", if clear_d { darken(Color32::from_rgb(255, 160, 80), 0.5) } else { Color32::from_rgb(255, 160, 80) }, th.btn_bg).clicked() && !clear_d {
                        self.yt_queue.clear();
                        self.set_status("Queue cleared");
                    }
                    if retro_btn(ui, "Close", Color32::from_rgb(200, 80, 80), th.btn_bg).clicked() {
                        closed = true;
                    }
                    ui.label(RichText::new(if self.yt_queue_processing { "RUNNING" } else { "STANDBY" })
                        .color(if self.yt_queue_processing { Color32::from_rgb(0, 255, 0) } else { th.btn_fg })
                        .monospace().size(11.0));
                });
                ui.add_space(4.0);
                match &self.yt_queue_active {
                    Some(a) => {
                        ui.label(RichText::new(format!("NOW: {}", truncate_mid(a, 46))).color(Color32::from_rgb(0, 255, 0)).monospace().size(11.0));
                    }
                    None if self.yt_queue_processing => {
                        ui.label(RichText::new("NOW: ...").color(Color32::from_rgb(0, 255, 0)).monospace().size(11.0));
                    }
                    _ => {}
                }
                if self.yt_queue_processing && self.yt_pct > -1.0 {
                    let status = if self.yt_speed.is_empty() {
                        "starting...".to_string()
                    } else {
                        format!("{}  ETA {}", self.yt_speed, self.yt_eta)
                    };
                    ui.add(egui::ProgressBar::new(self.yt_pct / 100.0)
                        .desired_width(f32::INFINITY)
                        .text(RichText::new(status).color(Color32::from_gray(200)).monospace().size(10.0))
                        .fill(Color32::from_rgb(28, 224, 255)));
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    if self.yt_queue.is_empty() && !self.yt_queue_processing {
                        ui.label(RichText::new("Queue empty. Add downloads with the YT row (QUEUE mode).").color(Color32::from_gray(140)).monospace().size(11.0));
                    }
                    let items: Vec<(String, String)> = self.yt_queue.clone();
                    for (i, it) in items.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{:>2}.", i + 1)).color(Color32::from_gray(150)).monospace().size(11.0));
                            ui.label(RichText::new(truncate_mid(&it.1, 36)).color(th.fg).monospace().size(11.0));
                            if retro_btn(ui, "X", Color32::from_rgb(255, 120, 120), th.btn_bg).clicked() {
                                if !self.yt_queue_processing {
                                    if let Some(idx) = self.yt_queue.iter().position(|x| x.0 == it.0) {
                                        self.yt_queue.remove(idx);
                                    }
                                }
                            }
                        });
                    }
                });
            });
        self.yt_queue_show = open && !closed;
    }
}
