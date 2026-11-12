// Whole-disc imaging (DVD / Blu-ray) to an ISO file.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

#[cfg(not(windows))]
use crate::tools::xdg_user_dir;
use crate::types::Msg;
use crate::util::{home_dir, sanitize_win};

pub fn videos_dir() -> PathBuf {
    #[cfg(not(windows))]
    let dir = xdg_user_dir("XDG_VIDEOS_DIR")
        .or_else(|| home_dir().map(|h| h.join("Videos")))
        .unwrap_or_else(|| PathBuf::from("."));
    #[cfg(windows)]
    let dir = home_dir()
        .map(|h| h.join("Videos"))
        .unwrap_or_else(|| PathBuf::from("."));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn disc_label(device: &str) -> Option<String> {
    for args in [
        vec!["-s", "LABEL", "-o", "value", device],
        vec!["-no", "LABEL", device],
    ] {
        let prog = if args[0] == "-no" { "lsblk" } else { "blkid" };
        if let Ok(out) = Command::new(prog).args(&args).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn target_path(kind: &str, device: &str) -> PathBuf {
    target_path_in(&videos_dir(), kind, device)
}

fn target_path_in(dir: &Path, kind: &str, device: &str) -> PathBuf {
    let label = disc_label(device)
        .map(|l| sanitize_win(&l))
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| kind.to_ascii_lowercase());
    let base = dir.join(format!("{}.iso", label));
    if !base.exists() {
        return base;
    }
    for n in 2..1000 {
        let p = dir.join(format!("{} ({}).iso", label, n));
        if !p.exists() {
            return p;
        }
    }
    base
}

pub fn device_size(device: &str) -> Option<u64> {
    if let Ok(out) = Command::new("blockdev")
        .args(["--getsize64", device])
        .output()
    {
        if out.status.success() {
            if let Ok(n) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }
    // Some optical devices/older util-linux builds do not answer
    // blockdev --getsize64; lsblk reads the same size from sysfs.
    if let Ok(out) = Command::new("lsblk")
        .args(["-b", "-d", "-n", "-o", "SIZE", device])
        .output()
    {
        if out.status.success() {
            if let Ok(n) = String::from_utf8_lossy(&out.stdout).trim().parse::<u64>() {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Copy all sectors of `device` to an ISO in `dir`. This preserves the whole
/// disc, including menus and extras. `cancel` is checked while dd runs so the
/// user (or app shutdown) can stop the copy and remove the partial image.
pub fn dump(
    device: &str,
    kind: &str,
    dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (device, kind, dir, tx, cancel);
        return Err("disc imaging is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
    if !Path::new(device).exists() {
        return Err(format!("{} not found", device));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create destination folder: {}", e))?;
    let target = target_path_in(dir, kind, device);
    // Write to a clearly-partial name, then rename only after dd succeeds. A
    // crash/kill can then never leave something that looks like a complete ISO.
    let partial = target.with_file_name(format!(
        "{}.{}.part",
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "disc.iso".to_string()),
        std::process::id()
    ));
    let total = device_size(device);
    let _ = tx.send(Msg::DiscDumpProgress {
        done: 0,
        total: total.unwrap_or(0),
        text: format!("Dumping {} to {}...", device, target.display()),
    });
    let block_device = device.starts_with("/dev/");
    let mut cmd = Command::new("dd");
    cmd.arg(format!("if={}", device))
        .arg(format!("of={}", partial.to_string_lossy()))
        .arg(format!("bs={}", if block_device { "4M" } else { "1M" }))
        .arg("conv=noerror,sync")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Larger direct reads avoid the page-cache bounce and are a bit faster on
    // optical drives. Regular-file test sources stay on buffered I/O.
    #[cfg(target_os = "linux")]
    if block_device {
        cmd.arg("iflag=direct");
    }
    // If the player dies (even with SIGKILL), make the kernel terminate dd so
    // it cannot keep reading the disc after the window is gone.
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("dd failed to start: {}", e))?;

    // Drain stderr in the background so a chatty dd can never fill the pipe.
    let mut stderr = child.stderr.take();
    let err_handle = stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });

    let mut last_pct = u64::MAX;
    let mut last_sent = Instant::now() - Duration::from_secs(5);
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&partial);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let err = err_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                if !status.success() {
                    let _ = std::fs::remove_file(&partial);
                    let tail = err.lines().rev().take(2).collect::<Vec<_>>().join(" | ");
                    return Err(format!(
                        "dd failed{}",
                        if tail.is_empty() { String::new() } else { format!(": {}", tail) }
                    ));
                }
                break;
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&partial);
                return Err(format!("dd error: {}", e));
            }
        }
        let done = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        let pct = total.map(|t| (done.saturating_mul(100) / t).min(100)).unwrap_or(0);
        // Send at most twice a second; also send immediately when the whole
        // percentage changes so the bar never appears stuck.
        if last_sent.elapsed() >= Duration::from_millis(500)
            || (total.is_some() && pct != last_pct)
        {
            last_pct = pct;
            last_sent = Instant::now();
            let text = match total {
                Some(t) if t > 0 => format!(
                    "Dumping {}... {}%  ({:.1} / {:.1} GB)",
                    kind,
                    pct,
                    done as f64 / 1e9,
                    t as f64 / 1e9
                ),
                _ => format!("Dumping {}... {:.1} GB", kind, done as f64 / 1e9),
            };
            let _ = tx.send(Msg::DiscDumpProgress {
                done,
                total: total.unwrap_or(0),
                text,
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let size = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        let _ = std::fs::remove_file(&partial);
        return Err("dd produced an empty image".into());
    }
    if let Err(e) = std::fs::rename(&partial, &target) {
        let _ = std::fs::remove_file(&partial);
        return Err(format!("could not finalize image: {}", e));
    }
    Ok(target)
    }
}

pub fn dir_size(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    let mut total = 0u64;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            total = total.saturating_add(dir_size(&p));
        } else if let Ok(m) = e.metadata() {
            total = total.saturating_add(m.len());
        }
    }
    total
}

