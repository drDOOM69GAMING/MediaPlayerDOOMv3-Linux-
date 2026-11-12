// Blu-ray sources (disc drive or ISO image).
//
// ffmpeg's `bluray:` protocol uses libbluray, and libaacs reads AACS keys from
// ~/.config/aacs/KEYDB.cfg (or $XDG_CONFIG_HOME/aacs/KEYDB.cfg) automatically.
#![allow(dead_code)]

pub const PREFIX: &str = "bluray:";

pub fn is_bluray(path: &str) -> bool {
    path.starts_with(PREFIX)
}

pub fn wrap(path: &str) -> String {
    if is_bluray(path) {
        path.to_string()
    } else {
        format!("{}{}", PREFIX, path)
    }
}

pub fn wrap_playlist(source: &str, playlist: Option<u32>) -> String {
    match playlist {
        Some(n) => format!("{}{}:{}", PREFIX, n, source),
        None => format!("{}{}", PREFIX, source),
    }
}

pub fn wrap_chapter(source: &str, playlist: Option<u32>, chapter: u32) -> String {
    match playlist {
        Some(n) => format!("{}{}:{}:{}", PREFIX, n, chapter, source),
        None => format!("{}{}", PREFIX, source),
    }
}

/// Path after the optional `bluray:<playlist>:` marker.
pub fn source(path: &str) -> &str {
    let mut rest = path.strip_prefix(PREFIX).unwrap_or(path);
    // Up to two numeric markers: playlist then chapter.
    for _ in 0..2 {
        if let Some((first, tail)) = rest.split_once(':') {
            if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) && tail.starts_with('/') {
                return tail;
            }
            if !first.is_empty() && first.chars().all(|c| c.is_ascii_digit()) {
                rest = tail;
                continue;
            }
        }
        break;
    }
    rest
}

pub fn playlist(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PREFIX)?;
    let (first, _) = rest.split_once(':')?;
    first.parse::<u32>().ok()
}

pub fn chapter(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PREFIX)?;
    let (_pl, rest) = rest.split_once(':')?;
    let (ch, _) = rest.split_once(':')?;
    if ch.starts_with('/') {
        return None;
    }
    ch.parse::<u32>().ok()
}

#[derive(Clone)]
pub struct TitleInfo {
    pub playlist: u32,
    pub secs: f32,
    pub chapters: u32,
    /// Audio languages in stream order, as reported by bd_list_titles.
    pub audio_langs: Vec<String>,
    /// 0-based audio stream index whose language is English, when known.
    pub english_audio: Option<usize>,
}

