#![allow(unused_imports)]
#![allow(dead_code)]
// PipeSource: ffmpeg stdout as a rodio Source (radio + video audio).
use std::collections::VecDeque;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;
use rodio::{Sample, Source};
use crate::consts::NO_WINDOW;
use crate::eq::EqShared;
use crate::tools::ffmpeg_path;
use crate::util::CommandNoWindow;
pub struct PipeSource {
    reader: BufReader<ChildStdout>,
    pending: VecDeque<i16>,
}

impl PipeSource {
    pub fn fill(&mut self) -> Option<()> {
        let mut chunk = [0u8; 4096];
        let n = self.reader.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        let n = n - (n % 2);
        let mut i = 0;
        while i < n {
            self.pending.push_back(i16::from_le_bytes([chunk[i], chunk[i + 1]]));
            i += 2;
        }
        Some(())
    }
}

impl Iterator for PipeSource {
    type Item = i16;
    fn next(&mut self) -> Option<i16> {
        if self.pending.is_empty() {
            self.fill()?;
        }
        self.pending.pop_front()
    }
}

impl rodio::Source for PipeSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        44100
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub fn spawn_radio_stream(url: &str, ffmpeg: &Path) -> Result<(Child, PipeSource), String> {
    let mut child = Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-hide_banner", "-loglevel", "error", "-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .no_window()
        .spawn()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no output")?;
    Ok((
        child,
        PipeSource { reader: BufReader::new(stdout), pending: VecDeque::new() },
    ))
}

pub fn spawn_video_audio(path: &str, ffmpeg: &Path, seek: f32) -> Result<(Child, PipeSource), String> {
    let dvd = crate::dvd::is_dvd(path);
    let bd = crate::bluray::is_bluray(path);
    let bd_playlist = if bd { crate::bluray::playlist(path) } else { None };
    let bd_url;
    let input = if dvd {
        crate::dvd::source(path)
    } else if bd {
        bd_url = format!("bluray:{}", crate::bluray::source(path));
        bd_url.as_str()
    } else {
        path
    };
    let mut args: Vec<String> = vec!["-hide_banner".to_string(), "-loglevel".to_string(), "error".to_string()];
    if dvd {
        args.push("-f".into());
        args.push("dvdvideo".into());
    }
    if let Some(pl) = bd_playlist {
        args.push("-playlist".into());
        args.push(pl.to_string());
    }
    if bd {
        if let Some(ch) = crate::bluray::chapter(path) {
            args.push("-chapter".into());
            args.push(ch.to_string());
        }
    }
    args.push("-re".into());
    if seek > 0.0 {
        args.push("-ss".to_string());
        args.push(format!("{:.3}", seek));
    }
    args.push("-i".into());
    args.push(input.to_string());
    if dvd {
        if let Some(map) = crate::ffmpeg::preferred_audio_map(path) {
            args.push("-map".into());
            args.push(map);
        }
    } else if bd {
        if let Some(map) = crate::bluray::english_audio_map(crate::bluray::source(path)) {
            args.push("-map".into());
            args.push(map);
        }
    }
    args.extend(["-vn", "-sn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"].iter().map(|s| s.to_string()));
    let mut cmd = Command::new(ffmpeg.to_string_lossy().to_string());
    cmd.args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .no_window();
    if bd {
        crate::bluray::configure_command(&mut cmd);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("ffmpeg video audio failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no audio output")?;
    Ok((
        child,
        PipeSource { reader: BufReader::new(stdout), pending: VecDeque::new() },
    ))
}