/// Bytes currently present in the ISO staging folder. The UDF writer builds a
/// temporary image there, so this tracks progress without an API callback.
fn staging_bytes(path: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    let mut total = 0u64;
    for e in rd.flatten() {
        if let Ok(m) = e.metadata() {
            total = total.saturating_add(m.len());
        }
    }
    total
}

fn contains_mkv(path: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(path) else { return false };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if contains_mkv(&p) {
                return true;
            }
        } else if p.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("mkv")).unwrap_or(false) {
            return true;
        }
    }
    false
}

/// First MKV found under a dump folder, used to auto-play a RIP + PLAY result.
pub fn first_mkv_in(path: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(path).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(found) = first_mkv_in(&p) {
                return Some(found);
            }
        } else if p.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("mkv")).unwrap_or(false) {
            return Some(p);
        }
    }
    None
}

fn parse_hms(v: &str) -> Option<u64> {
    let mut secs = 0u64;
    let mut parts = 0;
    for (i, p) in v.split(':').rev().enumerate() {
        let n: u64 = p.trim().parse().ok()?;
        secs += n.checked_mul(60u64.pow(i as u32))?;
        parts += 1;
    }
    if parts == 0 { None } else { Some(secs) }
}

/// Ask MakeMKV for its title list and return the longest title (usually the
/// main feature). Robot mode prints `TINFO:<id>,9,0,"H:MM:SS"` for duration.
pub fn makemkv_main_title(source_spec: &str) -> Option<u32> {
    // Must match the dump's --minlength=0 flag: without it MakeMKV hides
    // short clips and renumbers the titles, so the main feature's index would
    // point at an 8-second clip during the rip.
    let mut child = Command::new("makemkvcon")
        .args(["-r", "info", "--minlength=0", source_spec])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        let _ = tx.send(s);
    });
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let text = rx.recv_timeout(Duration::from_secs(2)).ok()?;
                return parse_makemkv_title_list(&text);
            }
            Ok(None) => {}
            Err(_) => return None,
        }
        if start.elapsed() > Duration::from_secs(120) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn parse_makemkv_title_list(text: &str) -> Option<u32> {
    let mut best: Option<(u32, u64)> = None;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("TINFO:") else { continue };
        let mut parts = rest.splitn(3, ',');
        let Some(id) = parts.next().and_then(|v| v.trim().parse::<u32>().ok()) else { continue };
        let Some(code) = parts.next().map(|v| v.trim()) else { continue };
        if code != "9" {
            continue;
        }
        let value = line.rsplit(',').next().unwrap_or("").trim().trim_matches('"');
        let Some(secs) = parse_hms(value) else { continue };
        if best.map(|(_, b)| secs > b).unwrap_or(true) {
            best = Some((id, secs));
        }
    }
    best.map(|(id, _)| id)
}

/// Unique output folder for a clean MKV dump, named `<label> (MKV)`.
pub fn mkv_output_dir(dir: &Path, label: &str) -> PathBuf {
    let trimmed = label.trim();
    let label = if trimmed.is_empty() {
        String::new()
    } else {
        sanitize_win(trimmed)
    };
    let label = if label.trim().is_empty() { "disc".to_string() } else { label };
    for n in 0..1000 {
        let name = if n == 0 {
            format!("{} (MKV)", label)
        } else {
            format!("{} (MKV) ({})", label, n + 1)
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{} (MKV)", label))
}

/// Local Blu-ray main-title dump without MakeMKV. bd_splice uses libbluray's
/// decryption paths, including libaacs and libbdplus with the user's own
/// tables. It produces the main movie, not menus or extras.
fn dump_bluray_mkv_local(
    source_spec: &str,
    out_dir: &Path,
    total: u64,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let source = if let Some(p) = source_spec.strip_prefix("file:") {
        p.to_string()
    } else if let Some(p) = source_spec.strip_prefix("iso:") {
        p.to_string()
    } else if source_spec == "disc:0" {
        crate::bluray::first_drive().ok_or("Local BD+ dump: no optical drive found")?
    } else {
        source_spec.to_string()
    };
    let playlist = crate::bluray::main_playlist(&source)
        .ok_or("Local BD+ dump: could not determine the main playlist")?;
    let splice = crate::tools::which("bd_splice")
        .ok_or("Local BD+ dump needs bd_splice, and it is not installed")?;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {}", out_dir.display(), e))?;
    let temp = out_dir.join(format!(".mpdoom-bd-splice-{}.ts", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let _ = tx.send(Msg::DiscDumpProgress {
        done: 0,
        total,
        text: format!(
            "Local BD+ dump with libbluray/bd_splice (playlist {})...",
            playlist
        ),
    });

    let mut cmd = Command::new(splice);
    cmd.args(["-p", &playlist.to_string()])
        .arg(&source)
        .arg(&temp)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    crate::bluray::configure_command(&mut cmd);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Local BD+ dump: bd_splice failed to start: {}", e))?;
    let err_handle = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let mut last_sent = Instant::now() - Duration::from_secs(5);
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&temp);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                    let _ = std::fs::remove_file(&temp);
                    let tail = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                    return Err(format!(
                        "Local BD+ dump failed{}",
                        if tail.is_empty() { String::new() } else { format!(": {}", tail) }
                    ));
                }
                break;
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&temp);
                return Err(format!("Local BD+ dump error: {}", e));
            }
        }
        if last_sent.elapsed() >= Duration::from_millis(500) {
            last_sent = Instant::now();
            let done = std::fs::metadata(&temp).map(|m| m.len()).unwrap_or(0);
            let _ = tx.send(Msg::DiscDumpProgress {
                done,
                total,
                text: format!(
                    "Local BD+ dump... {:.1} GB",
                    done as f64 / 1e9
                ),
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let out = out_dir.join("Main Movie.mkv");
    let _ = std::fs::remove_file(&out);
    let mut ff = Command::new(crate::tools::ffmpeg_path());
    ff.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&temp)
        .args(["-map", "0", "-c", "copy"])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let st = ff
        .output()
        .map_err(|e| format!("Local BD+ remux failed: ffmpeg: {}", e))?;
    let _ = std::fs::remove_file(&temp);
    if !st.status.success() || std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&out);
        let tail = String::from_utf8_lossy(&st.stderr)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(format!("Local BD+ remux failed: {}", tail));
    }
    let _ = tx.send(Msg::DiscDumpProgress {
        done: total,
        total,
        text: format!("Local BD+ dump complete: {}", out.display()),
    });
    Ok(out_dir.to_path_buf())
}

