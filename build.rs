//! Windows build glue, no crates needed:
//! - embed the manifest (modern Common Controls v6, per-monitor DPI) into the exes,
//!   for both the MSVC and the MinGW (windows-gnu) builds, so both are themed;
//! - link the extra Windows libraries the UI theming uses.

fn main() {
    println!("cargo:rerun-if-changed=polykara.manifest");
    let target = std::env::var("TARGET").unwrap_or_default();
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("polykara.manifest");

    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    } else if target.contains("windows-gnu") {
        // Embed the same manifest with windres, so the MinGW build is themed too.
        if let Err(error) = embed_manifest_gnu(&manifest) {
            println!("cargo:warning=manifest not embedded in the GNU build: {error}");
        }
    }
}

/// Compile the manifest into a COFF object with windres and link it into the bins.
fn embed_manifest_gnu(manifest: &std::path::Path) -> Result<(), String> {
    let out = std::env::var("OUT_DIR").map_err(|error| error.to_string())?;
    let rc = std::path::Path::new(&out).join("polykara.rc");
    let obj = std::path::Path::new(&out).join("polykara_manifest.o");
    // Resource id 1 (CREATEPROCESS_MANIFEST_RESOURCE_ID), type 24 (RT_MANIFEST).
    let path = manifest.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 24 \"{path}\"\n")).map_err(|error| error.to_string())?;

    let arch = std::env::var("TARGET").unwrap_or_default();
    let arch = arch.split('-').next().unwrap_or("x86_64");
    let windres = format!("{arch}-w64-mingw32-windres");
    let status = std::process::Command::new(&windres)
        .args(["-O", "coff", "-i"])
        .arg(&rc)
        .arg("-o")
        .arg(&obj)
        .status()
        .map_err(|error| format!("{windres}: {error}"))?;
    if !status.success() {
        return Err(format!("{windres} exited with {status}"));
    }
    println!("cargo:rustc-link-arg-bins={}", obj.display());
    Ok(())
}
