// Whole-disc imaging (DVD / Blu-ray) to an ISO file.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::Duration;

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
    let label = disc_label(device)
        .map(|l| sanitize_win(&l))
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| kind.to_ascii_lowercase());
    let base = videos_dir().join(format!("{}.iso", label));
    if !base.exists() {
        return base;
    }
    for n in 2..1000 {
        let p = videos_dir().join(format!("{} ({}).iso", label, n));
        if !p.exists() {
            return p;
        }
    }
    base
}

fn device_size(device: &str) -> Option<u64> {
    let out = Command::new("blockdev")
        .args(["--getsize64", device])
        .output()
        .ok()?;
    if out.status.success() {
        return String::from_utf8_lossy(&out.stdout).trim().parse::<u64>().ok().filter(|s| *s > 0);
    }
    None
}

/// Copy all sectors of `device` to `target` using dd. This preserves the whole
/// disc, including menus and extras.
pub fn dump(device: &str, kind: &str, tx: &Sender<Msg>) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let _ = (device, kind, tx);
        return Err("disc imaging is only supported on Linux".into());
    }
    #[cfg(not(windows))]
    {
    if !Path::new(device).exists() {
        return Err(format!("{} not found", device));
    }
    let target = target_path(kind, device);
    let total = device_size(device);
    let _ = tx.send(Msg::DiscDumpProgress(format!(
        "Dumping {} to {}...",
        device,
        target.display()
    )));
    let mut child = Command::new("dd")
        .arg(format!("if={}", device))
        .arg(format!("of={}", target.to_string_lossy()))
        .args(["bs=1M", "conv=noerror,sync"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
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
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let err = err_handle
                    .and_then(|h| h.join().ok())
                    .unwrap_or_default();
                if !status.success() {
                    let _ = std::fs::remove_file(&target);
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
                let _ = std::fs::remove_file(&target);
                return Err(format!("dd error: {}", e));
            }
        }
        if let (Some(total), Ok(meta)) = (total, std::fs::metadata(&target)) {
            if total > 0 {
                let pct = (meta.len().saturating_mul(100) / total).min(100);
                if pct != last_pct && (pct % 5 == 0 || pct >= 99) {
                    last_pct = pct;
                    let _ = tx.send(Msg::DiscDumpProgress(format!(
                        "Dumping {}... {}%  ({:.1} / {:.1} GB)",
                        kind,
                        pct,
                        meta.len() as f64 / 1e9,
                        total as f64 / 1e9
                    )));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        let _ = std::fs::remove_file(&target);
        return Err("dd produced an empty image".into());
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
        std::fs::write(&src, vec![0x5Au8; 1024 * 1024]).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let iso = dump(&src.to_string_lossy(), "DVD", &tx).expect("dd copy should work");
        assert!(iso.is_file());
        assert_eq!(std::fs::metadata(&iso).unwrap().len(), 1024 * 1024);
        let _ = std::fs::remove_file(&iso);
        let _ = std::fs::remove_file(&src);
    }

    #[test]
    fn target_names_are_unique_and_sanitized() {
        let kind = "DVD";
        let a = target_path(kind, "/dev/does-not-exist");
        let b = target_path(kind, "/dev/does-not-exist");
        assert_eq!(a, b, "same missing device should choose the same free name");
        assert!(a.to_string_lossy().ends_with(".iso"));
        assert!(!a.to_string_lossy().contains('\u{0}'));
    }
}