/// Decrypt/remux a disc or ISO to clean MKV files with MakeMKV. When MakeMKV
/// is missing and the source is a Blu-ray, fall back to the local bd_splice
/// main-title path, which uses the user's AACS/BD+ data.
pub fn dump_to_mkv(
    source_spec: &str,
    out_dir: &Path,
    total: u64,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    bluray: bool,
) -> Result<PathBuf, String> {
    #[cfg(not(windows))]
    if bluray && crate::tools::which("makemkvcon").is_none() {
        return dump_bluray_mkv_local(source_spec, out_dir, total, tx, cancel);
    }
    #[cfg(windows)]
    let _ = bluray;
    makemkv_dump(source_spec, out_dir, total, tx, cancel, "all")
}

/// RIP + PLAY: MakeMKV decrypts the disc (including AACS bus encryption) and
/// writes only the longest title, which the app plays as soon as it is ready.
pub fn dump_main_title_mkv(
    source_spec: &str,
    out_dir: &Path,
    total: u64,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (source_spec, out_dir, total, tx, cancel);
        return Err("MKV dumping is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
        if crate::tools::which("makemkvcon").is_none() {
            return Err(
                "RIP + PLAY needs MakeMKV (makemkvcon) for bus-encrypted discs; \
                 install MakeMKV or use DUMP TO ISO"
                    .into(),
            );
        }
        let _ = tx.send(Msg::DiscDumpProgress {
            done: 0,
            total,
            text: "RIP + PLAY: finding the main title with MakeMKV...".to_string(),
        });
        let title = makemkv_main_title(source_spec).unwrap_or(0);
        // The title lookup can leave a drive-holding guiserver behind; clear
        // it before the rip so MakeMKV does not fight itself for the drive.
        crate::bluray::release_stale_makemkv();
        makemkv_dump(source_spec, out_dir, total, tx, cancel, &title.to_string())
    }
}

#[cfg(not(windows))]
struct MkGuiserverGuard;

#[cfg(not(windows))]
impl Drop for MkGuiserverGuard {
    fn drop(&mut self) {
        crate::bluray::release_stale_makemkv();
    }
}

fn makemkv_dump(
    source_spec: &str,
    out_dir: &Path,
    total: u64,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
    titles: &str,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (source_spec, out_dir, total, tx, cancel, titles);
        return Err("MKV dumping is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
    let _guiserver_guard = MkGuiserverGuard;
    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {}", out_dir.display(), e))?;
    if total > 0 {
        if let Some(free) = free_space(out_dir) {
            let need = total.saturating_add(total / 20);
            if free < need {
                return Err(format!(
                    "not enough free space in {}: the MKV needs about {:.1} GB, only {:.1} GB available. Use DUMP TO MKV to choose another folder",
                    out_dir.display(),
                    need as f64 / 1e9,
                    free as f64 / 1e9
                ));
            }
        }
    }
    let _ = tx.send(Msg::DiscDumpProgress {
        done: 0,
        total,
        text: format!("Dumping to MKV in {}...", out_dir.display()),
    });
    let mut cmd = Command::new("makemkvcon");
    cmd.args(["-r", "mkv", "--minlength=0", source_spec, titles])
        .arg(out_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("MakeMKV failed to start (is makemkvcon installed?): {}", e))?;
    let err_handle = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let mut last_sent = Instant::now() - Duration::from_secs(5);
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(out_dir);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                if !status.success() {
                    let _ = std::fs::remove_dir_all(out_dir);
                    let tail = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                    return Err(format!(
                        "MakeMKV failed{}",
                        if tail.is_empty() { String::new() } else { format!(": {}", tail) }
                    ));
                }
                break;
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(out_dir);
                return Err(format!("MakeMKV error: {}", e));
            }
        }
        let done = dir_size(out_dir);
        if last_sent.elapsed() >= Duration::from_millis(500) {
            last_sent = Instant::now();
            let pct = if total > 0 {
                (done.saturating_mul(100) / total).min(100)
            } else {
                0
            };
            let text = if total > 0 {
                format!(
                    "Dumping to MKV... {}%  ({:.1} / {:.1} GB)",
                    pct,
                    done as f64 / 1e9,
                    total as f64 / 1e9
                )
            } else {
                format!("Dumping to MKV... {:.1} GB", done as f64 / 1e9)
            };
            let _ = tx.send(Msg::DiscDumpProgress {
                done,
                total,
                text,
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    if !contains_mkv(out_dir) {
        let _ = std::fs::remove_dir_all(out_dir);
        return Err("MakeMKV did not produce any MKV files".into());
    }
    Ok(out_dir.to_path_buf())
    }
}

/// Destination for a decrypted DVD image, named `<disc label> (decrypted).iso`.
pub fn decrypted_iso_path(dir: &Path, device: &str) -> PathBuf {
    let label = disc_label(device)
        .map(|l| sanitize_win(&l))
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| "dvd".to_string());
    for n in 0..1000 {
        let name = if n == 0 {
            format!("{} (decrypted).iso", label)
        } else {
            format!("{} (decrypted) ({}).iso", label, n + 1)
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{} (decrypted).iso", label))
}

/// Find the folder that directly contains `VIDEO_TS` under a dvdbackup output
/// tree.
fn find_video_ts_root(root: &Path) -> Option<PathBuf> {
    if root.join("VIDEO_TS").is_dir() {
        return Some(root.to_path_buf());
    }
    let rd = std::fs::read_dir(root).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("VIDEO_TS").is_dir() {
            return Some(p);
        }
    }
    None
}

/// Free bytes on the filesystem containing `path` (Linux).
#[cfg(not(windows))]
fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0 {
        Some(st.f_bavail as u64 * st.f_frsize as u64)
    } else {
        None
    }
}

