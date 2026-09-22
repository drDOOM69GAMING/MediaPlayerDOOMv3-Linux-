#![allow(unused_imports)]
#![allow(dead_code)]
// Deck/tape-deck painting widgets.
use eframe::egui;
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, RichText, Stroke, Ui, Vec2};
use crate::theme::{darken, hex, lighten, mix, ThemePalette};
pub fn retro_btn(ui: &mut egui::Ui, label: &str, fg: Color32, base: Color32) -> egui::Response {
    let w = label.chars().count() as f32 * 7.0 + 24.0;
    let h = 20.0;
    retro_btn_ex(ui, label, fg, base, w, h)
}

pub fn retro_btn_ex(ui: &mut egui::Ui, label: &str, fg: Color32, base: Color32, w: f32, h: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        let hovered = resp.hovered();
        let pressed = resp.is_pointer_button_down_on();
        let off = if pressed { 1.0 } else { 0.0 };
        let r = rect.shrink(0.5).translate(egui::vec2(off, off));
        let rr = 2.0;
        let top = mix(base, Color32::from_rgb(255, 255, 255), if pressed { 0.24 } else if hovered { 0.42 } else { 0.32 });
        let bot = darken(base, if pressed { 0.50 } else { 0.22 });
        painter.rect_filled(r, rr, bot);
        let hh = r.height();
        painter.rect_filled(
            egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + hh * 0.42)),
            rr,
            top,
        );
        let fr = r.shrink(2.0);
        painter.rect_filled(fr, 1.5, if pressed { darken(base, 0.08) } else { mix(base, Color32::BLACK, 0.13) });
        painter.line_segment(
            [fr.left_top(), fr.right_top()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.45) }),
        );
        painter.line_segment(
            [fr.left_top(), fr.left_bottom()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.3) }),
        );
        painter.line_segment(
            [fr.right_top(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.72)),
        );
        painter.line_segment(
            [fr.left_bottom(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.78)),
        );
        if hovered && !pressed {
            painter.rect_stroke(fr, 1.5, egui::Stroke::new(1.0, lighten(base, 0.2)), egui::StrokeKind::Inside);
        }
        painter.text(
            fr.center() + egui::vec2(0.0, off),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(if h > 26.0 { 11.5 } else { 10.0 }),
            fg,
        );
    }
    resp
}

pub fn deck_key(ui: &mut egui::Ui, label: &str, fg: Color32, base: Color32) -> egui::Response {
    let w = (label.chars().count() as f32 * 6.4 + 20.0).max(40.0);
    let h = 40.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        let pressed = resp.is_pointer_button_down_on();
        let hovered = resp.hovered();
        let off = if pressed { 1.0 } else { 0.0 };
        let r = rect.shrink(1.0).translate(egui::vec2(off, off));
        let top = mix(base, Color32::from_rgb(255, 255, 255), if pressed { 0.24 } else { 0.40 });
        let bot = darken(base, if pressed { 0.50 } else { 0.22 });
        let rr = 2.0;
        painter.rect_filled(r, rr, bot);
        let hh = r.height();
        painter.rect_filled(
            egui::Rect::from_min_max(r.min, egui::pos2(r.max.x, r.min.y + hh * 0.42)),
            rr,
            top,
        );
        let fr = r.shrink(3.0);
        painter.rect_filled(fr, 1.5, if pressed { darken(base, 0.08) } else { mix(base, Color32::BLACK, 0.14) });
        painter.line_segment(
            [fr.left_top(), fr.right_top()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.45) }),
        );
        painter.line_segment(
            [fr.left_top(), fr.left_bottom()],
            egui::Stroke::new(1.0, if pressed { darken(base, 0.6) } else { lighten(base, 0.3) }),
        );
        painter.line_segment(
            [fr.right_top(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.72)),
        );
        painter.line_segment(
            [fr.left_bottom(), fr.right_bottom()],
            egui::Stroke::new(1.0, darken(base, 0.78)),
        );
        if hovered && !pressed {
            painter.rect_stroke(fr, 1.5, egui::Stroke::new(1.0, lighten(base, 0.2)), egui::StrokeKind::Inside);
        }
        painter.text(
            fr.center() + egui::vec2(0.0, off),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(10.5),
            fg,
        );
    }
    resp
}