/// Ask libbluray which playlist is the main title and how long it is. The
/// duration lets the player skip a second optical probe before the first frame.
pub fn main_title_info(source: &str) -> Option<TitleInfo> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<TitleInfo>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(m) = cache.lock() {
        if let Some(v) = m.get(source) {
            return v.clone();
        }
    }
    let info = (|| {
        let mut cmd = std::process::Command::new("bd_list_titles");
        cmd.args(["-l", "-a"]).arg(source);
        configure_command(&mut cmd);
        let out = cmd.output().ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut main_idx: Option<usize> = None;
        let mut current_idx: Option<usize> = None;
        let mut audio_langs: Vec<String> = Vec::new();
        let mut main_audio: Option<usize> = None;
        let mut titles: Vec<(usize, u32, u32, u32)> = Vec::new(); // index, secs, playlist, chapters
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("Main title:") {
                main_idx = v.trim().parse().ok();
            }
            if let Some(v) = line.trim().strip_prefix("AUD:") {
                if current_idx == main_idx {
                    audio_langs = v.split_whitespace().map(|x| x.to_ascii_lowercase()).collect();
                }
                continue;
            }
            if !line.trim_start().starts_with("index:") {
                continue;
            }
            current_idx = line.split_whitespace().nth(1).and_then(|v| v.parse::<usize>().ok());
            let idx = line.split_whitespace().nth(1).and_then(|v| v.parse::<usize>().ok());
            let dur = line.split("duration:").nth(1).and_then(|rest| {
                let hm = rest.split_whitespace().next()?;
                let mut it = hm.split(':');
                let h: u32 = it.next()?.parse().ok()?;
                let m: u32 = it.next()?.parse().ok()?;
                let s: u32 = it.next()?.parse().ok()?;
                Some(h * 3600 + m * 60 + s)
            });
            let pl = line.split("(playlist:").nth(1).and_then(|rest| {
                rest.split_whitespace()
                    .next()?
                    .trim_end_matches(')')
                    .trim_end_matches(".mpls")
                    .parse::<u32>()
                    .ok()
            });
            let chapters = line.split("chapters:").nth(1).and_then(|rest| {
                rest.split_whitespace().next()?.parse::<u32>().ok()
            }).unwrap_or(0);
            if let (Some(i), Some(d), Some(p)) = (idx, dur, pl) {
                titles.push((i, d, p, chapters));
            }
        }
        if !audio_langs.is_empty() {
            main_audio = audio_langs.iter().position(|l| matches!(l.as_str(), "eng" | "en" | "english"));
        }
        let chosen = main_idx
            .and_then(|mi| titles.iter().find(|(i, _, _, _)| *i == mi).copied())
            .or_else(|| titles.iter().max_by_key(|(_, d, _, _)| *d).copied())?;
        Some(TitleInfo {
            playlist: chosen.2,
            secs: chosen.1 as f32,
            chapters: chosen.3,
            audio_langs: audio_langs.clone(),
            english_audio: main_audio,
        })
    })();
    if let Ok(mut m) = cache.lock() {
        m.insert(source.to_string(), info.clone());
    }
    info
}

pub fn main_playlist(source: &str) -> Option<u32> {
    main_title_info(source).map(|i| i.playlist)
}

pub fn audio_languages(source: &str) -> Vec<String> {
    main_title_info(source)
        .map(|i| i.audio_langs)
        .unwrap_or_default()
}

pub fn english_audio_map(source: &str) -> Option<String> {
    main_title_info(source)
        .and_then(|i| i.english_audio)
        .map(|n| format!("0:a:{}", n))
}

pub fn display_name(path: &str) -> String {
    if !is_bluray(path) {
        return crate::util::stem(path);
    }
    let src = source(path);
    if src.starts_with("/dev/") {
        format!("BLU-RAY ({})", src)
    } else {
        std::path::Path::new(src)
            .file_name()
            .map(|n| format!("BLU-RAY {}", n.to_string_lossy()))
            .unwrap_or_else(|| "BLU-RAY".to_string())
    }
}

#[cfg(target_os = "linux")]
pub fn drives() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("sr") && name[2..].chars().all(|c| c.is_ascii_digit()) {
                out.push(e.path().to_string_lossy().to_string());
            }
        }
    }
    for fixed in ["/dev/dvd", "/dev/cdrom"] {
        if std::path::Path::new(fixed).exists() {
            out.push(fixed.to_string());
        }
    }
    out.sort();
    out.dedup_by(|a, b| std::fs::canonicalize(a).ok() == std::fs::canonicalize(b).ok());
    out
}

#[cfg(not(target_os = "linux"))]
pub fn drives() -> Vec<String> {
    Vec::new()
}

pub fn first_drive() -> Option<String> {
    drives().into_iter().next()
}

/// Search the usual locations for a user-supplied AACS KEYDB.cfg. Keys are
/// never bundled with the app; only the user's own database is used.
pub fn keydb_candidates() -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute());
    let mut out = Vec::new();
    if let Some(x) = &xdg {
        out.push(x.join("aacs").join("KEYDB.cfg"));
    }
    if let Some(h) = &home {
        out.push(h.join(".config/aacs/KEYDB.cfg"));
        out.push(h.join(".aacs/KEYDB.cfg"));
    }
    out.push(std::path::PathBuf::from("/etc/xdg/aacs/KEYDB.cfg"));
    out.push(std::path::PathBuf::from("/usr/share/aacs/KEYDB.cfg"));
    out
}

