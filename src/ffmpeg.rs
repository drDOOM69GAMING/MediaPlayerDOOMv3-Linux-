#![allow(unused_imports)]
#![allow(dead_code)]
// ffmpeg/ffprobe helpers: probing, levels, intro/credits detection, cutting.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use crate::consts::NO_WINDOW;
use crate::tools::*;
use crate::util::*;
pub fn probe_video_duration(path: &str) -> f32 {
    let fp = ffprobe_path();
    if !fp.is_file() {
        return 0.0;
    }
    let dvd = crate::dvd::is_dvd(path);
    let bd = crate::bluray::is_bluray(path);
    let input = if dvd { crate::dvd::source(path) } else { path };
    let mut cmd = std::process::Command::new(fp);
    cmd.args(["-v", "error", "-show_entries", "format=duration", "-of", "default=noprint_wrappers=1:nokey=1"]);
    if dvd {
        cmd.args(["-f", "dvdvideo"]);
    }
    if bd {
        crate::bluray::configure_command(&mut cmd);
    }
    let out = cmd.arg(input).no_window().output();
    match out {
        Ok(o) => std::str::from_utf8(&o.stdout).ok()
            .and_then(|s| s.trim().parse::<f32>().ok())
            .unwrap_or(0.0),
        Err(_) => 0.0,
    }
}