/// Fast decrypted DVD image: `dvdbackup -M` reads through libdvdread/libdvdcss
/// (so CSS is removed), then `genisoimage` builds a playable DVD-Video ISO.
/// This is much quicker than MakeMKV for DVDs and does not need a license key.
pub fn dump_dvd_with_backup(
    device: &str,
    dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (device, dir, tx, cancel);
        return Err("decrypted disc imaging is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
        if !Path::new(device).exists() {
            return Err(format!("{} not found", device));
        }
        let Some(mkbackup) = crate::tools::dvdbackup_path() else {
            return Err("dvdbackup is not installed".into());
        };
        let Some(mkiso) = crate::tools::genisoimage_path() else {
            return Err("genisoimage is not installed".into());
        };
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create destination folder: {}", e))?;
        let target = decrypted_iso_path(dir, device);
        let partial = target.with_file_name(format!(
            "{}.part",
            target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "dvd.iso".to_string())
        ));
        let _ = std::fs::remove_file(&partial);
        let total = device_size(device).unwrap_or(0);
        // The rip is written as a VIDEO_TS tree and then read again to build
        // the ISO, so this path temporarily needs about twice the disc size.
        if total > 0 {
            if let Some(free) = free_space(dir) {
                let need = total.saturating_mul(22) / 10;
                if free < need {
                    return Err(format!(
                        "not enough free space for dvdbackup: need about {:.1} GB (VIDEO_TS + ISO), only {:.1} GB available",
                        need as f64 / 1e9,
                        free as f64 / 1e9
                    ));
                }
            }
        }
        let work = dir.join(format!(".mpdoom-dvd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work)
            .map_err(|e| format!("cannot create working folder: {}", e))?;
        let label = disc_label(device)
            .map(|l| sanitize_win(&l))
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| "dvd".to_string());

        // ---- stage 1: dvdbackup (decrypts via libdvdcss) ----
        let _ = tx.send(Msg::DiscDumpProgress {
            done: 0,
            total,
            text: format!(
                "Decrypting DVD with dvdbackup to {} (this removes CSS)...",
                target.display()
            ),
        });
        let mut cmd = Command::new(&mkbackup);
        cmd.args(["-M", "-p", "-i", device, "-o"])
            .arg(&work)
            .arg("-n")
            .arg(&label)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        let mut child = cmd.spawn().map_err(|e| {
            let _ = std::fs::remove_dir_all(&work);
            format!("dvdbackup failed to start: {}", e)
        })?;
        let err_handle = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                s
            })
        });
        let mut last_sent = Instant::now() - Duration::from_secs(5);
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&work);
                let _ = std::fs::remove_file(&partial);
                return Err("cancelled".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                    if !status.success() {
                        let _ = std::fs::remove_dir_all(&work);
                        let tail = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                        return Err(format!(
                            "dvdbackup failed{}",
                            if tail.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", tail)
                            }
                        ));
                    }
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_dir_all(&work);
                    return Err(format!("dvdbackup error: {}", e));
                }
            }
            let done = dir_size(&work);
            if last_sent.elapsed() >= Duration::from_millis(500) {
                last_sent = Instant::now();
                let pct = if total > 0 {
                    (done.saturating_mul(100) / total).min(100)
                } else {
                    0
                };
                let text = if total > 0 {
                    format!(
                        "Decrypting DVD... {}%  ({:.1} / {:.1} GB)",
                        pct,
                        done as f64 / 1e9,
                        total as f64 / 1e9
                    )
                } else {
                    format!("Decrypting DVD... {:.1} GB", done as f64 / 1e9)
                };
                let _ = tx.send(Msg::DiscDumpProgress { done, total, text });
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        let Some(disc_root) = find_video_ts_root(&work) else {
            let _ = std::fs::remove_dir_all(&work);
            return Err("dvdbackup produced no VIDEO_TS folder".into());
        };

        // ---- stage 2: build the ISO from the decrypted files ----
        let _ = tx.send(Msg::DiscDumpProgress {
            done: 0,
            total,
            text: format!("Building decrypted ISO {}...", target.display()),
        });
        let mut cmd = Command::new(&mkiso);
        let volume: String = label.chars().take(32).collect();
        cmd.args(["-dvd-video", "-udf", "-quiet", "-V"])
            .arg(&volume)
            .arg("-o")
            .arg(&partial)
            .arg(&disc_root)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        let mut child = cmd.spawn().map_err(|e| {
            let _ = std::fs::remove_dir_all(&work);
            let _ = std::fs::remove_file(&partial);
            format!("genisoimage failed to start: {}", e)
        })?;
        let err_handle = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                s
            })
        });
        let mut last_sent = Instant::now() - Duration::from_secs(5);
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&work);
                let _ = std::fs::remove_file(&partial);
                return Err("cancelled".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                    if !status.success() {
                        let _ = std::fs::remove_dir_all(&work);
                        let _ = std::fs::remove_file(&partial);
                        let tail = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                        return Err(format!(
                            "genisoimage failed{}",
                            if tail.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", tail)
                            }
                        ));
                    }
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_dir_all(&work);
                    let _ = std::fs::remove_file(&partial);
                    return Err(format!("genisoimage error: {}", e));
                }
            }
            let done = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
            if last_sent.elapsed() >= Duration::from_millis(500) {
                last_sent = Instant::now();
                let pct = if total > 0 {
                    (done.saturating_mul(100) / total).min(100)
                } else {
                    0
                };
                let text = if total > 0 {
                    format!(
                        "Building decrypted ISO... {}%  ({:.1} / {:.1} GB)",
                        pct,
                        done as f64 / 1e9,
                        total as f64 / 1e9
                    )
                } else {
                    format!("Building decrypted ISO... {:.1} GB", done as f64 / 1e9)
                };
                let _ = tx.send(Msg::DiscDumpProgress { done, total, text });
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let size = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            let _ = std::fs::remove_dir_all(&work);
            let _ = std::fs::remove_file(&partial);
            return Err("genisoimage produced an empty image".into());
        }
        if let Err(e) = std::fs::rename(&partial, &target) {
            let _ = std::fs::remove_dir_all(&work);
            let _ = std::fs::remove_file(&partial);
            return Err(format!("could not finalize decrypted image: {}", e));
        }
        let _ = std::fs::remove_dir_all(&work);
        Ok(target)
    }
}

