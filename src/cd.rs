// Audio CD detection and lossless ripping (Linux).
//
// The TOC is read straight from the kernel (CDROMREADTOCHDR/ENTRY), so no
// helper is needed just to know whether an audio CD is in the tray. Ripping
// uses cdparanoia for bit-perfect CDDA extraction and ffmpeg to encode FLAC
// (CD audio is 16-bit/44.1 kHz PCM; FLAC stores those samples losslessly).
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use sha1::{Digest, Sha1};

use crate::tools::{ffmpeg_path, music_folder, which};
use crate::types::{CdInfo, Msg};
use crate::util::{sanitize_win, work_dir};

const CDROMREADTOCHDR: libc::c_ulong = 0x5305;
const CDROMREADTOCENTRY: libc::c_ulong = 0x5306;
const CDROM_DATA_TRACK: u8 = 0x04;
const CDROM_LEADOUT: u8 = 0xAA;
const CDROM_MSF: u8 = 0x02;
const LEAD_IN_FRAMES: u32 = 150;
const XA_INTERVAL: u32 = 11400;

#[repr(C)]
#[derive(Default)]
struct CdromTocHdr {
    trk0: u8,
    trk1: u8,
    _pad: [u8; 2],
}

#[repr(C)]
#[derive(Default)]
struct CdromTocEntry {
    track: u8,
    adr_ctrl: u8,
    format: u8,
    _pad: u8,
    addr: u32,
    datamode: u8,
    _pad2: [u8; 3],
}

#[derive(Clone, Copy)]
struct RawTrack {
    number: u8,
    frames: u32,
    audio: bool,
}

fn msf_frames(addr: u32) -> u32 {
    let b = addr.to_le_bytes();
    b[0] as u32 * 60 * 75 + b[1] as u32 * 75 + b[2] as u32
}

fn read_toc(device: &Path) -> Option<(Vec<RawTrack>, u32)> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(device)
        .ok()?;
    let fd = file.as_raw_fd();

    let mut hdr = CdromTocHdr::default();
    if unsafe { libc::ioctl(fd, CDROMREADTOCHDR, &mut hdr as *mut _) } != 0 {
        return None;
    }
    if hdr.trk1 < hdr.trk0 || hdr.trk1 == 0 || hdr.trk1 > 99 {
        return None;
    }

    let mut tracks = Vec::new();
    for number in hdr.trk0..=hdr.trk1 {
        let mut e = CdromTocEntry::default();
        e.track = number;
        e.format = CDROM_MSF;
        if unsafe { libc::ioctl(fd, CDROMREADTOCENTRY, &mut e as *mut _) } != 0 {
            return None;
        }
        let ctrl = (e.adr_ctrl >> 4) & 0x0F;
        tracks.push(RawTrack {
            number,
            frames: msf_frames(e.addr),
            audio: ctrl & CDROM_DATA_TRACK == 0,
        });
    }

    let mut lead = CdromTocEntry::default();
    lead.track = CDROM_LEADOUT;
    lead.format = CDROM_MSF;
    if unsafe { libc::ioctl(fd, CDROMREADTOCENTRY, &mut lead as *mut _) } != 0 {
        return None;
    }
    Some((tracks, msf_frames(lead.addr)))
}

/// Find the first optical drive holding an audio CD.
pub fn detect() -> Option<CdInfo> {
    let mut devices: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/dev") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("sr") && name[2..].chars().all(|c| c.is_ascii_digit()) {
                devices.push(e.path());
            }
        }
    }
    let cdrom = PathBuf::from("/dev/cdrom");
    if cdrom.exists() {
        devices.push(cdrom);
    }
    devices.sort();
    devices.dedup_by(|a, b| a.canonicalize().ok() == b.canonicalize().ok());

    for dev in devices {
        if let Some(info) = read_disc(&dev) {
            return Some(info);
        }
    }
    None
}