pub fn probe_video_info(path: &str) -> (u32, u32, f32) {
    let fp = ffprobe_path();
    if !fp.is_file() {
        return (0, 0, 0.0);
    }
    let dvd = crate::dvd::is_dvd(path);
    let input = if dvd { crate::dvd::source(path) } else { path };
    let mut cmd = std::process::Command::new(fp);
    cmd.args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,avg_frame_rate", "-of", "default=noprint_wrappers=1:nokey=1"]);
    if dvd {
        cmd.args(["-f", "dvdvideo"]);
    }
    if crate::bluray::is_bluray(path) {
        crate::bluray::configure_command(&mut cmd);
    }
    let out = cmd.arg(input).no_window().output();
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

/// Read chapter markers via ffprobe: `(start_secs, end_secs, title)`.
///
/// Anime/MKV releases frequently ship named chapters ("Opening", "Part A",
/// "Ending"), which are far more reliable than any audio guess.
pub fn probe_chapters(path: &str) -> Vec<(f32, f32, String)> {
    let fp = ffprobe_path();
    if !fp.is_file() {
        return Vec::new();
    }
    let out = std::process::Command::new(fp)
        .args(["-v", "error", "-show_chapters", "-of", "json"])
        .arg(path)
        .no_window()
        .output();
    let Ok(o) = out else { return Vec::new() };
    parse_chapters_json(&String::from_utf8_lossy(&o.stdout))
}

pub fn parse_chapters_json(text: &str) -> Vec<(f32, f32, String)> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let Some(list) = json.get("chapters").and_then(|c| c.as_array()) else { return Vec::new() };
    let mut out = Vec::new();
    for ch in list {
        let start = ch.get("start_time").and_then(|v| v.as_str()).and_then(|s| s.parse::<f32>().ok());
        let end = ch.get("end_time").and_then(|v| v.as_str()).and_then(|s| s.parse::<f32>().ok());
        let title = ch
            .get("tags")
            .and_then(|t| t.get("title"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        if let (Some(start), Some(end)) = (start, end) {
            if end > start {
                out.push((start, end, title));
            }
        }
    }
    out
}

/// Choose intro-end and credits-start from named chapters.
///
/// Only explicit names count: "Opening"/"Intro"/"OP" for the intro, and
/// "Ending"/"Credits"/"ED"/"Outro" for the credits. "Prologue" and "Next
/// Episode Preview" are deliberately ignored.
pub fn bounds_from_chapters(chapters: &[(f32, f32, String)]) -> (f32, f32) {
    let mut intro_end = 0.0f32;
    let mut credits_start = 0.0f32;
    for (start, end, title) in chapters {
        let t = title.trim().to_ascii_lowercase();
        let is_intro = t.contains("opening")
            || t.contains("intro")
            || t == "op"
            || t.starts_with("op ")
            || t.starts_with("opening")
            || t.contains("オープニング");
        let is_credits = t.contains("ending")
            || t.contains("end credits")
            || t.contains("credits")
            || t.contains("outro")
            || t.contains("closing")
            || t == "ed"
            || t.starts_with("ed ")
            || t.contains("エンディング");
        if is_intro && *end > intro_end {
            intro_end = *end;
        }
        if is_credits && *start > 30.0 && (credits_start <= 0.0 || *start < credits_start) {
            credits_start = *start;
        }
    }
    if credits_start > 0.0 && credits_start <= intro_end {
        credits_start = 0.0;
    }
    (intro_end, credits_start)
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
    // Auto intro/credits detection is for local episode files, not DVDs.
    if crate::dvd::is_dvd(path) || crate::bluray::is_bluray(path) {
        return Vec::new();
    }
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
        .no_window();
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

/// Pick a percentile from a slice of dB values (0.0-1.0).
pub fn percentile(v: &[f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((s.len() - 1) as f32 * p.clamp(0.0, 1.0)).round() as usize;
    s[idx]
}

pub fn median_of(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s[s.len() / 2]
}

/// Find long, loud, continuous music runs (the anime OP/ED signature).
/// Returns `(start_secs, end_secs, mean_db)`.
fn loud_runs(
    levels: &[f32],
    secs_per_sample: f32,
    min_secs: f32,
    max_secs: f32,
    above_median: f32,
) -> Vec<(f32, f32, f32)> {
    if levels.len() < 20 || secs_per_sample <= 0.0 {
        return Vec::new();
    }
    let sm = smooth_curve(levels, 4);
    // Use a low percentile as the noise floor rather than the median: on an
    // episode where music occupies more than half the analysed window the
    // median is itself "loud", which used to make every run disappear.
    let floor = percentile(&sm, 0.15);
    let peak = percentile(&sm, 0.90);
    let dynamic = ((peak - floor) * 0.5).clamp(above_median, 20.0);
    let threshold = floor + dynamic;
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < sm.len() {
        if sm[i] >= threshold {
            let start = i;
            while i < sm.len() && sm[i] >= threshold {
                i += 1;
            }
            runs.push((start, i));
        } else {
            i += 1;
        }
    }
    // A song can have a quiet bridge (Dragon Ball ops do); merge runs
    // separated by up to 12 seconds so the theme stays one block.
    let gap = (25.0 / secs_per_sample).round().max(1.0) as usize;
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (a, b) in runs {
        if let Some(last) = merged.last_mut() {
            let gap_len = a.saturating_sub(last.1);
            // Short dropouts (a hard cut or a breath between phrases) always
            // belong to the same block. Longer gaps are only bridged when they
            // are quiet-but-not-silent (an OP's quiet bridge), so real silence
            // still separates the OP from an action scene or the ending theme.
            let short = gap_len <= (6.0 / secs_per_sample).round().max(1.0) as usize;
            let mut bridge = false;
            if !short && gap_len <= gap {
                let gap_levels = &levels[last.1..a];
                let gap_mean = if gap_levels.is_empty() {
                    f32::NEG_INFINITY
                } else {
                    gap_levels.iter().sum::<f32>() / gap_levels.len() as f32
                };
                bridge = gap_mean >= floor + 4.0;
            }
            if short || bridge {
                last.1 = b;
                continue;
            }
        }
        merged.push((a, b));
    }
    merged
        .into_iter()
        .filter_map(|(a, b)| {
            let secs = (b - a) as f32 * secs_per_sample;
            if secs < min_secs || secs > max_secs {
                return None;
            }
            let mean = levels[a..b].iter().sum::<f32>() / (b - a).max(1) as f32;
            Some((a as f32 * secs_per_sample, b as f32 * secs_per_sample, mean))
        })
        .collect()
}

/// Detect where an anime-style opening ends.
///
/// We look for a sustained 40-180 second loud block in roughly the first four
/// minutes and pick the loudest one, not merely the first: a noisy cold open
/// should not win just because it happens earlier. Returns 0.0 when nothing
/// looks confident, in which case no skipping happens.
pub fn detect_intro_end(levels: &[f32], secs_per_sample: f32) -> f32 {
    let window = (420.0 / secs_per_sample).ceil() as usize;
    let n = levels.len().min(window);
    if n < 40 {
        return 0.0;
    }
    let runs = loud_runs(&levels[..n], secs_per_sample, 40.0, 180.0, 4.0);
    // The opening is the *first* qualifying long block. Picking the loudest
    // one could select an action scene or even the ending theme later on.
    for (start, end, _mean) in runs {
        if end >= 45.0 && end <= 240.0 && start <= 180.0 {
            return end;
        }
    }
    0.0
}

/// Detect where the ending/credits start.
///
/// Looks at the last ~7 minutes for a sustained 35-180 second loud block that
/// reaches near the end of the episode, and returns its start. 0.0 means
/// "not confident", so nothing is skipped.
pub fn detect_credit_start(slice: &[f32], secs_per_sample: f32) -> f32 {
    if slice.len() < 40 || secs_per_sample <= 0.0 {
        return 0.0;
    }
    let window = (420.0 / secs_per_sample).ceil() as usize;
    let n = slice.len().min(window);
    let back = &slice[slice.len() - n..];
    let slice_secs = n as f32 * secs_per_sample;
    let runs = loud_runs(back, secs_per_sample, 35.0, 180.0, 3.5);
    let mut best: Option<(f32, f32, f32)> = None; // (start_secs, end_secs, mean)
    for (start, end, mean) in runs {
        // The ED song should still be running in the final three minutes.
        if end < slice_secs - 180.0 {
            continue;
        }
        // Prefer the latest run: the opening theme also matches the loudness
        // test, but the ending is by definition the last one in the episode.
        if best.map_or(true, |(_, best_end, _)| end > best_end) {
            best = Some((start, end, mean));
        }
    }
    best.map(|(start, _, _)| start).unwrap_or(0.0)
}

pub fn cut_video_file(source: &str, intro_end: f32, credits_start: f32) -> Result<String, String> {
    if crate::dvd::is_dvd(source) || crate::bluray::is_bluray(source) {
        return Err("cannot cut an optical disc title with mkvmerge".into());
    }
    let mm = mkvmerge_path();
    if !mm.is_file() {
        return Err("mkvmerge not available (install MKVToolNix)".into());
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
        .no_window()
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

/// Choose an English audio stream for optical discs, falling back to the first
/// audio stream. Returned in ffmpeg `-map 0:a:N` form.
pub fn preferred_audio_map(path: &str) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(m) = cache.lock() {
        if let Some(v) = m.get(path) {
            return Some(v.clone());
        }
    }
    let dvd = crate::dvd::is_dvd(path);
    let bd = crate::bluray::is_bluray(path);
    if !dvd && !bd {
        return None;
    }
    let fp = ffprobe_path();
    if !fp.is_file() {
        return None;
    }
    let input = if dvd { crate::dvd::source(path) } else { path };
    let mut cmd = std::process::Command::new(fp);
    cmd.args(["-v", "error", "-select_streams", "a", "-show_entries", "stream_tags=language", "-of", "json"]);
    if dvd {
        cmd.args(["-f", "dvdvideo"]);
    }
    if bd {
        crate::bluray::configure_command(&mut cmd);
    }
    let out = cmd.arg(input).no_window().output().ok()?;
    if !out.status.success() {
        return None;
    }
    let langs = parse_audio_languages(&String::from_utf8_lossy(&out.stdout))?;
    if langs.is_empty() {
        return None;
    }
    let chosen = langs.iter().position(|l| is_english(l)).unwrap_or(0);
    let map = format!("0:a:{}", chosen);
    if let Ok(mut m) = cache.lock() {
        m.insert(path.to_string(), map.clone());
    }
    Some(map)
}

pub fn parse_audio_languages(json: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let streams = v.get("streams")?.as_array()?;
    Some(
        streams
            .iter()
            .map(|s| {
                s.get("tags")
                    .and_then(|t| t.get("language"))
                    .and_then(|l| l.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
            })
            .collect(),
    )
}

pub fn is_english(lang: &str) -> bool {
    let l = lang.trim().to_ascii_lowercase();
    matches!(l.as_str(), "en" | "eng" | "english" | "en-us" | "en-gb")
}

#[cfg(test)]
mod media_pipeline_tests {
    use super::*;
    use crate::pipe::spawn_video_audio;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch() -> PathBuf {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let d = std::env::temp_dir().join(format!("mediaplayerofdoom-test-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Generate a tiny MP4 with video + audio using the same ffmpeg the app uses.
    fn make_video(dir: &Path) -> PathBuf {
        let out = dir.join("sample.mp4");
        let st = Command::new(ffmpeg_path())
            .args([
                "-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "testsrc=size=320x240:rate=24:duration=3",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=3",
                "-c:v", "mpeg4", "-c:a", "aac", "-shortest",
            ])
            .arg(&out)
            .status()
            .expect("ffmpeg should start");
        assert!(st.success(), "ffmpeg failed to generate test media");
        out
    }

    fn envelope(parts: &[(f32, f32)]) -> Vec<f32> {
        // (seconds, dB) pairs -> 0.5 s samples with a little natural jitter.
        let mut out = Vec::new();
        for (i, (secs, db)) in parts.iter().enumerate() {
            let n = (*secs / 0.5) as usize;
            for k in 0..n {
                let jitter = ((k % 7) as f32 - 3.0) * 0.3 + (i as f32 * 0.05);
                out.push(*db + jitter);
            }
        }
        out
    }

    #[test]
    fn detects_an_anime_style_opening_theme() {
        let levels = envelope(&[(60.0, -45.0), (90.0, -12.0), (150.0, -45.0)]);
        let end = detect_intro_end(&levels, 0.5);
        assert!((end - 150.0).abs() <= 2.0, "intro end was {end}");
    }

    #[test]
    fn intro_prefers_the_first_music_block_over_a_later_one() {
        let levels = envelope(&[(10.0, -45.0), (50.0, -12.0), (10.0, -45.0), (50.0, -12.0), (100.0, -45.0)]);
        let end = detect_intro_end(&levels, 0.5);
        assert!((end - 60.0).abs() <= 2.0, "intro end was {end}, should be the first block");
    }

    #[test]
    fn detects_an_ending_theme_near_the_end() {
        let levels = envelope(&[(420.0, -45.0), (90.0, -12.0), (90.0, -45.0)]);
        // lib_thread analyses the last 420 s and adds back_start afterwards.
        let start = detect_credit_start(&levels, 0.5);
        assert!((start - 240.0).abs() <= 2.0, "credit start was {start}");
    }

    #[test]
    fn detection_stays_quiet_when_there_is_no_music_block() {
        let levels = envelope(&[(600.0, -42.0)]);
        assert_eq!(detect_intro_end(&levels, 0.5), 0.0, "must not invent an intro");
        assert_eq!(detect_credit_start(&levels, 0.5), 0.0, "must not invent credits");
    }

    #[test]
    fn chapters_identify_opening_and_ending() {
        let ch = vec![
            (0.0, 108.65, "Opening".to_string()),
            (108.65, 191.92, "Prologue".to_string()),
            (191.92, 1351.559, "Part A".to_string()),
            (1351.559, 1448.656, "Ending".to_string()),
            (1448.656, 1478.978, "Next Episode Preview".to_string()),
        ];
        assert_eq!(bounds_from_chapters(&ch), (108.65, 1351.559));
    }

    #[test]
    fn chapters_ignore_unrelated_titles() {
        let ch = vec![
            (0.0, 30.0, "Prologue".to_string()),
            (1200.0, 1400.0, "Next Episode Preview".to_string()),
        ];
        assert_eq!(bounds_from_chapters(&ch), (0.0, 0.0));
    }

    #[test]
    fn parses_ffprobe_chapter_json() {
        let text = r#"{"chapters":[
          {"start_time":"0.000000","end_time":"108.650000","tags":{"title":"Opening"}},
          {"start_time":"1351.559000","end_time":"1448.656000","tags":{"title":"Ending"}}
        ]}"#;
        let ch = parse_chapters_json(text);
        assert_eq!(ch.len(), 2);
        assert_eq!(bounds_from_chapters(&ch), (108.65, 1351.559));
    }

    #[test]
    fn preferred_audio_parser_picks_english() {
        let json = r#"{"streams":[
            {"tags":{"language":"fre"}},
            {"tags":{"language":"eng"}},
            {"tags":{"language":"spa"}}
        ]}"#;
        let langs = parse_audio_languages(json).unwrap();
        assert_eq!(langs.iter().position(|l| is_english(l)), Some(1));
        assert_eq!(parse_audio_languages(r#"{"streams":[]}"#).unwrap().len(), 0);
        assert!(is_english("ENG"));
        assert!(!is_english("fre"));
    }

    #[test]
    fn helper_tools_resolve_on_this_platform() {
        assert!(ffmpeg_path().is_file(), "ffmpeg not found: use PATH or the tools folder");
        assert!(ffprobe_path().is_file(), "ffprobe not found: use PATH or the tools folder");
        assert_ne!(ffmpeg_path(), ffprobe_path());
    }

    #[test]
    fn probes_a_real_video() {
        let dir = scratch();
        let v = make_video(&dir);
        let (w, h, fps) = probe_video_info(&v.to_string_lossy());
        assert_eq!((w, h), (320, 240));
        assert!((fps - 24.0).abs() < 0.5, "fps was {fps}");
        let dur = probe_video_duration(&v.to_string_lossy());
        assert!((dur - 3.0).abs() < 0.3, "duration was {dur}");
        let levels = video_audio_levels(&v.to_string_lossy(), &ffmpeg_path(), 0.0, 2.0);
        assert!(!levels.is_empty());
        assert!(levels.iter().any(|db| *db > -60.0), "tone was not detected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flac_audio_pipes_decoded_samples_for_fast_seek() {
        let dir = scratch();
        let wav = dir.join("tone.wav");
        let flac = dir.join("tone.flac");
        let st = Command::new(ffmpeg_path())
            .args(["-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
            .arg(&wav)
            .status().expect("ffmpeg");
        assert!(st.success());
        let st = Command::new(ffmpeg_path())
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&wav)
            .args(["-c:a", "flac"]).arg(&flac)
            .status().expect("ffmpeg");
        assert!(st.success());
        let (mut child, mut pipe) = spawn_video_audio(&flac.to_string_lossy(), &ffmpeg_path(), 1.0)
            .expect("audio pipe should start");
        let mut got = 0usize;
        for _ in 0..(44100 / 10 * 2) {
            match pipe.next() {
                Some(_) => got += 1,
                None => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(got > 100, "only got {got} samples");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn video_audio_pipes_decoded_samples() {
        let dir = scratch();
        let v = make_video(&dir);
        let (mut child, mut pipe) = spawn_video_audio(&v.to_string_lossy(), &ffmpeg_path(), 0.0)
            .expect("audio pipe should start");
        let mut got = 0usize;
        for _ in 0..(44100 / 10 * 2) {
            match pipe.next() {
                Some(_) => got += 1,
                None => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(got > 100, "only got {got} audio samples");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cuts_a_video_with_mkvmerge_when_available() {
        if !mkvmerge_path().is_file() {
            eprintln!("mkvmerge not installed, skipping cut test");
            return;
        }
        let dir = scratch();
        let v = make_video(&dir);
        let cut = cut_video_file(&v.to_string_lossy(), 1.0, 2.0).expect("cut should succeed");
        assert!(Path::new(&cut).is_file());
        assert!(std::fs::metadata(&cut).unwrap().len() > 1024);
        let _ = std::fs::remove_file(&cut);
        let _ = std::fs::remove_dir_all(&dir);
    }
}


#[cfg(test)]
mod level_debug {
    use super::*;
    #[test]
    #[ignore]
    fn debug_levels() {
        let p = std::env::var("MPDOOM_TEST_MEDIA").expect("set MPDOOM_TEST_MEDIA");
        let dur = probe_video_duration(&p);
        let front_len = dur.min(420.0);
        let front = video_audio_levels(&p, &ffmpeg_path(), 0.0, front_len);
        let intro = detect_intro_end(&front, 0.5);
        let back_len = dur.min(420.0);
        let back_start = (dur - back_len).max(0.0);
        let back = video_audio_levels(&p, &ffmpeg_path(), back_start, back_len);
        let rel = detect_credit_start(&back, 0.5);
        let credits = if rel > 0.0 { back_start + rel } else { 0.0 };
        eprintln!("file: {p}");
        eprintln!("duration={:.1}s front_len={:.1} back_start={:.1}", dur, front_len, back_start);
        for (label, l) in [("front", &front), ("back", &back)] {
            eprintln!(
                "{label}: n={} p15={:.1} p50={:.1} p90={:.1} p99={:.1}",
                l.len(), percentile(l, 0.15), percentile(l, 0.5), percentile(l, 0.9), percentile(l, 0.99)
            );
        }
        let (ci, cc) = bounds_from_chapters(&probe_chapters(&p));
        eprintln!("CHAPTERS intro_end={:.1}s credits_start={:.1}s", ci, cc);
        eprintln!("DETECTED intro_end={:.1}s credits_start={:.1}s", intro, credits);
        let runs = loud_runs(&front, 0.5, 30.0, 200.0, 4.0);
        eprintln!("front loud runs (start,end,mean): {:?}", runs);
    }
}
