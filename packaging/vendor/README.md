# Vendored DVD helper

## dvdbackup

- Version: 0.4.2-7.1 (CachyOS `cachyos-extra-v3`, x86_64_v3)
- Upstream: https://dvdbackup.sourceforge.net/
- Upstream source: https://sourceforge.net/projects/dvdbackup/files/dvdbackup/dvdbackup-0.4.2/
- Package: `dvdbackup-0.4.2-7.1-x86_64.pkg.tar.zst`
- SHA-256 of this binary: `2a26ba76f47082c4da467d06fc9ac148e1d42dbc108a889777d2fb7abe88f061`
- License: GPL-3.0-only (see `dvdbackup-COPYING.txt`)
- Arch/CachyOS package metadata: `dvdbackup-PKGINFO.txt`
- Upstream README: `dvdbackup-README.txt`

`install.sh` copies this binary to
`$XDG_DATA_HOME/mediaplayerofdoom/tools/dvdbackup` (normally
`~/.local/share/mediaplayerofdoom/tools/dvdbackup`). The app prefers that
bundled copy, then falls back to a system `dvdbackup` on `PATH`.

The binary is dynamically linked against `libdvdread`; `libdvdcss` is loaded
by libdvdread at runtime to decrypt CSS DVDs. Those libraries are not bundled
here, because they are part of the operating system and must match its ABI.

On CachyOS/Arch install them with:

```sh
sudo pacman -S libdvdread libdvdnav libdvdcss
```

On Debian/Ubuntu/Mint the equivalent packages are:

```sh
sudo apt install libdvdread4 libdvdnav4 libdvdcss2
```
