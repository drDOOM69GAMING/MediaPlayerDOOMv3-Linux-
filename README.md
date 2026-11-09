# MediaPlayerDOOM v3

A retro DOOM-styled media player rendered in egui. Tape, CD, and Radio faces on the deck, plus a band/artist catalog, background pulsing speakers, recording, YouTube downloads, and a fullscreen in-app video player with a queue. Runs on Windows 10/11 and Linux (ported and tested on CachyOS/Arch with KDE Plasma on Wayland). On Windows it is a single self-contained exe; on Linux it uses the distro's ffmpeg/ffprobe/mkvmerge and downloads yt-dlp on first run.

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

**Linux (tested on CachyOS, Arch-based, KDE Plasma/Wayland)**
- Rust toolchain (`rustup` or the distro's `rust`)
- Runtime tools: `ffmpeg` (includes ffprobe) and `mkvmerge` (MKVToolNix) from your distro
- An XDG desktop portal for the file pickers (KDE/GNOME provide one)
- PipeWire or PulseAudio for sound

```bash
sudo pacman -S --needed base-devel rust ffmpeg mkvtoolnix-cli \
  alsa-lib openssl libxkbcommon wayland libx11
```

yt-dlp is downloaded automatically into the app data folder on first run if it
is not already on `PATH`. The Linux tray uses the pure-Rust StatusNotifierItem
implementation, so no GTK/libappindicator/libxdo packages are needed.

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
- **egui / eframe** (MIT/Apache-2.0) - the immediate-mode GUI toolkit that renders the whole DOOM-styled interface. <https://github.com/emilk/egui>
- **rodio / cpal** (MIT/Apache-2.0, Apache-2.0) - audio playback and device access.
- **symphonia** (MPL-2.0) - pure-Rust media demuxing/decoding.
- The **Rust** language and its crate ecosystem.

Full details, license texts, and donation links are in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). If you can, consider donating to FFmpeg, yt-dlp, and MKVToolNix - they give this all away for free.

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
