# Third-Party Notices

MediaPlayerDOOM **v2.8.0** is original software released under the MIT License (see `LICENSE`). The MIT license covers only the code written by the project author(s).

This application bundles and relies on several third-party components, each of which remains the property of its respective authors and is used under its own license. We are grateful to every project below - this program would not exist without them. **A huge thank you to all of their developers and contributors.**

## Embedded command-line tools

The release exe embeds the following programs and self-extracts them to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools` on first run. They are shipped unmodified, in their original form, and are invoked as separate processes by this app.

### FFmpeg and FFprobe

- **Used for:** transcoding, metadata extraction, audio analysis, video decoding.
- **Version embedded:** 9.0.1 (essentials build for Windows by `www.gyan.dev`)
- **License:** GPL v3 (this particular build is compiled with `--enable-gpl` and `--enable-version3`)
- **Copyright:** Copyright (c) 2000-2026 the FFmpeg developers
- **Source:** <https://ffmpeg.org/> - source code available at <https://ffmpeg.org/download.html>
- **Binary/source distribution used:** <https://www.gyan.dev/ffmpeg/builds/>

Thank you to the entire FFmpeg team and the gyan.dev build maintainers for making the world's most capable media toolkit free for everyone.

**GPL note:** FFmpeg is distributed under the **GNU GPL v3**. A copy of the license text is available at <https://www.gnu.org/licenses/gpl-3.0.html>. The corresponding source code can be obtained from the FFmpeg project (link above). This app merely invokes FFmpeg as an external program; it is not linked with or incorporated into FFmpeg.

### yt-dlp

- **Used for:** YouTube downloads (audio/video) from paste-in URLs or search terms.
- **Version embedded:** 2026.08.19
- **License:** The Unlicense (public domain) - see <https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE>
- **Project:** <https://github.com/yt-dlp/yt-dlp>

Thank you to the yt-dlp team for maintaining the best open-source YouTube downloader.

### MKVToolNix (`mkvmerge` / `mkvmerge` cut tool)

- **Used for:** cutting video at INTRO/CREDS markers to produce real, lossless cut files.
- **Version embedded:** v101.0
- **License:** GNU GPL v2 (with the typical "or later" grant) - see <https://gitlab.com/mbunkus/mkvtoolnix>
- **Project:** <https://mkvtoolnix.download/> - <https://gitlab.com/mbunkus/mkvtoolnix>

Thank you to Moritz Bunkus and the MKVToolNix contributors for the gold standard of Matroska tooling.

### Contributing to donations

If you appreciate what these projects do, please consider donating to or sponsoring **FFmpeg** (<https://ffmpeg.org/donate.html>), **yt-dlp** (GitHub sponsors), and **MKVToolNix** (<https://mkvtoolnix.download/>). They give their work away for free and earn nothing from being bundled in hobby projects like this one.

## Rust crates (build-time and runtime libraries)

The application is built with the Rust crates listed in `Cargo.toml`. Each crate is licensed under the terms shown (SPDX identifiers). Their full license texts are bundled with each crate in the Rust registry, and are also available at <https://crates.io/>.

### MIT OR Apache-2.0
`eframe`, `egui`, and the egui/emath/epaint family, `rodio`, `lofty`, `reqwest`, `serde`, `serde_json`, `regex`, `tray-icon`, `global-hotkey`, `raw-window-handle`, `windows-sys`, `flate2`, `image`, `native-tls`, `url`, `rand`, `byteorder`, `sha1`.

### Apache-2.0
`cpal`, `winit`, `hound`, `openssl`, `windows`.

### MIT
`rfd`, `zip`, `embed-resource`, `libloading`, `crossbeam`, `memmap2`, `walkdir`, `tempfile`.

### MPL-2.0
`symphonia` and the `symphonia-*` family.

### Other / permissive
Various transitive dependencies (e.g. `bitflags`, `bytemuck`, `async-*`, `wayland-*`, `objc2-*`, `glow`, `wgpu`) are licensed under MIT, Apache-2.0, BSD, Zlib, or ISC as indicated in crates.io; these are used on non-Windows/optional platforms or as plumbing.

Thank you to every crate author whose work is linked into this binary.

## Fonts

The egui/eframe UI uses the open-source fonts bundled with the `epaint` crate (recipes/emblems, Noto-style emoji glyphs), licensed under OFL/Apache terms respectively. The retro UI is drawn with themed colors on these fonts plus locally-embedded icon glyphs.

## Art

The DOOM-styled speaker, deck, and window art are original to this project. The DOOM aesthetic is inspired by id Software's classic *DOOM* games; this project is an independent fan-style work and is not affiliated with or endorsed by id Software / Bethesda.

---

*If you are a maintainer of any project listed here and feel this notice is missing required attribution, please open an issue on this repository and we will correct it promptly.*
