#![allow(unused_imports)]
#![allow(dead_code)]
// ffmpeg/ffprobe helpers: probing, levels, intro/credits detection, cutting.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::os::windows::process::CommandExt;
use crate::consts::NO_WINDOW;
use crate::tools::*;
use crate::util::*;
pub fn probe_video_duration(path: &str) -> f32 {
    let fp = tools_dir().join("ffprobe.exe");
    if !fp.is_file() {
        return 0.0;
    }
    let out = std::process::Command::new(fp)
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(path)
        .creation_flags(NO_WINDOW)
        .output();
    match out {
        Ok(o) => std::str::from_utf8(&o.stdout).ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(0.0),
        Err(_) => 0.0,
    }
}

pub fn probe_video_info(path: &str) -> (u32, u32, f32) {
    let fp = tools_dir().join("ffprobe.exe");
    if !fp.is_file() {
        return (0, 0, 0.0);
    }
    let out = std::process::Command::new(fp)
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,avg_frame_rate", "-of", "default=noprint_wrappers=1:nokey=1"])
        .arg(path)
        .creation_flags(NO_WINDOW)
        .output();
    match out {
        Ok(o) => {
            let lines: Vec<&str> = std::str::from_utf8(&o.stdout).unwrap_or("")
                .lines().map(|s| s.trim()).collect();
            let w = lines.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let h = lines.get(1).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
            let fps = lines.get(2).and_then(|s| {
                if let Some((n, d)) = s.split_once('/') {
                    let n: f32 = n.trim().parse().ok()?;
                    let d: f32 = d.trim().parse().ok()?;
                    if d > 0.0 { Some(n / d) } else { None }
                } else {
                    s.parse::<f32>().ok().filter(|f| *f > 0.0)
                }
            }).unwrap_or(0.0);
            if w == 0 || h == 0 { (0, 0, fps) } else { (w, h, fps) }
        }
        Err(_) => (0, 0, 0.0),
    }
}

pub fn fit_video_dims(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || max_w == 0 || max_h == 0 {
        return (640, 360);
    }
    let mut tw = w;
    let mut th = h;
    let ws = max_w as f32 / tw as f32;
    let hs = max_h as f32 / th as f32;
    let s = ws.min(hs);
    if s < 1.0 {
        tw = ((tw as f32) * s) as u32;
        th = ((th as f32) * s) as u32;
    }
    if tw % 2 != 0 { tw = tw.saturating_sub(1); }
    if th % 2 != 0 { th = th.saturating_sub(1); }
    (tw.max(2), th.max(2))
}

pub fn video_audio_levels(path: &str, ffmpeg: &Path, start: f32, len: f32) -> Vec<f32> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    if start > 0.0 {
        cmd.args(["-ss"]).arg(format!("{:.2}", start));
    }
    cmd.arg("-i").arg(path)
        .args(["-t"]).arg(format!("{:.2}", len))
        .args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(NO_WINDOW);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    let Some(mut stdout) = child.stdout.take() else { return vec![] };
    let mut out: Vec<f32> = Vec::new();
    let mut buf = vec![0u8; 8000];
    loop {
        let mut filled = 0;
        while filled < buf.len() {
            match stdout.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(_) => break,
            }
        }
        if filled < 2 {
            break;
        }
        let samples = filled / 2;
        let mut sum = 0.0f64;
        for i in 0..samples {
            let s = i16::from_le_bytes([buf[i * 2], buf[i * 2 + 1]]) as f64 / 32768.0;
            sum += s * s;
        }
        let rms = (sum / samples as f64).sqrt();
        let db = if rms > 1e-9 { 20.0 * rms.log10() } else { -120.0 };
        out.push(db as f32);
        if filled < buf.len() {
            break;
        }
    }
    let _ = child.wait();
    out
}

pub fn smooth_curve(v: &[f32], w: usize) -> Vec<f32> {
    let mut s = vec![0.0; v.len()];
    for i in 0..v.len() {
        let lo = i.saturating_sub(w);
        let hi = (i + w).min(v.len());
        let cnt = hi - lo;
        if cnt > 0 {
            s[i] = v[lo..hi].iter().sum::<f32>() / cnt as f32;
        }
    }
    s
}

