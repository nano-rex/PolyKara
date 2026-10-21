//! Embed the Windows manifest (modern controls, per-monitor DPI) into PolyKara.exe. No crates needed.

fn main() {
    println!("cargo:rerun-if-changed=polykara.manifest");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("polykara.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    }
}
