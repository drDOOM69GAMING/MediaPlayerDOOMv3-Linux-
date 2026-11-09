# MediaPlayerDOOM v3

A retro DOOM-styled media player rendered in egui. Tape, CD, and Radio faces on the deck, plus a band/artist catalog, background pulsing speakers, recording, YouTube downloads, and a fullscreen in-app video player with a queue. Runs on Windows 10/11 and Linux (ported and tested on CachyOS/Arch with KDE Plasma on Wayland). On Windows it is a single self-contained exe; on Linux it uses the distro's ffmpeg/ffprobe/mkvmerge and downloads yt-dlp on first run.

## Features

- **Local metadata cache (`Meta All`)**: one click downloads every playlist track's tags + cover art into your `%APPDATA%\MediaPlayerOFDOOM` folder (metacache.json + artcache). Playlists then load with zero re-scanning, and the NOW panel shows tags and art instantly - even offline. Runs on 8 parallel workers and resumes from whatever is already cached.

- **Deck modes**: switch the central deck between TAPE / CD / RADIO faces
  - **TAPE**: auto-reverse cassette with counters, winding, and tune slider
  - **CD**: spinning disc tray with track counter, LCDs, in-order playback, and **physical audio CD** support on Linux: discs in the optical tray are detected automatically and can be ripped as lossless FLAC (tags, cover art and correct `Artist/Album/01 - Title.flac` naming)
  - **RADIO**: FM/AM dial with click-and-drag tuning, presets, SCAN, live web streams, and an **ADD STATION** dialog to save your own FM/AM stream presets
  - **DVD**: play a DVD movie directly from the optical drive or from an `.iso`/`.img` image using ffmpeg's `dvdvideo` demuxer; encrypted discs are decrypted with libdvdcss when installed
  - **BLU-RAY**: play a Blu-ray movie from the optical drive or an `.iso`/`.img` using ffmpeg's `bluray` protocol. AACS keys are read from the user's own `~/.config/aacs/KEYDB.cfg` (or `$XDG_CONFIG_HOME/aacs/KEYDB.cfg`); no keys are shipped with the app. English audio is preferred and subtitles are disabled by default for DVD/Blu-ray
  - **Disc imaging**: both the DVD and Blu-ray decks have **DUMP DISC → ISO**, which copies the entire disc (menus, extras and all) to a uniquely named `.iso` in your XDG Videos folder using a background `dd` worker with progress
- **Bands catalog**: songs grouped by artist folder (A-Z, scrollable), rebuilt from whatever folder you add
- **Play order switch**: flip the retro wall switch under BANDS. ON = random shuffle, OFF = normal in-order playback
- **Smart shuffle**: weighted by your listening history, or plain shuffle
- **Live visualizer**: a real-time 12-band spectrum with peak caps in the NOW panel, synced to whatever is playing (music or radio). The deck's bass woofers pump with the volume, rattle on heavy bass, shove downward on beats, and pulse an expanding ring on loud hits
- **Recording**: capture the current track or a live stream (REC on the deck)
- **Video player**: click **Video** to play the matching movie in-app:
  - True fullscreen playback with letterboxing
  - Auto-hiding top bar (mouse to the top of the screen to reveal) with PLAY/PAUSE, prev/next, **+ ADD** (multi-file picker), and show/hide queue
  - Right-side queue panel with per-item play, remove, and CLEAR; auto-advances when a video ends
  - **Automatic intro/credits detection**: when **SKIP: ON**, the app first uses named **chapter metadata** (`Opening`/`Ending`, including Japanese titles) - exact and instant. Files without usable chapters fall back to audio analysis of sustained music blocks near the start and end. Auto-skipping is conservative and advisory: it never cuts or persists guesses, and manual markers always override it
  - **Series learning**: mark INTRO/CREDS once on any episode, and every other episode in that folder reuses those exact times (median of marked files), so Dragon Ball-style runs skip automatically without re-marking each episode
  - **Manual markers**: use the top bar **INTRO** / **CREDS** buttons to set the intro-end and credits-start at the current playback position; values persist per file and per folder across runs. With **SKIP: ON** these real markers are also fed to `mkvmerge` for a lossless cut
  - The top bar and queue stay visible while your mouse is over the queue panel, so you can scroll it and press CLEAR without the bar disappearing
- **Album art**: local `cover/folder` art always wins; online lookup only as a fallback when no local art exists (so pictures never get replaced by wrong web matches)
- **Lyrics**: Artist/Track auto-detected from the filename when tags are unknown
- **Playlist management**: drag-to-reorder, save/load JSON, playlist-only mode, sequential mode, per-CD directory ordering, and a live current-song highlight that scrolls to and marks whatever is playing
- **System tray** + global hotkeys (media keys where the desktop allows them)
- **Taskbar icon**: on Windows the app icon is applied directly on the real window handle at launch; on Linux the window/tray icon comes from the icon data embedded in the binary
- **YouTube**: paste a URL or search term to download audio/video via yt-dlp

## Requirements

**Windows 10/11 x64**
- Rust toolchain to build from source