pub fn keydb_path() -> Option<std::path::PathBuf> {
    keydb_candidates().into_iter().find(|p| p.is_file())
}

/// Usual locations for a user-supplied BD+ table set. libbdplus reads
/// `$XDG_CONFIG_HOME/bdplus` with `convtab/` and `vm0/` inside it.
pub fn bdplus_candidates() -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let xdg = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute());
    let mut out = Vec::new();
    if let Some(x) = &xdg {
        out.push(x.join("bdplus"));
    }
    if let Some(h) = &home {
        out.push(h.join(".config/bdplus"));
        out.push(h.join(".bdplus"));
    }
    out.push(std::path::PathBuf::from("/etc/xdg/bdplus"));
    out.push(std::path::PathBuf::from("/usr/share/bdplus"));
    out
}

pub fn bdplus_path() -> Option<std::path::PathBuf> {
    bdplus_candidates()
        .into_iter()
        .find(|p| p.is_dir() && (p.join("convtab").is_dir() || p.join("vm0").is_dir()))
}

pub fn bdplus_convtab_count() -> usize {
    bdplus_path()
        .and_then(|p| std::fs::read_dir(p.join("convtab")).ok())
        .map(|rd| rd.flatten().count())
        .unwrap_or(0)
}

/// A user-installed libbdplus.so can be dropped in the app tools folder or a
/// local lib directory. LIBBDPLUS_PATH lets libbluray load it without a system
/// package.
pub fn bdplus_lib_path() -> Option<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    dirs.push(crate::util::data_dir().join("tools"));
    dirs.push(crate::util::data_dir());
    if let Some(h) = std::env::var_os("HOME") {
        let h = std::path::PathBuf::from(h);
        dirs.push(h.join(".local/lib"));
    }
    dirs.push(std::path::PathBuf::from("/usr/local/lib"));
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with("libbdplus.so") {
                    return Some(e.path());
                }
            }
        }
    }
    None
}

/// One config directory that carries both the user's AACS KEYDB and BD+
/// tables, so libaacs and libbdplus see them even when they live outside the
/// effective XDG config path.
pub fn config_env() -> Option<std::path::PathBuf> {
    use std::sync::OnceLock;
    static ENV: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    ENV.get_or_init(|| {
        let key = keydb_path();
        let bdplus = bdplus_path();
        let effective = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
        let key_ok = key
            .as_ref()
            .map(|k| k == &effective.join("aacs/KEYDB.cfg"))
            .unwrap_or(true);
        let bdplus_ok = bdplus
            .as_ref()
            .map(|b| b == &effective.join("bdplus"))
            .unwrap_or(true);
        if key_ok && bdplus_ok {
            return None;
        }
        let dir = crate::util::data_dir().join("bd-env");
        std::fs::create_dir_all(&dir).ok()?;
        if let Some(k) = key {
            let aacs = dir.join("aacs");
            std::fs::create_dir_all(&aacs).ok()?;
            let link = aacs.join("KEYDB.cfg");
            let _ = std::fs::remove_file(&link);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&k, &link).ok()?;
            #[cfg(not(unix))]
            std::fs::copy(&k, &link).ok()?;
        }
        if let Some(b) = bdplus {
            let link = dir.join("bdplus");
            let _ = std::fs::remove_file(&link);
            #[cfg(unix)]
            std::os::unix::fs::symlink(&b, &link).ok()?;
            #[cfg(not(unix))]
            {
                let _ = (&b, &link);
            }
        }
        Some(dir)
    })
    .clone()
}

