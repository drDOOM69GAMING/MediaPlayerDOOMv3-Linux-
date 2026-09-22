# MediaPlayerDOOM v3

A retro DOOM-styled media player for Windows, rendered in egui. Tape, CD, and Radio faces on the deck, plus a band/artist catalog, background pulsing speakers, recording, YouTube downloads, and a fullscreen in-app video player with a queue. ffmpeg, ffprobe, and yt-dlp are embedded in the exe, so it runs fully standalone.

## Features

- **Local metadata cache (`Meta All`)**: one click downloads every playlist track's tags + cover art into your `%APPDATA%\MediaPlayerOFDOOM` folder (metacache.json + artcache). Playlists then load with zero re-scanning, and the NOW panel shows tags and art instantly - even offline. Runs on 8 parallel workers and resumes from whatever is already cached.

- **Deck modes**: switch the central deck between TAPE / CD / RADIO faces
  - **TAPE**: auto-reverse cassette with counters, winding, and tune slider
  - **CD**: spinning disc tray with track counter, LCDs, and in-order playback
  - **RADIO**: FM/AM dial with click-and-drag tuning, presets, SCAN, live web streams, and an **ADD STATION** dialog to save your own FM/AM stream presets
- **Bands catalog**: songs grouped by artist folder (A-Z, scrollable), rebuilt from whatever folder you add
- **Play order switch**: flip the retro wall switch under BANDS. ON = random shuffle, OFF = normal in-order playback
- **Smart shuffle**: weighted by your listening history, or plain shuffle
- **Live visualizer**: a real-time 12-band spectrum with peak caps in the NOW panel, synced to whatever is playing (music or radio). The deck's bass woofers pump with the volume, rattle on heavy bass, shove downward on beats, and pulse an expanding ring on loud hits
- **Recording**: capture the current track or a live stream (REC on the deck)
- **Video player**: click **Video** to play the matching movie in-app:
  - True fullscreen playback with letterboxing
  - Auto-hiding top bar (mouse to the top of the screen to reveal) with PLAY/PAUSE, prev/next, **+ ADD** (multi-file picker), and show/hide queue
  - Right-side queue panel with per-item play, remove, and CLEAR; auto-advances when a video ends
  - **Automatic intro/credits detection**: when **SKIP: ON**, the app analyzes each video's audio and finds where the intro ends and the credits/outro begins, then auto-skips the intro and jumps to the next video at the credits. Detected times are remembered per file and per folder
  - **Manual markers**: use the top bar **INTRO** / **CREDS** buttons to set the intro-end and credits-start for the current video at the current playback position; values persist per file/folder across runs
  - The top bar and queue stay visible while your mouse is over the queue panel, so you can scroll it and press CLEAR without the bar disappearing
- **Album art**: local `cover/folder` art always wins; online lookup only as a fallback when no local art exists (so pictures never get replaced by wrong web matches)
- **Lyrics**: Artist/Track auto-detected from the filename when tags are unknown
- **Playlist management**: drag-to-reorder, save/load JSON, playlist-only mode, sequential mode, per-CD directory ordering, and a live current-song highlight that scrolls to and marks whatever is playing
- **System tray** + global hotkeys (media keys)
- **Taskbar icon**: the app icon is applied directly on the real window handle at launch, so it always shows in the taskbar and Alt-Tab
- **YouTube**: paste a URL or search term to download audio/video via the embedded yt-dlp

## Requirements

- Windows 10/11 x64
- Rust toolchain to build from source

**The exe is self-contained (single file).** ffmpeg, ffprobe, and yt-dlp are embedded at build time. On first run they self-extract to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools`, and nothing appears next to the exe. You can move or copy the exe anywhere by itself and everything still works.

## Build

```
cargo build --release
```

The binary lands at `target/release/mediaplayerofdoom.exe`. It finds your music automatically by scanning all drives for the `Music` folder with the most files, or pick any folder with **ADD DIR**.

## Keyboard

`P` play/pause · `S`/`N` skip · `Left` previous · `R` record · `+`/`-` volume · `F` fullscreen · `M` mute · `D` mode · `V` video · `L` lyrics

## App Images
<img width="3840" height="2160" alt="stars for the star" src="https://github.com/user-attachments/assets/c68a0bfc-d7ab-4d31-a8fd-d9bdf186a1ca" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 133844" src="https://github.com/user-attachments/assets/49e8a167-8b41-4166-9239-08067e05e3c4" />
<img width="3824" height="2155" alt="Screenshot 2026-09-10 133812" src="https://github.com/user-attachments/assets/6a162706-f897-40e7-b048-7eb8326796f1" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 133734" src="https://github.com/user-attachments/assets/052d9c5d-37c1-475f-bdf8-a1cabd113df7" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 134017" src="https://github.com/user-attachments/assets/d040a5a5-f128-474b-9180-3e675a6eca9c" />

## Notes

- The **AUTO preamp** in the EQ measures the boosted response and backs off the gain automatically so you never clip, even with a loud preset.
- The EQ curve panel, preset cycling, band sliders, and the ON-OFF toggle all apply your changes live while music plays, with no restart and no temporary files.
- If you build from source and have `tools/ffmpeg.exe`, `tools/ffprobe.exe`, `tools/yt-dlp.exe` next to `Cargo.toml`, they are embedded into the exe by `build.rs` (that folder is gitignored). Without them the app falls back to downloading on first run.

## Acknowledgements

This app is built on the work of some incredible open-source projects, and it wouldn't exist without them:

- **FFmpeg / FFprobe** (GPL v3) - audio/video decoding, transcoding, metadata, and analysis. <https://ffmpeg.org/> - <https://www.gyan.dev/ffmpeg/builds/>
- **yt-dlp** (Unlicense) - YouTube downloads. <https://github.com/yt-dlp/yt-dlp>
- **MKVToolNix / mkvmerge** (GPL v2) - real lossless video cuts at INTRO/CREDS markers. <https://mkvtoolnix.download/>
- **egui / eframe** (MIT/Apache-2.0) - the immediate-mode GUI toolkit that renders the whole DOOM-styled interface. <https://github.com/emilk/egui>
- **rodio / cpal** (MIT/Apache-2.0, Apache-2.0) - audio playback and device access.
- **symphonia** (MPL-2.0) - pure-Rust media demuxing/decoding.
- The **Rust** language and its crate ecosystem.

Full details, license texts, and donation links are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). If you can, consider donating to FFmpeg, yt-dlp, and MKVToolNix - they give this all away for free.

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