**Linux (tested on CachyOS/KDE Plasma on both Wayland and X11)**
- Rust toolchain (`rustup` recommended, or the distro's `rust`)
- Runtime tools: `ffmpeg` (includes ffprobe), `mkvmerge` (MKVToolNix), `cdparanoia` for CD ripping, and libdvdread/libdvdnav for DVD playback (ffmpeg normally pulls these in). `libdvdcss` is needed only for encrypted commercial DVDs
- ALSA (PipeWire/PulseAudio provide it), plus X11 or Wayland libraries
- An XDG desktop portal and backend for the file pickers (KDE/GNOME/XFCE provide one)
- `xdg-utils` for "show in folder" (the app falls back to `gio`, `kioclient`, `thunar`, `nautilus`)

The app follows the XDG Base Directory spec and runs on any freedesktop desktop.
The two first-class targets are **CachyOS/Arch** and **Linux Mint/Debian**. You can
install their dependencies in one command:

```bash
./packaging/install-deps.sh
```

Or install them manually:

```bash
# Arch / CachyOS / Manjaro
sudo pacman -S --needed base-devel rust ffmpeg mkvtoolnix-cli \
  alsa-lib openssl libxkbcommon wayland libx11 xdg-utils

# Debian / Ubuntu / Mint / Pop!_OS
sudo apt update
sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev \
  libwayland-dev libx11-dev libssl-dev ffmpeg mkvtoolnix xdg-utils \
  xdg-desktop-portal

# Fedora
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config alsa-lib-devel \
  libxkbcommon-devel wayland-devel libX11-devel openssl-devel ffmpeg \
  mkvtoolnix xdg-utils xdg-desktop-portal

# openSUSE
sudo zypper install gcc gcc-c++ make pkg-config alsa-devel \
  libxkbcommon-devel wayland-devel libX11-devel libopenssl-devel ffmpeg \
  mkvtoolnix xdg-utils xdg-desktop-portal
```

`cargo build --release` produces a binary linked against your distro's glibc.
For an older distro, build there (or in a matching container) rather than
copying a binary built on a newer distro.

yt-dlp is taken from `PATH` or downloaded into the app data folder on first run
(glibc systems); if you use musl/Alpine, install the distro yt-dlp package.
The Linux tray uses the pure-Rust StatusNotifierItem implementation, so no
GTK/libappindicator/libxdo packages are needed. On GNOME, the optional tray
needs the AppIndicator/KStatusNotifierItem extension; playback and the rest of
the app work without it. Global media keys depend on the desktop (X11 usually
allows them; Wayland often reserves them), and the in-app shortcuts always work.

**Windows builds** are self-contained (single file). ffmpeg, ffprobe, and yt-dlp
are embedded at build time. On first run they self-extract to
`%LOCALAPPDATA%\MediaPlayerOFDOOM\tools`, and nothing appears next to the exe.
You can move or copy the exe anywhere by itself and everything still works.

## Build

```bash
cargo build --release
```

- Windows: `target/release/mediaplayerofdoom.exe`
- Linux: `target/release/mediaplayerofdoom`

It finds your music automatically (the XDG Music folder on Linux, and the
`Music` folder with the most files on Windows), or pick any folder with
**ADD DIR**.

Open a video directly (handy for episodes or testing) with:

```bash
./target/release/mediaplayerofdoom --video "/path/episode.mkv"
```

Start on the CD deck directly (useful with a disc in the tray):

```bash
./target/release/mediaplayerofdoom --mode cd
```

Play a DVD or Blu-ray ISO directly (or use the matching deck for the optical drive):

```bash
./target/release/mediaplayerofdoom --dvd "/path/movie.iso"
./target/release/mediaplayerofdoom --bluray "/path/movie.iso"
```

### Install with icon and launcher (Linux)

```bash
./install.sh
```

This builds the release binary, installs it to `~/.local/bin/mediaplayerofdoom`,
adds the app icon under `~/.local/share/icons/hicolor/.../apps`, and registers
`~/.local/share/applications/mediaplayerofdoom.desktop`. It also creates
`MediaPlayerDOOM.desktop` in the project folder and on your desktop.

**Linux note:** a bare ELF binary cannot carry a file-manager icon (that is a
Windows `.exe` feature). The icon comes from a `.desktop` launcher. Use
`MediaPlayerDOOM.desktop` or the application menu, not a direct click on
`target/release/mediaplayerofdoom`, if you want to see the artwork.

The app window uses the `mediaplayerofdoom` app-id, so KDE/GNOME can match the
launcher/taskbar icon. On Windows, the same artwork is embedded into the exe as
a multi-resolution `.ico` at build time.

### Where files live on Linux

- Data/tools/cache: `~/.local/share/mediaplayerofdoom/`
- Recordings: `~/Music/Recordings/` (or your `XDG_MUSIC_DIR`)
- Downloads: your Music folder

## Keyboard

`P` play/pause · `S`/`N` skip · `Left` previous · `Right` next · `Shift+Left/Right` seek 10s · `R` record · `+`/`-` volume · `F` fullscreen · `M` mute · `D` deck mode · `V` video · `L` lyrics · `O` playlist-only · `B` repeat · `J` search · `T` theme

Global media keys (Play/Pause, Next, Previous, Stop) and `Ctrl+Alt+M` / `Ctrl+Alt+Q` also work where the desktop permits global grabs. Wayland usually reserves media keys for the desktop, so use the in-app shortcuts there.

## App Images
<img width="3840" height="2160" alt="stars for the star" src="https://github.com/user-attachments/assets/c68a0bfc-d7ab-4d31-a8fd-d9bdf186a1ca" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 133844" src="https://github.com/user-attachments/assets/49e8a167-8b41-4166-9239-08067e05e3c4" />
<img width="3824" height="2155" alt="Screenshot 2026-09-10 133812" src="https://github.com/user-attachments/assets/6a162706-f897-40e7-b048-7eb8326796f1" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 133734" src="https://github.com/user-attachments/assets/052d9c5d-37c1-475f-bdf8-a1cabd113df7" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 134017" src="https://github.com/user-attachments/assets/d040a5a5-f128-474b-9180-3e675a6eca9c" />

## Notes

- The **AUTO preamp** in the EQ measures the boosted response and backs off the gain automatically so you never clip, even with a loud preset.
- The EQ curve panel, preset cycling, band sliders, and the ON-OFF toggle all apply your changes live while music plays, with no restart and no temporary files.
- If you build from source on Windows and have `tools/ffmpeg.exe`, `tools/ffprobe.exe`, `tools/yt-dlp.exe` next to `Cargo.toml`, they are embedded into the exe by `build.rs` (that folder is gitignored). Without them the app falls back to downloading on first run.
- On Linux the app resolves `ffmpeg`, `ffprobe`, `yt-dlp`, and `mkvmerge` from its own data folder first, then from `PATH`, so distro packages are used directly. Features that need a missing tool report a clear status instead of crashing.
- Linux data/cache paths follow the XDG spec (`$XDG_DATA_HOME/mediaplayerofdoom`, normally `~/.local/share/mediaplayerofdoom`). Windows keeps the original `%APPDATA%\MediaPlayerOFDOOM` location, so existing libraries and settings are untouched.


## Acknowledgements

This app is built on the work of some incredible open-source projects, and it wouldn't exist without them:

- **FFmpeg / FFprobe** (GPL v3) - audio/video decoding, transcoding, metadata, and analysis. <https://ffmpeg.org/> - <https://www.gyan.dev/ffmpeg/builds/>
- **yt-dlp** (Unlicense) - YouTube downloads. <https://github.com/yt-dlp/yt-dlp>
- **MKVToolNix / mkvmerge** (GPL v2) - real lossless video cuts at INTRO/CREDS markers. <https://mkvtoolnix.download/>
- **cdparanoia** (GPL v2) - bit-perfect CDDA extraction for lossless CD ripping. <https://xiph.org/paranoia/>
- **egui / eframe** (MIT/Apache-2.0) - the immediate-mode GUI toolkit that renders the whole DOOM-styled interface. <https://github.com/emilk/egui>
- **rodio / cpal** (MIT/Apache-2.0, Apache-2.0) - audio playback and device access.
- **symphonia** (MPL-2.0) - pure-Rust media demuxing/decoding.
- The **Rust** language and its crate ecosystem.

Full details, license texts, and donation links are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). If you can, consider donating to FFmpeg, yt-dlp, and MKVToolNix - they give this all away for free.

