use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let rc = Path::new(&manifest).join("assets").join("app_icon.rc");
    println!("cargo:rerun-if-changed={}", rc.display());
    if rc.is_file() {
        embed_resource::compile(&rc, embed_resource::NONE);
    }

    let tools = Path::new(&manifest).join("tools");
    // Bundled helper binaries are Windows .exe files. Only embed them when the
    // target is Windows; a Linux build must resolve the distro's tools instead.
    let target_is_windows = std::env::var("CARGO_CFG_TARGET_OS")
        .map(|os| os == "windows")
        .unwrap_or(false);
    let mut out = String::new();
    for (name, const_name) in [
        ("ffmpeg.exe", "EMBEDDED_FFMPEG"),
        ("ffprobe.exe", "EMBEDDED_FFPROBE"),
        ("yt-dlp.exe", "EMBEDDED_YTDLP"),
        ("mkvmerge.exe", "EMBEDDED_MKVMERGE"),
    ] {
        let p = tools.join(name);
        println!("cargo:rerun-if-changed={}", p.display());
        if target_is_windows && p.is_file() {
            let lit = p.to_string_lossy().replace('\\', "/");
            out.push_str(&format!(
                "pub const {}: Option<&'static [u8]> = Some(include_bytes!(\"{}\"));\n",
                const_name, lit
            ));
        } else {
            out.push_str(&format!(
                "pub const {}: Option<&'static [u8]> = None;\n",
                const_name
            ));
        }
    }
    let dst = Path::new(&std::env::var("OUT_DIR").unwrap()).join("embedded_tools.rs");
    std::fs::write(dst, out).unwrap();
}