/// Decrypt a CSS DVD to a playable ISO. Prefers the bundled `dvdbackup` plus
/// `genisoimage`; falls back to MakeMKV's `backup --decrypt` when those are
/// unavailable or the disc defeats dvdbackup. Either path removes CSS.
pub fn dump_decrypted_dvd(
    device: &str,
    dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    if crate::tools::dvdbackup_path().is_some() && crate::tools::genisoimage_path().is_some() {
        match dump_dvd_with_backup(device, dir, tx, cancel) {
            Ok(path) => return Ok(path),
            Err(e) if e == "cancelled" => return Err(e),
            Err(e) => {
                let _ = tx.send(Msg::DiscDumpProgress {
                    done: 0,
                    total: 0,
                    text: format!("dvdbackup could not finish ({}); trying MakeMKV...", e),
                });
            }
        }
    }
    dump_decrypted_dvd_makemkv(device, dir, tx, cancel)
}

/// Decrypt a CSS DVD to a playable ISO using MakeMKV's `backup` mode.
/// Unlike raw `dd`, this removes CSS so the image plays cleanly in the app or
/// any other player. Requires the physical disc and `makemkvcon`.
pub fn dump_decrypted_dvd_makemkv(
    device: &str,
    dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (device, dir, tx, cancel);
        return Err("decrypted disc imaging is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
    if !Path::new(device).exists() {
        return Err(format!("{} not found", device));
    }
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot create destination folder: {}", e))?;
    let target = decrypted_iso_path(dir, device);
    // MakeMKV treats a destination ending in `.iso` as something other than a
    // backup target, so write to the same name without the extension and
    // rename it once the backup is complete.
    let staging = target.with_extension("");
    let _ = std::fs::remove_file(&staging);
    let total = device_size(device);
    let _ = tx.send(Msg::DiscDumpProgress {
        done: 0,
        total: total.unwrap_or(0),
        text: format!(
            "Decrypting DVD with MakeMKV to {} (this removes CSS)...",
            target.display()
        ),
    });
    let mut cmd = Command::new("makemkvcon");
    // MakeMKV does NOT decrypt backups unless --decrypt is passed; without it
    // the resulting ISO is still encrypted and plays as chunky blocks.
    cmd.args(["-r", "backup", "--decrypt", "disc:0"])
        .arg(&staging)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    let mut child = cmd.spawn().map_err(|e| {
        format!("MakeMKV failed to start (is makemkvcon installed?): {}", e)
    })?;
    let err_handle = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let mut last_sent = Instant::now() - Duration::from_secs(5);
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&staging);
            let _ = std::fs::remove_file(&target);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                if !status.success() {
                    let _ = std::fs::remove_file(&staging);
                    let _ = std::fs::remove_file(&target);
                    let tail = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                    return Err(format!(
                        "MakeMKV failed{}",
                        if tail.is_empty() { String::new() } else { format!(": {}", tail) }
                    ));
                }
                break;
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&staging);
                let _ = std::fs::remove_file(&target);
                return Err(format!("MakeMKV error: {}", e));
            }
        }
        let done = std::fs::metadata(&staging).map(|m| m.len()).unwrap_or(0);
        if last_sent.elapsed() >= Duration::from_millis(500) {
            last_sent = Instant::now();
            let pct = total
                .map(|t| (done.saturating_mul(100) / t).min(100))
                .unwrap_or(0);
            let text = match total {
                Some(t) if t > 0 => format!(
                    "Decrypting DVD... {}%  ({:.1} / {:.1} GB)",
                    pct,
                    done as f64 / 1e9,
                    t as f64 / 1e9
                ),
                _ => format!("Decrypting DVD... {:.1} GB", done as f64 / 1e9),
            };
            let _ = tx.send(Msg::DiscDumpProgress {
                done,
                total: total.unwrap_or(0),
                text,
            });
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let size = std::fs::metadata(&staging).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        let _ = std::fs::remove_file(&staging);
        return Err("MakeMKV produced an empty image".into());
    }
    if let Err(e) = std::fs::rename(&staging, &target) {
        let _ = std::fs::remove_file(&staging);
        return Err(format!("could not finalize decrypted image: {}", e));
    }
    Ok(target)
    }
}