fn configure_local(cmd: &mut std::process::Command) {
    if let Some(dir) = config_env() {
        cmd.env("XDG_CONFIG_HOME", dir);
    }
    if let Some(lib) = bdplus_lib_path() {
        cmd.env("LIBBDPLUS_PATH", lib);
    }
}

pub fn configure_command(cmd: &mut std::process::Command) {
    if mmbd_active() {
        if let Some(dir) = mmbd_env_dir() {
            let mut paths = vec![dir];
            if let Some(old) = std::env::var_os("LD_LIBRARY_PATH") {
                paths.extend(std::env::split_paths(&old));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("LD_LIBRARY_PATH", joined);
            }
            // libmmbd supplies both APIs; a local LIBBDPLUS_PATH would take
            // precedence and bypass MakeMKV's BD+ handling.
            cmd.env_remove("LIBBDPLUS_PATH");
            return;
        }
    }
    configure_local(cmd);
}

/// Protection facts read from libbluray's `bd_info`. This is what the Blu-ray
/// deck shows so the user can see whether their KEYDB supplied the AACS key
/// and whether the disc also carries BD+.
#[derive(Clone, Debug, Default)]
pub struct DiscProtection {
    pub bluray: bool,
    pub bd_j: bool,
    pub aacs: bool,
    pub aacs_handled: bool,
    /// libaacs could not initialize AACS bus encryption for this drive/disc.
    pub bus_encryption: bool,
    pub bd_plus: bool,
    pub bd_plus_handled: bool,
    pub libbdplus: bool,
    pub bdplus_tables: usize,
    pub aacs_mkb: Option<u32>,
    pub disc_id: Option<String>,
    pub keydb_present: bool,
    pub keydb_match: bool,
    pub keydb_entries: u64,
}

impl DiscProtection {
    pub fn summary(&self) -> String {
        if !self.bluray {
            return "NO BLU-RAY DISC DETECTED".to_string();
        }
        let aacs = if self.aacs {
            if self.aacs_handled {
                "AACS KEY OK"
            } else {
                "AACS KEY MISSING"
            }
        } else {
            "NO AACS"
        };
        if self.bus_encryption {
            return format!("AACS BUS ENCRYPTION  •  {}  •  {}", if self.keydb_match { "KEYDB MATCH" } else { "KEYDB NO MATCH" }, if self.bd_plus { "BD+ YES" } else { "BD+ NO" });
        }
        let bd = if !self.bd_plus {
            "BD+ NO"
        } else if self.bd_plus_handled {
            "BD+ HANDLED"
        } else if self.bdplus_tables > 0 && !self.libbdplus {
            "BD+ TABLES / NO libbdplus"
        } else if self.bdplus_tables > 0 {
            "BD+ TABLES / NO KEY"
        } else {
            "BD+ NO TABLES"
        };
        let key = if self.keydb_match {
            "KEYDB MATCH"
        } else if self.aacs {
            "KEYDB NO MATCH"
        } else {
            "KEYDB N/A"
        };
        format!("{}  •  {}  •  {}", aacs, bd, key)
    }

    pub fn needs_makemkv(&self) -> bool {
        self.bus_encryption
            || (self.bd_plus && !self.bd_plus_handled)
            || (self.aacs && !self.aacs_handled)
    }
}

fn parse_yes(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "yes" | "true" | "1" | "detected"
    )
}

