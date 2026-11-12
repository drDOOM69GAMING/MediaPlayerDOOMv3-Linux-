# Changelog

## v3.2.51

Single consolidated release since v3.2.50. This includes all of the local
3.2.51-3.2.68 development work, published as one version.

### Disc playback
- AUDIO lists and cycles DVD/Blu-ray audio tracks; English is the default and the choice is remembered per title.
- DVD SCALE really upscales 480/576 video to 1620x1080 with Lanczos plus CAS (or a 3x3 unsharp fallback); the setting is saved.
- A/V sync: audio waits for the first new frame after a seek, track change or scale change; restarts resume from the audio clock.
- MOVIE TRACKS window with explicit audio/subtitle lists, RESCAN and the DVD scale toggle.
- Embedded DVD/Blu-ray bitmap subtitles are still not rendered; external text subtitles work.

### Disc art
- Disc covers in the sidebar: local cover/folder/poster/front images, ISO sidecars, mounted folders and Blu-ray META/DL thumbnails first.
- IMDb is the primary poster lookup; Wikipedia is the fallback. Posters are cached and a generic badge is shown when nothing is found.
- The poster also appears while a movie is loading or paused.

### Blu-ray, BD+ and AACS
- Detects BD+ tables in `~/.config/bdplus`, counts convtab files and parses `libbdplus detected` / `BD+ handled`.
- Combined XDG config with links to the user's AACS KEYDB and BD+ tables; a user-supplied `libbdplus.so` is loaded through `LIBBDPLUS_PATH`.
- Local libaacs, libbdplus and the user's BD+ tables are always tried first; MakeMKV is only the fallback.
- `bd_splice` + ffmpeg main-title fallback when MakeMKV is missing.
- Detects AACS bus-encryption failures from `bd_info`; the probe timeout is 60 seconds and stale `makemkvcon guiserver` processes are cleaned up.
- RIP + PLAY: MakeMKV finds the longest title with `--minlength=0`, checks free space, rips and starts the result automatically.
- Direct live decrypt through MakeMKV `libmmbd` for bus-encrypted or unhandled discs, with no rip.
- Uses real `/dev/srN` nodes and canonicalises `/dev/cdrom` and `/dev/dvd`, which libmmbd rejects.
- Bus-encrypted playback uses one ffmpeg process (video on stdout, PCM audio on a FIFO) so libmmbd never deadlocks with a second reader.
- Parent-death signals and orphan-reader cleanup prevent a crashed player from holding the optical drive.

### Radio
- BROWSE STATES covers all 50 U.S. states, Washington DC and all 13 Canadian provinces/territories.
- Stations come from Radio Browser, ranked by popularity; broadcast call signs and frequencies are shown first and internet-only networks are filtered out.
- Each region shows up to 12 stations with codec, bitrate and votes; PLAY starts immediately.
- SCAN ALL is a mode: it loads stations from every state/province (about 250, fetched in parallel), then SCAN steps through them manually.
- The SCAN ALL queue is shuffled, and each full pass reshuffles for a new random order.
- The radio dial follows every scan step: real FM/AM frequencies where known, and a position across the FM scale for web-only stations. The status line shows the station number (`x/y`).
- SCAN ALL stays enabled until toggled off; STOP does not disable it. Manual tuning or the dial exits the mode.
- Browser and scan stations are hidden WEB presets, deduplicated by URL, so the FM/AM dial stays clean.
- Radio ffmpeg children are reaped off the UI thread so a stuck stream cannot freeze the window.

## v3.2.50

- Widened the sidebar so band and album names have more room.
- Album rows show more characters before truncating.
- Hovering a band or album now shows the full name and track count. The right-click conversion hint lives in Help instead of on every row.
- The band filter box uses the wider sidebar.

## v3.2.49

- The band filter now matches album names as well as band names.
- A band matched through an album opens automatically so the album row is visible.
- Album rows are filtered to the matching albums when the band name itself does not match.
- The band + album search text is cached and rebuilt only after the band map changes.

## v3.2.48

- Added a live filter above the A to Z band list.
- Typing filters the band names immediately; X clears the filter.
- The filter does not affect the playlist or the normal search field.

## v3.2.47

- Library scans now cache each directory listing by modification time.
- Unchanged folders are reused; adding, removing or renaming an entry invalidates only that folder.
- The cache is merged, so scanning one added folder does not discard the rest of the library cache.
- The cache lives in `scan_cache.json` in the app data folder; deleting it forces a full walk.
- Added a test for cache reuse and invalidation after adding and deleting files.

## v3.2.46

- CD mode now supports in-track seeking with FWD and REW.
- A CD seek restarts cdparanoia at the requested track offset while keeping the same track identity.
- The CD auto-advance polls tightly so the next track starts as soon as the current buffer drains.
- Added tests for CDDA seek path formatting and the cdparanoia start-offset argument.