fn read_disc(device: &Path) -> Option<CdInfo> {
    let (raw, physical_leadout) = read_toc(device)?;
    let audio: Vec<RawTrack> = raw.iter().copied().filter(|t| t.audio).collect();
    if audio.is_empty() {
        return None;
    }
    let first = audio[0].number;
    let last = audio.last()?.number;

    // If a data track follows the audio session, MusicBrainz wants the gap
    // removed from the lead-out. CD-DA discs normally use the real lead-out.
    let leadout = match raw.iter().find(|t| t.number == last + 1 && !t.audio) {
        // Mixed-mode disc: the audio session ends before the data track.
        Some(next) => next.frames.saturating_sub(XA_INTERVAL),
        None => physical_leadout,
    };

    let offsets: Vec<u32> = audio.iter().map(|t| t.frames).collect();
    let disc_id = musicbrainz_id(first, last, leadout, &audio);
    let total_secs = (leadout.saturating_sub(offsets[0])) as f32 / 75.0;
    Some(CdInfo {
        drive: device.to_string_lossy().to_string(),
        tracks: audio.iter().map(|t| t.number).collect(),
        offsets,
        leadout,
        disc_id,
        total_secs,
    })
}

/// MusicBrainz disc ID: SHA-1 over the upper-case hex TOC, then its custom
/// Base64 alphabet (`+`→`.`, `/`→`_`, `=`→`-`).
pub fn musicbrainz_id(first: u8, last: u8, leadout: u32, audio: &[RawTrack]) -> String {
    let mut fo = [0u32; 100];
    fo[0] = leadout;
    for t in audio {
        if (t.number as usize) < fo.len() {
            fo[t.number as usize] = t.frames;
        }
    }
    let mut s = format!("{:02X}{:02X}", first, last);
    for v in fo {
        let _ = write!(s, "{:08X}", v);
    }
    let digest = Sha1::digest(s.as_bytes());
    base64_mb(&digest)
}

fn base64_mb(data: &[u8]) -> String {
    const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(STD[((n >> 18) & 63) as usize] as char);
        out.push(STD[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { STD[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { STD[(n & 63) as usize] as char } else { '=' });
    }
    out.replace('+', ".").replace('/', "_").replace('=', "-")
}

/// Keep watching the tray. The app compares disc IDs, so repeated `None`
/// messages are harmless.
fn log_cd(msg: &str) {
    use std::io::Write as _;
    let path = crate::util::data_dir().join("cd.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{}", msg);
    }
}

pub fn monitor(tx: Sender<Msg>) {
    let mut last = String::new();
    loop {
        let info = detect();
        let id = info.as_ref().map(|i| i.disc_id.clone()).unwrap_or_default();
        if id != last {
            log_cd(&if id.is_empty() {
                "no audio CD".to_string()
            } else {
                format!("audio CD {} ({} tracks) at {}", id, info.as_ref().map(|i| i.tracks.len()).unwrap_or(0), info.as_ref().map(|i| i.drive.clone()).unwrap_or_default())
            });
            last = id;
        }
        let _ = tx.send(Msg::CdDetected(info));
        std::thread::sleep(Duration::from_secs(5));
    }
}

pub fn eject(drive: &str) -> bool {
    if let Ok(file) = std::fs::OpenOptions::new().read(true).open(drive) {
        if unsafe { libc::ioctl(file.as_raw_fd(), 0x5309, 0) } == 0 {
            return true;
        }
    }
    Command::new("eject").arg(drive).status().map(|s| s.success()).unwrap_or(false)
}

/// Internal playlist path for one track streamed straight from the optical
/// drive. It carries the device, track number and TOC length so the playback
/// worker can start cdparanoia without touching the library or a temp file.
pub fn cdda_path(drive: &str, track: u8, duration_ms: u64) -> String {
    cdda_path_at(drive, track, duration_ms, 0)
}

/// CDDA path with a millisecond start offset inside the track.
pub fn cdda_path_at(drive: &str, track: u8, duration_ms: u64, start_ms: u64) -> String {
    if start_ms == 0 {
        format!("cdda://{}|{}|{}", drive, track, duration_ms)
    } else {
        format!("cdda://{}|{}|{}|{}", drive, track, duration_ms, start_ms)
    }
}

/// cdparanoia span for a track and an optional in-track start offset.
fn cdda_span(track: u8, start_ms: u64) -> String {
    if start_ms == 0 {
        return track.to_string();
    }
    let mm = start_ms / 60_000;
    let ss = (start_ms / 1000) % 60;
    let ff = (((start_ms % 1000) * 75) / 1000).min(74);
    format!("{}:[{:02}:{:02}.{:02}]", track, mm, ss, ff)
}

pub fn cdda_parts(path: &str) -> Option<(&str, u8, u64)> {
    let rest = path.strip_prefix("cdda://")?;
    let mut it = rest.split('|');
    let drive = it.next()?;
    let track = it.next()?.parse().ok()?;
    let duration_ms = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Some((drive, track, duration_ms))
}

pub fn is_cdda(path: &str) -> bool {
    path.starts_with("cdda://")
}

/// About six seconds of stereo CD audio. The reader thread stays this far
/// ahead of the sink, so a slow sector or a busy drive does not run the audio
/// callback dry.
const CDDA_BUFFER_SAMPLES: usize = 44_100 * 2 * 6;

struct CddaShared {
    samples: VecDeque<i16>,
    eof: bool,
    pid: i32,
    last_underrun: Instant,
}

/// A rodio source fed by a background cdparanoia reader. Nothing is written to
/// disk. Reading on the audio callback itself made the callback wait on the
/// drive, which produced the skipping. The callback now only pops buffered
/// samples; if the buffer is briefly empty it emits silence instead of
/// blocking or dropping the playback thread.
pub struct CddaSource {
    shared: std::sync::Arc<std::sync::Mutex<CddaShared>>,
    duration: Option<Duration>,
}

impl Iterator for CddaSource {
    type Item = i16;

    fn next(&mut self) -> Option<Self::Item> {
        let mut s = match self.shared.lock() {
            Ok(s) => s,
            Err(p) => p.into_inner(),
        };
        if let Some(v) = s.samples.pop_front() {
            Some(v)
        } else if s.eof {
            None
        } else {
            // Underrun: keep the output clock moving with silence rather than
            // stalling the audio callback on an empty pipe.
            let log_it = s.last_underrun.elapsed() >= Duration::from_secs(1);
            if log_it {
                s.last_underrun = Instant::now();
            }
            drop(s);
            if log_it {
                crate::log::av_log("cdda: buffer underrun");
            }
            Some(0)
        }
    }
}

impl rodio::Source for CddaSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        44_100
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }

    fn try_seek(&mut self, _pos: Duration) -> Result<(), rodio::source::SeekError> {
        Err(rodio::source::SeekError::NotSupported {
            underlying_source: "CDDA",
        })
    }
}

