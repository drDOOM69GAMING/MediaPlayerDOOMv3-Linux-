// Folder music conversion (MP3 / AAC / FLAC / Vorbis / Opus / WAV / AIFF /
// ALAC / WMA). The user picks a source folder and a 1-10 quality/compression
// level; finished files go to the OS music folder.
#![allow(dead_code)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::files::collect_audio_cancellable;
use crate::tools::{ffmpeg_path, ffprobe_path};
use crate::types::Msg;
use crate::util::home_dir;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConvertFormat {
    Same,
    Mp3,
    Aac,
    Flac,
    Vorbis,
    Opus,
    Wav,
    Aiff,
    Alac,
    Wma,
}

impl ConvertFormat {
    pub const ALL: [ConvertFormat; 10] = [
        ConvertFormat::Same,
        ConvertFormat::Mp3,
        ConvertFormat::Aac,
        ConvertFormat::Flac,
        ConvertFormat::Vorbis,
        ConvertFormat::Opus,
        ConvertFormat::Wav,
        ConvertFormat::Aiff,
        ConvertFormat::Alac,
        ConvertFormat::Wma,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ConvertFormat::Same => "Same as source",
            ConvertFormat::Mp3 => "MP3",
            ConvertFormat::Aac => "AAC (M4A)",
            ConvertFormat::Flac => "FLAC",
            ConvertFormat::Vorbis => "OGG Vorbis",
            ConvertFormat::Opus => "Opus",
            ConvertFormat::Wav => "WAV",
            ConvertFormat::Aiff => "AIFF",
            ConvertFormat::Alac => "ALAC (M4A)",
            ConvertFormat::Wma => "WMA",
        }
    }

    fn dest_label(self) -> &'static str {
        match self {
            ConvertFormat::Same => "same format",
            other => other.label(),
        }
    }

    fn folder_label(self) -> &'static str {
        match self {
            ConvertFormat::Same => "converted",
            ConvertFormat::Mp3 => "MP3",
            ConvertFormat::Aac => "AAC",
            ConvertFormat::Flac => "FLAC",
            ConvertFormat::Vorbis => "OGG",
            ConvertFormat::Opus => "OPUS",
            ConvertFormat::Wav => "WAV",
            ConvertFormat::Aiff => "AIFF",
            ConvertFormat::Alac => "ALAC",
            ConvertFormat::Wma => "WMA",
        }
    }

    pub fn quality_label(self, q: u8) -> String {
        let q = q.clamp(1, 10);
        match self {
            ConvertFormat::Same => format!("{} / 10 (per source format)", q),
            ConvertFormat::Mp3
            | ConvertFormat::Aac
            | ConvertFormat::Wma => format!("{} kbps", lossy_bitrate(q)),
            ConvertFormat::Opus => format!("{} kbps", opus_bitrate(q)),
            ConvertFormat::Vorbis => format!("q {}", q),
            ConvertFormat::Flac => {
                format!("compression level {} (lossless)", flac_level(q))
            }
            ConvertFormat::Wav | ConvertFormat::Aiff => {
                format!("{}-bit PCM (no lossy compression)", pcm_bits(q))
            }
            ConvertFormat::Alac => format!(
                "{}-bit ALAC (lossless)",
                if q <= 5 { 16 } else { 24 }
            ),
        }
    }

    /// Formats whose muxer can carry an attached cover picture.
    fn supports_art(self) -> bool {
        matches!(
            self,
            ConvertFormat::Mp3 | ConvertFormat::Aac | ConvertFormat::Flac | ConvertFormat::Alac
        )
    }

    fn extension(self) -> &'static str {
        match self {
            ConvertFormat::Same | ConvertFormat::Flac => "flac",
            ConvertFormat::Mp3 => "mp3",
            ConvertFormat::Aac | ConvertFormat::Alac => "m4a",
            ConvertFormat::Vorbis => "ogg",
            ConvertFormat::Opus => "opus",
            ConvertFormat::Wav => "wav",
            ConvertFormat::Aiff => "aiff",
            ConvertFormat::Wma => "wma",
        }
    }

    /// Resolve "same as source" to an encoder. `.m4a` is ambiguous (AAC vs
    /// ALAC), so it is probed with ffprobe; everything else is decided by the
    /// extension.
    fn resolved(self, source: &str) -> ConvertFormat {
        if self != ConvertFormat::Same {
            return self;
        }
        let ext = Path::new(source)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "mp3" => ConvertFormat::Mp3,
            "flac" => ConvertFormat::Flac,
            "ogg" | "oga" => ConvertFormat::Vorbis,
            "opus" => ConvertFormat::Opus,
            "wav" | "wave" => ConvertFormat::Wav,
            "aif" | "aiff" | "aifc" => ConvertFormat::Aiff,
            "wma" | "asf" => ConvertFormat::Wma,
            "m4a" | "mp4" => match probe_codec(source).as_deref() {
                Some("alac") => ConvertFormat::Alac,
                _ => ConvertFormat::Aac,
            },
            "aac" => ConvertFormat::Aac,
            "alac" => ConvertFormat::Alac,
            _ => ConvertFormat::Flac,
        }
    }

    fn codec_args(self, quality: u8) -> Vec<String> {
        let q = quality.clamp(1, 10);
        let mut a: Vec<String> = Vec::new();
        let s = |v: &str| v.to_string();
        match self {
            ConvertFormat::Same => return ConvertFormat::Flac.codec_args(q),
            ConvertFormat::Mp3 => {
                a.extend(["-c:a", "libmp3lame", "-b:a"].iter().map(|v| s(v)));
                a.push(format!("{}k", lossy_bitrate(q)));
                a.extend(["-id3v2_version", "3"].iter().map(|v| s(v)));
            }
            ConvertFormat::Aac => {
                a.extend(["-c:a", "aac", "-b:a"].iter().map(|v| s(v)));
                a.push(format!("{}k", lossy_bitrate(q)));
                a.extend(["-movflags", "+faststart"].iter().map(|v| s(v)));
            }
            ConvertFormat::Flac => {
                a.extend(["-c:a", "flac", "-compression_level"].iter().map(|v| s(v)));
                a.push(flac_level(q).to_string());
            }
            ConvertFormat::Vorbis => {
                a.extend(["-c:a", "libvorbis", "-q:a"].iter().map(|v| s(v)));
                a.push(q.to_string());
            }
            ConvertFormat::Opus => {
                a.extend(["-c:a", "libopus", "-b:a"].iter().map(|v| s(v)));
                a.push(format!("{}k", opus_bitrate(q)));
                a.extend(["-vbr", "on"].iter().map(|v| s(v)));
            }
            ConvertFormat::Wav => {
                a.push("-c:a".into());
                a.push(format!("pcm_s{}le", pcm_bits(q)));
            }
            ConvertFormat::Aiff => {
                a.push("-c:a".into());
                a.push(format!("pcm_s{}be", pcm_bits(q)));
            }
            ConvertFormat::Alac => {
                a.extend(["-c:a", "alac", "-sample_fmt"].iter().map(|v| s(v)));
                a.push(if q <= 5 { "s16p".into() } else { "s32p".to_string() });
                a.extend(["-movflags", "+faststart"].iter().map(|v| s(v)));
            }
            ConvertFormat::Wma => {
                a.extend(["-c:a", "wmav2", "-b:a"].iter().map(|v| s(v)));
                a.push(format!("{}k", lossy_bitrate(q)));
            }
        }
        a
    }
}

