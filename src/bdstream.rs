// Direct libbluray streamer.
//
// ffmpeg's `bluray:` protocol uses libbluray but does not expose its seek API:
// `-ss` stalls and `-chapter` is ignored on some discs. libbluray itself can
// seek instantly (`bd_seek_time`), so the player runs this tiny hidden mode as
// a child process, pipes decrypted M2TS data into ffmpeg, and restarts the
// child at the requested timestamp when the user seeks.
//
// The library is loaded with dlopen so the app still runs (for local media)
// on systems without libbluray.

#![allow(dead_code)]

#[cfg(target_os = "linux")]
pub use imp::{run_cli, spawn_stream};

#[cfg(not(target_os = "linux"))]
pub fn run_cli(_args: &[String]) -> i32 {
    eprintln!("Blu-ray streaming is only supported on Linux");
    1
}

#[cfg(not(target_os = "linux"))]
pub fn spawn_stream(_source: &str, _playlist: u32, _seek: f32) -> Option<std::process::ChildStdout> {
    None
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::CString;
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    use std::process::{ChildStdout, Command, Stdio};
    use std::sync::OnceLock;

    type Bd = libc::c_void;
    type BdOpen = unsafe extern "C" fn(*const libc::c_char, *const libc::c_char) -> *mut Bd;
    type BdClose = unsafe extern "C" fn(*mut Bd) -> libc::c_int;
    type BdGetTitles = unsafe extern "C" fn(*mut Bd, u8, u32) -> u32;
    type BdSelectPlaylist = unsafe extern "C" fn(*mut Bd, u32) -> libc::c_int;
    type BdSeekTime = unsafe extern "C" fn(*mut Bd, u64) -> i64;
    type BdRead = unsafe extern "C" fn(*mut Bd, *mut libc::c_void, libc::c_int) -> libc::c_int;

    struct Api {
        _handle: *mut libc::c_void,
        open: BdOpen,
        close: BdClose,
        get_titles: BdGetTitles,
        select_playlist: BdSelectPlaylist,
        seek_time: BdSeekTime,
        read: BdRead,
    }

    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    unsafe fn symbol<T: Copy>(handle: *mut libc::c_void, name: &[u8]) -> Option<T> {
        let p = libc::dlsym(handle, name.as_ptr() as *const libc::c_char);
        if p.is_null() {
            None
        } else {
            Some(std::mem::transmute_copy(&p))
        }
    }

    fn load() -> Option<Api> {
        for name in ["libbluray.so.4", "libbluray.so.2", "libbluray.so.1", "libbluray.so"] {
            let Ok(cname) = CString::new(name) else { continue };
            let handle = unsafe { libc::dlopen(cname.as_ptr(), libc::RTLD_NOW) };
            if handle.is_null() {
                continue;
            }
            let api = unsafe {
                match (
                    symbol::<BdOpen>(handle, b"bd_open\0"),
                    symbol::<BdClose>(handle, b"bd_close\0"),
                    symbol::<BdGetTitles>(handle, b"bd_get_titles\0"),
                    symbol::<BdSelectPlaylist>(handle, b"bd_select_playlist\0"),
                    symbol::<BdSeekTime>(handle, b"bd_seek_time\0"),
                    symbol::<BdRead>(handle, b"bd_read\0"),
                ) {
                    (Some(open), Some(close), Some(get_titles), Some(select_playlist), Some(seek_time), Some(read)) => {
                        Some(Api { _handle: handle, open, close, get_titles, select_playlist, seek_time, read })
                    }
                    _ => None,
                }
            };
            if api.is_some() {
                return api;
            }
            unsafe { libc::dlclose(handle) };
        }
        None
    }

    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(load).as_ref()
    }

    /// Hidden CLI mode: `mediaplayerofdoom --bd-stream <source> <playlist> <seconds>`.
    /// Writes the decrypted M2TS stream to stdout.
    pub fn run_cli(args: &[String]) -> i32 {
        if args.len() < 3 {
            eprintln!("usage: --bd-stream <source> <playlist> <seconds>");
            return 2;
        }
        let source = &args[0];
        let playlist: u32 = match args[1].parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("invalid playlist: {}", args[1]);
                return 2;
            }
        };
        let seek: f64 = args[2].parse::<f64>().unwrap_or(0.0).max(0.0);
        let Some(api) = api() else {
            eprintln!("libbluray not found");
            return 3;
        };
        let Ok(csource) = CString::new(source.as_str()) else {
            eprintln!("invalid source path");
            return 2;
        };
        let bd = unsafe { (api.open)(csource.as_ptr(), std::ptr::null()) };
        if bd.is_null() {
            eprintln!("could not open Blu-ray source {}", source);
            return 4;
        }
        // Load the title list before selecting a playlist; libbluray otherwise
        // warns that bd_play()/bd_get_titles() was not called.
        unsafe { (api.get_titles)(bd, 0x03, 0) }; // TITLES_RELEVANT
        if unsafe { (api.select_playlist)(bd, playlist) } <= 0 {
            eprintln!("could not select playlist {}", playlist);
            unsafe { (api.close)(bd) };
            return 5;
        }
        if seek > 0.05 {
            let ticks = (seek * 90000.0).round() as u64;
            let ret = unsafe { (api.seek_time)(bd, ticks) };
            if ret < 0 {
                // UO mask restrictions can forbid time search. Fall back to
                // the nearest chapter so seeking still gets close.
                eprintln!("time seek to {:.1}s refused; continuing from start", seek);
            }
        }
        // Avoid line-buffered stdout: this is a binary stream.
        let mut out = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(1) });
        let mut buf = vec![0u8; 188 * 64];
        loop {
            let n = unsafe {
                (api.read)(bd, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as libc::c_int)
            };
            if n <= 0 {
                break;
            }
            if out.write_all(&buf[..n as usize]).is_err() {
                break;
            }
        }
        let _ = out.flush();
        unsafe { (api.close)(bd) };
        0
    }

    /// Start the hidden streamer and return its stdout, ready to be used as
    /// ffmpeg's stdin. A waiter thread reaps the child when it exits.
    pub fn spawn_stream(source: &str, playlist: u32, seek: f32) -> Option<ChildStdout> {
        let exe = std::env::current_exe().ok()?;
        let mut cmd = Command::new(exe);
        cmd.arg("--bd-stream")
            .arg(source)
            .arg(playlist.to_string())
            .arg(format!("{:.3}", seek.max(0.0)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(if std::env::var_os("MPDOOM_AV_LOG").is_some() {
                Stdio::inherit()
            } else {
                Stdio::null()
            });
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                    Ok(())
                });
            }
        }
        // Keep the same AACS/XDG configuration ffmpeg would have used.
        crate::bluray::configure_command(&mut cmd);
        let mut child = cmd.spawn().ok()?;
        let stdout = child.stdout.take()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Some(stdout)
    }
}