impl Drop for CddaSource {
    fn drop(&mut self) {
        let mut s = match self.shared.lock() {
            Ok(s) => s,
            Err(p) => p.into_inner(),
        };
        s.eof = true;
        s.samples.clear();
        if s.pid > 0 {
            // The reader thread owns Child and may be blocked in read(); the
            // signal closes the pipe and lets it reap the process.
            unsafe {
                libc::kill(s.pid, libc::SIGKILL);
            }
            s.pid = -1;
        }
    }
}

fn cdda_reader(mut child: Child, shared: std::sync::Arc<std::sync::Mutex<CddaShared>>) {
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            if let Ok(mut s) = shared.lock() {
                s.eof = true;
                s.pid = -1;
            }
            let _ = child.wait();
            return;
        }
    };
    let mut raw = vec![0u8; 32 * 1024];
    loop {
        let stop = match shared.lock() {
            Ok(s) => s.eof,
            Err(p) => p.into_inner().eof,
        };
        if stop {
            break;
        }
        // Keep the drive a little ahead, then apply backpressure.
        let full = match shared.lock() {
            Ok(s) => s.samples.len() >= CDDA_BUFFER_SAMPLES,
            Err(p) => p.into_inner().samples.len() >= CDDA_BUFFER_SAMPLES,
        };
        if full {
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        match stdout.read(&mut raw) {
            Ok(0) => break,
            Ok(n) => {
                let mut s = match shared.lock() {
                    Ok(s) => s,
                    Err(p) => p.into_inner(),
                };
                if s.eof {
                    break;
                }
                for pair in raw[..n].chunks_exact(2) {
                    s.samples.push_back(i16::from_le_bytes([pair[0], pair[1]]));
                }
            }
            Err(_) => break,
        }
    }
    if let Ok(mut s) = shared.lock() {
        s.eof = true;
        s.pid = -1;
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn cdparanoia_exe() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("MPDOOM_CDPARANOIA") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    which("cdparanoia")
}

pub fn available() -> bool {
    cdparanoia_exe().is_some()
}

/// Start streaming one track from the drive. cdparanoia's default WAV output
/// is replaced with `-r`, which sends raw PCM to stdout. Paranoia checking is
/// disabled for playback: it is only needed for archival ripping and made the
/// drive stall and retry long enough to cause audible gaps.
pub fn cdda_source(drive: &str, track: u8, duration_ms: u64) -> Option<CddaSource> {
    cdda_source_at(drive, track, duration_ms, 0)
}

/// Start streaming one track at an in-track offset. Seeking restarts
/// cdparanoia at the requested sector; the drive is the only source, so there
/// is no sample-accurate seek inside the existing stream.
pub fn cdda_source_at(drive: &str, track: u8, duration_ms: u64, start_ms: u64) -> Option<CddaSource> {
    let exe = cdparanoia_exe()?;
    let mut child = Command::new(exe)
        .args(["-q", "-Z", "-r", "-d", drive])
        .arg(cdda_span(track, start_ms))
        // cdparanoia needs an explicit output name. Without it the process
        // waits forever and produces no stdout. A single dash means stdout.
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if child.stdout.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    // The reader takes ownership of the child through the closure; keep the
    // PID separately so Drop can stop it even while it is blocked in read().
    let pid = child.id() as i32;
    let shared = std::sync::Arc::new(std::sync::Mutex::new(CddaShared {
        samples: VecDeque::with_capacity(CDDA_BUFFER_SAMPLES),
        eof: false,
        pid,
        last_underrun: Instant::now(),
    }));
    let thread_shared = shared.clone();
    std::thread::spawn(move || cdda_reader(child, thread_shared));
    Some(CddaSource {
        shared,
        duration: (duration_ms > 0).then(|| Duration::from_millis(duration_ms)),
    })
}

/// Audio length of one track in seconds.
pub fn track_secs(info: &CdInfo, i: usize) -> f32 {
    track_durations(info).get(i).copied().unwrap_or(0.0)
}

#[derive(Clone, Default, Debug)]
pub struct CdMeta {
    pub artist: String,
    pub album: String,
    pub year: Option<String>,
    pub titles: Vec<String>,
    /// Per-track artist (important for compilations/soundtracks).
    pub track_artists: Vec<String>,
    pub release_id: Option<String>,
    pub disc: Option<u32>,
}

fn http_client() -> Option<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(format!(
            "{}/{} (CD lookup)",
            crate::consts::APP_NAME,
            crate::consts::APP_VERSION
        ))
        .build()
        .ok()
}

