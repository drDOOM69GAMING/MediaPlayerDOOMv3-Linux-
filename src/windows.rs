#![allow(unused_imports)]
#![allow(dead_code)]
// Tray icon, window/taskbar icon, hotkeys.
use std::error::Error;
use std::path::Path;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use eframe::egui;
use crate::consts::APP_NAME;
use crate::log::av_log;
use crate::util::data_dir;
pub fn open_in_explorer(path: &str) {
    let dir = Path::new(path).parent().unwrap_or(Path::new("."));
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe").arg(dir.to_string_lossy().as_ref()).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(dir).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
}

// ---------------------------------------------------------------------------
// System tray
//
// Windows keeps the native tray-icon crate. Linux uses ksni (pure Rust
// StatusNotifierItem over D-Bus), which works on KDE/GNOME/Wayland without
// pulling in GTK, libappindicator or libxdo.
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod linux_tray {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex, OnceLock};
    use ksni::blocking::{Handle, TrayMethods};

    pub fn queue() -> &'static Arc<Mutex<VecDeque<String>>> {
        static Q: OnceLock<Arc<Mutex<VecDeque<String>>>> = OnceLock::new();
        Q.get_or_init(|| Arc::new(Mutex::new(VecDeque::new())))
    }

    fn push(id: &str) {
        if let Ok(mut q) = queue().lock() {
            q.push_back(id.to_string());
        }
    }

    fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            out.extend_from_slice(&[px[3], px[0], px[1], px[2]]);
        }
        out
    }

    pub struct LinuxTray {
        icons: Vec<ksni::Icon>,
    }

    impl ksni::Tray for LinuxTray {
        fn id(&self) -> String {
            "mediaplayerofdoom".into()
        }
        fn title(&self) -> String {
            APP_NAME.into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icons.clone()
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            push("show");
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::{MenuItem, StandardItem};
            let item = |label: &str, id: &str| -> MenuItem<Self> {
                let id = id.to_string();
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |_t: &mut Self| push(&id)),
                    ..Default::default()
                }
                .into()
            };
            vec![
                item("Show", "show"),
                MenuItem::Separator,
                item("Play/Pause", "play_pause"),
                item("Skip", "skip"),
                item("Previous", "prev"),
                item("Stop", "stop"),
                MenuItem::Separator,
                item("Quit", "quit"),
            ]
        }
    }

    pub struct PlatformTray {
        _handle: Handle<LinuxTray>,
    }

    pub fn make_tray() -> Result<PlatformTray, Box<dyn Error + Send + Sync>> {
        let mut icons = Vec::new();
        for size in [24u32, 32, 48, 64] {
            if let Some((rgba, w, h)) = app_icon_rgba(size) {
                icons.push(ksni::Icon { width: w as i32, height: h as i32, data: rgba_to_argb(&rgba) });
            }
        }
        let tray = LinuxTray { icons };
        // assume_sni_available: start quietly even if Plasma's watcher is not
        // up yet (e.g. autostart racing the desktop).
        let handle = tray
            .assume_sni_available(true)
            .spawn()
            .map_err(|e| -> Box<dyn Error + Send + Sync> { format!("tray: {e}").into() })?;
        Ok(PlatformTray { _handle: handle })
    }
}

#[cfg(windows)]
mod win_tray {
    use super::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    pub struct PlatformTray {
        _tray: TrayIcon,
    }

    pub fn make_tray() -> Result<PlatformTray, Box<dyn Error + Send + Sync>> {
        let menu = Menu::new();
        let show = MenuItem::with_id(MenuId::new("show"), "Show", true, None);
        let playpause = MenuItem::with_id(MenuId::new("play_pause"), "Play/Pause", true, None);
        let skip = MenuItem::with_id(MenuId::new("skip"), "Skip", true, None);
        let prev = MenuItem::with_id(MenuId::new("prev"), "Previous", true, None);
        let stop_item = MenuItem::with_id(MenuId::new("stop"), "Stop", true, None);
        let quit = MenuItem::with_id(MenuId::new("quit"), "Quit", true, None);
        let sep = PredefinedMenuItem::separator();
        menu.append_items(&[&show, &playpause, &skip, &prev, &stop_item, &sep, &quit])?;
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(APP_NAME)
            .with_icon(build_icon())
            .build()?;
        Ok(PlatformTray { _tray: tray })
    }

    pub fn poll_menu() -> Option<String> {
        MenuEvent::receiver().try_recv().ok().map(|e| e.id.0)
    }
}

