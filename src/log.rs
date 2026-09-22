#![allow(unused_imports)]
#![allow(dead_code)]
// Debug logging.
use std::fs::OpenOptions;
use std::io::Write;
use crate::util::data_dir;
pub const AV_DEBUG_LOG: bool = false;

pub fn av_log(msg: &str) {
    if !AV_DEBUG_LOG {
        return;
    }
    use std::io::Write as _;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(data_dir().join("avsync.log")) {
        let _ = writeln!(f, "[{}] {}", ms, msg);
    }
}