pub fn lookup_metadata(info: &CdInfo) -> Option<CdMeta> {
    let client = http_client()?;
    let url = format!(
        "https://musicbrainz.org/ws/2/discid/{}?fmt=json&inc=artists+recordings+release-groups",
        info.disc_id
    );
    let res = client.get(&url).send().ok()?;
    if !res.status().is_success() {
        return None;
    }
    let json: serde_json::Value = res.json().ok()?;
    parse_mb_release(&json, info.tracks.len())
}

/// Front cover for the matched release, with the normal web-art chain as a
/// fallback. Used for the on-screen CD art; nothing is written to disk.
pub fn fetch_cover(meta: &CdMeta) -> Option<Vec<u8>> {
    let client = http_client()?;
    if let Some(id) = &meta.release_id {
        if let Ok(res) = client
            .get(format!("https://coverartarchive.org/release/{}/front-500", id))
            .send()
        {
            if res.status().is_success() {
                if let Ok(bytes) = res.bytes() {
                    if crate::util::art_bytes_ok(&bytes, 100) {
                        return Some(bytes.to_vec());
                    }
                }
            }
        }
    }
    if !crate::util::is_placeholder_meta(&meta.artist)
        && !crate::util::is_placeholder_meta(&meta.album)
    {
        crate::network::fetch_web_art(&meta.artist, &meta.album)
    } else {
        None
    }
}