fn lossy_bitrate(q: u8) -> u32 {
    96 + (q.clamp(1, 10) - 1) as u32 * 24
}

fn opus_bitrate(q: u8) -> u32 {
    64 + (q.clamp(1, 10) - 1) as u32 * 21
}

fn flac_level(q: u8) -> u32 {
    (((q.clamp(1, 10) - 1) as f32) * 12.0 / 9.0).round() as u32
}

fn pcm_bits(q: u8) -> u32 {
    if q <= 4 {
        16
    } else if q <= 8 {
        24
    } else {
        32
    }
}

fn probe_codec(path: &str) -> Option<String> {
    let out = Command::new(ffprobe_path())
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=nw=1:nk=1",
            path,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_ascii_lowercase();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn music_dir() -> PathBuf {
    crate::tools::music_dir()
        .or_else(|| home_dir().map(|h| h.join("Music")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Fallback destination used before a source folder has been chosen.
pub fn destination(format: ConvertFormat) -> PathBuf {
    // Dev/automation override so conversion tests never write into the real
    // music folder. Normal launches never set this.
    if let Ok(d) = std::env::var("MPDOOM_CONVERT_DEST") {
        return PathBuf::from(d);
    }
    let name = match format {
        ConvertFormat::Same => "Converted (same format)".to_string(),
        _ => format!("Converted {}", format.folder_label()),
    };
    music_dir().join(name)
}

/// Destination named after the folder the user picked, e.g.
/// `~/Music/My Album (MP3)`.
pub fn destination_for(src: &str, format: ConvertFormat) -> PathBuf {
    if let Ok(d) = std::env::var("MPDOOM_CONVERT_DEST") {
        return PathBuf::from(d);
    }
    let base = music_dir();
    let picked = Path::new(src)
        .file_name()
        .map(|n| crate::util::sanitize_win(&n.to_string_lossy()))
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "Converted".to_string());
    base.join(format!("{} ({})", picked, format.folder_label()))
}

#[derive(Clone)]
pub struct ConvertResult {
    pub folder: PathBuf,
    pub converted: usize,
    pub failed: usize,
}

pub fn convert_folder(
    src: &str,
    format: ConvertFormat,
    quality: u8,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<ConvertResult, String> {
    let dest = destination_for(src, format);
    convert_folder_to(src, format, quality, &dest, tx, cancel)
}

pub fn convert_folder_to(
    src: &str,
    format: ConvertFormat,
    quality: u8,
    dest_root: &Path,
    tx: &Sender<Msg>,
    cancel: &AtomicBool,
) -> Result<ConvertResult, String> {
    if !Path::new(src).is_dir() {
        return Err(format!("{} is not a folder", src));
    }
    let files_all = collect_audio_cancellable(src, cancel);
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    // If the user picked a folder that contains the destination (e.g. their
    // whole Music folder), don't re-convert previous output.
    let dest_canon = dest_root
        .canonicalize()
        .unwrap_or_else(|_| dest_root.to_path_buf());
    let files: Vec<String> = files_all
        .into_iter()
        .filter(|p| !Path::new(p).starts_with(&dest_canon))
        .collect();
    if files.is_empty() {
        return Err("No supported audio files found in that folder".into());
    }
    std::fs::create_dir_all(dest_root)
        .map_err(|e| format!("cannot create {}: {}", dest_root.display(), e))?;
    // Remove hidden partial files left by a previous crash/forced exit.
    if let Ok(rd) = std::fs::read_dir(dest_root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(".mpdoom-convert-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let total = files.len();
    let _ = tx.send(Msg::ConvertProgress {
        done: 0,
        total,
        text: format!("Converting {} file(s) to {}...", total, dest_root.display()),
    });
    let mut converted = 0usize;
    let mut failed = 0usize;
    for (i, file) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let resolved = format.resolved(file);
        let rel = Path::new(file)
            .strip_prefix(Path::new(src))
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                PathBuf::from(
                    Path::new(file)
                        .file_name()
                        .unwrap_or_else(|| std::ffi::OsStr::new("track")),
                )
            });
        let mut out = dest_root.join(rel);
        out.set_extension(resolved.extension());
        let out = unique_path(out);
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match convert_one(file, &out, resolved, quality, cancel) {
            Ok(()) => converted += 1,
            Err(e) if cancel.load(Ordering::Relaxed) => return Err(e),
            Err(_) => failed += 1,
        }
        let name = Path::new(file)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let _ = tx.send(Msg::ConvertProgress {
            done: i + 1,
            total,
            text: format!("{}/{}  {}", i + 1, total, name),
        });
    }
    Ok(ConvertResult {
        folder: dest_root.to_path_buf(),
        converted,
        failed,
    })
}

fn unique_path(p: PathBuf) -> PathBuf {
    if !p.exists() {
        return p;
    }
    let parent = p.parent().map(|x| x.to_path_buf()).unwrap_or_default();
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "track".to_string());
    let ext = p
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    for n in 2..1000 {
        let name = if ext.is_empty() {
            format!("{} ({})", stem, n)
        } else {
            format!("{} ({}).{}", stem, n, ext)
        };
        let q = parent.join(name);
        if !q.exists() {
            return q;
        }
    }
    p
}

fn temp_output(dst: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let ext = dst
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "tmp".to_string());
    dst.with_file_name(format!(
        ".mpdoom-convert-{}-{}.{}",
        std::process::id(),
        stamp,
        ext
    ))
}

fn convert_one(
    src: &str,
    dst: &Path,
    format: ConvertFormat,
    quality: u8,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let tmp = temp_output(dst);
    let mut cmd = Command::new(ffmpeg_path());
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(src);
    if format.supports_art() {
        // Keep embedded cover art where the output container supports it.
        cmd.args([
            "-map",
            "0:a:0",
            "-map",
            "0:v?",
            "-c:v",
            "copy",
            "-disposition:v",
            "attached_pic",
        ]);
    } else {
        cmd.args(["-map", "0:a:0", "-vn"]);
    }
    cmd.args(["-map_metadata", "0", "-sn", "-dn"]);
    cmd.args(format.codec_args(quality));
    cmd.arg(&tmp)
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
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;
    let err_handle = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let started = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let err = err_handle.and_then(|h| h.join().ok()).unwrap_or_default();
                if status.success() && std::fs::metadata(&tmp).map(|m| m.len() > 64).unwrap_or(false) {
                    return std::fs::rename(&tmp, dst).map_err(|e| {
                        let _ = std::fs::remove_file(&tmp);
                        format!("could not finalize {}: {}", dst.display(), e)
                    });
                }
                let _ = std::fs::remove_file(&tmp);
                let tail = err.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                return Err(format!(
                    "ffmpeg failed{}",
                    if tail.is_empty() { String::new() } else { format!(": {}", tail) }
                ));
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("ffmpeg error: {}", e));
            }
        }
        if started.elapsed() > Duration::from_secs(60 * 30) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err("conversion timed out".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_changes_mp3_bitrate() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("mpdoom-convert-quality-{}", stamp));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let wav = src.join("tone.wav");
        let st = Command::new(ffmpeg_path())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&wav)
            .status()
            .expect("generate wav");
        assert!(st.success());
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);
        let out_lo = root.join("lo");
        let out_hi = root.join("hi");
        convert_folder_to(&src.to_string_lossy(), ConvertFormat::Mp3, 1, &out_lo, &tx, &cancel)
            .expect("low quality");
        convert_folder_to(&src.to_string_lossy(), ConvertFormat::Mp3, 10, &out_hi, &tx, &cancel)
            .expect("high quality");
        let lo = std::fs::metadata(out_lo.join("tone.mp3")).unwrap().len();
        let hi = std::fs::metadata(out_hi.join("tone.mp3")).unwrap().len();
        assert!(
            hi > lo + lo / 2,
            "quality 10 output ({} bytes) should be much larger than quality 1 ({} bytes)",
            hi,
            lo
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn preserves_embedded_cover_art() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("mpdoom-convert-art-{}", stamp));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/app_icon.jpg");
        let source = src.join("cover.flac");
        let st = Command::new(ffmpeg_path())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.3",
                "-i",
            ])
            .arg(&icon)
            .args([
                "-map",
                "0:a",
                "-map",
                "1:v",
                "-c:a",
                "flac",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
            ])
            .arg(&source)
            .status()
            .expect("make flac with art");
        assert!(st.success());
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);
        for fmt in [ConvertFormat::Mp3, ConvertFormat::Aac] {
            let out = root.join(format!("out-{}", fmt.label().replace(' ', "_")));
            convert_folder_to(&src.to_string_lossy(), fmt, 8, &out, &tx, &cancel)
                .unwrap_or_else(|e| panic!("{}: {}", fmt.label(), e));
            let file = out.join(format!("cover.{}", fmt.extension()));
            assert!(file.is_file(), "{} not created", file.display());
            let probe = Command::new(ffprobe_path())
                .args([
                    "-v",
                    "error",
                    "-select_streams",
                    "v",
                    "-show_entries",
                    "stream=codec_name",
                    "-of",
                    "default=nw=1:nk=1",
                ])
                .arg(&file)
                .output()
                .expect("ffprobe cover");
            assert!(
                !String::from_utf8_lossy(&probe.stdout).trim().is_empty(),
                "{} lost its cover art",
                file.display()
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn destination_uses_the_picked_folder_name() {
        let p = destination_for("/tmp/Some Album", ConvertFormat::Mp3);
        assert!(
            p.to_string_lossy().ends_with("Some Album (MP3)"),
            "unexpected destination {}",
            p.display()
        );
    }

    #[test]
    fn formats_and_quality_labels() {
        assert_eq!(ConvertFormat::Mp3.extension(), "mp3");
        assert_eq!(ConvertFormat::Aac.extension(), "m4a");
        assert_eq!(ConvertFormat::Aiff.extension(), "aiff");
        assert!(ConvertFormat::Mp3.quality_label(10).contains("312"));
        assert!(ConvertFormat::Flac.quality_label(10).contains("12"));
        assert!(ConvertFormat::Wav.quality_label(10).contains("32-bit"));
    }

    #[test]
    fn converts_a_wav_to_every_format() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("mpdoom-convert-{}", stamp));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        let wav = src.join("tone.wav");
        let st = Command::new(ffmpeg_path())
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.3",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&wav)
            .status()
            .expect("generate wav");
        assert!(st.success());
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = AtomicBool::new(false);
        for fmt in ConvertFormat::ALL {
            let out = root.join(format!("out-{}", fmt.label().replace(' ', "_")));
            let r = convert_folder_to(
                &src.to_string_lossy(),
                fmt,
                8,
                &out,
                &tx,
                &cancel,
            )
            .unwrap_or_else(|e| panic!("{}: {}", fmt.label(), e));
            assert_eq!(r.converted, 1, "{} failed={}", fmt.label(), r.failed);
            let ext = fmt.resolved(&wav.to_string_lossy()).extension();
            let out_file = out.join(format!("tone.{}", ext));
            assert!(
                out_file.is_file(),
                "{} did not produce {}",
                fmt.label(),
                out_file.display()
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
