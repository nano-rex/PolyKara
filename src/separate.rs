//! Vocal/instrument separation by shelling out to a bundled separator, the way
//! Ultimate Vocal Remover does it: a prebuilt CPU model (e.g. demucs.cpp or a
//! UVR MDX-Net model) run as its own program. PolyKara only needs the vocal
//! stem — written to work/vocals/<id>.wav — which it then transcribes with
//! whisper.cpp to get an independent reading of the words that are actually sung.
//!
//! Nothing here is PolyKara-specific to one tool: the program, its model folder
//! and its argument order are taken from `[separation]` in polykara.toml so the
//! bundled separator can be swapped without code changes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{self, load_config, paths, resolve};

/// Names a bundled separator might go by, most specific first. PolyKara bundles
/// stems.cpp's `stems-split`; demucs.cpp and a generic name are also recognised.
fn program_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["stems-split.exe", "demucs.cpp.main.exe", "demucs.exe", "separator.exe"]
    } else {
        &["stems-split", "demucs.cpp.main", "demucs", "separator"]
    }
}

/// The separator program: POLYKARA_SEPARATOR, [separation] program, next to this
/// program, or on PATH.
pub fn find() -> Option<PathBuf> {
    if let Some(program) = std::env::var_os("POLYKARA_SEPARATOR") {
        return Some(program.into());
    }
    let configured = load_config().get("separation").str_or("program", "");
    if !configured.trim().is_empty() {
        let path = resolve(configured.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(folder) = exe.parent() {
            for place in folder.ancestors() {
                for name in program_names() {
                    let candidate = place.join(name);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    for name in program_names() {
        if let Some(found) = config::find_tool(name.trim_end_matches(".exe")) {
            return Some(found);
        }
    }
    None
}

pub fn available() -> bool {
    find().is_some()
}

/// The program path, for the self-test to report.
pub fn shown() -> String {
    find().map(|path| path.display().to_string()).unwrap_or_else(|| "MISSING".into())
}

/// The separation model: POLYKARA_SEPARATOR_MODEL, [separation] model_path, or the
/// first `.gguf` found in the usual model folders (stems.cpp uses htdemucs-f32.gguf).
pub fn model_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("POLYKARA_SEPARATOR_MODEL") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }
    let configured = load_config().get("separation").str_or("model_path", "");
    if !configured.trim().is_empty() {
        let path = resolve(configured.trim());
        if path.exists() {
            return Some(path);
        }
    }
    for folder in [paths().models.clone(), paths().root.join("data").join("separator"), paths().root.join("separator"), paths().models.join("separator")] {
        if let Ok(entries) = std::fs::read_dir(&folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|ext| ext.eq_ignore_ascii_case("gguf")).unwrap_or(false) {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Where the isolated vocal of a song is kept.
pub fn vocals_path(sid: &str) -> PathBuf {
    paths().work.join("vocals").join(format!("{sid}.wav"))
}

/// Isolate the vocal of `input` (a wav/flac) into work/vocals/<id>.wav and return
/// that path. The separator writes several stems into a per-song folder; the one
/// whose name mentions the vocal is copied out.
pub fn separate(sid: &str, input: &Path, dry_run: bool) -> Result<PathBuf, String> {
    let program = find().ok_or_else(|| {
        "a vocal separator is not installed: put demucs.cpp (or another UVR-style separator) in the PolyKara bin folder, or set POLYKARA_SEPARATOR".to_string()
    })?;
    let out_dir = paths().work.join("vocals").join(sid);
    let vocals = vocals_path(sid);
    // Default to stems.cpp's flags: `-m MODEL -i INPUT -o OUT --two-stems vocals`,
    // which writes OUT/vocals.wav (and OUT/no_vocals.wav). A different separator or
    // argument order is handled through [separation] args with {model}/{input}/{output_dir}.
    let configured = load_config().get("separation").str_or("args", "");
    if configured.trim().is_empty() && model_path().is_none() {
        return Err("the separation model is missing: put htdemucs-f32.gguf in the models folder, or set [separation] model_path".into());
    }
    let args: Vec<String> = if configured.trim().is_empty() {
        let mut args = Vec::new();
        if let Some(model) = model_path() {
            args.push("-m".to_string());
            args.push(model.display().to_string());
        }
        args.push("-i".to_string());
        args.push(input.display().to_string());
        args.push("-o".to_string());
        args.push(out_dir.display().to_string());
        args.push("--two-stems".to_string());
        args.push("vocals".to_string());
        args
    } else {
        configured
            .split_whitespace()
            .map(|token| match token {
                "{model}" => model_path().map(|path| path.display().to_string()).unwrap_or_default(),
                "{input}" => input.display().to_string(),
                "{output_dir}" => out_dir.display().to_string(),
                other => other.to_string(),
            })
            .filter(|token| !token.is_empty())
            .collect()
    };
    println!("$ {} {}", program.display(), args.join(" "));
    if dry_run {
        return Ok(vocals);
    }
    std::fs::create_dir_all(&out_dir).map_err(|error| error.to_string())?;
    let mut command = Command::new(&program);
    command.args(&args).stdin(Stdio::null());
    crate::tools::quiet(&mut command);
    let status = command.status().map_err(|error| format!("{} could not be started ({error})", program.display()))?;
    if !status.success() {
        return Err(format!("{} exited with status {}", program.display(), status.code().unwrap_or(-1)));
    }
    let produced = find_vocal_stem(&out_dir).ok_or_else(|| format!("the separator produced no vocal stem in {}", out_dir.display()))?;
    std::fs::create_dir_all(paths().work.join("vocals")).map_err(|error| error.to_string())?;
    if produced != vocals {
        std::fs::copy(&produced, &vocals).map_err(|error| error.to_string())?;
    }
    Ok(vocals)
}

/// Find the isolated-vocal audio file under `dir`. Prefers an exact "vocals"/"vocal"
/// stem and never returns the "no_vocals" / instrumental stem.
fn find_vocal_stem(dir: &Path) -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
                let audio = name.ends_with(".wav") || name.ends_with(".flac") || name.ends_with(".mp3") || name.ends_with(".ogg");
                let not_vocal = name.contains("no_vocal") || name.contains("no vocal") || name.contains("instrumental") || name.contains("accompaniment") || name.contains("karaoke");
                if audio && name.contains("vocal") && !not_vocal {
                    let stem = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
                    if stem == "vocals" || stem == "vocal" {
                        return Some(path);
                    }
                    fallback.get_or_insert(path);
                }
            }
        }
    }
    fallback
}
