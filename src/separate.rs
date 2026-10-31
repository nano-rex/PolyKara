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

/// Names a bundled separator might go by, most specific first.
fn program_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["demucs.cpp.main.exe", "demucs.exe", "separator.exe"]
    } else {
        &["demucs.cpp.main", "demucs", "separator"]
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

/// The model folder the separator needs, if any: [separation] model_path,
/// POLYKARA_SEPARATOR_MODEL, or a data folder under the PolyKara root.
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
    for folder in [paths().root.join("data").join("separator"), paths().root.join("separator"), paths().models.join("separator")] {
        if folder.exists() {
            return Some(folder);
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
    // Argument order [model] <input> <output-dir> matches demucs.cpp.main; a tool
    // that needs none, or a different order, is handled through [separation] args.
    let configured = load_config().get("separation").str_or("args", "");
    let args: Vec<String> = if configured.trim().is_empty() {
        let mut args = Vec::new();
        if let Some(model) = model_path() {
            args.push(model.display().to_string());
        }
        args.push(input.display().to_string());
        args.push(out_dir.display().to_string());
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

/// Find a "*vocal*" audio file anywhere under `dir`.
fn find_vocal_stem(dir: &Path) -> Option<PathBuf> {
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
                if audio && name.contains("vocal") {
                    return Some(path);
                }
            }
        }
    }
    None
}