#[cfg(target_os = "linux")]
pub use linux_tray::{make_tray, PlatformTray};
#[cfg(windows)]
pub use win_tray::{make_tray, PlatformTray};

/// Fallback for platforms without a tray implementation.
#[cfg(not(any(windows, target_os = "linux")))]
pub struct PlatformTray;
#[cfg(not(any(windows, target_os = "linux")))]
pub fn make_tray() -> Result<PlatformTray, Box<dyn Error + Send + Sync>> {
    Err("system tray is not supported on this platform".into())
}

/// Next tray menu action, if the user clicked one. Menu ids match the ones
/// handled by `PlayerApp::poll_ext`.
pub fn poll_tray_menu() -> Option<String> {
    #[cfg(windows)]
    {
        win_tray::poll_menu()
    }
    #[cfg(target_os = "linux")]
    {
        linux_tray::queue().lock().ok().and_then(|mut q| q.pop_front())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

pub fn app_icon_rgba(size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let bits = include_bytes!("../assets/app_icon.jpg");
    let img = image::load_from_memory(bits).ok()?;
    // The source art is 16:9, but every consumer (window icon, tray icon,
    // taskbar icon) needs a square RGBA buffer whose data length matches the
    // reported dimensions. Center-crop to a square first; the old code
    // thumbnailed to fit inside `size` and then claimed `size x size`, which
    // produced a truncated buffer and silently broke icons on every platform.
    let (w, h) = (img.width(), img.height());
    let side = w.min(h);
    let img = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let img = img.thumbnail(size, size);
    let (out_w, out_h) = (img.width(), img.height());
    let rgba = img.to_rgba8().into_raw();
    Some((rgba, out_w, out_h))
}

pub fn window_icon_data() -> Option<egui::IconData> {
    if let Some((rgba, w, h)) = app_icon_rgba(256) {
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

#[cfg(windows)]
pub fn build_icon() -> tray_icon::Icon {
    if let Some((rgba, w, h)) = app_icon_rgba(32) {
        if let Ok(i) = tray_icon::Icon::from_rgba(rgba, w, h) {
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
    tray_icon::Icon::from_rgba(rgba, 32, 32).unwrap()
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

static HOTKEY_ACTIONS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<u32, &'static str>>,
> = std::sync::OnceLock::new();

fn hotkey_actions() -> &'static std::sync::Mutex<std::collections::HashMap<u32, &'static str>> {
    HOTKEY_ACTIONS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Map a registered global-hotkey id to an action name.
///
/// `global-hotkey` only reports numeric ids in its event stream, so the
/// registration site records what each id means. Media keys are registered on
/// a best-effort basis: another app (or the desktop) may already own them.
pub fn hotkey_action(id: u32) -> Option<&'static str> {
    hotkey_actions().lock().ok().and_then(|m| m.get(&id).copied())
}

pub fn make_hotkeys() -> Option<HotkeyInfo> {
    let hkm = GlobalHotKeyManager::new().ok()?;
    let mut actions: std::collections::HashMap<u32, &'static str> = std::collections::HashMap::new();
    let mut register = |hk: HotKey, action: &'static str| -> Option<u32> {
        if hkm.register(hk).is_ok() {
            let id = hk.id();
            actions.insert(id, action);
            Some(id)
        } else {
            None
        }
    };
    let show_id = register(
        HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyM),
        "show",
    );
    let quit_id = register(
        HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyQ),
        "quit",
    );
    // Best-effort media keys. On Wayland the desktop usually owns these; the
    // in-app keyboard shortcuts still work there.
    let _ = register(HotKey::new(None, Code::MediaPlayPause), "play_pause");
    let _ = register(HotKey::new(None, Code::MediaTrackNext), "skip");
    let _ = register(HotKey::new(None, Code::MediaTrackPrevious), "prev");
    let _ = register(HotKey::new(None, Code::MediaStop), "stop");
    if let Ok(mut m) = hotkey_actions().lock() {
        m.extend(actions);
    }
    let show_id = show_id.unwrap_or(0);
    let quit_id = quit_id.unwrap_or(0);
    Some((hkm, show_id, quit_id))
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    #[test]
    fn app_icon_buffer_matches_reported_size() {
        for size in [16u32, 32, 64, 256] {
            let (rgba, w, h) = app_icon_rgba(size).expect("embedded icon should decode");
            assert_eq!(w, h, "icons must be square");
            assert_eq!(rgba.len(), (w * h * 4) as usize, "buffer size must match {w}x{h}");
        }
    }

    #[test]
    fn window_icon_is_valid() {
        let icon = window_icon_data().expect("window icon");
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
