#![allow(unused_imports)]
#![allow(dead_code)]
// App-wide constants.
pub const APP_NAME: &str = "Random Shuffle Player";
pub const APP_VERSION: &str = "3.0.1";

pub const AUDIO_FORMATS: [&str; 16] = [
    "mp3", "wav", "flac", "m4a", "m4b", "m4p", "ogg", "oga", "aac", "aiff", "aif", "wma", "wv", "mpc", "opus", "webm",
];

pub const VIZ_BANDS: usize = 12;
pub const VIZ_WIN: usize = 2048;
pub const VIZ_HOP: usize = 512;

pub const THEME_NAMES: [&str; 5] = ["Winamp", "Matrix", "Cyberpunk", "Amber", "Ocean"];

pub const NO_WINDOW: u32 = 0x0800_0000;
