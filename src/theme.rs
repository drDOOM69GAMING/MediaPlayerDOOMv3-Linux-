#![allow(unused_imports)]
#![allow(dead_code)]
// Color themes and palette helpers.
use eframe::egui;
use eframe::egui::{Color32, Pos2, Rect, Stroke, Vec2};
pub struct ThemePalette {
    pub bg: Color32,
    pub fg: Color32,
    pub accent: Color32,
    pub highlight: Color32,
    pub btn_bg: Color32,
    pub btn_fg: Color32,
    pub playing_fg: Color32,
    pub art_bg: Color32,
}

pub fn hex(c: &str) -> Color32 {
    let h = c.trim_start_matches('#');
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0);
    Color32::from_rgb(r, g, b)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t).round() as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t).round() as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t).round() as u8,
        a.a(),
    )
}

pub fn lighten(c: Color32, f: f32) -> Color32 {
    mix(c, Color32::from_rgb(255, 255, 255), f)
}

pub fn darken(c: Color32, f: f32) -> Color32 {
    mix(c, Color32::from_rgb(0, 0, 0), f)
}

pub fn paint_title_gradient(painter: &egui::Painter, rect: egui::Rect, accent: Color32, bg: Color32) {
    let top = darken(accent, 0.55);
    let deep = darken(accent, 0.82);
    let bands = 14;
    let mut prev_y = rect.min.y;
    for i in 0..bands {
        let t = i as f32 / bands as f32;
        let c = mix(mix(top, deep, t), bg, t * 0.85);
        let y = rect.min.y + (rect.max.y - rect.min.y) * t;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(rect.min.x, prev_y), egui::pos2(rect.max.x, y)),
            0.0,
            c,
        );
        prev_y = y;
    }
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x, rect.max.y - 1.0),
            egui::pos2(rect.max.x, rect.max.y),
        ),
        0.0,
        accent,
    );
}

pub fn palette(name: &str) -> ThemePalette {
    match name {
        "Winamp" => ThemePalette {
            bg: hex("#0b0b0b"), fg: hex("#c8c8c8"), accent: hex("#1ce0ff"), highlight: hex("#1a2525"),
            btn_bg: hex("#565656"), btn_fg: hex("#e8e8e8"), playing_fg: hex("#8cff1a"), art_bg: hex("#000000"),
        },
        "Cyberpunk" => ThemePalette {
            bg: hex("#0d0221"), fg: hex("#ff2a6d"), accent: hex("#05d9e8"), highlight: hex("#1a1a3a"),
            btn_bg: hex("#1a0a2e"), btn_fg: hex("#ff2a6d"), playing_fg: hex("#05d9e8"), art_bg: hex("#1a0030"),
        },
        "Amber" => ThemePalette {
            bg: hex("#1a1400"), fg: hex("#ffb000"), accent: hex("#cc8800"), highlight: hex("#332200"),
            btn_bg: hex("#2a2000"), btn_fg: hex("#ffb000"), playing_fg: hex("#ffdd00"), art_bg: hex("#221100"),
        },
        "Ocean" => ThemePalette {
            bg: hex("#0a1628"), fg: hex("#00d4ff"), accent: hex("#0088aa"), highlight: hex("#0f2844"),
            btn_bg: hex("#122a4a"), btn_fg: hex("#00d4ff"), playing_fg: hex("#00ffff"), art_bg: hex("#061020"),
        },
        _ => ThemePalette {
            bg: hex("#1e1e1e"), fg: hex("#00ff00"), accent: hex("#00aa00"), highlight: hex("#003300"),
            btn_bg: hex("#2a2a2a"), btn_fg: hex("#00ff00"), playing_fg: hex("#00ff00"), art_bg: hex("#002200"),
        },
    }
}

