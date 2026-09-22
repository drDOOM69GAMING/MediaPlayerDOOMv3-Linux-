#![allow(unused_imports)]
#![allow(dead_code)]
// PipeSource: ffmpeg stdout as a rodio Source (radio + video audio).
use std::collections::VecDeque;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;
use std::os::windows::process::CommandExt;
use rodio::{Sample, Source};
use crate::consts::NO_WINDOW;
use crate::eq::EqShared;
use crate::tools::ffmpeg_path;
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
        .args(["-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"])
        .stdout(std::process::Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no output")?;
    Ok((
        child,
        PipeSource { reader: BufReader::new(stdout), pending: VecDeque::new() },
    ))
}

pub fn spawn_video_audio(path: &str, ffmpeg: &Path, seek: f32) -> Result<(Child, PipeSource), String> {
    let mut args: Vec<String> = vec!["-re".to_string()];
    if seek > 0.0 {
        args.push("-ss".to_string());
        args.push(format!("{:.3}", seek));
    }
    args.extend(["-i", path, "-vn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"].iter().map(|s| s.to_string()));
    let mut child = Command::new(ffmpeg.to_string_lossy().to_string())
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("ffmpeg video audio failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no audio output")?;
    Ok((
        child,
        PipeSource { reader: BufReader::new(stdout), pending: VecDeque::new() },
    ))
}

