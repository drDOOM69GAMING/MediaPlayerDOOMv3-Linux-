# Changelog

## v3.2.40

- Blu-ray DUMP TO ISO now writes a genuine UDF 2.50 image natively, inside the app.
- The image uses a UDF 2.50 metadata partition with mirrored metadata, the layout standalone Blu-ray players expect.
- No external `udf250`, Python script, genisoimage or helper tool is required for the Blu-ray ISO.
- The image is staged beside the destination and published atomically; cancelling removes the partial image.
- Full-disc backup with menus and extras is unchanged.

## Metadata

- Cover art now falls back to the Cover Art Archive release-group image and Discogs, with TheAudioDB high-resolution art when available.
- Album ratings now fall back to Discogs community ratings and TheAudioDB scores when MusicBrainz has no community rating.
- Discogs requests are throttled to the public API limit and cached per album for the session.

## Startup

- Startup auto-play no longer waits for the deferred library scan; the cached song starts right after the window appears.
- The cached playlist is no longer re-sorted at every launch.
- The display cache is built once instead of twice.
- The metadata repair pass only touches entries with missing, placeholder or leading-track-number values.
- When a cached playlist exists, the library scan starts 30 seconds after launch so the window and first song do not compete with a cold-drive walk.
- Library scans run at lower CPU and I/O priority and yield to playback and the UI.
- The first song after launch gets a 5-second quiet read window before the next track is pre-opened, so a cold drive is not forced to seek away from the song that is still starting. Later tracks pre-open immediately, so normal track changes stay seamless.
- `MPDOOM_AV_LOG=1` now records startup phase timings.
- The bottom-left play-order label now shows drDOOM69GAMING.

## v3.2.39

- PLAY CD now streams each track directly from the optical drive with cdparanoia.
- Nothing is ripped, cached or written to disk when playing a CD from the tray.
- CD tracks start from the drive, follow disc order and use the normal transport, tray controls and auto-advance.
- CD audio is buffered a few seconds ahead and paranoia checking is disabled for playback, removing stutter and long drive stalls.
- The deck shows a reading-from-drive status while the optical drive spins up.
- CD artist, album, track titles and album art load in the background without delaying playback.
- Album art stays visible while moving between tracks on the same disc.
- The CD deck counts only the loaded disc and hides the library band browser while a disc is loaded.
- A library scan no longer replaces the CD track list while a disc is loaded.
- Ejecting the tray stops direct CD playback and restores the saved library.
- Movie playback now uses a 60 Hz present loop instead of the 30 Hz music UI loop.
- The video texture is reused for every frame instead of being recreated.
- The movie audio clock is sampled off the UI thread every few milliseconds.
- Video frames are paced on a monotonic wall-clock schedule with slow audio drift correction, so intervals stay even instead of alternating between short and long.
- DVD output is forced to a constant frame rate, so soft-telecine streams no longer deliver frames in bursts.
- DVD and ISO playback keep audio sync while presenting frames more evenly.

## v3.2.38

- CD ripping now shows a live progress bar with percentage, read speed multiplier and ETA.
- CD progress updates from the growing extraction output, so the bar moves during each track.
- CD mode always plays tracks in disc order; shuffle, folder order and the star filter no longer reorder a disc.
- Next and Previous follow the disc track list instead of the library.
- The disc stays loaded when switching decks and returns when CD mode is selected again.
- Added PLAY CD to play the audio CD in the optical drive without ripping it into the library.
- The saved library is preserved while a disc is in the CD deck.

## v3.2.37

- Added Set as Default Media Player for audio and video.
- The Help window exposes a Set as Default Media Player button.
- Added `mediaplayerofdoom --set-default` and `--set-default-player` command line flags.
- The action writes the applications menu desktop entry and registers 25 audio and video MIME types with xdg-mime.
- Desktop launches now detect audio files and play them in the music player.
- Video files passed by the desktop still open in the video player.
- Multiple selected files are added to the playlist or video queue.
- No manual editing of mimeapps.list is required.

## v3.2.36

- Tray Play/Pause, Stop, Skip and Previous now run through the background playback worker, so they work while the window is hidden and the UI is frozen.
- Tray playback commands no longer wait for a UI repaint.
- Hidden song notifications now replace the previous popup by notification ID instead of stacking.
- Notification expiry remains 3.5 seconds, with one notification per song.

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
