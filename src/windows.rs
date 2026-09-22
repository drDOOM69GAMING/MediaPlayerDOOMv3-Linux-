#![allow(unused_imports)]
#![allow(dead_code)]
// Tray icon, window/taskbar icon, hotkeys.
use std::error::Error;
use std::path::Path;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use eframe::egui;
use windows_sys::Win32::Foundation::HWND;
use crate::consts::APP_NAME;
use crate::log::av_log;
use crate::util::data_dir;
pub fn open_in_explorer(path: &str) {
    let dir = Path::new(path).parent().unwrap_or(Path::new("."));
    let _ = std::process::Command::new("explorer.exe").arg(dir.to_string_lossy().as_ref()).spawn();
}

pub fn make_tray() -> Result<TrayIcon, Box<dyn std::error::Error + Send + Sync>> {
    let menu = Menu::new();
    let show = MenuItem::with_id(MenuId::new("show"), "Show", true, None);
    let playpause = MenuItem::with_id(MenuId::new("play_pause"), "Play/Pause", true, None);
    let skip = MenuItem::with_id(MenuId::new("skip"), "Skip", true, None);
    let prev = MenuItem::with_id(MenuId::new("prev"), "Previous", true, None);
    let stop_item = MenuItem::with_id(MenuId::new("stop"), "Stop", true, None);
    let quit = MenuItem::with_id(MenuId::new("quit"), "Quit", true, None);
    let sep = PredefinedMenuItem::separator();
    menu.append_items(&[
        &show, &playpause, &skip, &prev, &stop_item, &sep, &quit,
    ])?;
    let icon = build_icon();
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(APP_NAME)
        .with_icon(icon)
        .build()?;
    Ok(tray)
}

pub fn app_icon_rgba(size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let bits = include_bytes!("../assets/app_icon.jpg");
    let img = image::load_from_memory(bits).ok()?;
    let img = img.thumbnail(size, size);
    let rgba = img.to_rgba8().into_raw();
    Some((rgba, size, size))
}

pub fn window_icon_data() -> Option<egui::IconData> {
    if let Some((rgba, w, h)) = app_icon_rgba(64) {
        let data = egui::IconData { rgba, width: w, height: h };
        return Some(data);
    }
    None
}

#[cfg(target_os = "windows")]
pub fn apply_taskbar_icon(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIconFromResourceEx, GetSystemMetrics, LR_DEFAULTCOLOR, SendMessageW, HICON, ICON_BIG,
        ICON_SMALL, SM_CXICON, SM_CXSMICON, WM_SETICON,
    };

    fn make_hicon(source: &image::RgbaImage, size: i32) -> Option<HICON> {
        let img = image::imageops::resize(source, size as u32, size as u32, image::imageops::Lanczos3);
        let mut png = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut png);
        img.write_to(&mut cursor, image::ImageFormat::Png).ok()?;
        let icon = unsafe {
            CreateIconFromResourceEx(
                png.as_ptr(),
                png.len() as u32,
                1,
                0x0003_0000,
                size,
                size,
                LR_DEFAULTCOLOR,
            )
        };
        (!icon.is_null()).then_some(icon)
    }

    let Ok(handle) = cc.window_handle() else { return };
    let RawWindowHandle::Win32(hw) = handle.as_raw() else { return };
    let Some((rgba, w, h)) = app_icon_rgba(256) else { return };
    let Some(source) = image::RgbaImage::from_raw(w, h, rgba) else { return };

    let hwnd = hw.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    // SAFETY: hwnd is a live window; WM_SETICON just installs the icon handle.
    unsafe {
        if let Some(icon) = make_hicon(&source, GetSystemMetrics(SM_CXICON)) {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        }
        if let Some(icon) = make_hicon(&source, GetSystemMetrics(SM_CXSMICON)) {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        }
    }
}

pub fn build_icon_fallback_rgba() -> Vec<u8> {
    let mut rgba: Vec<u8> = vec![0; 32 * 32 * 4];
    let t1 = (9.0, 13.0);
    let t2 = (9.0, 25.0);
    let t3 = (24.0, 19.0);
    for y in 0..32 {
        for x in 0..32 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - 16.0;
            let dy = py - 16.0;
            let d = (dx * dx + dy * dy).sqrt();
            let i = ((y * 32 + x) * 4) as usize;
            if point_in_tri(px, py, t1, t2, t3) && d <= 14.5 {
                rgba[i] = 0; rgba[i + 1] = 255; rgba[i + 2] = 0; rgba[i + 3] = 255;
            } else if d <= 15.0 {
                rgba[i] = 42; rgba[i + 1] = 42; rgba[i + 2] = 42; rgba[i + 3] = 255;
            } else if d <= 16.5 {
                rgba[i] = 0; rgba[i + 1] = 170; rgba[i + 2] = 0; rgba[i + 3] = 255;
            } else {
                rgba[i] = 0; rgba[i + 1] = 0; rgba[i + 2] = 0; rgba[i + 3] = 0;
            }
        }
    }
    rgba
}

pub fn build_icon() -> Icon {
    if let Some((rgba, w, h)) = app_icon_rgba(32) {
        if let Ok(i) = Icon::from_rgba(rgba, w, h) {
            return i;
        }
    }
    let mut rgba: Vec<u8> = vec![0; 32 * 32 * 4];
    let p1 = (9.0, 13.0);
    let p2 = (9.0, 25.0);
    let p3 = (24.0, 19.0);
    for y in 0..32 {
        for x in 0..32 {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - 16.0;
            let dy = py - 16.0;
            let d = (dx * dx + dy * dy).sqrt();
            let i = ((y * 32 + x) * 4) as usize;
            let in_tri = point_in_tri(px, py, p1, p2, p3);
            if in_tri && d <= 14.5 {
                rgba[i] = 0; rgba[i + 1] = 255; rgba[i + 2] = 0; rgba[i + 3] = 255;
            } else if d <= 15.0 {
                rgba[i] = 42; rgba[i + 1] = 42; rgba[i + 2] = 42; rgba[i + 3] = 255;
            } else if d <= 16.5 {
                rgba[i] = 0; rgba[i + 1] = 170; rgba[i + 2] = 0; rgba[i + 3] = 255;
            } else {
                rgba[i] = 0; rgba[i + 1] = 0; rgba[i + 2] = 0; rgba[i + 3] = 0;
            }
        }
    }
    Icon::from_rgba(rgba, 32, 32).unwrap()
}

pub fn point_in_tri(px: f32, py: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let d1 = cross(px, py, a, b);
    let d2 = cross(px, py, b, c);
    let d3 = cross(px, py, c, a);
    let neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);
    !(neg && pos)
}

pub fn cross(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0)
}

pub type HotkeyInfo = (GlobalHotKeyManager, u32, u32);

pub fn make_hotkeys() -> Option<HotkeyInfo> {
    let hkm = GlobalHotKeyManager::new().ok()?;
    let m_hk = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM);
    let q_hk = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyQ);
    if hkm.register(m_hk).is_err() {
        return None;
    }
    if hkm.register(q_hk).is_err() {
        let _ = hkm.unregister(m_hk);
        return None;
    }
    Some((hkm, m_hk.id(), q_hk.id()))
}