pub fn median_of(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[s.len() / 2]
}

pub fn detect_intro_end(levels: &[f32], secs_per_sample: f32) -> f32 {
    if levels.len() < 40 {
        return 0.0;
    }
    let n = (levels.len() as f32 * 0.6).min(levels.len() as f32) as usize;
    let sm = smooth_curve(&levels[..n], 4);
    let med = median_of(&sm);
    let active = med + 3.5;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < sm.len() {
        if sm[i] >= active {
            let s = i;
            while i < sm.len() && sm[i] >= active {
                i += 1;
            }
            runs.push((s, i));
        } else {
            i += 1;
        }
    }
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in runs {
        if let Some(last) = merged.last_mut() {
            if s as isize - last.1 as isize <= 4 {
                last.1 = e;
                continue;
            }
        }
        merged.push((s, e));
    }
    for (s, e) in merged {
        let len_secs = (e - s) as f32 * secs_per_sample;
        let end_secs = e as f32 * secs_per_sample;
        if len_secs >= 15.0 && end_secs >= 35.0 {
            return end_secs;
        }
    }
    0.0
}

pub fn detect_credit_start(slice: &[f32], secs_per_sample: f32) -> f32 {
    if slice.len() < 30 {
        return 0.0;
    }
    let sm = smooth_curve(slice, 4);
    let med = median_of(&sm);
    let active = med + 3.0;
    let mut i = sm.len();
    while i > 0 {
        if sm[i - 1] >= active {
            let mut s = i - 1;
            while s > 0 && sm[s - 1] >= active {
                s -= 1;
            }
            let len = (i - s) as f32 * secs_per_sample;
            if len >= 20.0 {
                return s as f32 * secs_per_sample;
            }
            i = s;
        } else {
            i -= 1;
        }
    }
    0.0
}

pub fn cut_video_file(source: &str, intro_end: f32, credits_start: f32) -> Result<String, String> {
    let mm = mkvmerge_path();
    if !mm.is_file() {
        return Err("mkvmerge.exe not available".into());
    }
    let src = Path::new(source);
    let stem = src.file_stem().unwrap_or_default().to_string_lossy();
    let parent = work_dir().join("cuts");
    if let Err(e) = std::fs::create_dir_all(&parent) {
        return Err(format!("cannot create temp cut dir: {}", e));
    }
    for stale in std::fs::read_dir(&parent).ok().into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(stale.path());
    }
    let cut_path = parent.join(format!("{}_cut.mkv", stem));
    let mut args: Vec<String> = Vec::new();
    args.push("--no-date".into());
    args.push("--no-track-tags".into());
    args.push("-o".into());
    args.push(cut_path.to_string_lossy().to_string());
    args.push("--split".into());
    let ie = intro_end.max(0.0) as u64;
    let cs = credits_start.max(0.0) as u64;
    let fmt_ts = |s: u64| format!("{:02}:{:02}:{:02}.000", s / 3600, (s % 3600) / 60, s % 60);
    let spec = if ie > 0 && (cs > 0 && cs > ie) {
        format!("parts:{}-{}", fmt_ts(ie), fmt_ts(cs))
    } else if ie > 0 {
        format!("parts:{}-", fmt_ts(ie))
    } else if cs > 0 {
        format!("parts:-{}", fmt_ts(cs))
    } else {
        return Err("no cut range".into());
    };
    args.push(spec);
    args.push(source.to_string());
    let out = std::process::Command::new(mm)
        .creation_flags(NO_WINDOW)
        .args(&args)
        .output()
        .map_err(|e| format!("mkvmerge failed to start: {}", e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("mkvmerge failed: {}", stderr.chars().take(200).collect::<String>()));
    }
    if !cut_path.is_file() {
        if let Some(found) = parent.read_dir().ok().and_then(|rd| {
            rd.flatten().find(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with(&format!("{}_cut", stem)) && n.ends_with(".mkv")
            })
        }) {
            return Ok(found.path().to_string_lossy().to_string());
        }
        return Err("mkvmerge finished but cut file not found".into());
    }
    Ok(cut_path.to_string_lossy().to_string())
}