## v3.2.26 changelog

- **Decrypted DVD → ISO**
  - New **DECRYPT → ISO** button in the DVD deck (next to the raw DUMP DISC → ISO button)
  - Uses MakeMKV’s backup mode to remove CSS while reading the **physical disc**, producing a clean, directly playable ISO
  - Raw `dd` images are still available for archival, but they preserve encryption; a raw CSS ISO will show chunky blocks when played
  - Same progress bar, live MB/s, ETA and CANCEL; the partial decrypted image is removed on cancel
  - Requires `makemkvcon`; if it is not installed the app reports the error instead of producing a corrupt image
- **Help menu updated**
  - Documents Convert Music, Meta All / covers / ratings / metadata repair, DVD and Blu-ray playback/seeking, raw vs decrypted ISO dumping, CD ripping, hardware acceleration and common troubleshooting
- Version bump to **v3.2.26**

## v3.2.25 changelog

- **Metadata repair from the filename/folder**
  - Missing or placeholder title/artist/album tags are repaired from the filename and folder layout when the library loads and whenever tags arrive
  - A tag that is only a track number (“01”, “Track 1”) is replaced with the real song name from the filename
  - Valid embedded tags are left alone; this changes the player’s labels and online art/rating lookups, not the files’ embedded tags
- **More complete cover/rating attempts**
  - Meta All still retries every missing cover (the remembered “no art” list is cleared at the start)
  - When a track has no usable album tag, the cover lookup now falls back to an **artist + song-title** search on iTunes/Deezer instead of immediately giving up
  - Completion status now reports how many tracks have **no MusicBrainz rating/match** and how many have no cover available, so the unavailable ones are distinguishable from skipped ones
  - Ratings still require MusicBrainz community data; albums/tracks with no rating remain unrated
- Version bump to **v3.2.25**

## v3.2.24 changelog

- **Disc-image dump speed**
  - ISO dumping now uses larger **4 MB direct reads** on Linux optical devices, and shows live **MB/s + ETA** in the dump progress bar
  - On the ID4 DVD this drive measures about 4–9 MB/s depending on whether it is reading the inner or outer tracks (the drive reports a 16× CAV maximum). A 7.5 GB DVD-9 therefore needs roughly 15–30 minutes; this is the drive/disc, not the app
  - Structural disc protection or a dirty/scratched disc can slow reads further; the progress bar now makes that visible
- Version bump to **v3.2.24**

## v3.2.23 changelog

- **DVD seeking fixed for real discs**
  - The chunky pixelation was not CSS/libdvdcss: libdvdcss was already installed and a single ffmpeg reader produced clean frames. The player was starting an extra ffprobe duration read on every seek while the new video and audio readers were opening; on a real DVD that thrashed the drive and corrupted the video
  - DVD metadata (dimensions, frame rate, duration) is now probed once and cached; seeks reuse it and start only the video/audio readers
  - DVDs no longer resume from a saved timestamp; a stale input seek on a real disc could land in a bad cell and stay macroblocked
  - Real PAL/NTSC dimensions and frame rate are probed instead of assuming 720x480/29.97
  - DVD video is deinterlaced (`yadif=deint=interlaced`) before scaling, removing combing/tearing on 480i/576i discs
- Version bump to **v3.2.23**

## v3.2.22 changelog

- **Converter window close fixed**
  - The in-window **CLOSE** button now actually closes the converter. The window state was being overwritten at the end of the frame, undoing the click
  - Added a **HIDE** button while a conversion is running; the conversion continues in the background and the Convert button reopens the progress window
  - The window’s X button now closes/hides it correctly too
- Version bump to **v3.2.22**

## v3.2.21 changelog

- **Converter fixes**
  - Output folder now matches the folder you picked: `~/Music/<picked folder> (<format>)`, e.g. `~/Music/Alapalooza (MP3)`. Subfolders inside the picked folder are still preserved
  - Closing or cancelling during conversion no longer freezes the UI. The folder scan is cancellable, cancellation is checked between files, `ffmpeg` is killed, and app shutdown no longer waits on the conversion worker
  - Quality 1–10 is verified end-to-end: MP3/AAC/Opus/WMA change bitrate, FLAC changes compression level, WAV/AIFF/ALAC change bit depth. The slider label now says **lossless** for FLAC/ALAC and **no lossy compression** for PCM, so small size changes are not mistaken for a no-op
  - Embedded artwork is preserved for MP3/AAC/FLAC/ALAC; tags are preserved everywhere
- Version bump to **v3.2.21**

## v3.2.20 changelog

- **New CONVERT MUSIC mode**
  - The **Convert** button in the deck toolbar opens a conversion window
  - Pick any folder of music; audio files are found recursively
  - Output formats: **Same as source, MP3, AAC (M4A), FLAC, OGG Vorbis, Opus, WAV, AIFF, ALAC (M4A), WMA**
  - One **1–10 quality/compression slider**: lossy formats map to bitrate/quality, FLAC maps to compression level, WAV/AIFF/ALAC map to 16/24/32-bit
  - Output goes to the OS music folder (`$XDG_MUSIC_DIR`, normally `~/Music`) under `Converted <format>`; subfolders and file names are preserved and collisions get `(2)`, `(3)` suffixes
  - Tags are copied; embedded cover art is preserved for MP3/AAC/FLAC/ALAC. Artwork is stripped for containers that cannot carry it
  - Live per-file progress and **CANCEL CONVERSION**; cancelling or exiting kills the active ffmpeg and removes the partial output
  - “Same as source” resolves by extension, with ffprobe used to distinguish AAC vs ALAC in `.m4a`
- Version bump to **v3.2.20**

## v3.2.19 changelog

- **Fix DVD seeking (green frames / stutter)**
  - DVD (MPEG-2, 720x480) now decodes in software and uses the stable CPU scale/RGBA path. It is far too small to need VAAPI, and VAAPI’s filter/format reinitialisation after a DVD seek was producing green frames and stutter on some drives
  - Blu-ray keeps the VAAPI hardware decode + GPU scale/pad/RGBA path
  - The DVD demuxer, audio, English-track selection, subtitles-off and seek behaviour are unchanged
- Version bump to **v3.2.19**

## v3.2.18 changelog

- **Faster cover downloading**
  - The web-art pool is now 20 workers instead of 6, across iTunes, Deezer, Cover Art Archive and TheAudioDB
  - Every explicit Meta All retries previously art-less tracks too (the remembered “no art” list is cleared at the start), so all missing songs get another attempt with the current source set
