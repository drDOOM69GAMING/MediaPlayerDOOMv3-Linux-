# Changelog

## v3.2.35

- Background auto-advance prepares and queues the next track on the audio sink, so playback continues when the window is minimized, obscured or hidden to the tray.
- The UI catches up to the background-started track with title, tags, art, history and seek position.
- Tray Hide and Show were added.
- KDE Wayland Hide uses KWin scripting for skip taskbar and minimize because winit cannot hide a Wayland window directly.
- Show restores and activates the window even while the UI loop is stopped.
- Tray Quit works while hidden and force exits after two seconds if the UI cannot process Close.
- Hidden songs produce one desktop notification per track with a replace tag and a four second debounce.
- Startup auto-play is restored without letting the library scan interrupt a manually selected track.
- KWin scripts are loaded with a unique plugin name and run only by the returned script ID.

## v3.2.34

- Beat-locked visualizer with kick-band and spectral-flux onset detection.
- Phase-locked BPM grid with octave folding and median interval filtering.
- Low spectrum bars get an on-beat lift.
- Twin woofers thump with the real kick, with stronger cone shove, surround flex, rattle and pulse rings.
- Silence is gated so beats are not invented.
- Verified against a synthetic 120 BPM click track.

## v3.2.33

- Professional EQ with standard ISO one-octave bands from 31 Hz to 16 kHz at Q 1.4.
- Twenty curated curves for music, speech, broadcast and monitoring.
- Automatic preamp with headroom and a soft-knee limiter.
- Flat response stays bit-transparent.
- New preset picker, FLAT reset, response grid, curve fill, band values and preamp percentage.
- YouTube download progress is always visible in the bottom status bar as well as the YT row and queue window.

## v3.2.32

- Full disc Blu-ray ISO dumping with MakeMKV backup decryption and a UDF image.
- AACS and BD plus status in the Blu-ray deck with Disc ID, KEYDB match and MKB version.
- CHECK AACS re-reads the disc.
- Support for classic and AACS 2.1 KEYDB formats.
- MakeMKV handles AACS and BD plus for full disc images.

## v3.2.31

- CUE sheet support for single-file APE and FLAC albums.
- CUE tracks get individual titles, track numbers, artist, album and duration.
- Playback seeks to each track start and stops at its end.
- CUE tracks work with ratings, cover grouping, band grouping, conversion and recording.
- The scanned library expands CUE albums into individual tracks.

## v3.2.30

- Monkey's Audio APE support was added to the library scan and file picker.
- APE, M4A, WMA, WavPack and Musepack use the ffmpeg to cached WAV fallback.
- Two difficult MP3 files with unusual ID3 tags now decode through an extension-based demuxer hint.
- WAV transcodes are written atomically so an interrupted conversion cannot look complete.

## v3.2.29

- Bundled CachyOS dvdbackup build for fast decrypted DVD ISO output.
- DUMP TO ISO prefers dvdbackup and genisoimage, with MakeMKV backup decryption as fallback.
- DVD dependency reporting for CachyOS, Arch, Debian, Ubuntu and Mint.
- New `mediaplayerofdoom --check-dvd` command.

## v3.2.28

- DVD and Blu-ray dumps are decrypted only.
- DUMP TO MKV decrypts and remuxes every title with MakeMKV.
- DUMP TO ISO creates a clean, playable image.
- Raw encrypted dump actions were removed from the UI.

## v3.2.27

- Fixed decrypted DVD ISO output by passing `--decrypt` to MakeMKV backup.

## v3.2.26

- Added decrypted DVD to ISO output with MakeMKV backup.
- Help was expanded with conversion, metadata, disc, CD, hardware and troubleshooting sections.

## v3.2.25

- Meta All repairs missing metadata from filenames and folders.
- Song title cover fallback was added.
- Meta All reports unrated and no cover counts.
- Disc dump speed, live MB/s and ETA were added.

## v3.2.24

- DVD seek fix with cached metadata, no per-seek duration probe, PAL and NTSC detection and yadif deinterlacing.

## v3.2.23

- Cover art pipeline improvements and a 20 worker cover download pool.
- MusicBrainz rating fallback for albums.
- Manual ratings are protected from Meta All.

## v3.2.22

- Lyrics and metadata improvements.
- Playlist and queue fixes.

## v3.2.21

- Audio conversion improvements and per file cancel support.

## v3.2.20

- Blu-ray AACS key database handling.
- KEYDB symlink environment for libaacs.
- Blu-ray status and key error reporting.

## v3.2.19

- DVD and Blu-ray playback improvements.
- Subtitle default off and English audio preference.

## v3.2.18

- Hardware acceleration detection for AMD, Intel and NVIDIA.
- Software fallback.

## v3.2.17

- CD ripping and MusicBrainz tagging.

## v3.2.16

- Recording improvements.
- Radio stream recording.

## v3.2.15

- Visualizer and speaker animation improvements.

## v3.2.14

- Playlist and library performance improvements.

## v3.2.13

- Metadata cache and art cache improvements.

## v3.2.12

- Queue and video player improvements.

## v3.2.11

- DVD seeking and playback fixes.

## v3.2.10

- Blu-ray playback fixes.

## v3.2.9

- YouTube download queue improvements.

## v3.2.8

- Lyrics and art lookup improvements.

## v3.2.7

- Conversion quality controls.

## v3.2.6

- XDG paths and desktop integration.

## v3.2.5

- Linux port fixes.

## v3.2.4

- Linux tray and desktop shortcut.

## v3.2.3

- Linux packaging.

## v3.2.2

- Linux dependency installer.

## v3.2.1

- Linux build fixes.

## v3.2.0

- Initial Linux port of MediaPlayerDOOM v3.
