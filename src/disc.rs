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

fn disc_label(device: &str) -> Option<String> {
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

fn device_size(device: &str) -> Option<u64> {
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

/// Decrypt a CSS DVD to a playable ISO using MakeMKV's `backup` mode.
/// Unlike raw `dd`, this removes CSS so the image plays cleanly in the app or
/// any other player. Requires the physical disc and `makemkvcon`.
pub fn dump_decrypted_dvd(
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

#[cfg(test)]
mod tests {
    use super::*;

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