- **Why ratings are slower than covers**
  - MusicBrainz permits roughly one request per second per client; each album needs a search plus a release lookup. Ratings therefore run sequentially by design — running 20 rating workers would only trigger 429/503 throttling and lose albums
  - Covers come from independent services and are downloaded in parallel; rating progress and ETA are shown separately
- Version bump to **v3.2.18**

## v3.2.17 changelog

- **Ratings appear on the song you are looking at**
  - Starting a track now looks up that **album’s** MusicBrainz rating automatically, so the RATE stars and every playlist row in that album fill in within a few seconds — even before Meta All reaches it
  - The bulk Meta All rating queue now starts with the currently playing album, then follows playlist order, instead of arbitrary album order
  - Meta All now shows `rated X / Y songs` alongside the progress bar and ETA, so you can see the total library rating progress
- **Better cover matching**
  - Album-only fallback for compilations, “Various Artists” folders and untagged tracks with a known album name
- **Cancel really cancels**
  - Queued cover and rating lookups for a cancelled Meta All run are dropped (generation-tagged), while completed tags/covers/ratings are kept
- Version bump to **v3.2.17**

## v3.2.16 changelog

- **Meta All now fetches real star ratings**
  - Uses **MusicBrainz community ratings** (1–5 stars) for each recording, with the album/release-group rating as a fallback for tracks nobody has rated individually
  - Ratings are only filled where you have not set one yourself; clicking a star still overrides and is remembered as a manual rating
  - Ratings are grouped by album, so one lookup rates every track on that album. MusicBrainz is rate-limited (about two requests per album), so a large library takes a while; the progress bar shows an ETA and **CANCEL** stops it
  - Persisted in `settings.json` (`ratings`, `manual_ratings`, `ratings_schema`); old settings are migrated once so pre-existing ratings are treated as manual
- **Meta All progress bar**
  - Shows overall percent, ETA, and a live breakdown: `tags … | covers done/total | ratings done/total`
  - Cover progress is counted per **album**, not per track, so the ETA reflects actual lookups (one album cover fills all its tracks)
  - CANCEL stops queued rating and cover lookups; completed tags/covers/ratings are kept
- **Cover art sources and efficiency**
  - iTunes, Deezer, **Cover Art Archive**, and a new **TheAudioDB** fallback are all tried for each album
  - One web lookup per album is written to every track on that album (a 12-track album is one request, not twelve)
  - Local folder art is still preferred before any web source
- Version bump to **v3.2.16**

## v3.2.15 changelog

- **Stop the startup drive hammering**
  - The player was queueing tag reads for all 18,400 playlist tracks on every launch, even though `metacache.json` already had every entry. Startup now seeds names from the cache and only auto-fetches a small budget (400) for genuinely uncached tracks; **Meta All** remains the explicit full-library fetch
  - `request_playlist_tags` now skips paths already present in the tag cache (the tag cache and display cache previously disagreed at startup, causing a full re-read)
  - Truly idle launches no longer read the music drive from end to end; the only remaining reads are the track that is actually playing
- **Fix the skip cascade**
  - The “sink empty” watchdog waited only 600 ms, so a cold 4 TB HDD that took longer to open/buffer a FLAC made the player skip good tracks in a loop, opening a new file and cover every couple of seconds. Patience is now 5 s
- **Fewer disk writes**
  - `settings.json` (a ~2 MB playlist) is debounced to one write per 10 s instead of every 2 s
  - `metacache.json`/`missing_art.json` are written at most every 30 s instead of every 3 s while batches stream in; final state is still saved on exit
- Version bump to **v3.2.15**

## v3.2.14 changelog

- **Whole-disc dump overhaul (DVD + Blu-ray)**
  - **Choose the destination folder** every time you click DUMP DISC → ISO (defaults to your Videos folder; cancelling the picker cancels the dump)
  - **Live progress bar** with percentage and GB written, for Blu-rays as well as DVDs
  - **CANCEL DUMP** button stops `dd`, kills the child process and deletes the partial image
  - Closing/exiting the player now stops the dump immediately. The `dd` child is also tied to the app with `PR_SET_PDEATHSIG`, so it cannot keep reading the disc after a crash or forced exit
  - Images are written as `<label>.iso.<pid>.part` and renamed to `.iso` only after the copy succeeds, so an interrupted dump can never be mistaken for a complete image
  - Dumping now stops disc playback first, so `dd` and the movie player do not fight over the drive
- Version bump to **v3.2.14**

## v3.2.13 changelog

- **Blu-ray playback fixes**
  - Resolve the disc's real main-title playlist via libbluray instead of letting ffmpeg auto-pick one that cannot seek (the Jumanji disc's main title is playlist `00001.mpls`)
  - Do not resume a Blu-ray from a saved timestamp; that was the change that made the disc "stop reading" and hang on launch
  - **Seeking now works.** ffmpeg's own `bluray:` protocol ignores libbluray's seek API (`-ss` stalls and `-chapter` is ignored on this disc). The player now runs a tiny built-in libbluray streamer (`--bd-stream`, loaded with `dlopen`) and feeds the decrypted M2TS stream to ffmpeg. Clicking the seek bar restarts that streamer at the clicked timestamp, so it jumps to the requested scene instead of the intro
  - The streamer is also used for normal Blu-ray playback, so video and audio both start from the same timestamp and stay in sync (measured drift within about ±50 ms after a seek)
  - Blu-ray audio language is taken from libbluray's own title/language list, preferring English; subtitles stay off
  - Blu-ray startup no longer runs extra ffprobe passes against the drive, so the first frame arrives sooner and the drive is not thrashed
  - STOP now actually closes optical playback and clears the DVD/Blu-ray NOW state; a STOP button is also present in the fullscreen movie bar
- **GPU vendor detection**
  - AMD/Intel are detected from `/sys/class/drm/*/device/vendor` and use VAAPI directly (`-hwaccel vaapi`); NVIDIA with the proprietary driver uses CUDA. This removes the failed CUDA probe on your AMD RX 6800
  - On VAAPI, scaling, letterbox padding and the NV12→RGBA conversion now stay on the GPU (`scale_vaapi`/`pad_vaapi`); ffmpeg's video process dropped from roughly 20% CPU to roughly 8% on the RX 6800
  - `MPDOOM_NO_HWACCEL=1` still forces software decoding
- **Corrected seeking behaviour:** the seek bar now honours the clicked position on Blu-ray (no more intro restart); libbluray is loaded at runtime so builds/systems without it still play local media
- Version bump to **v3.2.13**

## v3.2.11 changelog