pub fn parse_mb_release(json: &serde_json::Value, track_count: usize) -> Option<CdMeta> {
    let releases = json.get("releases")?.as_array()?;
    let mut fallback: Option<CdMeta> = None;
    for rel in releases {
        let album = rel.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let artist = rel
            .get("artist-credit")
            .and_then(|v| v.as_array())
            .map(|credits| {
                credits
                    .iter()
                    .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                    .collect::<Vec<_>>()
                    .join(" & ")
            })
            .unwrap_or_default();
        let year = rel
            .get("date")
            .and_then(|v| v.as_str())
            .and_then(|d| d.get(..4))
            .map(|s| s.to_string());
        let release_id = rel.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
        let media = rel.get("media").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for (mi, medium) in media.iter().enumerate() {
            let n = medium.get("track-count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let tracks = medium.get("tracks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            if tracks.is_empty() {
                continue;
            }
            let mut titles: Vec<(u32, String, String)> = tracks
                .iter()
                .map(|t| {
                    let pos = t
                        .get("position")
                        .and_then(|v| v.as_u64())
                        .or_else(|| t.get("number").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()))
                        .unwrap_or(0) as u32;
                    let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let ta = t
                        .get("artist-credit")
                        .and_then(|v| v.as_array())
                        .map(|credits| {
                            credits
                                .iter()
                                .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                                .collect::<Vec<_>>()
                                .join(" & ")
                        })
                        .unwrap_or_default();
                    (pos, title, ta)
                })
                .collect();
            titles.sort_by_key(|(p, _, _)| *p);
            let track_artists: Vec<String> = titles.iter().map(|(_, _, a)| a.clone()).collect();
            let titles: Vec<String> = titles.into_iter().map(|(_, t, _)| t).collect();
            let meta = CdMeta {
                artist: artist.clone(),
                album: album.clone(),
                year: year.clone(),
                titles,
                track_artists,
                release_id: release_id.clone(),
                disc: Some(medium.get("position").and_then(|v| v.as_u64()).unwrap_or((mi + 1) as u64) as u32),
            };
            if n == track_count || meta.titles.len() == track_count {
                return Some(meta);
            }
            if meta.titles.len() >= track_count && fallback.is_none() {
                fallback = Some(meta);
            }
        }
    }
    fallback.map(|mut m| {
        m.titles.truncate(track_count);
        m
    })
}

fn unique_album_dir(base: PathBuf, disc_id: &str) -> PathBuf {
    let marker = |dir: &Path| std::fs::read_to_string(dir.join(".cddiscid")).ok();
    if base.is_dir() && marker(&base).as_deref() == Some(disc_id) {
        return base;
    }
    if !base.exists() {
        return base;
    }
    let name = base.file_name().unwrap_or_default().to_string_lossy().to_string();
    let parent = base.parent().unwrap_or(Path::new(".")).to_path_buf();
    for n in 2..100 {
        let cand = parent.join(format!("{} ({})", name, n));
        if !cand.exists() || marker(&cand).as_deref() == Some(disc_id) {
            return cand;
        }
    }
    base
}

/// Audio length of each track in seconds from the raw TOC offsets.
fn track_durations(info: &CdInfo) -> Vec<f32> {
    (0..info.tracks.len())
        .map(|i| {
            let start = info.offsets.get(i).copied().unwrap_or(0);
            let end = info.offsets.get(i + 1).copied().unwrap_or(info.leadout);
            end.saturating_sub(start) as f32 / 75.0
        })
        .collect()
}

/// Extract one CDDA track to a WAV file. The callback receives the audio
/// seconds written so far and the read speed relative to real time (1.0x is a
/// standard audio CD). cdparanoia only reports coarse progress, so the growing
/// WAV file is the most reliable live signal and matches what the UI shows.
fn run_cdparanoia<F>(
    dev: &str,
    track: u8,
    wav: &Path,
    duration: f32,
    mut progress: F,
) -> Result<(), String>
where
    F: FnMut(f32, f32),
{
    let exe = cdparanoia_exe().ok_or("cdparanoia is not installed (install it to rip CDs)")?;
    let err_path = wav.with_extension("cdparanoia.err");
    let err_file = File::create(&err_path)
        .map_err(|e| format!("cannot create cdparanoia log: {}", e))?;
    let mut child = Command::new(exe)
        .args(["-q", "-w", "-d", dev])
        .arg(track.to_string())
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|e| format!("cdparanoia failed to start: {}", e))?;

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = std::fs::remove_file(&err_path);
                return Err(format!("cdparanoia failed: {}", e));
            }
        }
        let bytes = std::fs::metadata(wav)
            .map(|m| m.len().saturating_sub(44))
            .unwrap_or(0);
        let done = (bytes as f32 / 176_400.0).min(duration.max(0.0));
        let elapsed = start.elapsed().as_secs_f32();
        let speed = if elapsed > 0.25 { done / elapsed } else { 0.0 };
        progress(done, speed);
        std::thread::sleep(Duration::from_millis(150));
    };

    let elapsed = start.elapsed().as_secs_f32();
    progress(
        duration.max(0.0),
        if elapsed > 0.25 { duration.max(0.0) / elapsed } else { 0.0 },
    );
    if !status.success() || !wav.is_file() {
        let err = std::fs::read_to_string(&err_path).unwrap_or_default();
        let line = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        let _ = std::fs::remove_file(&err_path);
        return Err(format!("cdparanoia track {}: {}", track, line.trim()));
    }
    let _ = std::fs::remove_file(&err_path);
    Ok(())
}