/// Parse the stable text output of `bd_info`.
pub fn parse_bd_info(text: &str) -> DiscProtection {
    let mut d = DiscProtection::default();
    for line in text.lines() {
        let Some((key, val)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let val = val.trim();
        match key {
            "BluRay detected" => d.bluray = parse_yes(val),
            "BD-J detected" => d.bd_j = parse_yes(val),
            "AACS detected" => d.aacs = parse_yes(val),
            "AACS handled" => d.aacs_handled = parse_yes(val),
            "AACS MKB version" => d.aacs_mkb = val.parse().ok(),
            "Disc ID" => {
                if !val.is_empty() && val != "???" {
                    d.disc_id = Some(val.to_string());
                }
            }
            "BD+ detected" => d.bd_plus = parse_yes(val),
            "BD+ handled" => d.bd_plus_handled = parse_yes(val),
            "libbdplus detected" => d.libbdplus = parse_yes(val),
            _ => {}
        }
    }
    if text.to_ascii_lowercase().contains("bus encryption")
        || text.to_ascii_lowercase().contains("host key / certificate")
    {
        d.bus_encryption = true;
    }
    d
}

/// Look for a disc ID in a KEYDB.cfg. Returns (found, entry count).
fn keydb_lookup_in(path: &std::path::Path, disc_id: &str) -> (bool, u64) {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else {
        return (false, 0);
    };
    let mut found = false;
    let mut count = 0u64;
    for line in std::io::BufReader::new(file).lines().flatten() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        let id = line.split('|').next().unwrap_or("").trim();
        let id = if let Some(rest) = id.strip_prefix("0x").or_else(|| id.strip_prefix("0X")) {
            // AACS 2.1 KEYDB format: "0xDISCID = Name | ..."
            rest.chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect::<String>()
        } else {
            // Classic format: "DISCID | KEY | ..."
            id.split_whitespace().next().unwrap_or("").to_string()
        };
        if id.is_empty() {
            continue;
        }
        count += 1;
        if id.eq_ignore_ascii_case(disc_id) {
            found = true;
        }
    }
    (found, count)
}