- **Whole-disc ISO dumping for DVD and Blu-ray**
  - New **DUMP DISC → ISO** button in both optical decks
  - Copies every sector of the disc (menus, extras, all titles) with `dd` in a background thread - the UI stays responsive and shows percentage progress
  - Images are written to the XDG Videos folder (`~/Videos`), named from the disc label with automatic `(2)`, `(3)` suffixes instead of overwriting
  - Raw images of encrypted discs still need the user's AACS/CSS keys to play, just like the physical disc
- Version bump to **v3.2.11**

## v3.2.10 changelog

- **Hardware-accelerated video decoding**
  - ffmpeg now uses `-hwaccel auto`: VAAPI on AMD/Intel, CUDA/NVDEC on NVIDIA, with automatic software fallback
  - Tested on AMD Radeon RX 6800 (Mesa VAAPI): DVD/MPEG-2 and H.264 decode offload working, including a 6 s seek completing in under 100 ms
  - Set `MPDOOM_NO_HWACCEL=1` to force software decoding if a driver or profile misbehaves
- **Removed the per-frame RGB→RGBA conversion**
  - ffmpeg now emits RGBA directly, so the video thread no longer copies every pixel and the UI receives the buffer without an extra full-frame pass
- Version bump to **v3.2.10**

## v3.2.9 changelog

- **New Blu-ray deck mode**
  - Play Blu-ray movies from the optical drive or an `.iso`/`.img` via ffmpeg's `bluray` protocol
  - AACS keys are discovered on the user's system (`$XDG_CONFIG_HOME/aacs/KEYDB.cfg`, `~/.config/aacs/KEYDB.cfg`, `~/.aacs/KEYDB.cfg`, `/etc/xdg/aacs/KEYDB.cfg`); no keys are bundled or downloaded
  - If the KEYDB is in a non-standard location, the app creates a private config symlink for the ffmpeg child so libaacs still finds it
  - `--mode bluray` opens the deck; `--bluray /path/movie.iso` starts a movie immediately
- **Optical-disc defaults**
  - DVD and Blu-ray audio defaults to an **English** track when one is present (falls back to the first audio track)
  - Subtitles are explicitly disabled (`-sn`) for optical playback
- Added libbluray/libaacs to dependency installers; libbdplus is optional for BD+ discs
- Version bump to **v3.2.9**

## v3.2.8 changelog

- **New DVD deck mode**
  - Play a DVD movie directly from the optical drive or from an `.iso`/`.img` image
  - Uses ffmpeg's `dvdvideo` demuxer (libdvdread/libdvdnav); libdvdcss handles encrypted commercial discs when installed
  - The existing fullscreen video player, queue, seek bar and A/V clock are reused, with the main title selected automatically
  - Video is decoded through ffmpeg's **hardware acceleration** when available (VAAPI on AMD/Intel, CUDA on NVIDIA), which keeps 1080p Blu-ray seeking responsive; `MPDOOM_NO_HWACCEL=1` disables it if a driver misbehaves
  - `--mode dvd` opens the deck directly; `--dvd /path/movie.iso` starts a movie immediately
  - Verified with a real generated DVD-Video ISO: 720x480, 29.97 fps, audio drift within about 35 ms; seeking to 6 s reached the first frame in under 100 ms
  - DVD titles are excluded from mkvmerge auto-cut and automatic intro/credits detection (those are for episode files)
- Added libdvdread/libdvdnav/libdvdcss to the Arch/CachyOS dependency installer; documented libdvdcss for other distros
- Version bump to **v3.2.8**

## v3.2.7 changelog

- **Async UI work - no more multi-second freezes**
  - **Meta All** no longer validates every cached cover on the UI thread (that decoded 18k JPEGs synchronously). Cache validation now runs on a worker; the playlist stays responsive
  - Album art is decoded, downscaled and re-encoded on the art/network workers instead of on the UI thread; the UI only uploads the small final texture
  - Tag/cover label updates are coalesced into a single playlist rebuild per frame instead of rebuilding 18k labels for every 200-item worker batch
  - Playlist rebuilds no longer clone the entire path list first
  - The playlist pane no longer allocates an 18k-element index vector on every repaint when the search box is empty (the common case)
  - Tag batches no longer build a full-library `HashSet` per batch, and adding many files no longer does O(n²) membership checks
  - Current-song lookup checks the cached index first instead of scanning the full playlist every frame
- 88 tests pass
- Version bump to **v3.2.7**

## v3.2.6 changelog

- **Hard-drive performance**
  - Settings and history writes are now debounced (2 s) and stored as compact JSON. Previously every track change rewrote the full multi-MB library settings file, which caused constant disk writes and stalls - especially on HDDs
  - Library sorting uses cached sort keys, so loading/shuffling 18k+ paths no longer recomputes artist/title parsing for every comparison
  - Folder scanning skips `System Volume Information`, `$RECYCLE.BIN`, `lost+found`, `.Trash-*`, `node_modules` and similar trees. Symlinked music folders are still followed, with a canonical-path visited set so loops cannot recurse
  - A refresh that finds the same playlist no longer re-ingests, re-sorts, rewrites settings, or restarts the playing song
  - In sequential/directory playback, skipping uses the already-loaded playlist instead of re-walking the folder from disk every time
- **Playback performance**
  - Audio files are read through a 256 KiB buffer instead of the 8 KiB default, and Linux is asked for sequential read-ahead (`POSIX_FADV_SEQUENTIAL`) - noticeably fewer HDD stalls on long FLAC/MP3 files
  - Seeks in formats whose decoder cannot seek (notably FLAC) now ask ffmpeg to start at the target position, instead of decoding and discarding everything from the beginning of the track
- 88 tests pass
- Version bump to **v3.2.6**

## v3.2.5 changelog

- **Audio CD detection and lossless ripping**
  - CD mode now watches the optical drive and shows `AUDIO CD • N TRACKS • time` as soon as a music CD is inserted
  - **RIP CD → MUSIC** extracts the whole disc and writes `Music/Artist/Album (Year)/NN - Title.flac`
  - FLAC encoding is done by ffmpeg; CD audio is 16-bit/44.1 kHz PCM, so the FLAC is sample-for-sample lossless (verified: 16-bit/44.1 kHz stereo, correctly tagged)
  - Metadata comes from the MusicBrainz disc ID (computed from the kernel TOC), including artist, album, year and per-track titles; cover art is fetched from Cover Art Archive or the normal art sources
  - `cdparanoia` performs the bit-perfect CDDA extraction (ffmpeg on CachyOS/Arch lacks libcdio, so it cannot read a CD directly); ffmpeg then encodes/tags the FLAC
  - Existing rips of the same disc are recognised by a `.cddiscid` marker instead of being duplicated
  - Per-track artists are tagged correctly on compilations/soundtracks
  - Switching into CD mode now stops the previously playing music and resets the spectrum/speakers instead of leaving a frozen visualizer
  - Verified live on a real 13-track CD: correct metadata, filenames, tags and artwork (86 tests + live disc checks)
