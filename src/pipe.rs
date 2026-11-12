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
    reader: BufReader<Box<dyn std::io::Read + Send>>,
    pending: VecDeque<i16>,
}

impl PipeSource {
    pub fn from_reader(reader: Box<dyn std::io::Read + Send>) -> Self {
        Self { reader: BufReader::new(reader), pending: VecDeque::new() }
    }

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

/// FIFO used by the single-reader libmmbd path: one ffmpeg decrypts the disc
/// once and writes video to stdout and PCM audio to this FIFO.
pub fn mmbd_audio_path(gen: u64) -> std::path::PathBuf {
    crate::util::data_dir().join(format!("mmbd-audio-{}.pcm", gen))
}

#[cfg(unix)]
pub fn ensure_fifo(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
    if rc == 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::EEXIST) {
        Ok(())
    } else {
        Err(err)
    }
}

#[cfg(not(unix))]
pub fn ensure_fifo(_path: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::other("FIFOs are not supported on this platform"))
}

/// Remove leftover FIFOs from previous sessions. Open FIFOs stay valid for any
/// process still using them; only the name is unlinked.
pub fn cleanup_mmbd_audio() {
    let dir = crate::util::data_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("mmbd-audio-") && name.ends_with(".pcm") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

pub fn spawn_radio_stream(url: &str, ffmpeg: &Path) -> Result<(Child, PipeSource), String> {
    let mut child = Command::new(ffmpeg.to_string_lossy().to_string())
        .args(["-hide_banner", "-loglevel", "error", "-i", url, "-vn", "-ac", "2", "-ar", "44100", "-f", "s16le", "-"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .no_window()
        .die_with_parent()
        .spawn()
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no output")?;
    Ok((
        child,
        PipeSource::from_reader(Box::new(stdout)),
    ))
}

pub fn spawn_video_audio(path: &str, ffmpeg: &Path, seek: f32) -> Result<(Child, PipeSource), String> {
    spawn_video_audio_mapped(path, ffmpeg, seek, None)
}

pub fn spawn_video_audio_mapped(
    path: &str,
    ffmpeg: &Path,
    seek: f32,
    audio_map: Option<&str>,
) -> Result<(Child, PipeSource), String> {
    let dvd = crate::dvd::is_dvd(path);
    let bd = crate::bluray::is_bluray(path);
    let bd_playlist = if bd { crate::bluray::playlist(path) } else { None };
    // libbluray can seek instantly even though ffmpeg's bluray: protocol
    // cannot. Use our streamer when the playlist is known.
    let bd_stream = if bd {
        bd_playlist.and_then(|pl| {
            crate::bdstream::spawn_stream(crate::bluray::source(path), pl, seek)
        })
    } else {
        None
    };
    let using_stream = bd_stream.is_some();
    if using_stream {
        crate::log::av_log(&format!(
            "audio: libbluray stream playlist={} seek={:.1}s",
            bd_playlist.unwrap_or(0),
            seek
        ));
    }
    let bd_url;
    let input = if using_stream {
        "pipe:0"
    } else if dvd {
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
    if using_stream {
        args.push("-f".into());
        args.push("mpegts".into());
    } else {
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
    }
    args.push("-re".into());
    // The streamer already starts at the requested time; seeking a pipe is
    // not possible, so only ffmpeg's own sources get -ss.
    if seek > 0.0 && !using_stream {
        args.push("-ss".to_string());
        args.push(format!("{:.3}", seek));
    }
    args.push("-i".into());
    args.push(input.to_string());
    if let Some(map) = audio_map {
        args.push("-map".into());
        args.push(map.to_string());
    } else if dvd {
        if let Some(map) = crate::ffmpeg::preferred_audio_map(path) {
            args.push("-map".into());
            args.push(map);
        }
    } else if bd && !crate::bluray::mmbd_active() {
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
    if let Some(out) = bd_stream {
        cmd.stdin(std::process::Stdio::from(out));
    }
    if bd {
        crate::bluray::configure_command(&mut cmd);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("ffmpeg video audio failed to start: {}", e))?;
    let stdout = child.stdout.take().ok_or("ffmpeg gave no audio output")?;
    Ok((
        child,
        PipeSource::from_reader(Box::new(stdout)),
    ))
}