/// Build the Blu-ray image from the decrypted tree using the in-process UDF
/// 2.50 writer. Kept separate so the staging, cancellation and publish steps
/// can be tested without a drive.
fn build_bluray_udf_iso(
    bdmv_root: &Path,
    dir: &Path,
    target: &Path,
    label: &str,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let iso_stage = dir.join(format!(".mpdoom-iso-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&iso_stage);
    std::fs::create_dir_all(&iso_stage)
        .map_err(|e| format!("cannot create ISO staging folder: {}", e))?;
    let staged = iso_stage.join("bluray.iso");
    let mut volume: String = label
        .chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(31)
        .collect();
    if volume.trim().is_empty() {
        volume = "BLURAY".to_string();
    }
    let src_bytes = dir_size(bdmv_root);
    let estimate = src_bytes
        .saturating_add(src_bytes / 20)
        .saturating_add(16 * 1024 * 1024)
        .max(1);
    let options = libmkiso::UdfOptions {
        revision: libmkiso::UdfRevision::V250,
        partition: libmkiso::UdfPartition::Metadata { mirror: true },
        allocation: libmkiso::AllocationMode::Long,
        label: volume,
        ..Default::default()
    };
    let _ = tx.send(Msg::DiscDumpProgress {
        done: 0,
        total: estimate,
        text: format!("Building Blu-ray UDF 2.50 ISO {}...", target.display()),
    });
    let mut last_sent = Instant::now() - Duration::from_secs(5);
    let result = libmkiso::write_udf_with_cancel(
        bdmv_root,
        &staged,
        &options,
        || {
            if cancel.load(Ordering::Relaxed) {
                return Err(anyhow::anyhow!("cancelled"));
            }
            if last_sent.elapsed() >= Duration::from_millis(250) {
                last_sent = Instant::now();
                let done = staging_bytes(&iso_stage).min(estimate);
                let pct = if estimate > 0 {
                    (done.saturating_mul(100) / estimate).min(99)
                } else {
                    0
                };
                let _ = tx.send(Msg::DiscDumpProgress {
                    done,
                    total: estimate,
                    text: format!(
                        "Building Blu-ray ISO... {}%  ({:.1} / {:.1} GB)",
                        pct,
                        done as f64 / 1e9,
                        estimate as f64 / 1e9
                    ),
                });
            }
            Ok(())
        },
    );
    if let Err(e) = result {
        let was_cancel = cancel.load(Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&iso_stage);
        if was_cancel {
            return Err("cancelled".into());
        }
        return Err(format!("Blu-ray UDF 2.50 build failed: {}", e));
    }
    let size = std::fs::metadata(&staged).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        let _ = std::fs::remove_dir_all(&iso_stage);
        return Err("the Blu-ray ISO builder produced an empty image".into());
    }
    let _ = tx.send(Msg::DiscDumpProgress {
        done: estimate,
        total: estimate,
        text: "Blu-ray ISO complete".to_string(),
    });
    if let Err(e) = std::fs::rename(&staged, target) {
        let _ = std::fs::remove_dir_all(&iso_stage);
        return Err(format!("could not finalize Blu-ray image: {}", e));
    }
    let _ = std::fs::remove_dir_all(&iso_stage);
    Ok(())
}

/// Full Blu-ray disc to a clean ISO. MakeMKV's `backup --decrypt` handles
/// both AACS and BD+ (including discs where libaacs has no matching key), then
/// the in-process UDF 2.50 writer wraps the decrypted BDMV tree. Menus, extras
/// and every title are preserved; nothing encrypted is written.
pub fn dump_bluray_iso(
    device: &str,
    dir: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (device, dir, tx, cancel);
        return Err("Blu-ray imaging is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
        if !Path::new(device).exists() {
            return Err(format!("{} not found", device));
        }
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create destination folder: {}", e))?;
        let total = device_size(device).unwrap_or(0);
        if total > 0 {
            if let Some(free) = free_space(dir) {
                let need = total.saturating_mul(23) / 10;
                if free < need {
                    return Err(format!(
                        "not enough free space for the Blu-ray ISO: need about {:.1} GB (decrypted BDMV + ISO), only {:.1} GB available",
                        need as f64 / 1e9,
                        free as f64 / 1e9
                    ));
                }
            }
        }
        let target = decrypted_iso_path(dir, device);
        let work = dir.join(format!(".mpdoom-bd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work)
            .map_err(|e| format!("cannot create working folder: {}", e))?;
        let label = disc_label(device)
            .map(|l| sanitize_win(&l))
            .filter(|l| !l.trim().is_empty())
            .unwrap_or_else(|| "bluray".to_string());

        // ---- stage 1: decrypted full-disc backup (AACS + BD+) ----
        let _ = tx.send(Msg::DiscDumpProgress {
            done: 0,
            total,
            text: format!(
                "Decrypting Blu-ray with MakeMKV (AACS/BD+) to {}...",
                target.display()
            ),
        });
        let mut cmd = Command::new("makemkvcon");
        cmd.args(["-r", "backup", "--decrypt", "disc:0"])
            .arg(&work)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("MakeMKV failed to start (is makemkvcon installed?): {}", e))?;
        let err_handle = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut s = String::new();
                let _ = e.read_to_string(&mut s);
                s
            })
        });
        let mut last_sent = Instant::now() - Duration::from_secs(5);
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&work);
                return Err("cancelled".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                    if !status.success() {
                        let _ = std::fs::remove_dir_all(&work);
                        let tail = err.lines().rev().take(4).collect::<Vec<_>>().join(" | ");
                        return Err(format!(
                            "MakeMKV Blu-ray backup failed{}",
                            if tail.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", tail)
                            }
                        ));
                    }
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = std::fs::remove_dir_all(&work);
                    return Err(format!("MakeMKV error: {}", e));
                }
            }
            let done = dir_size(&work);
            if last_sent.elapsed() >= Duration::from_millis(500) {
                last_sent = Instant::now();
                let pct = if total > 0 {
                    (done.saturating_mul(100) / total).min(100)
                } else {
                    0
                };
                let text = if total > 0 {
                    format!(
                        "Decrypting Blu-ray... {}%  ({:.1} / {:.1} GB)",
                        pct,
                        done as f64 / 1e9,
                        total as f64 / 1e9
                    )
                } else {
                    format!("Decrypting Blu-ray... {:.1} GB", done as f64 / 1e9)
                };
                let _ = tx.send(Msg::DiscDumpProgress { done, total, text });
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        let Some(bdmv_root) = find_bdmv_root(&work) else {
            let _ = std::fs::remove_dir_all(&work);
            return Err("MakeMKV backup produced no BDMV folder".into());
        };

        if let Err(e) = build_bluray_udf_iso(&bdmv_root, dir, &target, &label, tx, cancel) {
            let _ = std::fs::remove_dir_all(&work);
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&work);
        Ok(target)
    }
}