- `cdparanoia` added to all dependency installers
- Version bump to **v3.2.5**

## v3.2.4 changelog

- **Fixed the desktop shortcut not appearing on KDE Plasma**: the launcher contained a quoted `TryExec=`, which Plasma interpreted as an invalid command path and therefore hid the shortcut. `TryExec` is no longer written; the icon and launcher now show on the desktop
- Verified on Plasma 6.8: the shortcut appears with the artwork, and the app creates it when missing while leaving an existing one untouched
- Version bump to **v3.2.4**

## v3.2.3 changelog

- Desktop shortcut behaviour is now exactly "make sure it's there": on launch the release build checks `Desktop/MediaPlayerDOOM.desktop` and creates it (with the artwork icon) only when missing. If it already exists, it does not rewrite it and creates no other launchers on that path
- The application-menu entry and theme icon are installed by `install.sh`; after that the runtime only guards the desktop shortcut
- **Cross-distro portability**: documented build dependencies for Arch/CachyOS, Debian/Ubuntu/Mint, Fedora, and openSUSE; XDG paths throughout; X11 and Wayland both tested; opener fallbacks (`gio`, `kioclient`, `thunar`, `nautilus`) when `xdg-open` is absent; Linux-only desktop code is compile-gated so Windows/macOS builds still work
- **First-class CachyOS/Arch + Linux Mint/Debian support**: `packaging/install-deps.sh` installs the right toolchain/dependencies for the detected distro, and a CI matrix builds/tests on Ubuntu 24.04 (Mint's base) and Arch Linux (CachyOS's base)
- Version bump to **v3.2.3**

## v3.2.2 changelog

- **Self-healing desktop integration**: every launch of the release build writes/updates `~/.local/share/applications/mediaplayerofdoom.desktop`, the hicolor icon, and `Desktop/MediaPlayerDOOM.desktop`, all pointing at the running executable and using the artwork. If a shortcut is deleted, the next start puts it back
- Debug builds are deliberately excluded so `cargo run` does not create menu entries for `target/debug`
- Cache refreshes run in the background (`kbuildsycoca6`, `gtk-update-icon-cache`) and never delay startup
- Version bump to **v3.2.2**

## v3.2.1 changelog

- **Proper application icon everywhere**
  - Generated a square, multi-resolution icon from the original artwork (center crop, so it is no longer squashed) and replaced the old 256x140 `.ico`
  - Windows embeds the new icon in the exe via `build.rs`
  - Linux gets a real launcher/taskbar entry: `mediaplayerofdoom.desktop`, a 512x512 icon in the hicolor theme, and `StartupWMClass=mediaplayerofdoom`
  - The window now sets the Wayland app-id (`mediaplayerofdoom`) so the desktop can associate the running window with that icon
  - New `install.sh` installs the binary, icon and desktop entry for the current user (`~/.local`)
  - Tray icons continue to use the same artwork at several sizes, and the artwork also appears beside the app name in the player's title bar
  - If KDE still shows the old generic taskbar icon after installing, log out/in once or run `systemctl --user restart plasma-plasmashell` - Plasma caches taskbar icons
  - New `--windowed` / `-w` flag for opening in a normal window instead of fullscreen
  - `install.sh` also creates `MediaPlayerDOOM.desktop` in the project folder and on the desktop, so the artwork is visible in Dolphin/launchers (a bare Linux executable always gets a generic file-manager icon)
  - The release app checks for `Desktop/MediaPlayerDOOM.desktop` when it opens. If the shortcut is missing it creates it with the icon; if it is already there it does nothing at all (no rewrite, no extra launchers)
  - The installer refreshes KDE's icon cache/sycoca and adds rustup's cargo path if needed
- Version bump to **v3.2.1**

## v3.2.0 changelog

- **Automatic intro/credits detection is back, safely**
  - **Named chapters are checked first** (`Opening`, `Ending`, `OP`/`ED`, plus Japanese `オープニング`/`エンディング`). When present they are exact and require no analysis - verified on a real *Dragon Ball* episode whose chapters marked the opening at 0:00-1:48 and the ending at 22:31
  - For files without usable chapters, audio is analysed at 0.5 s resolution; the app looks for long, loud, continuous music blocks near the start and end instead of trusting a fixed 90 s guess
  - The opening is the first qualifying block (40-180 s), so an action scene or the ending theme cannot be mistaken for it
  - The ending is the latest qualifying block that reaches the final three minutes, so the opening is never chosen as the credits
  - Noisy dialogue and silence are rejected; ambiguous episodes produce no markers and simply play normally
  - Guess-based skipping only ever seeks - it never writes settings and never auto-cuts, so a wrong guess can't damage a file
- **Series learning from your timestamps**: mark one episode's INTRO/CREDS and every other episode in the folder reuses the median of those exact times automatically. This is the most reliable mode for long anime runs
- **A/V sync verified on a real episode**: after an auto intro skip, measured audio/video drift stayed within about ±40 ms (typically under 25 ms) (the old Windows sync issue came from restarting only one stream)
- **Credits now actually stop playback**: reaching the credits marker pauses both the video reader and the ffmpeg audio pipe; previously the audio kept playing under the "PLAY to replay" message
- **Manual markers preserve auto-detected values**: setting just INTRO or just CREDS keeps the other automatically detected time instead of resetting it to zero
- **Open a video from the command line** with `--video /path/episode.mkv`
- ffmpeg helper output is quiet now (`-loglevel error`, stderr piped away), so the terminal/session log stays clean
- 83 tests pass, including chapter and real-audio detection tests; verified on a real *Dragon Ball* episode (chapter-based intro 1:48.65, credits 22:31.6)
- Version bump to **v3.2.0**

## v3.1.1 changelog

- **Album art overhaul - three sources instead of one**
  - iTunes (mainstream), **Deezer** (indie/electronic/European), and **MusicBrainz + Cover Art Archive** (obscure, classical, metal, punk) are tried in order; the first usable image wins
  - Album/artist matching scores every result instead of blindly taking result #1, and rejects unrelated albums with the same title
  - A looser album-only search is the final fallback for compilations and Various Artists releases
- **Bad artwork is detected and repaired**
  - Cached covers are decoded and validated before use; truncated/corrupt/blank placeholder images are deleted and re-fetched automatically
  - Cache writes are now atomic (temp file + rename), so an interrupted write can never leave a half-written cover
  - Tracks that previously had "no art" are given another chance after upgrading; click **Meta All** once to re-check and repair the library
  - Local art scanning now accepts `webp`/`gif`/`bmp`, ranks `cover`/`folder`/`front` correctly regardless of case, and skips back/disc/booklet scans and non-images
  - Embedded pictures are validated too, and the NOW panel no longer stays on "Loading art..." forever when there is no art anywhere
  - Album art is drawn aspect-correct instead of stretched to fill the square
