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

// Main UI panels: title bar, status bar, side panel, central deck panel and auxiliary windows (radio, help, add-radio, lyrics).

impl PlayerApp {
    /// Dump button + live progress + cancel, shared by the DVD/Blu-ray decks.
    pub(crate) fn draw_disc_dump(&mut self, ui: &mut egui::Ui, kind: &str, th: &ThemePalette) {
        if self.disc_dumping {
            let frac = if self.disc_dump_total > 0 {
                (self.disc_dump_done as f32 / self.disc_dump_total as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let speed = if self.disc_dump_speed > 0.05 {
                format!("  {:.1} MB/s", self.disc_dump_speed)
            } else {
                String::new()
            };
            let eta = if self.disc_dump_speed > 0.05 && self.disc_dump_total > self.disc_dump_done {
                let s = ((self.disc_dump_total - self.disc_dump_done) as f32
                    / 1e6
                    / self.disc_dump_speed) as u64;
                if s >= 60 {
                    format!("  ~{}m", s / 60)
                } else {
                    format!("  ~{}s", s)
                }
            } else {
                String::new()
            };
            let text = if self.disc_dump_total > 0 {
                format!(
                    "{:.0}%  {:.1}/{:.1} GB{}{}",
                    frac * 100.0,
                    self.disc_dump_done as f64 / 1e9,
                    self.disc_dump_total as f64 / 1e9,
                    speed,
                    eta
                )
            } else {
                format!("{} dumping...{}", kind, speed)
            };
            ui.add(
                egui::ProgressBar::new(frac)
                    .desired_width(300.0)
                    .text(text),
            );
            if retro_btn(ui, "CANCEL DUMP", Color32::from_rgb(255, 120, 120), th.btn_bg).clicked() {
                self.cancel_disc_dump();
            }
        } else if retro_btn(ui, "DUMP TO MKV", th.fg, th.btn_bg)
            .on_hover_text(
                "Decrypt and remux every title to clean MKV files with MakeMKV. \
                 Works from the open disc image or the physical disc; output is never encrypted.",
            )
            .clicked()
        {
            self.start_mkv_dump(kind);
        }
    }

    /// DVD dump pair: clean MKV from the open image/disc, plus a decrypted ISO.
    pub(crate) fn draw_dvd_dump_buttons(&mut self, ui: &mut egui::Ui, th: &ThemePalette) {
        self.draw_disc_dump(ui, "DVD", th);
        if !self.disc_dumping {
            if retro_btn(ui, "DUMP TO ISO", th.fg, th.btn_bg)
                .on_hover_text(
                    "Decrypt the physical DVD to a clean, playable ISO with MakeMKV \
                     (removes CSS; requires the disc in the drive).",
                )
                .clicked()
            {
                self.start_dvd_decrypted_dump();
            }
        }
    }

    /// Blu-ray dump pair: clean MKV, or a full decrypted disc ISO.
    pub(crate) fn draw_bluray_dump_buttons(&mut self, ui: &mut egui::Ui, th: &ThemePalette) {
        self.draw_disc_dump(ui, "BLU-RAY", th);
        if !self.disc_dumping {
            if retro_btn(ui, "DUMP TO ISO", th.fg, th.btn_bg)
                .on_hover_text(
                    "Full decrypted Blu-ray ISO with menus and extras. MakeMKV backup \
                     handles AACS and BD+, then the BDMV tree is written as a UDF ISO.",
                )
                .clicked()
            {
                self.start_bluray_decrypted_dump();
            }
        }
    }

    pub(crate) fn draw_sleep_timer(&mut self, ctx: &egui::Context) {
        if self.fading {
            let t = (Instant::now() - self.fade_start).as_secs_f32();
            let frac = (1.0 - t / 3.0).clamp(0.0, 1.0);
            self.sink.set_volume(self.fade_from_vol as f32 / 100.0 * frac);
            if t >= 4.0 {
                self.request_quit(ctx);
            }
            return;
        }
        if let Some(d) = self.sleep_deadline {
            if Instant::now() >= d {
                self.seek_to(0.0);
                self.fade_start = Instant::now();
                self.fade_from_vol = self.state.volume;
                self.fading = true;
                self.sleep_deadline = None;
            }
        }
    }

    pub(crate) fn draw_title_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::Panel::top("title_panel").show(ui, |ui| {
            let rect = ui.max_rect();
            paint_title_gradient(&ui.painter().with_clip_rect(rect), rect, th.accent, th.bg);
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if let Some(tex) = &self.app_icon_tex {
                    ui.add(egui::Image::new((tex.id(), egui::vec2(18.0, 18.0))));
                }
                ui.label(RichText::new(APP_NAME).color(Color32::from_rgb(255, 240, 160)).monospace().strong().size(14.0));
                ui.separator();
                let w = (ui.available_width() - 12.0).max(160.0);
                let (brect, _) = ui.allocate_exact_size(egui::vec2(w, 22.0), egui::Sense::hover());
                let painter = ui.painter().with_clip_rect(brect);
                painter.rect_filled(brect, 2.0, darken(th.bg, 0.1));
                painter.rect_stroke(brect, 2.0, egui::Stroke::new(1.0, darken(th.accent, 0.6)), egui::StrokeKind::Inside);
                let marquee = if self.state.is_paused {
                    format!("II  {}", self.marquee_text())
                } else {
                    self.marquee_text()
                };
                if !marquee.is_empty() {
                    let cx = 8.4;
                    let tw = marquee.chars().count() as f32 * cx;
                    let t = ui.ctx().input(|i| i.time) as f32;
                    let total = brect.width() + tw + 60.0;
                    let mut x = brect.min.x + brect.width() - (t * 40.0).rem_euclid(total);
                    if x + tw < brect.min.x {
                        x += total;
                    }
                    painter.text(
                        egui::pos2(x, brect.center().y),
                        egui::Align2::LEFT_CENTER,
                        marquee,
                        egui::FontId::monospace(13.0),
                        th.playing_fg,
                    );
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.running).color(th.fg).monospace());
            });
            if !self.error.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&self.error).color(Color32::from_rgb(255, 80, 80)).monospace().strong());
                });
            }
            ui.add_space(1.0);
        });
    }

    pub(crate) fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::Panel::bottom("status_panel").show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter().with_clip_rect(rect).rect_filled(
                egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + 1.0)),
                0.0,
                th.accent,
            );
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.status).color(th.fg).monospace().size(11.0));
                if self.yt_pct > -1.0 {
                    let (n, m) = self.yt_item_counter();
                    let status = if !self.yt_speed.is_empty() {
                        format!("{} ETA {}", self.yt_speed, self.yt_eta)
                    } else if !self.yt_band_label.is_empty() {
                        self.yt_band_label.clone()
                    } else {
                        "starting...".to_string()
                    };
                    ui.add(
                        egui::ProgressBar::new(self.yt_bar.clamp(0.0, 1.0))
                            .desired_width(190.0)
                            .text(
                                RichText::new(format!(
                                    "YT {:.0}%  {}",
                                    self.yt_bar * 100.0,
                                    status
                                ))
                                .color(Color32::from_gray(225))
                                .monospace()
                                .size(9.0),
                            )
                            .fill(Color32::from_rgb(28, 224, 255)),
                    );
                    ui.label(
                        RichText::new(format!("{}/{}", n, m))
                            .color(Color32::from_gray(150))
                            .monospace()
                            .size(9.0),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if retro_btn(ui, "Exit", Color32::from_rgb(255, 110, 110), th.btn_bg).clicked() {
                        let c = self.ctx.clone();
                        self.request_quit(&c);
                    }
                    ui.label(RichText::new("Ctrl+Alt+M=Show | Ctrl+Alt+Q=Quit").color(Color32::from_gray(110)).monospace().size(9.0));
                });
            });
            ui.add_space(1.0);
        });
    }

    pub(crate) fn draw_side_panel(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::Panel::left("side_panel").show(ui, |ui| {
            ui.set_max_width(250.0);
            let (w, h) = (210.0, 200.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
            let p = ui.painter().with_clip_rect(rect);
            engrave(&p, rect, th.btn_bg);
            p.rect_filled(rect.shrink(2.0), 2.0, th.art_bg);
            if let Some(tex) = &self.art_tex {
                let ts = tex.size();
                let (tw, th) = (ts[0].max(1) as f32, ts[1].max(1) as f32);
                let avail = rect.shrink(4.0);
                let scale = (avail.width() / tw).min(avail.height() / th);
                let dst = egui::Rect::from_center_size(
                    avail.center(),
                    egui::vec2(tw * scale, th * scale),
                );
                p.image(
                    tex.id(),
                    dst,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                p.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    match self.art_state { 1 => "Loading art...", _ => "No Art" },
                    egui::FontId::monospace(13.0),
                    Color32::from_gray(90),
                );
            }
            ui.add_space(6.0);
            ui.label(RichText::new(&self.meta.0).color(th.fg).monospace().strong().size(13.0));
            ui.label(RichText::new(&self.meta.1).color(th.accent).monospace().size(11.0));
            ui.label(RichText::new(&self.meta.2).color(Color32::from_gray(140)).monospace().size(10.0));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("NOW").color(th.playing_fg).monospace().size(9.0));
                ui.label(RichText::new(&self.playing).color(th.playing_fg).monospace().size(12.0));
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("RATE").color(darken(th.accent, 0.4)).monospace().size(9.0));
                let cur = self.state.current_song.clone();
                if let Some(path) = cur {
                    let r = self.song_rating(&path);
                    let mut pick: Option<u8> = None;
                    let c0 = ui.cursor().min;
                    for k in 0..5usize {
                        let sr = egui::Rect::from_min_size(
                            egui::pos2(c0.x + k as f32 * 16.0, c0.y),
                            egui::vec2(16.0, 20.0),
                        );
                        let sresp = ui.interact(sr, ui.id().with(("side_star", k)), egui::Sense::click());
                        if sresp.clicked() {
                            pick = Some((k + 1) as u8);
                        }
                        let filled = (r as usize) > k;
                        let (glyph, sc) = if filled {
                            ("★", Color32::from_rgb(255, 210, 80))
                        } else {
                            ("☆", if sresp.hovered() { Color32::from_gray(210) } else { Color32::from_gray(95) })
                        };
                        ui.painter().text(
                            sr.center(),
                            egui::Align2::CENTER_CENTER,
                            glyph,
                            egui::FontId::proportional(14.0),
                            sc,
                        );
                    }
                    if let Some(n) = pick {
                        self.rate_song(&path, if r == n { 0 } else { n });
                    }
                    ui.advance_cursor_after_rect(egui::Rect::from_min_size(
                        c0,
                        egui::vec2(5.0 * 16.0, 20.0),
                    ));
                    ui.label(
                        RichText::new(if r > 0 { format!("{}/5", r) } else { "unrated".to_string() })
                            .color(if r > 0 { Color32::from_rgb(255, 210, 80) } else { Color32::from_gray(90) })
                            .monospace()
                            .size(9.0),
                    );
                } else {
                    ui.label(RichText::new("---").color(Color32::from_gray(90)).monospace().size(9.0));
                }
            });

            let dt = ui.input(|i| i.stable_dt);
            self.update_viz(dt);
            let (vrect, _) = ui.allocate_exact_size(egui::vec2(200.0, 96.0), egui::Sense::hover());
            let painter = ui.painter().with_clip_rect(vrect);
            engrave(&painter, vrect, th.btn_bg);
            painter.rect_filled(vrect.shrink(2.0), 2.0, th.art_bg);
            let vr = vrect.shrink(10.0);
            engrave(&painter, egui::Rect::from_min_max(vr.min - egui::vec2(2.0, 2.0), vr.max + egui::vec2(2.0, 2.0)), th.art_bg);
            let segs: usize = 12;
            let seg_h = vr.height() / segs as f32;
            let grid_col = Color32::from_rgba_unmultiplied(255, 255, 255, 12);
            for s in 1..segs {
                let y = vr.max.y - s as f32 * seg_h;
                painter.line_segment(
                    [egui::pos2(vr.min.x, y), egui::pos2(vr.max.x, y)],
                    egui::Stroke::new(1.0, grid_col),
                );
            }
            let bw = 11.0f32;
            let pitch = 15.0f32;
            let ox = vr.min.x + 2.0;
            for (i, &h) in self.viz_bars.iter().enumerate() {
                let x = ox + i as f32 * pitch;
                let lit = (segs as f32 * (h / vr.height()).clamp(0.0, 1.0)).round() as usize;
                for s in 0..segs {
                    let f = (s + 1) as f32 / segs as f32;
                    let r = egui::Rect::from_min_max(
                        egui::pos2(x, vr.max.y - f * vr.height()),
                        egui::pos2(x + bw, vr.max.y - f * vr.height() + seg_h),
                    );
                    let c = if s < lit {
                        if f > 0.78 { Color32::from_rgb(255, 70, 70) }
                        else if f > 0.58 { Color32::from_rgb(255, 170, 60) }
                        else if f > 0.38 { Color32::from_rgb(255, 220, 90) }
                        else { Color32::from_rgb(70, 230, 110) }
                    } else {
                        Color32::from_rgba_unmultiplied(255, 255, 255, 14)
                    };
                    painter.rect_filled(r, 1.0, c);
                }
                let ph = self.viz_peaks[i].min(vr.height()).max(0.0);
                let py = vr.max.y - ph;
                let pr = egui::Rect::from_min_size(egui::pos2(x - 1.0, py - 1.5), egui::vec2(bw + 2.0, 3.0));
                painter.rect_filled(pr, 1.5, Color32::from_rgb(240, 244, 255));
            }
            grill_dots(&painter, vr, Color32::from_rgba_unmultiplied(0, 0, 0, 40));
            ui.add_space(4.0);
            let playing_now = !self.state.is_paused && (self.state.current_song.is_some() || self.radio_on);
            let en = if playing_now { ((self.viz_rms - 0.10).max(0.0) / 0.90).clamp(0.0, 1.0) } else { 0.0 };
            let kick = if playing_now { self.viz_pop } else { 0.0 };
            let pulse = (en * 0.85 + kick * 0.85).clamp(0.0, 1.0);
            ui.horizontal(|ui| {
                speaker_woofer(ui, 76.0, th.btn_bg, th.accent, pulse, kick);
                ui.add_space(10.0);
                speaker_woofer(ui, 76.0, th.btn_bg, th.accent, pulse, kick);
            });
            let bpm_txt = if playing_now && self.viz_bpm >= 40.0 {
                format!("{:>3.0} BPM", self.viz_bpm)
            } else {
                "--- BPM".to_string()
            };
            ui.label(RichText::new(bpm_txt).color(Color32::from_gray(120)).monospace().size(10.0));
            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                let v = self.state.volume as f32 / 100.0;
                let nv = rotary(ui, &format!("VOL {}%", self.state.volume), v, th.playing_fg, th.btn_bg);
                if (nv - v).abs() > 0.0001 {
                    self.set_volume((nv * 100.0).round() as u32);
                }
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "-", th.btn_fg, th.btn_bg).clicked() { self.decrease_volume(); }
                        if retro_btn(ui, "+", th.btn_fg, th.btn_bg).clicked() { self.increase_volume(); }
                    });
                });
            });
            ui.add_space(4.0);
            ui.separator();
            let mut bands: Vec<String> = self.band_map.keys().cloned().collect();
            bands.sort_by_key(|b| b.to_lowercase());
            let mut sel = self.band_sel.clone();
            let mut all_clicked = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new("BANDS (A-Z)").color(darken(th.accent, 0.35)).monospace().size(8.0));
                if retro_btn(ui, "All", if sel.is_empty() { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() { sel = String::new(); all_clicked = true; }
                if retro_btn(ui, "Clear Bands", th.btn_fg, th.btn_bg).clicked() {
                    self.clear_bands();
                    sel = String::new();
                }
            });
            if bands.is_empty() {
                ui.label(RichText::new("Add music to build the band list").color(Color32::from_gray(90)).monospace().size(9.0));
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("band_scroll")
                    .auto_shrink([false, false])
                    .max_height(260.0)
                    .show(ui, |ui| {
                        let mut last_key = String::new();
                        for b in bands.iter() {
                            let ch = b.chars().next().map(|c| c.to_ascii_uppercase()).unwrap_or('#');
                            let key = if ch.is_ascii_alphabetic() { ch.to_string() } else { "#".to_string() };
                            if key != last_key {
                                last_key = key.clone();
                                ui.vertical_centered(|ui| {
                                    ui.label(RichText::new(&key).color(th.accent).monospace().strong().size(11.0));
                                });
                                ui.separator();
                            }
                            let n = self.band_map.get(b).map(|v| v.len()).unwrap_or(0);
                            let hit = sel == *b;
                            let text = format!("{}  ({})", b, n);
                            if ui.selectable_label(hit, RichText::new(text).color(if hit { th.playing_fg } else { th.fg }).monospace().size(10.0)).clicked() {
                                sel = b.clone();
                            }
                        }
                    });
                if all_clicked || self.band_sel != sel {
                    self.choose_band(if all_clicked { String::new() } else { sel });
                }
            }
            ui.add_space(4.0);
            self.draw_play_switch(ui, &th);
            ui.add_space(4.0);
            let pp = ui.painter().with_clip_rect(ui.clip_rect());
                let panel_right = ui.max_rect().right();
                let cy = ui.max_rect().center().y;
                let free = panel_right - ui.min_rect().right();
                if free > 40.0 {
                    print_sideways(
                        &pp,
                        panel_right - 1.0,
                        cy + 1.0,
                        "drDOOM69GAMING",
                        12.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, 110),
                    );
                    print_sideways(
                        &pp,
                        panel_right - 7.0,
                        cy,
                        "drDOOM69GAMING",
                        12.0,
                        Color32::from_rgba_unmultiplied(255, 205, 120, 235),
                    );
                }
        });
    }

    pub(crate) fn draw_central(&mut self, ui: &mut egui::Ui) {
        let th = self.theme_colors();
        egui::CentralPanel::default().show(ui, |ui| {
            let bg_rect = ui.available_rect_before_wrap();
            let bg = ui.interact(bg_rect, ui.id().with("bg_dbl_click"), egui::Sense::click());
            if bg.double_clicked() && !self.video_on {
                self.toggle_fullscreen();
            }
            let playing = self.state.current_song.is_some() && !self.state.is_paused;
            let (deck_plaque, deck_plaque_fg) = match self.deck_mode {
                DeckMode::Tape => ("  CASSETTE DECK  •  REV C", th.accent),
                DeckMode::Disc => ("  CD DECK  •  COMPACT DISC", th.playing_fg),
                DeckMode::Dvd => ("  DVD DECK  •  DVD VIDEO", Color32::from_rgb(160, 200, 255)),
                DeckMode::BluRay => ("  BLU-RAY DECK  •  BD VIDEO", Color32::from_rgb(130, 210, 255)),
                DeckMode::Radio => ("  RADIO TUNER  •  FM / AM", Color32::from_rgb(255, 170, 80)),
            };
            plaque(ui, ui.available_width(), deck_plaque, deck_plaque_fg, th.bg);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("SW v{}", APP_VERSION)).color(darken(th.accent, 0.35)).monospace().size(8.0));
            });

            match self.deck_mode {
                DeckMode::Disc => {
                    let cd = self.cd_physical.clone();
                    let cd_ripping = self.cd_ripping;
                    ui.horizontal(|ui| {
                        let (tray, _) = ui.allocate_exact_size(egui::vec2(300.0, 108.0), egui::Sense::hover());
                        let p = ui.painter().with_clip_rect(tray);
                        p.rect_filled(tray, 6.0, darken(th.btn_bg, 0.4));
                        p.rect_stroke(tray, 6.0, egui::Stroke::new(1.5, darken(th.accent, 0.4)), egui::StrokeKind::Inside);
                        let slot = egui::Rect::from_min_max(
                            egui::pos2(tray.min.x + 14.0, tray.min.y + 20.0),
                            egui::pos2(tray.max.x - 14.0, tray.max.y - 16.0),
                        );
                        p.rect_filled(slot, 3.0, Color32::from_rgb(22, 24, 28));
                        p.rect_stroke(slot, 3.0, egui::Stroke::new(1.0, Color32::from_gray(46)), egui::StrokeKind::Inside);
                        if self.disc_in {
                            let t = ui.input(|i| i.time) as f32;
                            let c = slot.center();
                            let cx = 4.0;
                            let wave = (t * 5.0).sin() * cx;
                            let r = 30.0;
                            p.circle_filled(c + egui::vec2(wave * 0.15, 0.0), r + 1.0, Color32::from_gray(225));
                            p.circle_filled(c, r, Color32::from_gray(150));
                            p.circle_filled(c + egui::vec2(wave * 0.15, 0.0), r - 8.0, Color32::from_gray(64));
                            p.circle_filled(c, 3.5, Color32::from_gray(235));
                            let a = t * 3.0;
                            let shine = c + egui::vec2(a.cos(), a.sin()) * (r * 0.7);
                            p.circle_filled(shine, 4.5, Color32::from_rgba_unmultiplied(255, 255, 255, 80));
                            let _ = cx;
                            p.text(
                                egui::pos2(slot.center().x, slot.max.y - 8.0),
                                egui::Align2::CENTER_BOTTOM,
                                format!("{} TRACKS", self.state.playlist.len()),
                                egui::FontId::monospace(9.0),
                                Color32::from_gray(120),
                            );
                        } else {
                            p.text(
                                slot.center(),
                                egui::Align2::CENTER_CENTER,
                                "NO DISC",
                                egui::FontId::monospace(16.0),
                                Color32::from_gray(85),
                            );
                        }
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            if self.disc_in {
                                ui.label(RichText::new(&self.disc_label).color(th.playing_fg).monospace().strong().size(14.0));
                                ui.add_space(2.0);
                                ui.label(RichText::new(format!("{} TRACKS  •  CD", self.state.playlist.len())).color(Color32::from_gray(140)).monospace().size(10.0));
                                let now = self.state.current_song.clone().map(|s| stem(&s)).unwrap_or_else(|| "--".to_string());
                                ui.label(RichText::new(format!("♪ {}", now)).color(th.fg).monospace().size(11.0));
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("POS").color(darken(th.accent, 0.35)).monospace().size(8.0));
                                    lcd(ui, &fmt_secs(self.pos), th.playing_fg, th.bg);
                                    ui.label(RichText::new("/").color(Color32::from_gray(110)).monospace().size(10.0));
                                    lcd(ui, &fmt_secs(self.len_secs), th.playing_fg, th.bg);
                                });
                            } else {
                                ui.label(RichText::new("NO DISC IN DECK").color(Color32::from_gray(120)).monospace().size(12.0));
                                ui.label(RichText::new("INSERT DISC  ->  load an album folder").color(Color32::from_gray(90)).monospace().size(10.0));
                            }
                            ui.add_space(7.0);
                            if retro_btn(ui, if self.disc_in { "⏏ EJECT DISC" } else { "▼ INSERT DISC" }, if self.disc_in { th.playing_fg } else { th.fg }, th.btn_bg).clicked() {
                                if self.disc_in { self.eject_disc(); } else { self.insert_disc(); }
                            }
                            ui.add_space(6.0);
                            if let Some(cd) = &cd {
                                ui.label(
                                    RichText::new(format!(
                                        "AUDIO CD  •  {} TRACKS  •  {}",
                                        cd.tracks.len(),
                                        fmt_secs(cd.total_secs)
                                    ))
                                    .color(th.playing_fg)
                                    .monospace()
                                    .size(10.0),
                                );
                                ui.horizontal(|ui| {
                                    let rip_lbl = if cd_ripping { "RIPPING CD..." } else { "RIP CD → MUSIC" };
                                    if retro_btn(ui, rip_lbl, Color32::from_rgb(120, 255, 120), th.btn_bg).clicked() && !cd_ripping {
                                        self.start_cd_rip();
                                    }
                                    if retro_btn(ui, "⏏ EJECT TRAY", th.fg, th.btn_bg).clicked() {
                                        self.eject_cd_tray();
                                    }
                                });
                                ui.label(
                                    RichText::new("FLAC • lossless 16-bit / 44.1 kHz")
                                        .color(Color32::from_gray(135))
                                        .monospace()
                                        .size(9.0),
                                );
                            } else {
                                ui.label(
                                    RichText::new("NO AUDIO CD IN TRAY")
                                        .color(Color32::from_gray(110))
                                        .monospace()
                                        .size(10.0),
                                );
                            }
                        });
                    });
                }
                DeckMode::Radio => {
                    ui.horizontal(|ui| {
                        let (lo, hi) = self.radio_band_range();
                        let span = hi - lo;
                        let dial_resp = ui.allocate_response(egui::vec2(ui.available_width().min(560.0), 132.0), egui::Sense::click_and_drag());
                        let rect = dial_resp.rect;
                        let p = ui.painter().with_clip_rect(rect.expand(2.0));
                        p.rect_filled(rect, 8.0, darken(th.btn_bg, 0.35));
                        p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, darken(th.btn_bg, 0.7)), egui::StrokeKind::Inside);
                        let screen = rect.shrink(8.0);
                        p.rect_filled(screen, 4.0, Color32::from_rgb(18, 20, 16));
                        p.rect_stroke(screen, 4.0, egui::Stroke::new(1.0, Color32::from_gray(40)), egui::StrokeKind::Inside);
                        let map = |f: f32| screen.min.x + screen.width() * ((f - lo) / span);
                        for s in self.radio_presets.iter().filter(|s| s.band.eq_ignore_ascii_case(&self.radio_band)) {
                            if s.freq >= lo && s.freq <= hi {
                                let x = map(s.freq);
                                p.line_segment(
                                    [egui::pos2(x, screen.max.y - 74.0), egui::pos2(x, screen.max.y - 62.0)],
                                    egui::Stroke::new(1.0, th.accent),
                                );
                            }
                        }
                        let n = 20;
                        for i in 0..=n {
                            let f = lo + span * (i as f32 / n as f32);
                            let x = map(f);
                            let major = i % 5 == 0;
                            let ty = if major { 30.0 } else { 14.0 };
                            p.line_segment(
                                [egui::pos2(x, screen.max.y - 56.0), egui::pos2(x, screen.max.y - 56.0 - ty)],
                                egui::Stroke::new(1.0, if major { Color32::from_gray(170) } else { Color32::from_gray(90) }),
                            );
                            if major {
                                p.text(
                                    egui::pos2(x, screen.max.y - 16.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    format!("{}", f.round()),
                                    egui::FontId::monospace(9.0),
                                    Color32::from_gray(120),
                                );
                            }
                        }
                        let nx = map(self.radio_freq).clamp(screen.min.x + 4.0, screen.max.x - 4.0);
                        let needle = if self.radio_tuned.is_some() { th.playing_fg } else { Color32::from_rgb(190, 120, 60) };
                        p.line_segment(
                            [egui::pos2(nx, screen.min.y + 4.0), egui::pos2(nx, screen.max.y - 4.0)],
                            egui::Stroke::new(2.0, needle),
                        );
                        let tri = [
                            egui::pos2(nx - 8.0, screen.max.y - 16.0),
                            egui::pos2(nx + 8.0, screen.max.y - 16.0),
                            egui::pos2(nx, screen.max.y - 4.0),
                        ];
                        p.add(egui::Shape::convex_polygon(tri.to_vec(), needle, egui::Stroke::NONE));
                        if dial_resp.dragged() {
                            let d = dial_resp.drag_delta().x / rect.width() * span;
                            self.radio_freq = (self.radio_freq + d).clamp(lo, hi);
                            self.radio_last_moved = ui.input(|i| i.time);
                        }
                        if dial_resp.clicked() {
                            if let Some(pos) = dial_resp.interact_pointer_pos() {
                                let f = lo + span * ((pos.x - screen.min.x) / screen.width());
                                self.radio_freq = f.clamp(lo, hi);
                                self.radio_last_moved = ui.input(|i| i.time);
                            }
                        }
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                let fm_on = self.radio_band == "FM";
                                let am_on = self.radio_band == "AM";
                                if retro_btn(ui, "FM", if fm_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !fm_on {
                                    self.radio_band = "FM".to_string();
                                    self.radio_freq = 88.1;
                                    self.radio_last_moved = ui.input(|i| i.time);
                                }
                                if retro_btn(ui, "AM", if am_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !am_on {
                                    self.radio_band = "AM".to_string();
                                    self.radio_freq = 860.0;
                                    self.radio_last_moved = ui.input(|i| i.time);
                                }
                            });
                            ui.add_space(4.0);
                            lcd(ui, &fmt_freq(self.radio_freq), th.playing_fg, th.bg);
                            let stname = self.radio_tuned.and_then(|i| self.radio_presets.get(i)).map(|s| s.name.clone());
                            let playing = self.radio_on && !self.state.is_paused;
                            ui.label(RichText::new(stname.as_deref().unwrap_or("-- STATIC --")).color(if playing { th.playing_fg } else { Color32::from_gray(110) }).monospace().size(13.0));
                            if playing {
                                ui.label(RichText::new("● LIVE").color(Color32::from_rgb(0, 255, 90)).monospace().size(11.0));
                            }
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if retro_btn(ui, "TUNE-", th.btn_fg, th.btn_bg).clicked() {
                                    self.tune_radio_to(self.radio_freq - if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                                }
                                if retro_btn(ui, "SCAN", th.playing_fg, th.btn_bg).clicked() {
                                    self.scan_radio(ui.input(|i| i.time));
                                }
                                if retro_btn(ui, "TUNE+", th.btn_fg, th.btn_bg).clicked() {
                                    self.tune_radio_to(self.radio_freq + if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                                }
                                if retro_btn(ui, if playing { "PAUSE" } else { "PLAY" }, th.playing_fg, th.btn_bg).clicked() {
                                    if self.state.is_paused { self.unpause_music(); } else { self.pause_music(); }
                                }
                                if retro_btn(ui, "STOP", th.btn_fg, th.btn_bg).clicked() {
                                    self.stop_radio();
                                }
                                if retro_btn(ui, "REC", if self.recording { th.playing_fg } else { Color32::from_rgb(255, 120, 120) }, mix(th.btn_bg, Color32::from_rgb(205, 45, 45), 0.35)).clicked() {
                                    self.record_radio();
                                }
                            });
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("PRESETS").color(darken(th.accent, 0.35)).monospace().size(9.0));
                                if retro_btn(ui, "+ ADD", th.playing_fg, th.btn_bg).clicked() {
                                    self.show_add_radio = true;
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                let rows: Vec<(usize, f32, String)> = self
                                    .radio_presets
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, s)| s.band.eq_ignore_ascii_case(&self.radio_band))
                                    .map(|(i, s)| (i, s.freq, s.name.clone()))
                                    .collect();
                                for (idx, freq, name) in rows {
                                    let current = self.radio_playing_idx == Some(idx);
                                    if ui.selectable_label(current, RichText::new(format!("{} {}", fmt_freq(freq), name)).color(if current { th.playing_fg } else { Color32::from_gray(150) }).monospace().size(9.0)).clicked() {
                                        self.tune_radio_to(freq, ui.input(|i| i.time));
                                        self.start_radio(idx);
                                    }
                                }
                            });
                        });
                    });
                }
                DeckMode::Dvd => {
                    let drive = crate::dvd::first_drive();
                    let current = self.video_current.clone().filter(|p| crate::dvd::is_dvd(p));
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("DVD VIDEO").color(th.accent).monospace().strong().size(16.0));
                            ui.label(
                                RichText::new(match &drive {
                                    Some(d) => format!("DRIVE  {}", d),
                                    None => "NO OPTICAL DRIVE DETECTED".to_string(),
                                })
                                .color(Color32::from_gray(150))
                                .monospace()
                                .size(10.0),
                            );
                            ui.label(
                                RichText::new("MPEG-2 • libdvdread/libdvdnav • libdvdcss for encrypted discs")
                                    .color(Color32::from_gray(115))
                                    .monospace()
                                    .size(9.0),
                            );
                            let deps = crate::tools::dvd_deps();
                            if !deps.all_present() {
                                ui.label(
                                    RichText::new(deps.package_hint())
                                        .color(Color32::from_rgb(255, 190, 120))
                                        .monospace()
                                        .size(9.0),
                                );
                            }
                        });
                    });
                    ui.add_space(8.0);
                    if let Some(src) = &current {
                        ui.label(
                            RichText::new(format!("NOW  {}", crate::dvd::display_name(src)))
                                .color(th.playing_fg)
                                .monospace()
                                .size(12.0),
                        );
                        ui.horizontal(|ui| {
                            if retro_btn(ui, "⏹ STOP", th.btn_fg, th.btn_bg).clicked() {
                                self.close_video();
                            }
                            if crate::dvd::source(src).starts_with("/dev/") {
                                if retro_btn(ui, "⏏ EJECT TRAY", th.fg, th.btn_bg).clicked() {
                                    self.eject_dvd();
                                }
                            }
                            self.draw_dvd_dump_buttons(ui, &th);
                        });
                    } else {
                        ui.label(
                            RichText::new("Insert a DVD or choose a DVD ISO image")
                                .color(Color32::from_gray(120))
                                .monospace()
                                .size(11.0),
                        );
                        ui.horizontal(|ui| {
                            if retro_btn(ui, "▶ PLAY DVD", Color32::from_rgb(140, 220, 255), th.btn_bg).clicked() {
                                self.play_dvd_drive();
                            }
                            if retro_btn(ui, "OPEN DVD ISO...", th.fg, th.btn_bg).clicked() {
                                self.open_dvd_iso();
                            }
                            self.draw_dvd_dump_buttons(ui, &th);
                        });
                    }
                }
                DeckMode::BluRay => {
                    let drive = crate::bluray::first_drive();
                    let keydb = crate::bluray::keydb_path();
                    let current = self.video_current.clone().filter(|p| crate::bluray::is_bluray(p));
                    let prot_source = current
                        .as_deref()
                        .map(crate::bluray::source)
                        .map(|s| s.to_string())
                        .or_else(|| drive.clone());
                    if let Some(src) = &prot_source {
                        self.request_bluray_protection(src, false);
                    }
                    let prot = self.bluray_prot.clone();
                    let prot_pending = self.bluray_prot_pending;
                    let mut check_aacs = false;
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("BLU-RAY VIDEO").color(th.accent).monospace().strong().size(16.0));
                            ui.label(
                                RichText::new(match &drive {
                                    Some(d) => format!("DRIVE  {}", d),
                                    None => "NO OPTICAL DRIVE DETECTED".to_string(),
                                })
                                .color(Color32::from_gray(150))
                                .monospace()
                                .size(10.0),
                            );
                            ui.label(
                                RichText::new(match &keydb {
                                    Some(k) => format!("AACS KEYDB  {}", k.display()),
                                    None => "AACS KEYDB  not found (~/.config/aacs/KEYDB.cfg)".to_string(),
                                })
                                .color(if keydb.is_some() { Color32::from_rgb(120, 255, 150) } else { Color32::from_rgb(255, 160, 120) })
                                .monospace()
                                .size(9.0),
                            );
                            ui.label(
                                RichText::new("English audio preferred • subtitles off • libbluray/libaacs")
                                    .color(Color32::from_gray(115))
                                    .monospace()
                                    .size(9.0),
                            );
                            if prot_pending {
                                ui.label(
                                    RichText::new("CHECKING AACS / BD+ / KEYDB ...")
                                        .color(Color32::from_gray(150))
                                        .monospace()
                                        .size(9.0),
                                );
                            } else if let Some(p) = &prot {
                                let color = if !p.bluray {
                                    Color32::from_gray(130)
                                } else if p.needs_makemkv() {
                                    Color32::from_rgb(255, 190, 120)
                                } else {
                                    Color32::from_rgb(120, 255, 150)
                                };
                                ui.label(
                                    RichText::new(p.summary())
                                        .color(color)
                                        .monospace()
                                        .size(9.0)
                                        .strong(),
                                );
                                if let Some(id) = &p.disc_id {
                                    ui.label(
                                        RichText::new(format!("DISC ID  {}", id))
                                            .color(Color32::from_gray(140))
                                            .monospace()
                                            .size(8.0),
                                    );
                                }
                                if p.needs_makemkv() {
                                    ui.label(
                                        RichText::new(
                                            "BD+ / missing key: full-disc ISO dump uses MakeMKV backup --decrypt",
                                        )
                                        .color(Color32::from_rgb(255, 190, 120))
                                        .monospace()
                                        .size(8.0),
                                    );
                                }
                                if p.keydb_match && p.keydb_entries > 0 {
                                    ui.label(
                                        RichText::new(format!(
                                            "KEYDB  {} entries • this disc is present",
                                            p.keydb_entries
                                        ))
                                        .color(Color32::from_gray(120))
                                        .monospace()
                                        .size(8.0),
                                    );
                                }
                                if retro_btn(ui, "↻ CHECK AACS", th.fg, th.btn_bg)
                                    .on_hover_text(
                                        "Re-read AACS/BD+ and check this disc's ID against your KEYDB",
                                    )
                                    .clicked()
                                {
                                    check_aacs = true;
                                }
                            }
                        });
                    });
                    if check_aacs {
                        if let Some(src) = &prot_source {
                            self.request_bluray_protection(src, true);
                        }
                    }
                    ui.add_space(8.0);
                    if let Some(src) = &current {
                        ui.label(
                            RichText::new(format!("NOW  {}", crate::bluray::display_name(src)))
                                .color(th.playing_fg)
                                .monospace()
                                .size(12.0),
                        );
                        ui.horizontal(|ui| {
                            if retro_btn(ui, "⏹ STOP", th.btn_fg, th.btn_bg).clicked() {
                                self.close_video();
                            }
                            if crate::bluray::source(src).starts_with("/dev/") {
                                if retro_btn(ui, "⏏ EJECT TRAY", th.fg, th.btn_bg).clicked() {
                                    self.eject_bluray();
                                }
                            }
                            self.draw_bluray_dump_buttons(ui, &th);
                        });
                    } else {
                        ui.label(
                            RichText::new("Insert a Blu-ray disc or choose a Blu-ray ISO/folder")
                                .color(Color32::from_gray(120))
                                .monospace()
                                .size(11.0),
                        );
                        ui.horizontal(|ui| {
                            if retro_btn(ui, "▶ PLAY BLU-RAY", Color32::from_rgb(140, 220, 255), th.btn_bg).clicked() {
                                self.play_bluray_drive();
                            }
                            if retro_btn(ui, "OPEN BLU-RAY ISO...", th.fg, th.btn_bg).clicked() {
                                self.open_bluray_iso();
                            }
                            self.draw_bluray_dump_buttons(ui, &th);
                        });
                    }
                }
                DeckMode::Tape => {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("COUNTER").color(darken(th.accent, 0.35)).monospace().size(8.0));
                            lcd(ui, &fmt_secs(self.pos), th.playing_fg, th.bg);
                            ui.add_space(2.0);
                            lcd(ui, &fmt_secs(self.len_secs), th.playing_fg, th.bg);
                        });
                        ui.add_space(10.0);
                        let wind = if self.winding.is_some() { 1.0 } else { 0.0 };
                        let wind_dir = self.winding.unwrap_or(0.0);
                        cassette(
                            ui,
                            (ui.available_width() - 12.0).max(150.0),
                            66.0,
                            self.tape_anim,
                            self.tape_side,
                            self.tape_songs,
                            self.tape_cap,
                            th.playing_fg,
                            th.btn_bg,
                            playing,
                            wind,
                            wind_dir,
                        );
                    });

                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("TUNE").color(th.accent).monospace().size(10.0));
                        let max = if self.len_secs > 0.0 { self.len_secs } else { 600.0 };
                        let mut p = self.pos;
                        let dial = ui.add(
                            egui::Slider::new(&mut p, 0.0..=max)
                                .show_value(false)
                                .trailing_fill(true),
                        );
                        if dial.dragged() {
                            self.pos = p;
                            self.dragging = true;
                        } else if self.dragging && dial.drag_stopped() {
                            self.set_status(format!("TUNE seek | pos={:.1} len={:.1} song={}", p, self.len_secs, self.state.current_song.as_deref().unwrap_or("NONE")));
                            self.dragging = false;
                            self.seek_to(p);
                            self.pos = p;
                        }
                        let dr = dial.rect;
                        let painter = ui.painter().with_clip_rect(dr.expand(2.0));
                        let n = 12;
                        for i in 0..=n {
                            let x = dr.min.x + dr.width() * (i as f32 / n as f32);
                            let len = if i % 3 == 0 { 6.0 } else { 2.5 };
                            painter.line_segment(
                                [egui::pos2(x, dr.center().y - len), egui::pos2(x, dr.center().y + len)],
                                egui::Stroke::new(1.0, darken(th.accent, 0.55)),
                            );
                        }
                        let frac = if max > 0.0 { (self.pos / max).clamp(0.0, 1.0) } else { 0.0 };
                        let nx = dr.min.x + dr.width() * frac;
                        painter.line_segment(
                            [egui::pos2(nx, dr.min.y + 1.0), egui::pos2(nx, dr.max.y - 1.0)],
                            egui::Stroke::new(1.0, th.playing_fg),
                        );
                        painter.text(
                            egui::pos2(dr.center().x, dr.max.y + 5.0),
                            egui::Align2::CENTER_TOP,
                            if self.len_secs > 0.0 { format!("{:>2}%", (self.pos / max * 100.0).round() as u32) } else { String::from("--%") },
                            egui::FontId::monospace(8.0),
                            darken(th.accent, 0.4),
                        );
                        ui.add_space(10.0);
                    });
                }
            }

            ui.horizontal(|ui| {
                ui.label(RichText::new("DECK").color(darken(th.accent, 0.4)).monospace().size(9.0));
                let modes = [(DeckMode::Tape, "TAPE"), (DeckMode::Disc, "CD"), (DeckMode::Dvd, "DVD"), (DeckMode::BluRay, "BLU-RAY"), (DeckMode::Radio, "RADIO")];
                for (m, lbl) in modes {
                    let on = self.deck_mode == m;
                    if retro_btn(ui, lbl, if on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() {
                        self.set_deck_mode(m);
                    }
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                let red = Color32::from_rgb(150, 26, 26);
                let rec_red = Color32::from_rgb(205, 45, 45);
                let wn = ui.input(|i| i.time);
                let rew = deck_key(ui, "REW", th.btn_fg, th.btn_bg);
                if rew.drag_started() {
                    self.wind_arm = Some((-1.0, wn));
                }
                if deck_key(ui, "PLAY", Color32::from_rgb(255, 235, 235), mix(th.btn_bg, red, 0.45)).clicked() {
                    self.play_press();
                }
                let fwd = deck_key(ui, "FWD", th.btn_fg, th.btn_bg);
                if fwd.drag_started() {
                    self.wind_arm = Some((1.0, wn));
                }
                if let Some((d, t)) = self.wind_arm {
                    if self.winding.is_none() && wn - t >= 0.25 {
                        self.set_winding(Some(d));
                    }
                }
                if rew.clicked() && self.winding.is_none() {
                    self.seek_skip(-10.0);
                    self.set_status(format!("REW -10s | pos={:.1}", self.pos));
                }
                if fwd.clicked() && self.winding.is_none() {
                    self.seek_skip(10.0);
                    self.set_status(format!("FWD +10s | pos={:.1}", self.pos));
                }
                if rew.drag_stopped() {
                    self.wind_arm.take();
                    if self.winding == Some(-1.0) {
                        self.set_winding(None);
                    }
                }
                if fwd.drag_stopped() {
                    self.wind_arm.take();
                    if self.winding == Some(1.0) {
                        self.set_winding(None);
                    }
                }
                if deck_key(ui, "STOP", th.btn_fg, th.btn_bg).clicked() {
                    if self.video_on || self.video_current.is_some() {
                        self.close_video();
                    }
                    self.stop_music();
                    if self.radio_on {
                        self.stop_radio();
                    }
                }
                let pause_ready = self.play_started.is_some() || self.state.is_paused || self.radio_on;
                let pause_col = if pause_ready { Color32::from_rgb(255, 220, 130) } else { darken(th.btn_fg, 0.5) };
                let pause_lbl = if self.state.is_paused { "RESUME" } else { "PAUSE" };
                if deck_key(ui, pause_lbl, pause_col, th.btn_bg).clicked() && pause_ready {
                    self.toggle_pause();
                }
                let rec_lbl = if self.recording && (ui.input(|i| i.time) * 2.0).fract() < 0.5 {
                    "● REC"
                } else {
                    "REC"
                };
                if deck_key(ui, rec_lbl, Color32::from_rgb(255, 160, 160), mix(th.btn_bg, rec_red, 0.35)).clicked() {
                    self.req_record();
                }
                if deck_key(ui, "EJECT", th.btn_fg, th.btn_bg).clicked() {
                    if self.tape_out {
                        self.tape_out = false;
                        self.set_status("Tape in");
                    } else {
                        self.tape_out = true;
                        self.tape_reversing = false;
                        self.stop_music();
                        if self.radio_on {
                            self.stop_radio();
                        }
                        self.set_status("EJECT");
                    }
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "Skip", th.playing_fg, th.btn_bg).clicked() { self.skip_song(); }
                        if retro_btn(ui, "Prev", th.btn_fg, th.btn_bg).clicked() { self.prev_song(); }
                    });
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "Change Dir", th.playing_fg, th.btn_bg).clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                self.scanning = true;
                                self.set_status("Scanning library...");
                                let _ = self.lib_tx.send(LibCmd::Scan(dir.to_string_lossy().to_string()));
                            }
                        }
                        if retro_btn(ui, "Refresh", th.btn_fg, th.btn_bg).clicked() {
                            self.scanning = true;
                            if let Some(d) = self.state.current_dir.clone() {
                                self.set_status("Scanning library...");
                                let _ = self.lib_tx.send(LibCmd::Scan(d));
                            } else {
                                self.set_status("Locating music folder...");
                                let _ = self.lib_tx.send(LibCmd::FindMusicFolder);
                            }
                        }
                    });
                });
            });

            ui.add_space(6.0);
            ui.separator();
            ui.horizontal(|ui| {
                if retro_btn(ui, "Save", th.btn_fg, th.btn_bg).clicked() { self.save_song(); }
                if retro_btn(ui, "Clear", th.btn_fg, th.btn_bg).clicked() { self.clear_playlist(); }
                if retro_btn(ui, "Shuffle", th.playing_fg, th.btn_bg).clicked() { self.shuffle_playlist(); }
                if retro_btn(ui, "Smart", th.playing_fg, th.btn_bg).clicked() { self.smart_shuffle(); }
                if retro_btn(ui, "Add", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(files) = rfd::FileDialog::new().add_filter("Audio", &AUDIO_FORMATS).pick_files() {
                        let entries: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
                        self.set_status("Adding files...");
                        let _ = self.lib_tx.send(LibCmd::AddMany(entries));
                    }
                }
                if retro_btn(ui, "Convert", th.fg, th.btn_bg)
                    .on_hover_text("Convert a folder of music to MP3/AAC/FLAC/OGG/Opus/WAV/AIFF/ALAC/WMA")
                    .clicked()
                {
                    self.convert_open = true;
                }
                if self.meta_all_active {
                    let scanning = self.meta_all_art_scanning;
                    let frac = if scanning { 0.0 } else { self.meta_all_frac() };
                    let eta = self.meta_all_eta_text();
                    let label = if scanning {
                        "checking cover cache...".to_string()
                    } else {
                        format!("{:.0}%  {}", frac * 100.0, eta)
                    };
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_width(220.0)
                            .text(label),
                    );
                    if retro_btn(ui, "CANCEL", Color32::from_rgb(255, 120, 120), th.btn_bg)
                        .on_hover_text("Stop Meta All (already-completed tags/covers/ratings are kept)")
                        .clicked()
                    {
                        self.cancel_meta_all();
                    }
                } else if retro_btn(ui, "Meta All", th.playing_fg, th.btn_bg)
                    .on_hover_text("Fetch tags, cover art (iTunes/Deezer/Cover Art Archive/TheAudioDB) and MusicBrainz community ratings for every playlist track")
                    .clicked()
                {
                    self.download_all_meta();
                }
            });
            if self.meta_all_active {
                ui.label(
                    RichText::new(self.meta_all_detail_text())
                        .color(Color32::from_gray(150))
                        .monospace()
                        .size(9.0),
                );
            }

            ui.horizontal(|ui| {
                let accent = |on: bool| if on { th.playing_fg } else { th.btn_fg };
                if retro_btn(ui, &format!("Repeat: {}", if self.state.repeat_enabled { "On" } else { "Off" }), accent(self.state.repeat_enabled), th.btn_bg).clicked() { self.toggle_repeat(); }
                if retro_btn(ui, &format!("Playlist Only: {}", if self.state.playlist_only_mode { "On" } else { "Off" }), accent(self.state.playlist_only_mode), th.btn_bg).clicked() { self.toggle_playlist_only(); }
                if retro_btn(ui, &format!("Sequential: {}", if self.state.playlist_only_sequential { "On" } else { "Off" }), accent(self.state.playlist_only_sequential), th.btn_bg).clicked() { self.toggle_playlist_sequential(); }
                if retro_btn(ui, &format!("Dir Seq: {}", if self.state.dir_sequential { "On" } else { "Off" }), accent(self.state.dir_sequential), th.btn_bg).clicked() { self.toggle_dir_sequential(); }
                let stars_lbl = if self.rating_filter == 0 { "All".to_string() } else { format!("{}★+", self.rating_filter) };
                if retro_btn(ui, &format!("★{}", stars_lbl), accent(self.rating_filter > 0), th.btn_bg).on_hover_text("Star filter: auto-play only songs rated this many stars. Click to cycle: All -> 1+ ... -> 5+.").clicked() { self.cycle_rating_filter(); }
            });

            ui.horizontal(|ui| {
                if retro_btn(ui, "Save Pl", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_file_name("playlist.json").add_filter("JSON", &["json"]).save_file() {
                        let pl = self.state.playlist.clone();
                        let _ = self.lib_tx.send(LibCmd::SavePlaylist(p.to_string_lossy().to_string(), pl));
                    }
                }
                if retro_btn(ui, "Load Pl", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("JSON", &["json"]).pick_file() {
                        let _ = self.lib_tx.send(LibCmd::LoadPlaylist(p.to_string_lossy().to_string()));
                    }
                }
                if retro_btn(ui, "Look Up", th.btn_fg, th.btn_bg).clicked() {
                    if let Some(s) = self.state.current_song.clone() {
                        open_in_explorer(&s);
                    }
                }
                if retro_btn(ui, "Video", if self.video_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() {
                    if self.video_on { self.close_video(); } else { self.play_video(); }
                }
                if retro_btn(ui, "Lyrics", th.btn_fg, th.btn_bg).clicked() { self.fetch_lyrics(); }
                if retro_btn(ui, "Help", th.btn_fg, th.btn_bg).clicked() { self.show_help = true; }
            });

            ui.horizontal_wrapped(|ui| {
                if retro_btn(ui, &format!("Theme: {}", self.state.theme), th.playing_fg, th.btn_bg).clicked() { self.cycle_theme(); }
                if retro_btn(ui, &format!("EQ {}", if self.eq_on { self.eq_name() } else { "off" }), if self.eq_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() { self.eq_toggle(); }
                ui.label(RichText::new("CURVE:").color(th.fg).monospace().size(10.0));
                let mut pick: Option<&'static str> = None;
                egui::ComboBox::from_id_salt("eq_curve_pick")
                    .selected_text(if self.eq_custom.is_some() { "Custom" } else { self.eq_name() })
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for preset in EQ_PRESETS.iter() {
                            let selected = self.eq_custom.is_none() && preset.name == self.state.eq_preset;
                            if ui
                                .selectable_label(selected, preset.name)
                                .on_hover_text(preset.description)
                                .clicked()
                            {
                                pick = Some(preset.name);
                            }
                        }
                    });
                if let Some(name) = pick {
                    self.eq_select(name);
                }
                if retro_btn(ui, "◀", th.btn_fg, th.btn_bg).clicked() { self.cycle_eq_back(); }
                if retro_btn(ui, "▶", th.btn_fg, th.btn_bg).clicked() { self.cycle_eq(); }
                if retro_btn(ui, "FLAT", th.btn_fg, th.btn_bg).on_hover_text("Reset to the reference curve").clicked() { self.eq_select("Flat"); }
                ui.separator();
                ui.label(RichText::new("Sleep:").color(th.fg).monospace().size(11.0));
                ui.add(egui::TextEdit::singleline(&mut self.sleep_input).desired_width(46.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                if retro_btn(ui, "Sleep", th.btn_fg, th.btn_bg).clicked() {
                    let minutes = self.sleep_input.trim().parse::<i64>().unwrap_or(0);
                    self.set_sleep_timer(minutes);
                }
                let sleep_label = if self.sleep_minutes > 0 { format!("Timer: {}m", self.sleep_minutes) } else { "Timer: Off".to_string() };
                lcd(ui, &sleep_label, if self.sleep_minutes > 0 { th.playing_fg } else { th.btn_fg }, th.bg);
            });

            if self.eq_on {
                let gains = self.eq_cur();
                let preamp = eq_auto_preamp(&gains, 44100.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("EQ CURVE:").color(th.fg).monospace().size(11.0));
                    let mut pts = vec![(0.0f32, 0.0f32); 128];
                    eq_curve_db(&gains, 44100.0, &mut pts);
                    let (graph_rect, _) = ui.allocate_exact_size(egui::vec2(480.0, 84.0), egui::Sense::hover());
                    if ui.is_rect_visible(graph_rect) {
                        let p = ui.painter().with_clip_rect(graph_rect);
                        p.rect_filled(graph_rect, 3.0, Color32::from_rgb(6, 8, 10));
                        p.rect_stroke(graph_rect, 3.0, egui::Stroke::new(1.0, darken(th.accent, 0.55)), egui::StrokeKind::Inside);
                        let x_for = |f: f32| graph_rect.left() + (f.log10() - 20.0f32.log10()) / (20000.0f32.log10() - 20.0f32.log10()) * graph_rect.width();
                        let y_for = |db: f32| graph_rect.center().y - db / 18.0 * (graph_rect.height() * 0.86 / 2.0);
                        let line_col = if gains.iter().all(|&g| g.abs() < 0.5) { th.btn_fg } else { th.accent };
                        // Fill under the curve with thin translucent columns so
                        // the response reads at a glance without drawing a
                        // concave polygon.
                        for &(f, db) in pts.iter() {
                            if db.abs() < 0.15 {
                                continue;
                            }
                            let x = x_for(f);
                            let y = y_for(db);
                            p.line_segment(
                                [egui::pos2(x, graph_rect.center().y), egui::pos2(x, y)],
                                egui::Stroke::new(2.2, line_col.linear_multiply(0.06)),
                            );
                        }
                        for db in [-12.0f32, -6.0, 0.0, 6.0, 12.0] {
                            let y = y_for(db);
                            p.hline(
                                graph_rect.left() + 4.0..=graph_rect.right() - 4.0,
                                y,
                                egui::Stroke::new(1.0, if db == 0.0 { Color32::from_gray(62) } else { Color32::from_gray(30) }),
                            );
                            p.text(
                                egui::pos2(graph_rect.right() - 5.0, y),
                                egui::Align2::RIGHT_CENTER,
                                format!("{:+}", db as i32),
                                egui::FontId::monospace(7.0),
                                Color32::from_gray(92),
                            );
                        }
                        for &f0 in EQ_BANDS.iter() {
                            let x = x_for(f0 as f32);
                            p.line_segment([egui::pos2(x, graph_rect.top() + 4.0), egui::pos2(x, graph_rect.bottom() - 4.0)], egui::Stroke::new(1.0, Color32::from_gray(26)));
                        }
                        let mut prev: Option<egui::Pos2> = None;
                        for &(f, db) in pts.iter() {
                            let cpt = egui::pos2(x_for(f), y_for(db));
                            if let Some(pp) = prev {
                                p.line_segment([pp, cpt], egui::Stroke::new(2.0, line_col));
                            }
                            prev = Some(cpt);
                        }
                        for (i, &f0) in EQ_BANDS.iter().enumerate() {
                            let g = gains[i];
                            let cpt = egui::pos2(x_for(f0 as f32), y_for(g as f32));
                            p.circle_filled(cpt, 3.0, if g.abs() >= 0.5 { th.playing_fg } else { Color32::from_gray(70) });
                            if g.abs() >= 0.1 {
                                p.text(
                                    egui::pos2(cpt.x, cpt.y - 5.0),
                                    egui::Align2::CENTER_BOTTOM,
                                    format!("{:+}", g),
                                    egui::FontId::monospace(7.0),
                                    Color32::from_gray(135),
                                );
                            }
                            let ftext = if f0 >= 1000.0 { format!("{:.0}k", f0 / 1000.0) } else { format!("{}", f0 as i32) };
                            p.text(egui::pos2(x_for(f0 as f32), graph_rect.bottom() - 8.0), egui::Align2::CENTER_TOP, ftext, egui::FontId::monospace(8.0), Color32::from_gray(110));
                        }
                        p.text(egui::pos2(graph_rect.left() + 6.0, graph_rect.top() + 8.0), egui::Align2::LEFT_TOP, format!("{} · preamp {:.0}% · 1-octave", self.eq_name(), preamp * 100.0), egui::FontId::monospace(9.0), th.playing_fg);
                    }
                });
                ui.label(RichText::new(self.eq_desc()).color(Color32::from_gray(155)).monospace().size(9.0));
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().slider_width = 70.0;
                    for i in 0..10 {
                        let mut g = gains[i];
                        ui.vertical(|ui| {
                            let resp = ui.add(
                                egui::Slider::new(&mut g, -12.0..=12.0)
                                    .step_by(0.5)
                                    .show_value(false)
                                    .orientation(egui::SliderOrientation::Vertical)
                                    .custom_formatter(|v, _| format!("{:+}", v as i32)),
                            );
                            if resp.changed() {
                                self.eq_set(i, g);
                            }
                            let f0 = EQ_BANDS[i];
                            let ftext = if f0 >= 1000.0 { format!("{:.0}k", f0 / 1000.0) } else { format!("{}", f0 as i32) };
                            ui.label(RichText::new(ftext).color(th.btn_fg).monospace().size(8.0));
                        });
                    }
                });
            }

            ui.separator();

            ui.horizontal(|ui| {
                ui.label(RichText::new("SEARCH:").color(th.fg).monospace().size(11.0));
                let resp = ui.add(egui::TextEdit::singleline(&mut self.search_query).hint_text("type to filter, click a row to play").desired_width(380.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                if resp.secondary_clicked() {
                    resp.request_focus();
                }
                resp.context_menu(|ui| {
                    if ui.button("Cut").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut); ui.close(); }
                    if ui.button("Copy").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy); ui.close(); }
                    if ui.button("Paste").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste); ui.close(); }
                });
                if self.search_focus {
                    resp.request_focus();
                    self.search_focus = false;
                }
                if retro_btn(ui, "J", Color32::from_rgb(255, 240, 160), th.btn_bg).clicked() {
                    self.search_focus = true;
                }
            });

            ui.horizontal(|ui| {
                ui.label(RichText::new("YT:").color(th.playing_fg).monospace().size(11.0));
                let resp = ui.add(egui::TextEdit::singleline(&mut self.yt_query).hint_text("paste a YouTube URL or search term").desired_width(340.0).font(egui::TextStyle::Monospace).text_color(th.accent));
                if resp.secondary_clicked() {
                    resp.request_focus();
                }
                resp.context_menu(|ui| {
                    if ui.button("Cut").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut); ui.close(); }
                    if ui.button("Copy").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy); ui.close(); }
                    if ui.button("Paste").clicked() { let _ = self.ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste); ui.close(); }
                });
                if self.yt_focus {
                    resp.request_focus();
                    self.yt_focus = false;
                }
                if retro_btn(ui, if self.yt_queue_mode { "QUEUE" } else { "SINGLE" },
                    if self.yt_queue_mode { Color32::from_rgb(120, 220, 255) } else { Color32::from_rgb(255, 240, 160) },
                    th.btn_bg,
                ).clicked() {
                    self.yt_queue_mode = !self.yt_queue_mode;
                }
                if self.yt_queue_mode {
                    if retro_btn(ui, "Add", Color32::from_rgb(255, 240, 160), th.btn_bg).clicked() {
                        let q = self.yt_query.clone();
                        self.yt_queue_add(&q);
                    }
                    if retro_btn(ui, &format!("Queue ({})", self.yt_queue.len()), th.btn_fg, th.btn_bg).clicked() {
                        self.yt_queue_show = true;
                    }
                } else {
                    let busy = self.yt_queue_processing;
                    if retro_btn(ui, if busy { "QUEUE BUSY" } else { "Download" },
                        if busy { darken(th.btn_fg, 0.5) } else { Color32::from_rgb(255, 240, 160) },
                        th.btn_bg,
                    ).clicked() && !busy {
                        let q = self.yt_query.clone();
                        if q.trim().is_empty() {
                            self.set_error("Enter a YouTube URL or search term");
                        } else {
                            self.yt_item_no = 0;
                            self.yt_begin_item();
                            self.set_status(format!("YouTube: saving to {}", music_folder().display()));
                            let _ = self.yt_tx.send(YtCmd::Download { query: q, auto_play: true, chunks: 1, group: None });
                        }
                    }
                }
                ui.label(RichText::new("-> ").color(th.btn_fg).monospace().size(10.0));
                let folder = music_folder().to_string_lossy().to_string();
                ui.label(RichText::new(truncate_mid(&folder, 46)).color(Color32::from_gray(120)).monospace().size(10.0));
            });
            if self.yt_pct > -1.0 {
                // Counter, percentage, stage and elapsed time beside the bar.
                // A bar that resets to 0% for the next item reads as a glitch
                // unless something says which item is running.
                let (n, m) = self.yt_item_counter();
                let elapsed = self.yt_started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                let status = if !self.yt_speed.is_empty() {
                    format!("{} ETA {}", self.yt_speed, self.yt_eta)
                } else if !self.yt_band_label.is_empty() {
                    self.yt_band_label.clone()
                } else {
                    "starting...".to_string()
                };
                let pct = (self.yt_bar * 100.0).clamp(0.0, 100.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("▼").color(th.accent).monospace().size(10.0));
                    ui.add(egui::ProgressBar::new(self.yt_bar.clamp(0.0, 1.0))
                        .desired_width(f32::INFINITY)
                        .text(RichText::new(format!("{:.0}%  {}", pct, status)).color(Color32::from_gray(230)).monospace().size(10.0))
                        .fill(Color32::from_rgb(28, 224, 255)));
                    ui.label(RichText::new(format!("{}/{}  {}s", n, m, elapsed))
                        .color(Color32::from_gray(150)).monospace().size(10.0));
                });
            }

            let query = if self.search_query.is_empty() {
                String::new()
            } else {
                self.search_query.to_lowercase()
            };
            // The common case (no search) no longer builds an 18k-element index
            // vector every repaint. Only a real search allocates one.
            let cur_idx = self
                .state
                .current_song
                .as_deref()
                .and_then(|c| {
                    self.playing_pl_idx
                        .filter(|i| self.state.playlist.get(*i).map(|p| p.as_str() == c).unwrap_or(false))
                })
                .or_else(|| {
                    self.state
                        .current_song
                        .as_deref()
                        .and_then(|c| self.state.playlist.iter().position(|p| p.as_str() == c))
                });
            let filtered = !query.is_empty();
            let rows: Vec<usize> = if filtered {
                self.state
                    .playlist
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| {
                        let d = self.display_cache[*i].to_lowercase();
                        let path = p.to_lowercase();
                        d.contains(&query) || path.contains(&query)
                    })
                    .map(|(i, _)| i)
                    .collect()
            } else {
                Vec::new()
            };
            let row_count = if filtered { rows.len() } else { self.state.playlist.len() };

            ui.horizontal(|ui| {
                ui.label(RichText::new("PLAYLIST").color(darken(th.accent, 0.35)).monospace().strong().size(10.0));
                let total = self.state.playlist.len();
                let (now_txt, now_col) = match cur_idx {
                    Some(ci) => (format!("NOW {:>4}/{}", ci + 1, total), th.playing_fg),
                    None => (format!("NOW  -/{}", total), Color32::from_gray(90)),
                };
                ui.label(RichText::new(now_txt).color(now_col).monospace().size(10.0));
                let shown = self.state.current_song.as_deref().map(stem).unwrap_or_default();
                ui.label(RichText::new(truncate_mid(&shown, 42)).color(now_col).monospace().size(10.0));
            });
            ui.add_space(2.0);

            let mut play_choice: Option<usize> = None;
            let row_h = 20.0;
            let mut sa = egui::ScrollArea::vertical().auto_shrink([false, false]);
            if cur_idx != self.pl_last_cur {
                self.pl_last_cur = cur_idx;
                if let Some(ci) = cur_idx {
                    let pos = if filtered {
                        rows.iter().position(|i| *i == ci)
                    } else if ci < row_count {
                        Some(ci)
                    } else {
                        None
                    };
                    if let Some(pos) = pos {
                        let avail_h = ui.available_height().max(1.0);
                        let spacing_y = ui.spacing().item_spacing.y;
                        let sh = row_h + spacing_y;
                        let target = (pos as f32 * sh) - avail_h * 0.5;
                        let max_off = (sh * row_count as f32 - spacing_y - avail_h).max(0.0);
                        sa = sa.vertical_scroll_offset(target.clamp(0.0, max_off));
                    }
                }
            }
            sa.show_rows(ui, row_h, row_count, |ui, range| {
                    for i in range {
                        let idx = if filtered { rows[i] } else { i };
                        let is_current = cur_idx == Some(idx);
                        let color = if is_current { th.playing_fg } else { th.fg };
                        let mark = if is_current { "▶ " } else { "  " };
                        let row_text = format!("{} {:>3}  {}", mark, idx + 1, self.display_cache[idx]);
                        let (row_rect, resp) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width().max(1.0), row_h),
                            egui::Sense::click(),
                        );
                        if resp.hovered() && !is_current {
                            ui.painter().rect_filled(row_rect, 2.0, Color32::from_gray(255).linear_multiply(0.05));
                        }
                        // Star zone on the right; the row text is clipped so it never runs underneath.
                        let star_zone_w = 5.0 * 13.0 + 6.0;
                        let text_right = (row_rect.right() - star_zone_w - 4.0).max(row_rect.min.x);
                        let tp = ui.painter().with_clip_rect(egui::Rect::from_min_max(
                            row_rect.min + egui::vec2(4.0, 0.0),
                            egui::pos2(text_right, row_rect.max.y),
                        ));
                        tp.text(
                            egui::pos2(row_rect.min.x + 4.0, row_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            row_text.as_str(),
                            egui::FontId::monospace(12.0),
                            color,
                        );
                        let p_path = self.state.playlist[idx].clone();
                        let p_rating = self.song_rating(&p_path);
                        let mut star_pick: Option<u8> = None;
                        let sx0 = row_rect.right() - 4.0 - 5.0 * 13.0;
                        for k in 0..5usize {
                            let sr = egui::Rect::from_min_size(
                                egui::pos2(sx0 + k as f32 * 13.0, row_rect.center().y - 8.0),
                                egui::vec2(13.0, 16.0),
                            );
                            let sresp = ui.interact(sr, ui.id().with(("star", idx, k)), egui::Sense::click());
                            if sresp.clicked() {
                                star_pick = Some((k + 1) as u8);
                            }
                            let filled = (p_rating as usize) > k;
                            let (glyph, sc) = if filled {
                                ("★", Color32::from_rgb(255, 210, 80))
                            } else {
                                ("☆", if sresp.hovered() { Color32::from_gray(210) } else { Color32::from_gray(95) })
                            };
                            ui.painter().text(
                                sr.center(),
                                egui::Align2::CENTER_CENTER,
                                glyph,
                                egui::FontId::proportional(12.0),
                                sc,
                            );
                        }
                        if let Some(n) = star_pick {
                            self.rate_song(&p_path, if p_rating == n { 0 } else { n });
                        }
                        if is_current {
                            let hl = resp.rect.expand(1.0);
                            ui.painter().rect_filled(hl, 2.0, th.playing_fg.linear_multiply(0.25));
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(hl.min, egui::vec2(3.0, hl.height())),
                                1.0,
                                th.playing_fg,
                            );
                        }
                        if resp.clicked() && self.drag_from.is_none() {
                            play_choice = Some(idx);
                        }
                        if resp.drag_started() {
                            self.drag_from = Some(idx);
                            self.drag_active = true;
                        }
                        if self.drag_active {
                            if resp.hovered() && self.drag_from != Some(idx) {
                                self.drag_hover = Some(idx);
                            }
                            if resp.drag_stopped() {
                                let from = self.drag_from;
                                let to = self.drag_hover;
                                self.drag_from = None;
                                self.drag_hover = None;
                                self.drag_active = false;
                                if let (Some(f), Some(t)) = (from, to) {
                                    if f != t {
                                        self.move_playlist(f, t);
                                    }
                                }
                            }
                            if self.drag_hover == Some(idx) {
                                ui.painter().hline(
                                    resp.rect.min.x..=resp.rect.max.x,
                                    resp.rect.bottom(),
                                    egui::Stroke::new(2.0, th.accent),
                                );
                            }
                        }
                    }
                });
            if let Some(i) = play_choice {
                let p = self.state.playlist[i].clone();
                self.playing_pl_idx = Some(i);
                self.play_song(&p);
            }

            ui.add_space(4.0);
            let log_frame = egui::Frame::default()
                .fill(Color32::from_rgb(4, 4, 6))
                .stroke(egui::Stroke::new(1.0, th.btn_fg))
                .inner_margin(egui::Margin::symmetric(6, 3));
            log_frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("[YT LOG]").color(th.accent).monospace().size(10.0));
                    if !self.yt_log.is_empty() && retro_btn(ui, "Clear", th.btn_fg, th.btn_bg).clicked() {
                        self.yt_log.clear();
                    }
                });
                if self.yt_pct > -1.0 {
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
                egui::ScrollArea::vertical().max_height(80.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if self.yt_log.is_empty() {
                        ui.label(RichText::new("idle - paste a YouTube URL or search term above").color(Color32::from_gray(90)).monospace().size(10.0));
                    } else {
                        for line in self.yt_log.iter().rev().take(24).rev() {
                            ui.label(RichText::new(truncate_mid(line, 130)).color(if line.starts_with("ERROR") || line.starts_with("Deleting") { Color32::from_rgb(255, 120, 120) } else { Color32::from_gray(170) }).monospace().size(10.0));
                        }
                    }
                });
            });
        });
    }

    pub(crate) fn show_radio_window(&mut self, ctx: &egui::Context) {
        let th = self.theme_colors();
        let mut open = self.show_radio;
        egui::Window::new("RADIO TUNER")
            .resizable(true)
            .collapsible(false)
            .default_width(430.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("BAND:").color(th.fg).monospace().size(11.0));
                    let fm_on = self.radio_band == "FM";
                    let am_on = self.radio_band == "AM";
                    if retro_btn(ui, "FM", if fm_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !fm_on {
                        self.radio_band = "FM".to_string();
                        self.radio_freq = 88.1;
                        self.radio_last_moved = ui.input(|i| i.time);
                    }
                    if retro_btn(ui, "AM", if am_on { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() && !am_on {
                        self.radio_band = "AM".to_string();
                        self.radio_freq = 860.0;
                        self.radio_last_moved = ui.input(|i| i.time);
                    }
                });
                ui.add_space(4.0);
                let (lo, hi) = self.radio_band_range();
                let span = hi - lo;
                let dial_resp = ui.allocate_response(egui::vec2(ui.available_width().min(360.0), 150.0), egui::Sense::click_and_drag());
                let rect = dial_resp.rect;
                let p = ui.painter().with_clip_rect(rect.expand(2.0));
                p.rect_filled(rect, 8.0, darken(th.btn_bg, 0.35));
                p.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, darken(th.btn_bg, 0.7)), egui::StrokeKind::Inside);
                let screen = rect.shrink(8.0);
                p.rect_filled(screen, 4.0, darken(Color32::from_rgb(18, 20, 16), 0.0));
                p.rect_stroke(screen, 4.0, egui::Stroke::new(1.0, Color32::from_gray(40)), egui::StrokeKind::Inside);
                let map = |f: f32| screen.min.x + screen.width() * ((f - lo) / span);
                for s in self.radio_presets.iter().filter(|s| s.band.eq_ignore_ascii_case(&self.radio_band)) {
                    if s.freq >= lo && s.freq <= hi {
                        let x = map(s.freq);
                        p.line_segment(
                            [egui::pos2(x, screen.min.y + 8.0), egui::pos2(x, screen.min.y + 20.0)],
                            egui::Stroke::new(1.0, th.accent),
                        );
                    }
                }
                let n = 20;
                for i in 0..=n {
                    let f = lo + span * (i as f32 / n as f32);
                    let x = map(f);
                    let major = i % 5 == 0;
                    let ty = if major { 30.0 } else { 16.0 };
                    p.line_segment(
                        [egui::pos2(x, screen.min.y + 26.0), egui::pos2(x, screen.min.y + 26.0 + ty)],
                        egui::Stroke::new(1.0, if major { Color32::from_gray(170) } else { Color32::from_gray(90) }),
                    );
                    if major {
                        let lab = format!("{}", f.round());
                        p.text(
                            egui::pos2(x, screen.min.y + 60.0),
                            egui::Align2::CENTER_TOP,
                            lab,
                            egui::FontId::monospace(9.0),
                            Color32::from_gray(120),
                        );
                    }
                }
                for s in self.radio_presets.iter().filter(|s| s.band.eq_ignore_ascii_case(&self.radio_band)) {
                    if s.freq >= lo && s.freq <= hi {
                        let x = map(s.freq);
                        p.text(
                            egui::pos2(x, screen.min.y + 70.0),
                            egui::Align2::CENTER_TOP,
                            &s.name,
                            egui::FontId::monospace(7.5),
                            Color32::from_gray(150),
                        );
                    }
                }
                let nx = map(self.radio_freq).clamp(screen.min.x + 4.0, screen.max.x - 4.0);
                p.line_segment(
                    [egui::pos2(nx, screen.min.y + 4.0), egui::pos2(nx, screen.max.y - 4.0)],
                    egui::Stroke::new(1.6, if self.radio_tuned.is_some() { th.playing_fg } else { Color32::from_rgb(190, 120, 60) }),
                );
                let tri = [
                    egui::pos2(nx - 6.0, screen.max.y - 12.0),
                    egui::pos2(nx + 6.0, screen.max.y - 12.0),
                    egui::pos2(nx, screen.max.y - 3.0),
                ];
                p.add(egui::Shape::convex_polygon(tri.to_vec(), if self.radio_tuned.is_some() { th.playing_fg } else { Color32::from_rgb(190, 120, 60) }, egui::Stroke::NONE));
                ui.add_space(2.0);
                if dial_resp.dragged() {
                    let d = dial_resp.drag_delta().x / rect.width() * span;
                    self.radio_freq = (self.radio_freq + d).clamp(lo, hi);
                    self.radio_last_moved = ui.input(|i| i.time);
                }
                if dial_resp.clicked() {
                    if let Some(pos) = dial_resp.interact_pointer_pos() {
                        let f = lo + span * ((pos.x - screen.min.x) / screen.width());
                        self.radio_freq = f.clamp(lo, hi);
                        self.radio_last_moved = ui.input(|i| i.time);
                    }
                }
                let stname = self.radio_tuned.and_then(|i| self.radio_presets.get(i)).map(|s| s.name.clone());
                let playing = self.radio_on && !self.state.is_paused;
                ui.horizontal(|ui| {
                    lcd(ui, &fmt_freq(self.radio_freq), th.playing_fg, th.bg);
                    ui.label(RichText::new(stname.as_deref().unwrap_or("-- STATIC --")).color(if playing { th.playing_fg } else { Color32::from_gray(110) }).monospace().size(13.0));
                    if playing {
                        ui.label(RichText::new("● LIVE").color(Color32::from_rgb(0, 255, 90)).monospace().size(11.0));
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if retro_btn(ui, "TUNE-", th.btn_fg, th.btn_bg).clicked() {
                        self.tune_radio_to(self.radio_freq - if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                    }
                    if retro_btn(ui, "TUNE+", th.btn_fg, th.btn_bg).clicked() {
                        self.tune_radio_to(self.radio_freq + if self.radio_band == "AM" { 10.0 } else { 0.1 }, ui.input(|i| i.time));
                    }
                    if retro_btn(ui, "SCAN", th.playing_fg, th.btn_bg).clicked() {
                        self.scan_radio(ui.input(|i| i.time));
                    }
                    if retro_btn(ui, if playing { "PAUSE" } else { "PLAY" }, th.playing_fg, th.btn_bg).clicked() {
                        if self.state.is_paused {
                            self.unpause_music();
                        } else {
                            self.pause_music();
                        }
                    }
                    if retro_btn(ui, "STOP", th.btn_fg, th.btn_bg).clicked() {
                        self.stop_radio();
                    }
                    if retro_btn(ui, "REC", if self.recording { th.playing_fg } else { Color32::from_rgb(255, 120, 120) }, mix(th.btn_bg, Color32::from_rgb(205, 45, 45), 0.35)).clicked() {
                        self.record_radio();
                    }
                });
                ui.add_space(4.0);
                ui.separator();
                let rows: Vec<(usize, f32, String)> = self
                    .radio_presets
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.band.eq_ignore_ascii_case(&self.radio_band))
                    .map(|(i, s)| (i, s.freq, s.name.clone()))
                    .collect();
                egui::ScrollArea::vertical().max_height(140.0).show_rows(ui, 20.0, rows.len(), |ui, range| {
                    for i in range {
                        let (idx, freq, name) = &rows[i];
                        let current = self.radio_playing_idx == Some(*idx);
                        let lab = format!("{:>8}   {}", fmt_freq(*freq), name);
                        if ui.selectable_label(current, RichText::new(lab).color(if current { th.playing_fg } else { th.fg }).monospace()).clicked() {
                            self.tune_radio_to(*freq, ui.input(|i| i.time));
                            self.start_radio(*idx);
                        }
                    }
                });
            });
        self.show_radio = open;
    }

    pub(crate) fn show_convert_window(&mut self, ctx: &egui::Context) {
        let th = self.theme_colors();
        let mut open = self.convert_open;
        let mut close_requested = false;
        egui::Window::new("CONVERT MUSIC")
            .resizable(false)
            .collapsible(false)
            .default_width(600.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SOURCE FOLDER").color(th.accent).monospace().size(11.0));
                    if retro_btn(ui, "CHOOSE FOLDER...", th.btn_fg, th.btn_bg).clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.convert_src = Some(p.to_string_lossy().to_string());
                        }
                    }
                });
                ui.label(
                    RichText::new(
                        self.convert_src
                            .as_deref()
                            .unwrap_or("No folder selected"),
                    )
                    .color(Color32::from_gray(170))
                    .monospace()
                    .size(10.0),
                );
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new("OUTPUT FORMAT").color(th.accent).monospace().size(11.0));
                    egui::ComboBox::from_id_salt("convert_output_format")
                        .selected_text(self.convert_format.label())
                        .show_ui(ui, |ui| {
                            for f in crate::convert::ConvertFormat::ALL {
                                ui.selectable_value(&mut self.convert_format, f, f.label());
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("QUALITY 1-10").color(th.accent).monospace().size(11.0));
                    let label = self.convert_format.quality_label(self.convert_quality);
                    ui.add(
                        egui::Slider::new(&mut self.convert_quality, 1..=10)
                            .text(label)
                            .clamping(egui::SliderClamping::Always),
                    );
                });
                let dest = if let Some(src) = &self.convert_src {
                    crate::convert::destination_for(src, self.convert_format)
                } else {
                    crate::convert::destination(self.convert_format)
                };
                ui.label(
                    RichText::new(format!("SAVE TO  {}", dest.display()))
                        .color(Color32::from_gray(150))
                        .monospace()
                        .size(9.0),
                );
                ui.label(
                    RichText::new("1 = smallest/fastest, 10 = best quality. Tags and embedded cover art (MP3/AAC/FLAC/ALAC) are preserved.")
                        .color(Color32::from_gray(115))
                        .monospace()
                        .size(9.0),
                );
                ui.add_space(8.0);

                if self.convert_active {
                    let frac = if self.convert_total > 0 {
                        self.convert_done as f32 / self.convert_total as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(frac.clamp(0.0, 1.0))
                            .desired_width(440.0)
                            .text(format!("{}/{}", self.convert_done, self.convert_total)),
                    );
                    ui.label(
                        RichText::new(&self.convert_text)
                            .color(th.fg)
                            .monospace()
                            .size(9.0),
                    );
                    ui.horizontal(|ui| {
                        if retro_btn(
                            ui,
                            "CANCEL CONVERSION",
                            Color32::from_rgb(255, 120, 120),
                            th.btn_bg,
                        )
                        .clicked()
                        {
                            self.cancel_convert();
                        }
                        if retro_btn(ui, "HIDE", th.btn_fg, th.btn_bg)
                            .on_hover_text("Hide this window; the conversion keeps running")
                            .clicked()
                        {
                            close_requested = true;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        if retro_btn(ui, "START CONVERSION", th.playing_fg, th.btn_bg).clicked() {
                            self.start_convert();
                        }
                        if let Some(d) = &self.convert_dest {
                            if retro_btn(ui, "OPEN FOLDER", th.btn_fg, th.btn_bg).clicked() {
                                open_in_explorer(d);
                            }
                        }
                        if retro_btn(ui, "CLOSE", th.btn_fg, th.btn_bg).clicked() {
                            close_requested = true;
                        }
                    });
                }
            });
        // The in-window CLOSE/HIDE buttons and the window X are both honoured.
        // (Assigning `open` alone used to undo the in-window CLOSE click.)
        self.convert_open = open && !close_requested;
    }

    pub(crate) fn show_help_window(&mut self, ctx: &egui::Context) {
        egui::Window::new("SYSTEM MANUAL")
            .resizable(true)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                let th = self.theme_colors();
                egui::ScrollArea::vertical()
                    .max_height(640.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                let g = Color32::from_rgb(0, 255, 0);
                let dim = Color32::from_rgb(100, 200, 100);
                let acc = th.accent;

                ui.label(
                    RichText::new(format!("RANDOM SHUFFLE PLAYER v{}", APP_VERSION))
                        .color(acc)
                        .monospace()
                        .strong(),
                );
                ui.add_space(6.0);

                ui.label(RichText::new("PLAYBACK FORMATS").color(acc).monospace().strong());
                ui.label(RichText::new("Plays ............... MP3, FLAC, M4A/AAC/ALAC, WAV, APE, AIFF, OGG/Vorbis, Opus, WMA, WavPack, Musepack, WebM audio").color(dim).monospace());
                ui.label(RichText::new("ffmpeg fallback ..... Formats rodio cannot open directly (M4A, APE, WMA, WavPack, Musepack) are converted to a cached WAV on first play").color(dim).monospace());
                ui.label(RichText::new("CUE sheets .......... Single-file .ape/.flac images with a .cue are expanded into individual tracks (tags, art, seeking and conversion)").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("CONVERT MUSIC").color(acc).monospace().strong());
                ui.label(RichText::new("Convert button ...... Convert a folder recursively (MP3, AAC, FLAC, OGG, Opus, WAV, AIFF, ALAC, WMA, or same as source)").color(dim).monospace());
                ui.label(RichText::new("Quality 1-10 ........ Lossy: bitrate; FLAC: compression level; WAV/AIFF/ALAC: bit depth").color(dim).monospace());
                ui.label(RichText::new("Output .............. ~/Music/<picked folder> (<format>); tags and embedded art preserved where supported").color(dim).monospace());
                ui.label(RichText::new("Progress / Cancel ... Per-file progress; HIDE keeps it running; CANCEL stops ffmpeg").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("META ALL (TAGS, COVERS, RATINGS)").color(acc).monospace().strong());
                ui.label(RichText::new("Meta All ............ Fetch tags, covers and MusicBrainz ratings; progress bar, ETA and CANCEL").color(dim).monospace());
                ui.label(RichText::new("Covers .............. local folder art, iTunes, Deezer, Cover Art Archive, TheAudioDB; one lookup per album").color(dim).monospace());
                ui.label(RichText::new("Song fallback ....... If the album is unknown, search by artist + song title (iTunes/Deezer)").color(dim).monospace());
                ui.label(RichText::new("Ratings ............. MusicBrainz community 1-5 stars; current album auto-fetched; album fallback").color(dim).monospace());
                ui.label(RichText::new("Your stars .......... Clicking stars sets a manual rating that Meta All never overwrites").color(dim).monospace());
                ui.label(RichText::new("Metadata repair ..... Missing title/artist/album and track-number-only tags are filled from the filename/folder").color(dim).monospace());
                ui.label(RichText::new("Not everything ...... Songs with no MusicBrainz rating or no available art stay unrated/art-less; final status reports counts").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DVD / BLU-RAY").color(acc).monospace().strong());
                ui.label(RichText::new("Play ................ Drive or ISO; English audio preferred; subtitles off; fullscreen player with seek bar and STOP").color(dim).monospace());
                ui.label(RichText::new("DVD seeking ......... Software MPEG-2 decode + CPU filters; metadata probed once so seeks do not thrash the drive").color(dim).monospace());
                ui.label(RichText::new("Blu-ray ............. VAAPI hardware decode + GPU scale/pad/RGBA; libbluray streamer gives real time seeking").color(dim).monospace());
                ui.label(RichText::new("AACS / BD+ .......... Deck shows Disc ID, AACS key/KEYDB match and BD+ status; CHECK AACS re-reads the disc").color(dim).monospace());
                ui.label(RichText::new("DVD tools ........... libdvdread + libdvdnav + libdvdcss; CachyOS/Arch: sudo pacman -S libdvdread libdvdnav libdvdcss").color(dim).monospace());
                ui.label(RichText::new("DVD tools (Debian) .. libdvdread4 + libdvdnav4 + libdvdcss2; sudo apt install libdvdread4 libdvdnav4 libdvdcss2").color(dim).monospace());
                ui.label(RichText::new("Dump tools .......... dvdbackup (bundled CachyOS build) + genisoimage; MakeMKV is used for MKV and as ISO fallback").color(dim).monospace());
                ui.label(RichText::new("DUMP TO MKV ........ Decrypt and remux every title to clean MKV files (MakeMKV); works from the open image or the physical disc").color(dim).monospace());
                ui.label(RichText::new("DUMP TO ISO (DVD) ... dvdbackup + genisoimage (or MakeMKV backup --decrypt) to a clean, playable ISO").color(dim).monospace());
                ui.label(RichText::new("DUMP TO ISO (BD) .... Full disc with menus/extras: MakeMKV backup --decrypt removes AACS/BD+, then a UDF ISO is built").color(dim).monospace());
                ui.label(RichText::new("Dump space .......... ISO dumps temporarily need about 2.3x the disc size (decrypted tree + ISO); it is removed after building").color(dim).monospace());
                ui.label(RichText::new("No encryption ....... Every video output is decrypted; raw encrypted images are never produced or offered").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("CD MODE").color(acc).monospace().strong());
                ui.label(RichText::new("Insert audio CD ..... Detect tracks/TOC; rip whole disc losslessly to FLAC with MusicBrainz tags and cover art").color(dim).monospace());
                ui.label(RichText::new("Eject ............... Open the tray when the rip is done").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("GRAPHICS / HARDWARE").color(acc).monospace().strong());
                ui.label(RichText::new("AMD / Intel ......... VAAPI hardware decode (auto-detected from /sys/class/drm render node)").color(dim).monospace());
                ui.label(RichText::new("NVIDIA .............. CUDA/NVDEC when the proprietary driver is present").color(dim).monospace());
                ui.label(RichText::new("Disable ............. MPDOOM_NO_HWACCEL=1 forces software decoding").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("TROUBLESHOOTING").color(acc).monospace().strong());
                ui.label(RichText::new("DVD blocks .......... Encrypted image? Use DUMP TO MKV from the image or DUMP TO ISO; raw encrypted images are unsupported").color(dim).monospace());
                ui.label(RichText::new("Blu-ray BD+ ......... MakeMKV handles BD+ and AACS; DUMP TO ISO is the fallback when libaacs has no matching key").color(dim).monospace());
                ui.label(RichText::new("Blu-ray BD+ ......... MakeMKV handles BD+ and AACS; DUMP TO ISO is the fallback for discs where libaacs has no key").color(dim).monospace());
                ui.label(RichText::new("Check DVD tools ..... Run `mediaplayerofdoom --check-dvd` to list dvdbackup, genisoimage and the DVD libraries").color(dim).monospace());
                ui.label(RichText::new("Ratings missing ..... MusicBrainz has no community rating for that track/album; no public star feed covers everything").color(dim).monospace());
                ui.label(RichText::new("Covers missing ...... No local/web match; Meta All retries every run and reports the count without art").color(dim).monospace());
                ui.label(RichText::new("Dump slow ........... Optical CAV/riplock; check the live MB/s and ETA; dirty discs are slower").color(dim).monospace());
                ui.add_space(8.0);

                ui.label(RichText::new("KEYBOARD SHORTCUTS").color(acc).monospace().strong());
                ui.label(RichText::new("SPACE .............. Play / Pause").color(g).monospace());
                ui.label(RichText::new("LEFT ................ Previous Track").color(g).monospace());
                ui.label(RichText::new("RIGHT ............... Skip / Next Track").color(g).monospace());
                ui.label(RichText::new("UP .................. Volume Up").color(g).monospace());
                ui.label(RichText::new("DOWN ................ Volume Down").color(g).monospace());
                ui.label(RichText::new("+  /  - ............. Volume Buttons").color(g).monospace());
                ui.label(RichText::new("S ................... Smart Shuffle (toggle)").color(g).monospace());
                ui.label(RichText::new("T ................... Cycle Theme").color(g).monospace());
                ui.label(RichText::new("F ................... Toggle Fullscreen").color(g).monospace());
                ui.label(RichText::new("M ................... Mute / Unmute").color(g).monospace());
                ui.label(RichText::new("L ................... Toggle Lyrics").color(g).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DECK CONTROLS (TAPE MODE)").color(acc).monospace().strong());
                ui.label(RichText::new("PLAY ................ Start / Pause playback").color(dim).monospace());
                ui.label(RichText::new("STOP ................ Stop playback").color(dim).monospace());
                ui.label(RichText::new("REW ................. Press & hold to rewind").color(dim).monospace());
                ui.label(RichText::new("FWD ................. Press & hold to fast-forward").color(dim).monospace());
                ui.label(RichText::new("SKIP ................ Next track").color(dim).monospace());
                ui.label(RichText::new("PREV ................ Previous track").color(dim).monospace());
                ui.label(RichText::new("REC ................. Record to file").color(dim).monospace());
                ui.label(RichText::new("EJECT ............... Insert / Eject tape").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DECK CONTROLS (CD MODE)").color(acc).monospace().strong());
                ui.label(RichText::new("INSERT DISC ......... Load music from folder").color(dim).monospace());
                ui.label(RichText::new("EJECT DISC .......... Remove disc").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DECK CONTROLS (RADIO MODE)").color(acc).monospace().strong());
                ui.label(RichText::new("FM / AM ............. Switch band").color(dim).monospace());
                ui.label(RichText::new("TUNE- / TUNE+ ........ Step frequency").color(dim).monospace());
                ui.label(RichText::new("SCAN ................ Auto-scan for stations").color(dim).monospace());
                ui.label(RichText::new("+ ADD ............... Add a station (name / band / freq / URL)").color(dim).monospace());
                ui.label(RichText::new("Click dial .......... Tune to frequency").color(dim).monospace());
                ui.label(RichText::new("Drag dial ........... Sweep, auto-locks nearest station").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("TUNE SLIDER").color(acc).monospace().strong());
                ui.label(RichText::new("Drag seek bar to scrub through the current song").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("PLAYLIST").color(acc).monospace().strong());
                ui.label(RichText::new("Click row ........... Play selected song").color(dim).monospace());
                ui.label(RichText::new("Drag & drop ......... Add files or folders").color(dim).monospace());
                ui.label(RichText::new("Band buttons ......... Filter by artist").color(dim).monospace());
                ui.label(RichText::new("ALL .................. Show all songs (A-Z)").color(dim).monospace());
                ui.label(RichText::new("Search bar ........... Filter playlist").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("SYSTEM TRAY / HIDE").color(acc).monospace().strong());
                ui.label(RichText::new("Hide ................ Right-click the tray icon -> Hide; removes the taskbar entry but keeps the tray icon, audio and auto-advance running").color(dim).monospace());
                ui.label(RichText::new("Show ................ Right-click the tray icon -> Show restores and activates the window; works while the UI is hidden").color(dim).monospace());
                ui.label(RichText::new("Now playing ......... While hidden, a desktop notification pops up on each new track (artist, title, album)").color(dim).monospace());
                ui.label(RichText::new("Quit ................ Tray Quit works while hidden; if the UI cannot process Close it force-exits after 2 seconds").color(dim).monospace());
                ui.label(RichText::new("KDE Wayland ......... Hide uses KWin scripting (skipTaskbar + minimize); other desktops fall back to minimize").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("DEFAULT PLAYER").color(acc).monospace().strong());
                ui.label(RichText::new("Set default ......... Register MediaPlayerDOOM for audio and video files with xdg-mime; no config editing needed").color(dim).monospace());
                if retro_btn(ui, "SET AS DEFAULT MEDIA PLAYER", th.playing_fg, th.btn_bg)
                    .on_hover_text("Set MediaPlayerDOOM as the default handler for supported audio and video file types")
                    .clicked()
                {
                    match crate::desktop::set_default_player() {
                        Ok(n) => self.set_status(format!("Default media player set for {} file types", n)),
                        Err(e) => self.set_error(format!("Could not set default player: {}", e)),
                    }
                }
                ui.add_space(6.0);

                ui.label(RichText::new("EXTRAS").color(acc).monospace().strong());
                ui.label(RichText::new("Sleep Timer .......... Auto-close after time").color(dim).monospace());
                ui.label(RichText::new("Look Up ............. Open song folder in explorer").color(dim).monospace());
                ui.label(RichText::new("EQ .................. 20 professional curves: Harman, Loudness, Broadcast, Podcast, Vocal Air, Acoustic, Jazz, Classical, Rock, Metal, Hip-Hop, EDM, De-Mud, Dialogue").color(dim).monospace());
                ui.label(RichText::new("Visualizer .......... Beat-locked 12-band spectrum and twin woofers; kick + spectral-flux onset detector, phase-locked BPM pulse").color(dim).monospace());
                ui.label(RichText::new("EQ bands ............ ISO 1-octave bands 31 Hz-16 kHz, Q 1.4; auto preamp + soft limiter; preset picker, arrow cycling and FLAT reset").color(dim).monospace());
                ui.label(RichText::new("EQ graph ............ Real response, ±12 dB grid, band points, curve description and preamp percentage; all changes apply live").color(dim).monospace());
                ui.add_space(6.0);

                ui.label(RichText::new("YOUTUBE DOWNLOAD").color(acc).monospace().strong());
                ui.label(RichText::new("Progress ............ Live bar in the YT row, download queue and bottom status bar: percentage, stage, speed, ETA and item count").color(dim).monospace());
                ui.label(RichText::new("Stages .............. lookup -> download -> mp3 transcode/tagging -> save; the bar never jumps to 100% before the file is complete").color(dim).monospace());
                ui.add_space(6.0);
                ui.label(RichText::new("YouTube Download ..... Paste URL in status bar").color(dim).monospace());
                ui.label(RichText::new("Convert .............. Convert a folder of music to another format").color(dim).monospace());
                ui.label(RichText::new("Video Player ......... Double-click video in playlist").color(dim).monospace());
                ui.add_space(8.0);

                if retro_btn(ui, "CLOSE", Color32::from_rgb(0, 255, 0), th.btn_bg).clicked() {
                    self.show_help = false;
                }
                    });
            });
    }

    pub(crate) fn show_add_radio_window(&mut self, ctx: &egui::Context) {
        let th = self.theme_colors();
        let mut open = self.show_add_radio;
        egui::Window::new("ADD STATION")
            .resizable(false)
            .collapsible(false)
            .default_width(400.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("NAME:").color(th.fg).monospace().size(11.0));
                    ui.add(egui::TextEdit::singleline(&mut self.add_name).desired_width(240.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("BAND:").color(th.fg).monospace().size(11.0));
                    if retro_btn(ui, "FM", if self.add_band == "FM" { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() {
                        self.add_band = "FM".to_string();
                    }
                    if retro_btn(ui, "AM", if self.add_band == "AM" { th.playing_fg } else { th.btn_fg }, th.btn_bg).clicked() {
                        self.add_band = "AM".to_string();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("FREQ:").color(th.fg).monospace().size(11.0));
                    ui.add(egui::TextEdit::singleline(&mut self.add_freq).hint_text(if self.add_band == "AM" { "e.g. 1120" } else { "e.g. 98.5" }).desired_width(140.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("URL:").color(th.fg).monospace().size(11.0));
                    ui.add(egui::TextEdit::singleline(&mut self.add_url).hint_text("https:// or http:// audio stream").desired_width(300.0).font(egui::TextStyle::Monospace).text_color(th.playing_fg));
                });
                ui.add_space(6.0);
                let (lo, hi) = if self.add_band == "AM" { (530.0, 1700.0) } else { (87.9, 107.9) };
                ui.label(RichText::new(format!("{} RANGE: {} - {}", self.add_band, lo, hi)).color(Color32::from_gray(110)).monospace().size(9.0));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if retro_btn(ui, "SAVE", th.playing_fg, th.btn_bg).clicked() {
                        let name = self.add_name.trim().to_string();
                        let url = self.add_url.trim().to_string();
                        let freq = self.add_freq.trim().parse::<f32>();
                        if !name.is_empty() && !url.is_empty() {
                            if let Ok(f) = freq {
                                if f >= lo && f <= hi {
                                    let band = self.add_band.clone();
                                    self.radio_presets.push(RadioStation { name: name.clone(), band: band.clone(), freq: f, url: url.clone() });
                                    save_radio(&self.radio_presets);
                                    let idx = self.radio_presets.len() - 1;
                                    self.radio_band = band;
                                    self.radio_freq = f;
                                    self.radio_last_moved = ui.input(|i| i.time);
                                    self.start_radio(idx);
                                    self.add_name.clear();
                                    self.add_freq.clear();
                                    self.add_url.clear();
                                    self.show_add_radio = false;
                                    self.set_status(format!("Station added: {} @ {}", name, fmt_freq(f)));
                                } else {
                                    self.set_error(format!("Frequency must be in {} range {} - {}", self.add_band, lo, hi));
                                }
                            } else {
                                self.set_error("Frequency must be a number");
                            }
                        } else {
                            self.set_error("Station needs a name and an audio stream URL");
                        }
                    }
                    if retro_btn(ui, "CANCEL", th.btn_fg, th.btn_bg).clicked() {
                        self.show_add_radio = false;
                    }
                });
            });
        self.show_add_radio = open;
    }

    pub(crate) fn ui_lyrics(&mut self) {
        let ctx = self.ctx.clone();
        let max_h: f32 = (ctx.viewport_rect().height() * 0.7).clamp(120.0, 900.0);
        egui::Window::new(format!("Lyrics - {}", self.lyrics_title))
            .collapsible(false)
            .default_size([520.0, 440.0])
            .show(&ctx, |ui| {
                let th = self.theme_colors();
                ui.horizontal(|ui| {
                    if retro_btn(ui, "CLOSE", Color32::from_rgb(0, 255, 0), th.btn_bg).clicked() {
                        self.lyrics = None;
                    }
                    ui.label(RichText::new(&self.lyrics_title).color(th.accent).monospace().size(12.0));
                });
                ui.add_space(4.0);
                if let Some(lyr) = self.lyrics.clone() {
                    egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                        ui.label(RichText::new(lyr).color(Color32::from_rgb(0x39, 0xFF, 0x14)).monospace());
                    });
                }
            });
    }

    pub(crate) fn fetch_lyrics(&mut self) {
        if let Some(s) = self.state.current_song.clone() {
            let (mut artist, mut title) = (self.meta.1.clone(), self.meta.0.clone());
            if let Some(t) = strip_track_no(&title) {
                title = t;
            }
            if artist.is_empty() || artist == "Unknown" {
                if let Some((a, _)) = folder_artist_album(&s) {
                    artist = a;
                } else if let Some(idx) = title.find(" - ") {
                    let a = title[..idx].trim();
                    let t = title[idx + 3..].trim();
                    if !a.is_empty() && !t.is_empty() {
                        artist = a.to_string();
                        title = t.to_string();
                    }
                }
            }
            self.lyrics_for = Some(s.clone());
            self.set_status("Fetching lyrics...");
            let _ = self.net_tx.send(NetCmd::Lyrics(artist, title));
            let _ = s;
        }
    }
}