pub fn tape_reel_at(painter: &egui::Painter, c: egui::Pos2, r: f32, fg: Color32, base: Color32, ang: f32) {
    painter.circle_filled(c, r, base);
    painter.circle(c, r, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.62)));
    painter.circle(c, r * 0.80, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.5)));
    let step = std::f32::consts::TAU / 6.0;
    for k in 0..6 {
        let a = ang + k as f32 * step;
        let p1 = c + egui::vec2(a.cos() * r * 0.30, a.sin() * r * 0.30);
        let p2 = c + egui::vec2(a.cos() * r * 0.90, a.sin() * r * 0.90);
        painter.line_segment([p1, p2], egui::Stroke::new(1.0, fg));
    }
    painter.circle(c, r * 0.30, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.45)));
    painter.circle_filled(c, r * 0.16, fg);
}

pub fn cassette(
    ui: &mut egui::Ui,
    width: f32,
    height: f32,
    anim: f32,
    side: char,
    songs: u32,
    cap: u32,
    accent: Color32,
    base: Color32,
    playing: bool,
    wind: f32, // 0..1 how hard we're winding
    wind_dir: f32,
) {
    let (slot, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    painter.rect_filled(slot, 4.0, darken(base, 0.55));
    painter.rect_stroke(slot, 4.0, egui::Stroke::new(1.0, darken(base, 0.75)), egui::StrokeKind::Inside);
    painter.rect_filled(slot.shrink(3.0), 3.0, darken(base, 0.42));
    painter.line_segment(
        [slot.left_top() + egui::vec2(3.0, 5.0), slot.right_top() + egui::vec2(-3.0, 5.0)],
        egui::Stroke::new(1.0, lighten(base, 0.1)),
    );
    painter.line_segment(
        [slot.left_bottom() + egui::vec2(3.0, -5.0), slot.right_bottom() + egui::vec2(-3.0, -5.0)],
        egui::Stroke::new(1.0, lighten(base, 0.05)),
    );
    if anim >= 0.97 {
        painter.text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            "EJECT",
            egui::FontId::monospace(11.0),
            darken(accent, 0.2),
        );
        return;
    }
    let t = ui.ctx().input(|i| i.time) as f32;
    let tr = slot.translate(egui::vec2(0.0, anim * (height + 12.0)));
    let shell_top = mix(Color32::from_gray(215), Color32::from_gray(255), 0.30);
    let shell_bot = Color32::from_gray(150);
    let rr = 4.0;
    painter.rect_filled(tr, rr, shell_bot);
    let hh = tr.height();
    painter.rect_filled(
        egui::Rect::from_min_max(tr.min, egui::pos2(tr.max.x, tr.min.y + hh * 0.45)),
        rr,
        shell_top,
    );
    painter.rect_stroke(tr, rr, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Inside);
    let label = egui::Rect::from_min_max(
        tr.min + egui::vec2(8.0, 5.0),
        egui::pos2(tr.max.x - 8.0, tr.min.y + hh * 0.42),
    );
    painter.rect_filled(label, 2.0, Color32::from_gray(232));
    painter.rect_stroke(label, 2.0, egui::Stroke::new(1.0, Color32::from_gray(170)), egui::StrokeKind::Inside);
    painter.text(label.left_top() + egui::vec2(6.0, 3.0), egui::Align2::LEFT_TOP, "MPDOOM", egui::FontId::monospace(9.0), Color32::from_rgb(160, 30, 30));
    painter.text(label.right_bottom() + egui::vec2(-6.0, -3.0), egui::Align2::RIGHT_BOTTOM, format!("SIDE {}  {}/{}", side, songs, cap.min(99)), egui::FontId::monospace(9.0), Color32::from_gray(60));
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(tr.min.x + 8.0, tr.min.y + hh * 0.46),
            egui::pos2(tr.max.x - 8.0, tr.max.y - 6.0),
        ),
        3.0,
        Color32::from_gray(34),
    );
    let win = egui::Rect::from_min_max(
        egui::pos2(tr.min.x + 14.0, tr.min.y + hh * 0.50),
        egui::pos2(tr.max.x - 14.0, tr.max.y - 12.0),
    );
    painter.rect_filled(win, 2.0, Color32::from_rgb(20, 24, 28));
    let strand_c = if playing { accent } else { Color32::from_gray(120) };
    let strand_w = win.height() * 0.30;
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(win.min.x, win.center().y - strand_w * 0.5),
            egui::pos2(win.max.x, win.center().y + strand_w * 0.5),
        ),
        1.0,
        strand_c.linear_multiply(if playing { 0.85 } else { 0.35 }),
    );
    let reel_r = win.height() * 0.42;
    let lc = egui::pos2(win.min.x + reel_r + 4.0, win.center().y);
    let rc = egui::pos2(win.max.x - reel_r - 4.0, win.center().y);
    let base_speed = if playing { 6.0 } else { 0.0 };
    let l_speed = base_speed + wind * (if wind_dir < 0.0 { 14.0 } else { -3.0 });
    let r_speed = base_speed + wind * (if wind_dir > 0.0 { 14.0 } else { -3.0 });
    tape_reel_at(&painter, lc, reel_r, strand_c, Color32::from_rgb(52, 58, 64), t * l_speed);
    tape_reel_at(&painter, rc, reel_r, strand_c, Color32::from_rgb(52, 58, 64), t * r_speed);
    for s in [
        tr.min + egui::vec2(10.0, 8.0),
        egui::pos2(tr.max.x - 10.0, tr.min.y + 8.0),
        tr.min + egui::vec2(10.0, hh - 9.0),
        egui::pos2(tr.max.x - 10.0, tr.min.y + hh - 9.0),
    ] {
        painter.circle_filled(s, 2.2, Color32::from_gray(120));
        painter.circle_filled(s, 1.2, Color32::from_gray(60));
    }
}