- **Better metadata fallback for untagged libraries**: `Artist/Album/track` and multi-disc (`CD 1`, `Disc 2`) folder layouts are understood, and old cache entries that stored the album name as the artist are repaired
- 74 tests pass (plus an ignored live network test for the three art sources)
- Version bump to **v3.1.1**

## v3.1.0 changelog

- **Linux port (CachyOS / Arch tested)**: builds and runs natively on X11 and Wayland with the same UI and features as the Windows build.
  - XDG data paths (`~/.local/share/mediaplayerofdoom`) and XDG Music folder detection, including removable drives under `/run/media`, `/media`, and `/mnt`
  - Uses the distro's `ffmpeg`, `ffprobe`, and `mkvmerge` from `PATH`; downloads the Linux `yt-dlp` binary on first run
  - Pure-Rust StatusNotifierItem tray (`ksni`) on Linux - no GTK, libappindicator, or libxdo dependencies
  - Recordings save to `~/Music/Recordings` instead of next to the executable
  - `xdg-open` for "show in folder"; process spawning is portable; media keys register where the desktop allows
- **Fixed the embedded icon pipeline**: the 16:9 artwork is now center-cropped to a real square before resizing, so window/tray/taskbar icons get a correctly sized RGBA buffer on Windows and Linux instead of a truncated one.
- **Keyboard shortcuts now match the documentation**: `P` play/pause, `S`/`N` skip, `Left` previous, `Right` next, `R` record, `M` mute, `D` deck mode, `V` video, `L` lyrics, `F` fullscreen, `O` playlist-only.
- **Media/global hotkeys**: Play/Pause, Next, Previous, and Stop are registered best-effort in addition to `Ctrl+Alt+M` / `Ctrl+Alt+Q`.
- Added media pipeline tests (ffmpeg probe/decode/transcode/record, mkvmerge cut) and icon/size checks; **58 tests pass**.
- Version bump to **v3.1.0**

## v3.0.1 changelog

- **Meta All is now truly idempotent and honest**: tracks confirmed to have no cover art anywhere (no local picture, no iTunes match) are remembered in `%APPDATA%\MediaPlayerOFDOOM\missing_art.json` and skipped on every future run, so re-clicking Meta All does zero re-scanning and reports the real state - "All done here! Everything is already cached locally (N tracks) - add more music and hit Meta All again to download more meta."
- **Parallel web art**: cover lookups for a big missing-art batch now run through a dedicated **6-worker pool** instead of one slow sequential stream on the net thread - a library with thousands of missing covers drains in minutes and each result is persisted the moment it lands, so interrupted runs resume instead of repeating.
- Version bump to **v3.0.1**

## v3.0.0 changelog