/// Extract and encode the whole disc. Returns (folder, files, label).
pub fn rip(info: &CdInfo, tx: &Sender<Msg>) -> Result<(PathBuf, Vec<String>, String), String> {
    let total = info.tracks.len();
    let _ = tx.send(Msg::CdProgress {
        track: 0,
        total,
        frac: 0.0,
        speed: 0.0,
        text: "Looking up CD metadata...".into(),
    });
    let mut meta = lookup_metadata(info).unwrap_or_default();
    if meta.titles.len() < total {
        meta.titles.resize(total, String::new());
    }
    let artist = if meta.artist.trim().is_empty() { "Unknown Artist".to_string() } else { meta.artist.clone() };
    let album = if meta.album.trim().is_empty() { "Unknown Album".to_string() } else { meta.album.clone() };
    let album_folder = match meta.year.as_deref() {
        Some(y) if !y.is_empty() => format!("{} ({})", album, y),
        _ => album.clone(),
    };
    let base = music_folder()
        .join(sanitize_win(&artist))
        .join(sanitize_win(&album_folder));
    let dir = unique_album_dir(base, &info.disc_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {}", dir.display(), e))?;

    let wav_dir = work_dir().join("cd").join(&info.disc_id);
    let _ = std::fs::create_dir_all(&wav_dir);
    let durations = track_durations(info);
    let total_secs = durations.iter().sum::<f32>().max(1.0);
    let mut done_before = 0.0f32;
    let mut files = Vec::new();
    for (i, track) in info.tracks.iter().enumerate() {
        let title = meta.titles.get(i).cloned().unwrap_or_default();
        let shown = if title.trim().is_empty() { format!("Track {:02}", track) } else { title.clone() };
        let track_artist = meta
            .track_artists
            .get(i)
            .filter(|a| !a.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| artist.clone());
        let _ = tx.send(Msg::CdProgress {
            track: i + 1,
            total,
            frac: (done_before / total_secs).clamp(0.0, 1.0),
            speed: 0.0,
            text: format!("Ripping {} / {}: {}", i + 1, total, shown),
        });
        let wav = wav_dir.join(format!("track{:02}.wav", track));
        let _ = std::fs::remove_file(&wav);
        let dur = durations.get(i).copied().unwrap_or(0.0);
        let mut last_sent = Instant::now();
        let mut last_speed = 0.0f32;
        run_cdparanoia(&info.drive, *track, &wav, dur, |done, speed| {
            last_speed = speed;
            if last_sent.elapsed() >= Duration::from_millis(150) {
                let frac = ((done_before + done) / total_secs).clamp(0.0, 0.999);
                let _ = tx.send(Msg::CdProgress {
                    track: i + 1,
                    total,
                    frac,
                    speed,
                    text: format!("Ripping {} / {}: {}", i + 1, total, shown),
                });
                last_sent = Instant::now();
            }
        })?;
        done_before += dur;
        let _ = tx.send(Msg::CdProgress {
            track: i + 1,
            total,
            frac: (done_before / total_secs).clamp(0.0, 1.0),
            speed: last_speed,
            text: format!("Encoding {} / {}: {}", i + 1, total, shown),
        });

        let fname = format!("{:02} - {}.flac", track, sanitize_win(&shown));
        let out = dir.join(fname);
        let ff = ffmpeg_path();
        let title_tag = if title.trim().is_empty() { format!("Track {:02}", track) } else { title };
        let mut cmd = Command::new(&ff);
        cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&wav)
            .args(["-c:a", "flac", "-compression_level", "8"])
            .args(["-metadata", &format!("title={}", title_tag)])
            .args(["-metadata", &format!("artist={}", track_artist)])
            .args(["-metadata", &format!("album={}", album)])
            .args(["-metadata", &format!("album_artist={}", artist)])
            .args(["-metadata", &format!("track={}/{}", track, total)])
            .args(["-metadata", &format!("tracktotal={}", total)]);
        if let Some(y) = &meta.year {
            cmd.args(["-metadata", &format!("date={}", y)]);
        }
        if let Some(d) = meta.disc {
            cmd.args(["-metadata", &format!("disc={}", d)]);
        }
        let st = cmd.arg(&out).output().map_err(|e| format!("ffmpeg failed: {}", e))?;
        if !st.status.success() || !out.is_file() {
            let err = String::from_utf8_lossy(&st.stderr);
            return Err(format!("ffmpeg could not encode track {}: {}", track, err.lines().next().unwrap_or("")));
        }
        let _ = std::fs::remove_file(&wav);
        files.push(out.to_string_lossy().to_string());
    }

    // Cover art: release-specific CAA first, then the normal search chain.
    let mut art: Option<Vec<u8>> = None;
    if let (Some(id), Some(client)) = (meta.release_id.clone(), http_client()) {
        if let Ok(res) = client
            .get(format!("https://coverartarchive.org/release/{}/front-500", id))
            .send()
        {
            if res.status().is_success() {
                if let Ok(bytes) = res.bytes() {
                    if crate::util::art_bytes_ok(&bytes, 100) {
                        art = Some(bytes.to_vec());
                    }
                }
            }
        }
    }
    if art.is_none() && !crate::util::is_placeholder_meta(&artist) && !crate::util::is_placeholder_meta(&album) {
        art = crate::network::fetch_web_art(&artist, &album);
    }
    if let Some(bytes) = art {
        if let Some(small) = crate::files::normalize_art(&bytes) {
            let _ = std::fs::write(dir.join("cover.jpg"), small);
        }
    }
    let _ = std::fs::write(dir.join(".cddiscid"), &info.disc_id);
    let _ = std::fs::remove_dir_all(&wav_dir);
    let label = format!("{} - {}", artist, album);
    Ok((dir, files, label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracks(items: &[(u8, u32)]) -> Vec<RawTrack> {
        items.iter().map(|&(number, frames)| RawTrack { number, frames, audio: true }).collect()
    }

    #[test]
    fn musicbrainz_id_known_vector() {
        let a = tracks(&[
            (1, 150), (2, 9078), (3, 13528), (4, 34182), (5, 53768), (6, 70987),
            (7, 96424), (8, 118425), (9, 136793), (10, 159514), (11, 179777), (12, 198006),
        ]);
        assert_eq!(musicbrainz_id(1, 12, 224556, &a), "T_prJXQSrqbnH8OE.dgOKsHm5Uw-");
    }

    #[test]
    fn parses_musicbrainz_release_and_tracks() {
        let json: serde_json::Value = serde_json::from_str(r#"{
          "releases": [{
            "id": "rid-1",
            "title": "The Album",
            "date": "2004-05-01",
            "artist-credit": [{"name": "The Band"}],
            "media": [{
              "position": 1,
              "track-count": 2,
              "tracks": [
                {"position": 1, "number": "1", "title": "First Song"},
                {"position": 2, "number": "2", "title": "Second Song"}
              ]
            }]
          }]
        }"#).unwrap();
        let m = parse_mb_release(&json, 2).unwrap();
        assert_eq!(m.artist, "The Band");
        assert_eq!(m.album, "The Album");
        assert_eq!(m.year.as_deref(), Some("2004"));
        assert_eq!(m.titles, vec!["First Song", "Second Song"]);
        assert_eq!(m.track_artists, vec!["", ""]);
        assert_eq!(m.release_id.as_deref(), Some("rid-1"));
    }

    static CDDA_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn cdda_source_streams_raw_pcm() {
        use rodio::Source as _;
        use std::os::unix::fs::PermissionsExt;
        let _guard = CDDA_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!("mpdoom-cdda-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-cdparanoia");
        std::fs::write(&script, "#!/bin/sh\nprintf '\\001\\000\\377\\377'\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        std::env::set_var("MPDOOM_CDPARANOIA", &script);
        let mut src = cdda_source("/dev/sr0", 1, 500).unwrap();
        let mut first = None;
        for _ in 0..2000 {
            match src.next() {
                Some(0) => std::thread::sleep(Duration::from_millis(1)),
                other => {
                    first = other;
                    break;
                }
            }
        }
        assert_eq!(first, Some(1));
        assert_eq!(src.next(), Some(-1));
        assert_eq!(src.total_duration(), Some(Duration::from_millis(500)));
        std::env::remove_var("MPDOOM_CDPARANOIA");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cdda_paths_round_trip() {
        let p = cdda_path("/dev/sr0", 7, 123_456);
        assert!(is_cdda(&p));
        assert_eq!(cdda_parts(&p), Some(("/dev/sr0", 7, 123_456)));
        assert!(!is_cdda("/music/song.flac"));
        assert_eq!(cdda_parts("/music/song.flac"), None);
    }

    #[test]
    fn cdda_seek_spans_use_track_offsets() {
        assert_eq!(cdda_span(1, 0), "1");
        assert_eq!(cdda_span(1, 90_000), "1:[01:30.00]");
        assert_eq!(cdda_span(2, 61_500), "2:[01:01.37]");
        let p = cdda_path_at("/dev/sr0", 3, 200_000, 90_000);
        assert!(is_cdda(&p));
        assert_eq!(cdda_parts(&p), Some(("/dev/sr0", 3, 200_000)));
    }

    #[test]
    fn cdda_source_at_passes_start_offset() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = CDDA_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!("mpdoom-cdda-args-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let args_file = dir.join("args");
        let script = dir.join("fake-cdparanoia");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '\\001\\000\\377\\377'\n",
                args_file.display()
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        std::env::set_var("MPDOOM_CDPARANOIA", &script);
        let src = cdda_source_at("/dev/sr0", 3, 200_000, 90_000);
        assert!(src.is_some());
        for _ in 0..200 {
            if args_file.is_file() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let text = std::fs::read_to_string(&args_file).unwrap_or_default();
        assert!(
            text.lines().any(|l| l == "3:[01:30.00]"),
            "cdparanoia args were: {:?}",
            text
        );
        drop(src);
        std::env::remove_var("MPDOOM_CDPARANOIA");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn track_durations_follow_toc() {
        let info = CdInfo {
            drive: "/dev/sr0".into(),
            tracks: vec![1, 2],
            offsets: vec![150, 150 + 75 * 10],
            leadout: 150 + 75 * 25,
            disc_id: "test".into(),
            total_secs: 25.0,
        };
        assert_eq!(track_durations(&info), vec![10.0, 15.0]);
    }

    #[test]
    fn disc_id_has_musicbrainz_shape() {
        let a = tracks(&[(1, 150), (2, 10000)]);
        let id = musicbrainz_id(1, 2, 20000, &a);
        assert_eq!(id.len(), 28);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'));
    }
}

#[cfg(test)]
mod live_cd_tests {
    use super::*;
    #[test]
    #[ignore]
    fn live_cd_detect_and_lookup() {
        let info = detect().expect("no audio CD detected");
        eprintln!(
            "drive={} tracks={} total={:.1}s disc_id={}",
            info.drive,
            info.tracks.len(),
            info.total_secs,
            info.disc_id
        );
        eprintln!("track numbers: {:?}", info.tracks);
        eprintln!("offsets: {:?} leadout={}", info.offsets, info.leadout);
        let meta = lookup_metadata(&info);
        eprintln!("metadata: {:?}", meta);
        assert!(!info.disc_id.is_empty());
        assert_eq!(info.tracks.len(), info.offsets.len());
    }
}

#[cfg(test)]
mod live_rip_tests {
    use super::*;
    #[test]
    #[ignore]
    fn live_cd_rip() {
        let info = detect().expect("no audio CD detected");
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || rip(&info, &tx));
        while let Ok(msg) = rx.recv() {
            if let Msg::CdProgress { track, total, frac, speed, text } = msg {
                eprintln!("[{}/{}] {:.0}% {:.1}x {}", track, total, frac * 100.0, speed, text);
            }
        }
        let (dir, files, label) = handle.join().unwrap().expect("rip failed");
        eprintln!("RIPPED: {} -> {} ({} files)", label, dir.display(), files.len());
        for f in &files {
            eprintln!("  {}", f);
        }
        assert!(!files.is_empty());
        assert!(files.iter().all(|f| f.ends_with(".flac")));
    }
}
