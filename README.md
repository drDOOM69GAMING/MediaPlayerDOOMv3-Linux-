# MediaPlayerDOOM v3 for Linux

![Version](https://img.shields.io/badge/version-3.2.42-blue)
![Platform](https://img.shields.io/badge/platform-Linux%20x86__64-informational)
![License](https://img.shields.io/badge/license-MIT-green)

It covers music, audio CDs, DVDs, Blu-rays, conversion, metadata, equalization, beat-synced visuals and YouTube downloads in one desktop app.

This repository is the Linux port. It is tested on CachyOS and Arch with KDE Plasma on Wayland and X11. Debian, Ubuntu and Linux Mint are also supported. Linux is the only supported target here.

## Highlights

### Music and library

- MP3, FLAC, M4A and AAC, ALAC, APE, WAV, AIFF, OGG, Opus, WMA, WavPack, Musepack and WebM audio
- Automatic ffmpeg fallback for formats the built-in decoder cannot open, including M4A and APE
- CUE sheet support that expands single-file APE and FLAC albums into individual tracks
- Artist and album browser with A to Z bands, expandable album lists and a full library view
- Band names are merged across folder layouts, so loose "Artist - Album" folders join the same artist
- Weighted smart shuffle or plain shuffle, with one red random / green in-order play-order switch for the loaded list
- M3U and JSON playlist support
- Drag to reorder the queue
- Lyrics lookup
- Recording of the current track or radio stream
- Uninterrupted auto-advance that keeps playing while the window is minimized or hidden
- Fast startup from the cached library, with the background scan deferred and low-priority

### Metadata, art and ratings

- Meta All fills tags, and cover art for the whole playlist
- Local cover and folder art wins over online results
- Cover sources include iTunes, Deezer, Cover Art Archive, TheAudioDB and Discogs
- Song title fallback for tracks whose album is unknown
- MusicBrainz community star ratings with Discogs and TheAudioDB fallbacks
- Manual star ratings are never overwritten
- Metadata repair from the filename and folder for missing title, artist, album and track numbers
- Persistent XDG metadata cache and art cache

### Conversion and CD ripping

- Convert a folder recursively to MP3, AAC, FLAC, OGG, Opus, WAV, AIFF, ALAC or WMA
- Right-click a band or album in the browser to convert only that selection with the same format and compression choices
- Clear all star ratings from Help > System Manual, with a confirmation
- Quality 1 to 10 controls bitrate, FLAC compression, or PCM and ALAC bit depth
- Tags and embedded cover art are preserved where the container supports them
- Physical audio CDs are detected automatically and ripped to lossless FLAC with MusicBrainz tags and cover art
- CD ripping shows a live progress bar with percentage, read speed and ETA
- PLAY CD streams the tray disc directly through cdparanoia; nothing is ripped or written to disk
- CD audio is buffered ahead for smooth playback, with album art and titles loading in the background
- CD playback always follows track order, with shuffle and star filters ignored
- CD mode shows only the loaded disc and hides the library band browser
- Per file progress with cancel and a hide option that keeps the conversion running

### EQ, visualizer and deck

- Standard ISO one-octave EQ bands from 31 Hz to 16 kHz
- Twenty curated curves including Harman, Loudness, Broadcast, Podcast, Vocal Air, Acoustic, Jazz, Classical, Rock, Metal, Hip-Hop, EDM, De-Mud and Dialogue
- Automatic preamp with headroom and a soft-knee limiter
- Flat response stays bit-transparent
- Preset picker, arrow cycling, FLAT reset and live band sliders
- Response graph with grid lines, curve fill, band values and preamp percentage
- Real-time 12-band spectrum with peak caps
- Kick-band and spectral-flux beat detector with a phase-locked BPM grid
- Twin deck woofers that thump with the beat, rattle on bass and pulse on peaks
- Tape, CD, Radio, DVD and Blu-ray deck faces

### DVD and Blu-ray

- DVD playback from the optical drive or an ISO image using ffmpeg dvdvideo and libdvdcss
- Blu-ray playback from the optical drive, an ISO image or a folder using libbluray and libaacs
- English audio is preferred and subtitles are off by default
- Real time seeking for both formats
- Smooth frame pacing with a 60 Hz movie present loop, a reused video texture, wall-clock presentation and slow audio drift correction
- DUMP TO MKV decrypts and remuxes every title with MakeMKV
- DVD DUMP TO ISO uses the bundled CachyOS dvdbackup plus genisoimage, with MakeMKV as fallback
- Blu-ray DUMP TO ISO uses MakeMKV backup with decryption and builds a full disc UDF 2.50 image with menus and extras, using the native writer built into the app
- AACS and BD plus status with Disc ID, KEYDB match and MKB version
- CHECK AACS re-reads the disc and updates the status
- AACS keys are read from the user KEYDB and are never bundled with the app

### YouTube and downloads

- Paste a URL or search term to download music through yt-dlp
- Live progress bar in the YT row, the download queue and the bottom status bar
- Percentage, stage, speed, ETA and item counter
- Batch queue with album grouping
- Tags and cover art are written to the downloaded MP3

### Desktop integration

- System tray with Hide, Show, Play/Pause, Skip, Previous, Stop and Quit
- Hide removes the taskbar entry on KDE Wayland through KWin scripting and keeps audio and auto-advance alive
- Show restores and activates the window
- While hidden, each new song appears as a single desktop notification for a few seconds
- Quit works while hidden, with a hard exit fallback if the UI loop cannot process the close
- Global media keys where the desktop allows them
- KDE, GNOME, Xfce and other freedesktop desktops
- XDG base directory paths for data, cache, recordings and music
- Hardware acceleration detection for AMD, Intel and NVIDIA
- **Default media player**: a Set as Default action and `--set-default` command register MediaPlayerDOOM for 25 audio and video MIME types with xdg-mime. Opening an audio file starts the music player, while video files open in the video player
- **Update check**: the app asks GitHub for the latest release shortly after launch and every six hours. When a newer release exists, a clickable banner appears at the top of the window and opens the release page. Nothing downloads or installs without the user

## Requirements

Tested on CachyOS and Arch with KDE Plasma on Wayland and X11. Debian, Ubuntu and Linux Mint are also supported.

- Rust toolchain with Cargo
- FFmpeg and ffprobe
- MKVToolNix and mkvmerge
- cdparanoia for CD ripping
- libdvdread, libdvdnav and libdvdcss for DVDs
- libbluray and libaacs for Blu-rays
- MakeMKV for Blu-ray MKV and Blu-ray ISO dumps
- dvdbackup and genisoimage for the fast decrypted DVD ISO path
- ALSA, PipeWire or PulseAudio
- X11 or Wayland libraries
- An XDG desktop portal for the file picker
- xdg-utils for Show in folder
- yt-dlp, resolved from PATH or downloaded to the app data folder

The fastest way to install the dependencies is:

```bash
./packaging/install-deps.sh
```

Manual commands:

```bash
# CachyOS and Arch
sudo pacman -S --needed base-devel rust ffmpeg mkvtoolnix-cli cdparanoia \
  libdvdread libdvdnav libdvdcss dvdbackup cdrtools \
  libbluray libaacs alsa-lib openssl libxkbcommon libxkbcommon-x11 \
  wayland libx11 xdg-utils

# Debian, Ubuntu and Mint
sudo apt update
sudo apt install build-essential pkg-config libasound2-dev libxkbcommon-dev \
  libxkbcommon-x11-dev libwayland-dev libx11-dev libssl-dev ffmpeg mkvtoolnix \
  cdparanoia libbluray2 libaacs0 dvdbackup genisoimage xdg-utils \
  xdg-desktop-portal xdg-desktop-portal-gtk
```

Check the DVD stack at any time:

```bash
mediaplayerofdoom --check-dvd
```

## Install

### Install from source

```bash
git clone https://github.com/drDOOM69GAMING/MediaPlayerDOOMv3-Linux-.git
cd MediaPlayerDOOMv3-Linux-
./packaging/install-deps.sh
./install.sh
```

The installer builds the release binary and installs it to `~/.local/bin/mediaplayerofdoom`. It also installs the icon, the desktop launcher and the bundled CachyOS dvdbackup helper.

### Release binary

Download the Linux binary from the latest release:

https://github.com/drDOOM69GAMING/MediaPlayerDOOMv3-Linux-/releases

The binary still needs the runtime tools listed above. Install them first with `packaging/install-deps.sh` or your package manager.

## Build

```bash
cargo build --release
```

The output is `target/release/mediaplayerofdoom`.

For a development build:

```bash
cargo run --release
```

## Usage

```bash
# Start on the tape deck
mediaplayerofdoom --mode tape

# Start on the CD deck
mediaplayerofdoom --mode cd

# Open a video directly
mediaplayerofdoom --video "/path/episode.mkv"

# Play a DVD or Blu-ray image
mediaplayerofdoom --dvd "/path/movie.iso"
mediaplayerofdoom --bluray "/path/movie.iso"

# Start windowed instead of fullscreen
mediaplayerofdoom --windowed
```

## Keyboard

`P` play or pause, `S` or `N` skip, `Left` previous, `Right` next, `Shift+Left` and `Shift+Right` seek 10 seconds, `R` record, `+` and `-` volume, `F` fullscreen, `M` mute, `D` deck mode, `V` video, `L` lyrics, `B` repeat, `J` search, `T` theme.

## Tray and Hide mode

- Right-click the tray icon and choose Hide to remove the window from the taskbar while audio keeps playing.
- Right-click and choose Show to restore and activate the window.
- On KDE Wayland, Hide uses KWin scripting for skip taskbar and minimize because winit cannot hide a Wayland window directly.
- On other desktops, Hide falls back to minimize so the app never claims a taskbar hide that did not happen.
- While hidden, each new song produces one desktop notification with artist, title and album.
- Quit works while hidden. If the UI cannot process Close, the app force exits after two seconds.

## File locations

- Data, tools and cache: `$XDG_DATA_HOME/mediaplayerofdoom`, normally `~/.local/share/mediaplayerofdoom`
- Recordings: `$XDG_MUSIC_DIR/Recordings`
- Downloads: your Music folder
- Settings: `~/.local/share/mediaplayerofdoom/settings.json`
- AACS key database: `~/.config/aacs/KEYDB.cfg`

## Troubleshooting

### DVD playback shows blocks

The source is probably a raw encrypted ISO. Use DUMP TO MKV from that image or DUMP TO ISO from the physical disc. Raw encrypted images are archival only.

### Blu-ray does not play

Check that `~/.config/aacs/KEYDB.cfg` contains the disc ID. Open the Blu-ray deck and press CHECK AACS. For BD plus discs, use DUMP TO MKV or DUMP TO ISO because MakeMKV handles BD plus.

### Blu-ray ISO dump

Full disc Blu-ray dumps need MakeMKV. The result preserves menus and extras and is decrypted. The ISO uses UDF 1.02 and is readable by libbluray, this app and VLC.

### MusicBrainz ratings are missing

MusicBrainz does not provide a community rating for every track or album. Tracks without a public rating stay unrated. Manual stars always work.

### Cover art is missing

Meta All retries every run and reports how many tracks still have no art. Some albums have no usable public cover.

### YouTube download

yt-dlp is resolved from PATH or downloaded to the app data folder. If a download fails, the YT log keeps the last error. Update yt-dlp if YouTube changes.

## Known limitations

- Single-file CUE albums are supported, but one CUE sheet must describe one image file.
- Blu-ray full disc ISOs use UDF 1.02 for broad software compatibility.
- BD plus and AACS discs without a matching libaacs key require MakeMKV for full disc dumps.
- Hardware players may require UDF 2.50 for Blu-ray ISO playback.
- MusicBrainz ratings and some covers depend on public data availability.
- Hide to tray removes the taskbar entry only on KDE Wayland. Other desktops fall back to minimize.

## License and acknowledgements

MediaPlayerDOOM is released under the MIT License. See `LICENSE`.

Built with FFmpeg, yt-dlp, MKVToolNix, cdparanoia, libdvdread, libdvdnav, libdvdcss, libbluray, libaacs, dvdbackup, egui, rodio, lofty and other open source projects. See `THIRD-PARTY-NOTICES.md` for details.

## Changelog

See `CHANGELOG.md` and the GitHub releases page.