pub fn rotary(ui: &mut egui::Ui, label: &str, val01: f32, fg: Color32, base: Color32) -> f32 {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(44.0, 46.0), egui::Sense::drag());
    let mut out = val01;
    if resp.dragged() {
        let dy = ui.input(|i| i.pointer.delta().y);
        out = (val01 - dy / 160.0).clamp(0.0, 1.0);
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(ui.clip_rect());
        let c = rect.center();
        let rad = 15.0;
        painter.circle_filled(c, rad + 2.0, darken(base, 0.75));
        painter.circle_filled(c, rad, mix(base, Color32::from_rgb(255, 255, 255), 0.16));
        painter.circle_filled(c, rad - 2.5, darken(base, 0.15));
        let ang = -2.3561945_f32 + val01 * 4.7123890_f32;
        let p1 = c + egui::vec2(ang.cos() * rad * 0.20, ang.sin() * rad * 0.20);
        let p2 = c + egui::vec2(ang.cos() * (rad - 4.0), ang.sin() * (rad - 4.0));
        painter.line_segment([p1, p2], egui::Stroke::new(3.0, fg));
        for k in 0..5 {
            let a = -2.3561945_f32 + k as f32 * 1.1780972_f32;
            let q = c + egui::vec2(a.cos() * (rad + 2.0), a.sin() * (rad + 2.0));
            let q2 = c + egui::vec2(a.cos() * (rad + 4.0), a.sin() * (rad + 4.0));
            painter.line_segment([q, q2], egui::Stroke::new(1.0, darken(base, 0.6)));
        }
        painter.circle_filled(c, 2.0, fg);
        painter.text(
            egui::pos2(rect.center().x, rect.max.y - 9.5),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(8.5),
            fg,
        );
    }
    out
}

pub fn plaque(ui: &mut egui::Ui, width: f32, text: &str, fg: Color32, base: Color32) {
    let h = 15.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, h), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    painter.rect_filled(rect, 1.0, darken(base, 0.12));
    painter.rect_stroke(rect, 1.0, egui::Stroke::new(1.0, darken(base, 0.55)), egui::StrokeKind::Inside);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(9.0), fg);
}

pub fn grill_dots(painter: &egui::Painter, rect: egui::Rect, dot: Color32) {
    let step = 7.0;
    let mut x = rect.min.x + step / 2.0;
    while x < rect.max.x {
        let mut y = rect.min.y + step / 2.0;
        while y < rect.max.y {
            painter.circle_filled(egui::pos2(x, y), 1.4, dot);
            y += step;
        }
        x += step;
    }
}

