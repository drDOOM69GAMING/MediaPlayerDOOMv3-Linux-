//! Single-instance file handoff.
//!
//! A second launch connects to a Unix socket owned by the running player and
//! forwards the files it was asked to open. The running window then plays them
//! instead of starting a second audio engine and a second library.

use std::io::{Read, Write};
use std::path::PathBuf;

#[cfg(unix)]
pub type InstanceListener = std::os::unix::net::UnixListener;
#[cfg(not(unix))]
pub struct InstanceListener;

pub enum Instance {
    Primary(InstanceListener),
    Forwarded,
    Unavailable,
}

pub fn socket_path() -> PathBuf {
    crate::util::data_dir().join("instance.sock")
}

#[cfg(unix)]
pub fn acquire(files: &[String]) -> Instance {
    use std::os::unix::net::{UnixListener, UnixStream};

    let path = socket_path();
    let connect = |payload: &str| -> bool {
        let Ok(mut stream) = UnixStream::connect(&path) else {
            return false;
        };
        let _ = stream.write_all(payload.as_bytes());
        let _ = stream.write_all(b"\n");
        let _ = stream.flush();
        true
    };

    // An empty payload asks the running window to come forward. A list of
    // newline-separated paths asks it to open those files.
    if !files.is_empty() {
        if connect(&files.join("\n")) {
            return Instance::Forwarded;
        }
    } else if connect("") {
        return Instance::Forwarded;
    }

    // No live listener. Remove whatever stale socket was left by a crash and
    // bind a fresh one for this process.
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            Instance::Primary(listener)
        }
        Err(_) => {
            // Another process won the bind race. Forward if it is listening.
            if !files.is_empty() && connect(&files.join("\n")) {
                Instance::Forwarded
            } else if files.is_empty() && connect("") {
                Instance::Forwarded
            } else {
                Instance::Unavailable
            }
        }
    }
}

#[cfg(not(unix))]
pub fn acquire(_files: &[String]) -> Instance {
    Instance::Unavailable
}

#[cfg(unix)]
pub fn spawn_listener(
    listener: InstanceListener,
    tx: std::sync::mpsc::Sender<crate::types::Msg>,
) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut payload = String::new();
            let _ = stream.read_to_string(&mut payload);
            let files: Vec<String> = payload
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            if files.is_empty() {
                let _ = tx.send(crate::types::Msg::FocusWindow);
            } else {
                let _ = tx.send(crate::types::Msg::OpenFiles(files));
            }
        }
    });
}

#[cfg(not(unix))]
pub fn spawn_listener(
    _listener: InstanceListener,
    _tx: std::sync::mpsc::Sender<crate::types::Msg>,
) {
}