- **Local metadata cache** - the new **Meta All** button on the playlist toolbar downloads tags (title/artist/album/duration) and cover art for every track in the current playlist into `%APPDATA%\MediaPlayerOFDOOM\metacache.json` + `artcache\`, so:
  - Playlists load and render track names instantly - no lofty re-scan of thousands of files every session
  - The NOW panel shows tags and cover art immediately when a song changes, straight from local disk (no blank "Unknown" flash, no network wait)
  - Everything is persisted under your appdata folder, survives restarts, and loads once at startup
- **Parallel downloads** - both the tag reader and the cover-art scanner run on **8-worker thread pools**, so a full library downloads several times faster than a single sequential pass
- **Resume support** - pressing Meta All never re-downloads what's already cached; it skips completed tracks and only fetches the gaps (visible in the status bar: "resuming - X tags + Y covers already cached; fetching Z tags and W covers...")
- **Cover art pipeline** - local `folder/cover` art or embedded pictures are found first and normalized to small ~220px JPEGs; tracks with artist+album but no art and no local picture get a web cover lookup (iTunes) as a last resort
- **M4A/M4B/M4P crash fix** - opening MPEG-4 audio through the built-in decoder no longer panic-blocks the app; those files route straight to the ffmpeg transcode path (cached WAV in the work folder), and any file the built-in decoder can't open is skipped with a clean log entry in `crash.log` instead of killing playback
- **Visualizer per-band auto-gain** - each of the 12 spectrum bars is normalized against its own typical level, so the treble bars follow real treble content instead of being pinned to zero by bass/mid dominance; the mapping is centered so bars sit mid-range and only real hits reach the top, with a snappier envelope (0.22 s release) that tracks the beat
- Version bump to **v3.0.0**

## v2.8.0 changelog

- **Real-time in-app equalizer** (replaces the old offline process-and-swap EQ that had to be disabled): a 10-band EQ (60 Hz-16 kHz) processes the audio live in the playback chain, so band changes, presets, and the ON/OFF toggle apply instantly mid-song with zero interruption.
- **EQ curve panel**: a 20 Hz-20 kHz response graph shows the combined EQ curve with band markers and current values, plus the **AUTO** preamp percentage.
- **Presets**: `◀ EQ` / `EQ ▶` cycle through preset curves (Bass Boost, Rock, Pop, etc.), the same style of graphic-equalizer presets the AQUA system EQ uses.
- **10 vertical band sliders** (-12 to +12 dB) with per-band labels; touch any slider and it becomes your **Custom** curve.
- **EQ settings are saved**: EQ on/off, the active preset, and your custom curve reload with the app.
- **Works everywhere**: the EQ applies to music, the tape, radio streams, and video audio alike.
- **EQ auto-preamp**: because boosting bands can clip, the app measures the boosted response (20 Hz-20 kHz sweep) and scales it down so the loudest part sits right at full scale - loud, clean, no distortion.

## v2.7.0 changelog

- **Playlist now tracks the playing song live**: every track change scrolls the list to center the actual row and marks it green with a play triangle and a filled background, so you always see what is playing. The scroll lands on the right row even as tracks advance.
- **NOW readout**: a header above the playlist shows exactly which entry is playing, like `NOW 0423/500`, along with the current song name, updating in real time.
- **Taskbar icon fix**: the app icon is now applied directly on the real window handle instead of relying on the windowing library's self-set access, so the icon shows correctly in the Windows taskbar and Alt-Tab on every launch.
- **Speaker cleanup**: the thin outer echo circles around the deck speakers were removed for a cleaner look while keeping the cone motion, bass rattle, and pulse ring.

## v2.6.0 changelog

- **Auto-skip now cuts for real**: with SKIP on AUTO, the manual INTRO and CREDS markers you set are fed to mkvmerge (the same muxer that trims for radio/CD), producing a real cut file that starts at the intro marker, so playback never drifts out of sync with the audio.
- **Marker buttons survive the first skip**: after an auto-cut, the INTRO / CREDS buttons and the seek bar keep pointing at the same frame times from your original markers, so you still get full control instead of a single one-shot skip. The marker values are carried over to the cut file automatically.
- **No more mid-play spoiling**: the cut plays from position zero as its own file rather than seeking inside the playlist track, which is what was letting the audio and video fall out of step.
- **Cuts go to the system temp folder**: cut files are written to your Windows temp directory (not next to your media), old temporary cuts are cleared each time a new cut is made and when the app exits, so nothing piles up on your videos' folders.

- **Live audio visualizer**: the NOW panel now runs a real-time 12-band spectrum (about 30 Hz to 16 kHz) with peak caps, driven by the actual audio of whatever is playing, including the radio. The bars are loudness-relative, so they follow the music: gold/orange at steady volume, red flashes only on genuine peaks, and they fall away during quiet passages instead of staying pinned at full red.
- **Punchy bass woofers**: the twin deck speakers analyze the audio and move with it. The cones pump in and out with volume, rattle when the bass kicks, shove downward on beats, and throw expanding shockwave rings; hard hits flash the speaker red.
- **Radio ADD STATION**: a + ADD button in the radio deck opens a dialog to name a station, pick FM or AM, set the frequency, and paste the stream URL. It saves to your presets and starts playing immediately, so AM presets (or any custom stream) are fully usable.
- **Radio dial now always lands**: dragging the dial sweeps and snaps to the nearest preset, and playback starts as soon as it settles. No more silent dead spots between stations.
- **STOP now completely stops the radio**: the stream is cut and stays silent until you tune or play again (previously the radio could restart by itself about a second after stopping).
- **Transport cleanup**: the standalone PAUSE deck key is gone. PLAY handles both start and pause, one obvious control.

## v2.4.0 changelog

- **Original aspect ratio (ORIG)**: the video decode now preserves each file's true aspect ratio instead of stretching everything to 16:9, so 4:3 shows and the ORIG aspect button matches the source.
- **Audio/video sync**: video frames are paced at the file's real framerate (not a fixed 24fps), and playback audio waits until the first frame is on screen - voices and picture stay in sync.
- **Cursor auto-hide**: in fullscreen the mouse cursor hides after 5 seconds of no movement and reappears as soon as you move it again.
- **Automatic intro/credits detection**: videos with **SKIP: ON** are analyzed on play - the app measures the audio front/back, finds where the intro ends and credits/outro begins, then auto-skips the intro and hops to the next video when credits roll.
- **Manual INTRO / CREDS markers** in the video top bar: click during playback to mark intro-end or credits-start at the current position for that file/folder; persisted in settings.
- **Queue stays open while you use it**: the bar and queue show whenever the mouse is over the top strip or the queue panel, and hide the moment it leaves - so you can scroll, play rows, and hit CLEAR without them vanishing mid-use. The old SHOW QUEUE pin toggle is removed.
- **Per-file + per-folder skip memory**: detected and manual bound times are stored both for the exact file and for its folder, so the whole series skips consistently.
- **Auto-hide controls**: the fullscreen video top bar and queue appear only while the mouse is over the top strip or the queue panel, and disappear when the mouse leaves - nothing stays stuck on screen.

## v2.3.0 changelog

- **REW/FWD buttons now work with a click** (10 second skip) in addition to press-and-hold winding.
- **CD last-folder memory**: inserting a disc remembers the last folder used.
- **All-band A-Z ordering**: the ALL button sorts bands alphabetically, songs within each band by title.
- **CD to Tape restore**: switching from CD mode to Tape always restores the full A-Z library playlist.
- **Currently-playing playlist index**: the app tracks which playlist row is playing for reliable highlight and playback.
- **Resume tape state fix**: resuming a tape now correctly sets the current song and playing state before playback begins.
- **Random/shuffle fallback**: shuffle and dir-sequential modes always find a valid playlist index so playback doesn't lose track.
- **Help window expanded**: full documentation of keyboard shortcuts, deck controls (Tape/CD/Radio), tune slider, playlist controls, and extras.
- **Duplicate Radio button removed** from the button bar.
- **Scroll-follow removed**: playlist scrolls freely without locking to the playing song.
- **Playlist highlight**: the currently playing song is marked with a filled triangle and colored text in the playlist.
- **EQ disabled (forced Flat)**: carried forward from v2.1; the equalizer is locked until the background encode is reworked.

## v2.1 changelog

- **EQ disabled (forced Flat).** Using the equalizer could wedge the background thread that handles metadata and video decode, causing video to get stuck on "LOADING..." and metadata/pictures to go wrong. A hover hint now explains it; re-enable in a future build after the EQ encode is moved off the shared worker.
- **Album art fix.** Web album-art lookup could overwrite a correct local `cover/folder` picture with a wrong match. Local art now always wins; web art is only used when no local art exists.
- **Video player overhaul**: fullscreen playback with letterboxing, auto-hiding top control bar, multi-file **+ ADD**, right-side queue (per-item play/remove/CLEAR), auto-advance at end of video, and stale-closed-event protection so manually closing never swallows the queue.
- **Video queue skip fix**: skipping forward/backwards (and the queue's per-item selection) no longer kills the video window while the audio keeps playing. Close events from the *previous* video are ignored once a new one has started. Adding videos while nothing is playing now starts playback immediately instead of silently queuing. Video ffmpeg also launches with a hidden window, so no console box flashes when a movie loads.
- **Background crash-proofing**: the library worker (metadata, album art, video decode, EQ encode) and the network worker (lyrics, art lookup, downloads) run every message inside a panic guard; any failure surfaces "Background task crashed..." in the status bar instead of killing the app.
- **EQ redesign (background render)**: when it returns, EQ renders in the background and swaps in the processed file without stopping the currently-playing song. (Currently disabled per above.)
- **Self-contained tools**: ffmpeg / ffprobe / yt-dlp are embedded in the exe and extract on first run to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools` (was: a folder next to the exe). The exe now works standalone from anywhere.
- **Radio fix**: Rock 107.1 (KJML) stream URL corrected to `https://ice42.securenetsystems.net/KJML` in both the code and the on-disk radio prefs.
- **Lyrics fix**: title/artist parsed from the filename when tags are unknown; lyrics lookups no longer crash the network worker.
- **Play order fixes**: disc/CD and playlist-only-sequential playback always advance in order even with the random switch on; random skip no longer replays the current song immediately.
- **Crash logging**: any panic writes a trace to `%APPDATA%\MediaPlayerOFDOOM\crash.log`.