/// Run `bd_info` with a timeout so a busy/absent drive cannot leave the deck
/// stuck on "CHECKING..." forever.
fn run_bd_info(source: &str) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut cmd = Command::new("bd_info");
    cmd.arg(source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Always assess the disc with the local stack, never through libmmbd,
    // otherwise a successful MakeMKV decrypt would hide the fact that local
    // libaacs cannot open the drive and disable live decrypt on the next try.
    configure_local(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let mut stderr = child.stderr.take();
    let (tx, rx) = std::sync::mpsc::channel();
    let err_handle = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr.take() {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        let _ = tx.send(s);
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let mut text = rx.recv_timeout(Duration::from_secs(2)).ok()?;
                let err = err_handle.join().unwrap_or_default();
                if !err.is_empty() {
                    text.push('\n');
                    text.push_str(&err);
                }
                return Some(text);
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        // Bus-encrypted discs can take 20-30 s while libaacs retries the host
        // key, so allow plenty of time; this runs on a worker thread.
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// MakeMKV ships libmmbd.so, a drop-in libaacs/libbdplus that decrypts the
/// disc live through MakeMKV's service. It handles AACS bus encryption and
/// BD+ without ripping, so playback works even when the system libaacs cannot
/// initialize the drive.
pub fn mmbd_path() -> Option<std::path::PathBuf> {
    let mut dirs: Vec<std::path::PathBuf> = vec![
        std::path::PathBuf::from("/usr/lib"),
        std::path::PathBuf::from("/usr/lib64"),
        std::path::PathBuf::from("/usr/local/lib"),
        std::path::PathBuf::from("/opt/makemkv/lib"),
        crate::util::data_dir().join("tools"),
        crate::util::data_dir(),
    ];
    if let Some(ld) = std::env::var_os("LD_LIBRARY_PATH") {
        dirs.extend(std::env::split_paths(&ld));
    }
    for d in dirs {
        for name in ["libmmbd.so.0", "libmmbd.so"] {
            let p = d.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub fn mmbd_available() -> bool {
    mmbd_path().is_some()
}

static MMBD_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Select MakeMKV's live-decrypt libraries for the next ffmpeg/libbluray
/// processes. The probe turns this on only when local libaacs/libbdplus cannot
/// handle the disc, so ordinary AACS discs and working BD+ tables stay local.
pub fn set_mmbd(on: bool) {
    MMBD_ACTIVE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn mmbd_active() -> bool {
    MMBD_ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Private directory with libaacs.so.0 and libbdplus.so.0 symlinked to
/// libmmbd.so.0, used through LD_LIBRARY_PATH.
pub fn mmbd_env_dir() -> Option<std::path::PathBuf> {
    let lib = mmbd_path()?;
    let dir = crate::util::data_dir().join("mmbd");
    std::fs::create_dir_all(&dir).ok()?;
    for name in ["libaacs.so.0", "libbdplus.so.0"] {
        let link = dir.join(name);
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&lib, &link).ok()?;
        #[cfg(not(unix))]
        return None;
    }
    Some(dir)
}

/// MakeMKV leaves a `makemkvcon guiserver` process running after every
/// command, and that process keeps the optical drive open. While it is alive,
/// libbluray/libaacs cannot initialize the drive. Terminate stale guiservers
/// only when no real MakeMKV job or MakeMKV GUI is active.
#[cfg(target_os = "linux")]
pub fn release_stale_makemkv() {
    let mut guiservers: Vec<u32> = Vec::new();
    let mut active = false;
    let Ok(rd) = std::fs::read_dir("/proc") else { return };
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(raw) = std::fs::read(format!("/proc/{}/cmdline", pid)) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }
        let args: Vec<String> = raw
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect();
        let exe = args
            .first()
            .map(|s| s.rsplit('/').next().unwrap_or(s).to_string())
            .unwrap_or_default();
        if exe != "makemkvcon" && exe != "makemkv" {
            continue;
        }
        if exe == "makemkvcon" && args.iter().any(|a| a == "guiserver") {
            guiservers.push(pid);
        } else {
            active = true;
        }
    }
    if active || guiservers.is_empty() {
        return;
    }
    for pid in &guiservers {
        unsafe {
            libc::kill(*pid as i32, libc::SIGTERM);
        }
    }
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_millis(700) {
        if guiservers
            .iter()
            .all(|p| !std::path::Path::new(&format!("/proc/{}", p)).exists())
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[cfg(not(target_os = "linux"))]
pub fn release_stale_makemkv() {}

/// Probe the current disc with `bd_info` and check the user's KEYDB for the
/// disc's AACS ID. This can take a moment on a large KEYDB, so callers should
/// run it on a worker thread.
pub fn protection_info(source: &str) -> Option<DiscProtection> {
    if !mmbd_active() {
        release_stale_makemkv();
    }
    let text = run_bd_info(source)?;
    let mut info = parse_bd_info(&text);
    if !info.bluray && info.disc_id.is_none() && !info.aacs && !info.bd_plus {
        return None;
    }
    info.keydb_present = keydb_path().is_some();
    info.bdplus_tables = bdplus_convtab_count();
    // Local libaacs/libbdplus first. MakeMKV's live decrypt is only selected
    // when the local stack cannot open the disc.
    set_mmbd(
        mmbd_available()
            && (info.bus_encryption
                || (info.aacs && !info.aacs_handled)
                || (info.bd_plus && !info.bd_plus_handled)),
    );
    if let (Some(id), Some(path)) = (info.disc_id.clone(), keydb_path()) {
        let (found, entries) = keydb_lookup_in(&path, &id);
        info.keydb_match = found;
        info.keydb_entries = entries;
    }
    Some(info)
}

pub fn eject(path: &str) -> bool {    if !is_bluray(path) {
        return false;
    }
    let src = source(path);
    if !src.starts_with("/dev/") {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(file) = std::fs::OpenOptions::new().read(true).open(src) {
            use std::os::unix::io::AsRawFd;
            if unsafe { libc::ioctl(file.as_raw_fd(), 0x5309, 0) } == 0 {
                return true;
            }
        }
        return std::process::Command::new("eject")
            .arg(src)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_parses_aacs_key_and_bdplus() {
        let text = "Using libbluray version 1.5.1\nVolume Identifier   : JUMANJI\nBluRay detected     : yes\nBD-J detected       : yes\nAACS detected       : yes\nlibaacs detected    : yes\nDisc ID             : 6146196C7EC14BCA70544B2D08DD5885A26D7224\nAACS MKB version    : 64\nAACS handled        : yes\n\nBD+ detected        : no\n";
        let p = parse_bd_info(text);
        assert!(p.bluray && p.bd_j && p.aacs && p.aacs_handled);
        assert!(!p.bd_plus);
        assert_eq!(p.aacs_mkb, Some(64));
        assert_eq!(
            p.disc_id.as_deref(),
            Some("6146196C7EC14BCA70544B2D08DD5885A26D7224")
        );
        assert!(!p.needs_makemkv());
        let bdplus = parse_bd_info("BluRay detected: yes\nAACS detected: yes\nAACS handled: no\nBD+ detected: yes\n");
        assert!(bdplus.needs_makemkv());
        assert!(bdplus.bd_plus);
    }

    #[test]
    fn mmbd_env_dir_links_both_apis() {
        if mmbd_path().is_none() {
            return;
        }
        let dir = mmbd_env_dir().expect("mmbd env dir");
        assert!(dir.join("libaacs.so.0").exists());
        assert!(dir.join("libbdplus.so.0").exists());
    }

    #[test]
    fn protection_flags_aacs_bus_encryption() {
        let text = "BluRay detected     : yes\nAACS detected       : yes\nAACS handled        : yes\nBD+ detected        : no\n\nmmc.c:472: Host key / Certificate (id 0xffff800001c1) has been revoked by your drive ?\naacs.c:1368: Unable to initialize bus encryption required by drive and disc\n";
        let p = parse_bd_info(text);
        assert!(p.bus_encryption, "stderr must mark bus encryption");
        assert!(p.needs_makemkv(), "bus encryption needs MakeMKV");
        assert!(p.summary().contains("BUS ENCRYPTION"));
    }

    #[test]
    fn keydb_lookup_finds_disc_id_case_insensitively() {
        let dir = std::env::temp_dir().join(format!("mpdoom-keydb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("KEYDB.cfg");
        std::fs::write(
            &db,
            "; comment\n6146196C7EC14BCA70544B2D08DD5885A26D7224 | 0x00112233445566778899AABBCCDDEEFF | MKBv64\n0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA = Other Disc | 0x00 |\n",
        )
        .unwrap();
        let (found, entries) = keydb_lookup_in(
            &db,
            "6146196c7ec14bca70544b2d08dd5885a26d7224",
        );
        assert!(found);
        assert_eq!(entries, 2);
        let (found21, _) =
            keydb_lookup_in(&db, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert!(found21, "AACS 2.1 0xDISCID entries must match");
        let (missing, _) = keydb_lookup_in(&db, "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
        assert!(!missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn inspect_real_bluray_from_env() {
        let Some(src) = std::env::var_os("MPDOOM_TEST_BD") else {
            return;
        };
        let info = protection_info(&src.to_string_lossy()).unwrap_or_default();
        println!("{:?}", info);
        println!("{}", info.summary());
        println!("needs_makemkv={}", info.needs_makemkv());
    }

    #[test]
    fn prefix_parsing() {
        let p = wrap_chapter("/dev/sr0", Some(1), 5);
        assert_eq!(p, "bluray:1:5:/dev/sr0");
        assert_eq!(source(&p), "/dev/sr0");
        assert_eq!(playlist(&p), Some(1));
        assert_eq!(chapter(&p), Some(5));
        let iso = wrap_playlist("/home/user/Movie.iso", Some(100));
        assert_eq!(source(&iso), "/home/user/Movie.iso");
        assert_eq!(playlist(&iso), Some(100));
        assert_eq!(chapter(&iso), None);
    }
}
