#![allow(unused_imports)]
#![allow(dead_code)]
// Tray icon, window/taskbar icon, hotkeys.
use std::error::Error;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use eframe::egui;
use lofty::prelude::*;
use crate::consts::APP_NAME;
use crate::log::av_log;
use crate::util::data_dir;

/// Set by PlayerApp so the tray menu can show/hide the window even after the
/// UI event loop has stopped painting a hidden window.
static TRAY_CTX: OnceLock<egui::Context> = OnceLock::new();
static TRAY_HIDDEN: OnceLock<Arc<AtomicBool>> = OnceLock::new();
static TRAY_CONTROL: OnceLock<Arc<std::sync::Mutex<std::collections::VecDeque<String>>>> =
    OnceLock::new();

pub fn set_tray_context(
    ctx: egui::Context,
    hidden: Arc<AtomicBool>,
    control: Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
) {
    let _ = TRAY_CTX.set(ctx);
    let _ = TRAY_HIDDEN.set(hidden);
    let _ = TRAY_CONTROL.set(control);
}

/// Wayland cannot hide a window with winit (`set_visible` is a no-op), so use
/// KDE's own scripting API to set `skipTaskbar` and minimize. Returns true only
/// when KWin accepted the script, so the UI never claims Hide worked when it
/// did not.
fn kwin_toggle(hide: bool) -> bool {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = hide;
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let qdbus = if crate::tools::which("qdbus6").is_some() {
            "qdbus6"
        } else if crate::tools::which("qdbus").is_some() {
            "qdbus"
        } else {
            return false;
        };
        let script = format!(
            r#"var wins = workspace.windowList();
for (var i = 0; i < wins.length; i++) {{
    var w = wins[i];
    var rc = (w.resourceClass || "").toLowerCase();
    var rn = (w.resourceName || "").toLowerCase();
    var cap = (w.caption || "");
    if (rc.indexOf("mediaplayerofdoom") >= 0 || rn.indexOf("mediaplayerofdoom") >= 0 || cap.indexOf("Random Shuffle Player") >= 0) {{
        w.skipTaskbar = {hide};
        w.minimized = {hide};
        {activate}
    }}
}}
"#,
            hide = hide,
            activate = if hide { "" } else { "workspace.activeWindow = w;" }
        );
        let path = data_dir().join("tray-toggle.js");
        if std::fs::write(&path, script).is_err() {
            return false;
        }
        let path_s = path.to_string_lossy().to_string();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let plugin = format!("mediaplayerofdoom_tray_{}", stamp);
        let load = std::process::Command::new(qdbus)
            .args([
                "org.kde.KWin",
                "/Scripting",
                "org.kde.kwin.Scripting.loadScript",
                &path_s,
                &plugin,
            ])
            .output();
        let Ok(load) = load else { return false };
        if !load.status.success() {
            return false;
        }
        let Ok(id) = String::from_utf8_lossy(&load.stdout).trim().parse::<u64>() else {
            return false;
        };
        let script_path = format!("/Scripting/Script{}", id);
        let ran = std::process::Command::new(qdbus)
            .args(["org.kde.KWin", &script_path, "org.kde.kwin.Script.run"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        // Do not call Scripting.start(): it re-runs every script ever loaded,
        // including old Hide scripts, which made Show appear to do nothing.
        let _ = std::process::Command::new(qdbus)
            .args([
                "org.kde.KWin",
                "/Scripting",
                "org.kde.kwin.Scripting.unloadScript",
                &plugin,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        ran
    }
}

/// Hide the main window. On KDE's Wayland session the KWin script removes the
/// taskbar entry and minimizes; non-KDE sessions fall back to minimize (and the
/// caller is told the taskbar hide was not guaranteed).
pub fn hide_window(ctx: &egui::Context) -> bool {
    if kwin_toggle(true) {
        return true;
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        return true;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // X11 honors set_visible; Wayland does not.
        if std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("x11"))
            .unwrap_or(false)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        false
    }
}

/// Restore the main window after Hide. KWin unminimizes and activates it;
/// other sessions get the normal minimize/visible commands.
pub fn show_window(ctx: &egui::Context) {
    let _ = kwin_toggle(false);
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
}

/// Test hook: exercise the same tray show/hide path used by a menu click.
pub fn simulate_tray(id: &str) {
    #[cfg(target_os = "linux")]
    {
        linux_tray::simulate(id);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = id;
    }
}

/// Desktop popup for a track. Uses the desktop notification service on Linux;
/// other platforms simply keep the tray tooltip path.
pub fn notify_song(artist: &str, title: &str, album: &str) {
    let title = title.trim();
    let title = if title.is_empty() { APP_NAME } else { title };
    let summary = if crate::util::is_placeholder_meta(artist) {
        title.to_string()
    } else {
        format!("{} — {}", artist.trim(), title)
    };
    let body = if crate::util::is_placeholder_meta(album) {
        APP_NAME.to_string()
    } else {
        format!("{} · {}", album.trim(), APP_NAME)
    };
    #[cfg(target_os = "linux")]
    {
        static LAST_ID: std::sync::Mutex<u32> = std::sync::Mutex::new(0);
        let mut cmd = std::process::Command::new("notify-send");
        cmd.args([
            "-a",
            APP_NAME,
            "-i",
            "mediaplayerofdoom",
            "-t",
            "3500",
            "-p",
            "-h",
            "string:x-canonical-private-synchronous:mediaplayerofdoom",
        ]);
        if let Ok(last) = LAST_ID.lock() {
            if *last != 0 {
                cmd.args(["-r", &last.to_string()]);
            }
        }
        cmd.arg(&summary).arg(&body);
        if let Ok(out) = cmd.output() {
            if let Ok(id) = String::from_utf8_lossy(&out.stdout).trim().parse::<u32>() {
                if let Ok(mut last) = LAST_ID.lock() {
                    *last = id;
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (summary, body);
    }
}

/// Resolve tags for a path (including CUE tracks) and show a desktop popup.
pub fn notify_song_path(path: &str) {
    let mut title = String::new();
    let mut artist = String::new();
    let mut album = String::new();
    if let Some(t) = crate::cue::track_for(path) {
        title = t.title;
        artist = t.artist;
        album = t.album;
    } else if let Some(probed) = crate::files::load_tagged(path) {
        if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
            if let Some(t) = tag.title() { title = t.to_string(); }
            if let Some(a) = tag.artist() { artist = a.to_string(); }
            if let Some(al) = tag.album() { album = al.to_string(); }
        }
    }
    let (title, artist, album) = crate::library::repaired_meta(path, &title, &artist, &album);
    notify_song(&artist, &title, &album);
}
pub fn open_in_explorer(path: &str) {
    let disk = crate::cue::disk_path(path);
    let dir = Path::new(&disk).parent().unwrap_or(Path::new("."));
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
        // Show and Quit must work even when the window is hidden and egui is
        // no longer running `ui()`, so handle them directly from the tray
        // thread. Hide is queued so the UI can show the notification before it
        // minimizes.
        if matches!(id, "show" | "quit") {
            if let (Some(ctx), Some(hidden)) = (super::TRAY_CTX.get(), super::TRAY_HIDDEN.get()) {
                match id {
                    "show" => {
                        hidden.store(false, std::sync::atomic::Ordering::Relaxed);
                        super::show_window(ctx);
                    }
                    "quit" => {
                        // If the UI loop is frozen while hidden, a plain Close
                        // command can be ignored. Unminimize first, then fall
                        // back to a direct exit if the app is still alive.
                        let _ = super::show_window(ctx);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        std::thread::spawn(|| {
                            std::thread::sleep(std::time::Duration::from_secs(2));
                            std::process::exit(0);
                        });
                    }
                    _ => {}
                }
                ctx.request_repaint();
                return;
            }
        }
        if id != "hide" {
            // Playback commands go to the background worker so they still work
            // while the UI loop is frozen in Hide mode.
            if let Some(control) = super::TRAY_CONTROL.get() {
                if let Ok(mut q) = control.lock() {
                    q.push_back(id.to_string());
                }
                return;
            }
        }
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
                item("Hide", "hide"),
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

    pub fn simulate(id: &str) {
        push(id);
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
        let hide = MenuItem::with_id(MenuId::new("hide"), "Hide", true, None);
        let playpause = MenuItem::with_id(MenuId::new("play_pause"), "Play/Pause", true, None);
        let skip = MenuItem::with_id(MenuId::new("skip"), "Skip", true, None);
        let prev = MenuItem::with_id(MenuId::new("prev"), "Previous", true, None);
        let stop_item = MenuItem::with_id(MenuId::new("stop"), "Stop", true, None);
        let quit = MenuItem::with_id(MenuId::new("quit"), "Quit", true, None);
        let sep = PredefinedMenuItem::separator();
        menu.append_items(&[&show, &hide, &sep, &playpause, &skip, &prev, &stop_item, &sep, &quit])?;
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