/// Find the folder that directly contains `BDMV` under a MakeMKV backup tree.
fn find_bdmv_root(root: &Path) -> Option<PathBuf> {
    if root.join("BDMV").is_dir() {
        return Some(root.to_path_buf());
    }
    let rd = std::fs::read_dir(root).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("BDMV").is_dir() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makemkv_title_list_picks_longest_title() {
        let text = "TCOUNT:3\nTINFO:0,9,0,\"0:02:11\"\nTINFO:1,9,0,\"2:13:45\"\nTINFO:2,9,0,\"0:31:00\"\nTINFO:1,16,0,\"Main Feature\"\n";
        assert_eq!(parse_makemkv_title_list(text), Some(1));
        assert_eq!(parse_hms("2:13:45"), Some(8025));
        assert_eq!(parse_hms("0:07"), Some(7));
    }

    use super::*;

    #[test]
    fn native_bluray_udf_image_round_trips() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mpdoom-udf-{}", n));
        let src = dir.join("src");
        std::fs::create_dir_all(src.join("BDMV/PLAYLIST")).unwrap();
        std::fs::create_dir_all(src.join("BDMV/STREAM")).unwrap();
        std::fs::write(src.join("BDMV/index.bdmv"), b"index").unwrap();
        std::fs::write(src.join("BDMV/PLAYLIST/00000.mpls"), b"playlist").unwrap();
        std::fs::write(src.join("BDMV/STREAM/00000.m2ts"), vec![7u8; 4096]).unwrap();
        let out = dir.join("out.iso");
        let options = libmkiso::UdfOptions {
            revision: libmkiso::UdfRevision::V250,
            partition: libmkiso::UdfPartition::Metadata { mirror: true },
            allocation: libmkiso::AllocationMode::Long,
            label: "TEST".into(),
            ..Default::default()
        };
        libmkiso::write_udf_with_options(&src, &out, &options).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        let reader = libmkiso::UdfReader::open(&bytes, libmkiso::UdfLimits::default()).unwrap();
        let names: Vec<&str> = reader.entries().iter().map(|e| e.name.as_str()).collect();
        assert!(names.iter().any(|n| n.ends_with("index.bdmv")));
        assert!(names.iter().any(|n| n.ends_with("00000.mpls")));
        assert!(names.iter().any(|n| n.ends_with("00000.m2ts")));
        let idx = reader
            .entries()
            .iter()
            .position(|e| e.name.ends_with("00000.m2ts"))
            .unwrap();
        let payload = reader.read_entry(idx, 8192).unwrap();
        assert_eq!(payload.len(), 4096);
        assert!(payload.iter().all(|b| *b == 7));
        if std::env::var_os("MPDOOM_KEEP_UDF_TEST").is_some() {
            eprintln!("kept {}", out.display());
        } else {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn builds_bluray_iso_and_cleans_staging() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mpdoom-bdiso-{}", n));
        let src = dir.join("src");
        let out_dir = dir.join("out");
        std::fs::create_dir_all(src.join("BDMV/STREAM")).unwrap();
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(src.join("BDMV/index.bdmv"), b"index").unwrap();
        std::fs::write(src.join("BDMV/STREAM/00000.m2ts"), vec![3u8; 8192]).unwrap();
        let target = out_dir.join("disc.iso");
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);
        build_bluray_udf_iso(&src, &out_dir, &target, "TEST", &tx, &cancel).unwrap();
        assert!(target.is_file());
        assert!(std::fs::metadata(&target).unwrap().len() > 0);
        assert!(!out_dir
            .join(format!(".mpdoom-iso-{}", std::process::id()))
            .exists());
        let bytes = std::fs::read(&target).unwrap();
        let reader = libmkiso::UdfReader::open(&bytes, libmkiso::UdfLimits::default()).unwrap();
        assert!(reader
            .entries()
            .iter()
            .any(|e| e.name.ends_with("00000.m2ts")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelled_bluray_iso_leaves_no_output() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mpdoom-bdcancel-{}", n));
        let src = dir.join("src");
        let out_dir = dir.join("out");
        std::fs::create_dir_all(src.join("BDMV")).unwrap();
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(src.join("BDMV/index.bdmv"), b"index").unwrap();
        let target = out_dir.join("disc.iso");
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(true);
        assert_eq!(
            build_bluray_udf_iso(&src, &out_dir, &target, "TEST", &tx, &cancel),
            Err("cancelled".into())
        );
        assert!(!target.exists());
        assert!(!out_dir
            .join(format!(".mpdoom-iso-{}", std::process::id()))
            .exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dumps_a_regular_source_to_iso() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let src = std::env::temp_dir().join(format!("mpdoom-dd-{}.bin", n));
        let dir = std::env::temp_dir().join(format!("mpdoom-out-{}", n));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src, vec![0x5Au8; 1024 * 1024]).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);
        let iso = dump(&src.to_string_lossy(), "DVD", &dir, &tx, &cancel)
            .expect("dd copy should work");
        assert!(iso.is_file());
        assert_eq!(std::fs::metadata(&iso).unwrap().len(), 1024 * 1024);
        let _ = std::fs::remove_file(&iso);
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_removes_partial_image() {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let src = std::env::temp_dir().join(format!("mpdoom-dd-cancel-{}.bin", n));
        let dir = std::env::temp_dir().join(format!("mpdoom-out-cancel-{}", n));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&src, vec![0x5Au8; 8 * 1024 * 1024]).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(true);
        let res = dump(&src.to_string_lossy(), "DVD", &dir, &tx, &cancel);
        assert!(res.is_err(), "cancelled dump should fail");
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 0, "partial image must be removed");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mkv_output_names_are_unique_and_sanitized() {
        let dir = std::env::temp_dir().join(format!("mpdoom-mkv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let a = mkv_output_dir(&dir, "ID4");
        assert_eq!(a, dir.join("ID4 (MKV)"));
        std::fs::create_dir_all(&a).expect("create collision");
        let b = mkv_output_dir(&dir, "ID4");
        assert_ne!(a, b, "existing folder must not be reused");
        assert!(b.to_string_lossy().ends_with("ID4 (MKV) (2)"));
        let empty = mkv_output_dir(&dir, "   ");
        assert_eq!(empty, dir.join("disc (MKV)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dir_size_sums_nested_files() {
        let dir = std::env::temp_dir().join(format!("mpdoom-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("create temp dir");
        std::fs::write(dir.join("a.mkv"), vec![0u8; 100]).unwrap();
        std::fs::write(dir.join("sub/b.mkv"), vec![0u8; 250]).unwrap();
        assert_eq!(dir_size(&dir), 350);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_video_ts_root_in_dvdbackup_layout() {
        let dir = std::env::temp_dir().join(format!("mpdoom-vts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let disc = dir.join("ID4");
        std::fs::create_dir_all(disc.join("VIDEO_TS")).expect("create temp tree");
        assert_eq!(find_video_ts_root(&dir), Some(disc.clone()));
        assert_eq!(find_video_ts_root(&disc), Some(disc.clone()));
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(find_video_ts_root(&empty), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn free_space_reports_a_positive_value() {
        let dir = std::env::temp_dir();
        assert!(free_space(&dir).unwrap_or(0) > 0);
    }

    #[test]
    fn finds_bdmv_root_in_makemkv_backup() {
        let dir = std::env::temp_dir().join(format!("mpdoom-bdmv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let direct = dir.join("direct");
        std::fs::create_dir_all(direct.join("BDMV")).unwrap();
        assert_eq!(find_bdmv_root(&direct), Some(direct.clone()));
        let nested = dir.join("nested");
        std::fs::create_dir_all(nested.join("disc/BDMV")).unwrap();
        assert_eq!(find_bdmv_root(&nested), Some(nested.join("disc")));
        let empty = dir.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(find_bdmv_root(&empty), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn target_names_are_unique_and_sanitized() {
        let kind = "DVD";
        // Use an isolated folder: the real Videos directory is shared with
        // other tests running in parallel, which made this check flaky.
        let dir = std::env::temp_dir().join(format!("mpdoom-disc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let a = target_path_in(&dir, kind, "/dev/does-not-exist");
        let b = target_path_in(&dir, kind, "/dev/does-not-exist");
        assert_eq!(a, b, "same missing device should choose the same free name");
        assert!(a.to_string_lossy().ends_with("dvd.iso"));
        assert!(!a.to_string_lossy().contains('\u{0}'));
        std::fs::write(&a, b"x").expect("create collision");
        let c = target_path_in(&dir, kind, "/dev/does-not-exist");
        assert_ne!(a, c, "existing file must not be overwritten");
        assert!(c.to_string_lossy().ends_with(" (2).iso"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