pub fn speaker_woofer(ui: &mut egui::Ui, d: f32, base: Color32, accent: Color32, pulse: f32, kick: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(d, d), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let c0 = rect.center();
    let t = ui.input(|i| i.time) as f32;
    let rattle = (pulse - 0.55 + kick * 0.6).clamp(0.0, 1.0);
    let dx = ((t * 97.0).sin() * 0.5 + (t * 61.0).cos() * 0.5) * rattle * 2.6;
    let dy = pulse * pulse * 6.0;
    let c = c0 + egui::vec2(dx, dy);
    let r = d * 0.5 - 3.0;
    painter.circle_filled(c0, r + 2.0, darken(base, 0.6));
    painter.circle(c0, r + 2.0, Color32::TRANSPARENT, egui::Stroke::new(1.5, darken(base, 0.75)));
    painter.circle_filled(c0, r, darken(base, 0.42));
    painter.circle_filled(c0, r * 0.88, darken(base, 0.30));
    painter.circle(c0, r * 0.88, Color32::TRANSPARENT, egui::Stroke::new(1.0, lighten(base, 0.08)));
    painter.circle(c0, r * 0.80, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.5)));
    let surge = 1.0 + pulse * 0.45 + kick * 0.55;
    let flex = r * 0.80 * (1.18 + pulse * 0.34 + kick * 0.22);
    painter.circle(c, flex, Color32::TRANSPARENT, egui::Stroke::new(1.4, mix(base, Color32::BLACK, 0.55 - pulse * 0.35)));
    if pulse > 0.02 {
        painter.circle_filled(c, r * 0.95, accent.linear_multiply(0.05 + pulse * 0.22));
    }
    painter.circle_filled(c, r * 0.80, darken(base, 0.14));
    let cone_r = r * 0.80;
    for i in 1..=6 {
        let k = i as f32 / 6.0;
        let rr = cone_r * k * surge;
        let col = mix(base, Color32::BLACK, if i % 2 == 0 { 0.30 } else { 0.38 });
        painter.circle(c, rr.max(1.0), Color32::TRANSPARENT, egui::Stroke::new(1.0, col));
    }
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
        let p1 = c + egui::vec2(a.cos() * cone_r * 0.28, a.sin() * cone_r * 0.28);
        let p2 = c + egui::vec2(a.cos() * cone_r * 0.94 * surge, a.sin() * cone_r * 0.94 * surge);
        painter.line_segment([p1, p2], egui::Stroke::new(1.0, darken(base, 0.5)));
    }
    painter.circle_filled(c, cone_r * 0.30 * surge, lighten(base, 0.10 + pulse * 0.46));
    painter.circle(c, cone_r * 0.30 * surge, Color32::TRANSPARENT, egui::Stroke::new(1.0, darken(base, 0.5)));
    if pulse > 0.85 {
        painter.circle_filled(c, cone_r * 0.62, Color32::from_rgba_unmultiplied(255, 60, 60, (pulse * 90.0) as u8));
    }
    let glow = mix(accent, Color32::WHITE, 0.3);
    painter.line_segment(
        [
            c0 + egui::vec2(-r * 0.55, -r * 0.45),
            c0 + egui::vec2(-r * 0.15, -r * 0.62),
        ],
        egui::Stroke::new(1.5, glow.linear_multiply(0.25)),
    );
}

pub fn lcd(ui: &mut egui::Ui, text: &str, led: Color32, base: Color32) {
    let w = text.chars().count() as f32 * 9.2 + 18.0;
    let h = 24.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(ui.clip_rect());
    let inner = rect.shrink(1.0);
    painter.rect_filled(inner, 2.0, darken(base, 0.30));
    painter.line_segment(
        [inner.left_top(), inner.right_top()],
        egui::Stroke::new(1.0, darken(base, 0.72)),
    );
    painter.line_segment(
        [inner.left_top(), inner.left_bottom()],
        egui::Stroke::new(1.0, darken(base, 0.72)),
    );
    painter.line_segment(
        [inner.right_top(), inner.right_bottom()],
        egui::Stroke::new(1.0, lighten(base, 0.25)),
    );
    painter.line_segment(
        [inner.left_bottom(), inner.right_bottom()],
        egui::Stroke::new(1.0, lighten(base, 0.25)),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::monospace(15.0),
        led,
    );
}

pub fn engrave(painter: &egui::Painter, rect: egui::Rect, base: Color32) {
    let inner = rect.shrink(2.0);
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, darken(base, 0.72)), egui::StrokeKind::Inside);
    painter.rect_stroke(inner, 2.0, egui::Stroke::new(1.0, lighten(base, 0.2)), egui::StrokeKind::Inside);
}

pub fn print_sideways(painter: &egui::Painter, x: f32, y_center: f32, text: &str, size: f32, color: Color32) {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as f32;
    let line_h = size * 1.2;
    for (i, ch) in chars.iter().enumerate() {
        let y = y_center + (i as f32 - (n - 1.0) / 2.0) * line_h;
        painter.text(
            egui::pos2(x, y),
            egui::Align2::CENTER_CENTER,
            ch.to_string(),
            egui::FontId::monospace(size),
            color,
        );
    }
}

