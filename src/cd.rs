// Audio CD detection and lossless ripping (Linux).
//
// The TOC is read straight from the kernel (CDROMREADTOCHDR/ENTRY), so no
// helper is needed just to know whether an audio CD is in the tray. Ripping
// uses cdparanoia for bit-perfect CDDA extraction and ffmpeg to encode FLAC
// (CD audio is 16-bit/44.1 kHz PCM; FLAC stores those samples losslessly).
#![allow(dead_code)]

use std::fmt::Write as _;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::time::Duration;

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

fn run_cdparanoia(dev: &str, track: u8, wav: &Path) -> Result<(), String> {
    let exe = which("cdparanoia").ok_or("cdparanoia is not installed (install it to rip CDs)")?;
    let out = Command::new(exe)
        .args(["-q", "-w", "-d", dev])
        .arg(track.to_string())
        .arg(wav)
        .output()
        .map_err(|e| format!("cdparanoia failed to start: {}", e))?;
    if !out.status.success() || !wav.is_file() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        return Err(format!("cdparanoia track {}: {}", track, line.trim()));
    }
    Ok(())
}

/// Extract and encode the whole disc. Returns (folder, files, label).
pub fn rip(info: &CdInfo, tx: &Sender<Msg>) -> Result<(PathBuf, Vec<String>, String), String> {
    let _ = tx.send(Msg::CdProgress { track: 0, total: info.tracks.len(), text: "Looking up CD metadata...".into() });
    let mut meta = lookup_metadata(info).unwrap_or_default();
    if meta.titles.len() < info.tracks.len() {
        meta.titles.resize(info.tracks.len(), String::new());
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
    let total = info.tracks.len();
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
        let _ = tx.send(Msg::CdProgress { track: i + 1, total, text: format!("Ripping {} / {}: {}", i + 1, total, shown) });
        let wav = wav_dir.join(format!("track{:02}.wav", track));
        let _ = std::fs::remove_file(&wav);
        run_cdparanoia(&info.drive, *track, &wav)?;

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
            if let Msg::CdProgress { track, total, text } = msg {
                eprintln!("[{}/{}] {}", track, total, text);
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