## v3.2.45

- Added a SUBTITLE STYLE window with size, bottom margin and outline sliders.
- The style is saved in settings and applied the next time a subtitle is shown.
- Added a SUB STYLE button to the fullscreen and windowed video controls.
- The player now remembers the selected audio and subtitle track per video file.
- Added an Auto-select English subtitles option in the subtitle style window.
- Shift-click COVER clears the cached cover for the album and starts the normal local/web lookup again.
- Removed the redundant J button next to Search; the J key still focuses the field.
- Help documents both controls.

## v3.2.44

- The video player now lists and cycles audio tracks on local video files with the AUDIO button.
- Embedded subtitles can be selected with the SUBS button.
- Sidecar subtitle files such as `video.srt`, `video.eng.srt` and `video.ass` are found automatically.
- OPEN SUBS loads an arbitrary `.srt`, `.ass`, `.ssa` or `.vtt` file.
- Subtitles are rendered into the video frames, so they work with the existing reader and windowed playback.
- The video stream probe runs off the UI thread and only when a new file opens, not on every seek.
- Help and README document the new video controls.

## v3.2.43

- A second launch now forwards its files to the running window instead of starting a second player and a second audio engine.
- A normal second launch brings the existing window forward.
- The single-instance socket lives in the app data folder, uses 0600 permissions and is removed on a normal exit; stale sockets are cleaned up on the next launch.
- Help > System Manual now has a Check for Updates button.
- The rating row has a COVER button that replaces the current album cover with a local image.
- A chosen cover is written to the art cache for every track in the same album, so it survives restarts and track changes.

## v3.2.42

- The bottom bar now keeps a Support physical media tagline next to the playback status.
- Restyled the deck buttons with deeper bevels, rounded corners, clearer hover and pressed states, and larger labels.
- The transport keys use the same raised button style.
- The deck controls are now laid out like a boombox panel: a fixed left label column, equal-width transport keys and aligned button rows.
- The playlist, mode and file buttons line up in columns instead of drifting with label length.

## v3.2.41

### Band browser

- Each band row has a + and - expander that lists that band's albums with track counts.
- Selecting an album loads only that album and starts its first track.
- Band grouping now merges the same artist across folder layouts, so an existing `Disturbed` folder and a loose `Disturbed - Divisive` folder become one band with both albums.
- Folder names ending in a year, an edition name or a store suffix no longer create separate artists.
- Existing band libraries are repaired once on load, without losing any tracks.

### Conversion

- Right-click a band or an album in the browser to convert just that selection.
- The small conversion window lists MP3, AAC, FLAC, OGG Vorbis, Opus, WAV, AIFF, ALAC and WMA with compression 1 to 10.
- Output goes to the OS music folder as `<name> (<format>)`, keeping one subfolder per album when a whole band is converted.

### Ratings

- Help > System Manual now has a Clear All Star Ratings action.
- It asks for confirmation, removes manual and automatic ratings, and turns the star filter off.

### Playback

- Removed the duplicate NOW line under the cover art; the title line already shows the current song.
- Removed the redundant Playlist Only, Sequential and Dir Seq toggles.
- The red random / green in-order switch is now the single play-order control.
- Playback order applies to the loaded list: All songs, one band, or one album.
- Smart shuffle still weights the random order toward songs you play more.
- The O shortcut was removed.

### Updates

- The app checks the latest GitHub release shortly after launch and every six hours.
- When a newer release exists, a yellow banner appears at the top of the window.
- Clicking the banner opens the release page in the default browser.
- No update is downloaded or installed automatically.

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
- On-demand art now falls back from an album lookup to a song-title and MusicBrainz recording lookup.
- The old no-art cache is cleared once so tracks skipped by earlier source sets are retried with the new sources.

## Startup

- Startup auto-play no longer waits for the deferred library scan; the cached song starts right after the window appears.
- The cached playlist is no longer re-sorted at every launch.
- The display cache is built once instead of twice.
- The metadata repair pass only touches entries with missing, placeholder or leading-track-number values.
- When a cached playlist exists, the library scan starts 30 seconds after launch so the window and first song do not compete with a cold-drive walk.
- Library scans run at lower CPU and I/O priority and yield to playback and the UI.
- The first song after launch gets a 5-second quiet read window before the next track is pre-opened, so a cold drive is not forced to seek away from the song that is still starting. Later tracks pre-open immediately, so normal track changes stay seamless.
- `MPDOOM_AV_LOG=1` now records startup phase timings.
- The bottom-left play-order label shows the current mode and the drDOOM69GAMING credit.
- The play-order switch and credit are pinned to the bottom of the sidebar; the content above them scrolls in short windows so band names never sit under the switch.
- The play-order switch is red for random shuffle and green for in-order playback.
- Long song and album titles wrap inside the sidebar instead of widening it.
- The status bar truncates long status text and keeps the Exit button fixed.

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
