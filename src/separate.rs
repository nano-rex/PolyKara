//! Vocal/instrument separation by shelling out to a bundled separator, the way
//! Ultimate Vocal Remover does it: a prebuilt CPU model (e.g. demucs.cpp or a
//! UVR MDX-Net model) run as its own program. Generating subtitles splits the
//! song three ways, all kept under work/:
//!   - the vocal        → work/vocals/<id>.wav        (transcribed with whisper.cpp
//!                          to read the words that are actually sung — the subtitles);
//!   - the instrumental → work/instrumental/<id>.wav  (the karaoke backing);
//!   - the video        → work/video/<id>.<ext>       (the picture with no audio).
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

/// Where the isolated instrumental (karaoke backing) of a song is kept.
pub fn instrumental_path(sid: &str) -> PathBuf {
    paths().work.join("instrumental").join(format!("{sid}.wav"))
}

/// The silent video stream split out of a song's media, if one was made.
pub fn video_path(sid: &str) -> Option<PathBuf> {
    let folder = paths().work.join("video");
    let entries = std::fs::read_dir(&folder).ok()?;
    entries
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.file_stem().map(|stem| stem.to_string_lossy() == sid).unwrap_or(false))
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
    // Keep the instrumental (karaoke backing) stem too. Best-effort: a missing
    // instrumental never fails subtitle generation.
    match find_instrumental_stem(&out_dir) {
        Some(instrumental) => {
            let target = instrumental_path(sid);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if instrumental != target {
                match std::fs::copy(&instrumental, &target) {
                    Ok(_) => println!("{sid}: saved the instrumental track to {}", target.display()),
                    Err(error) => println!("{sid}: the instrumental stem could not be saved ({error})"),
                }
            }
        }
        None => println!("{sid}: the separator produced no instrumental stem; skipping it"),
    }
    Ok(vocals)
}

/// The song's isolated vocal when it exists and is not older than the song's media.
pub fn fresh_vocals(sid: &str) -> Option<PathBuf> {
    let vocals = vocals_path(sid);
    let made = std::fs::metadata(&vocals).and_then(|meta| meta.modified()).ok()?;
    let media = crate::manifest::source(sid).ok()?;
    let source = std::fs::metadata(&media).and_then(|meta| meta.modified()).ok()?;
    (made >= source).then_some(vocals)
}

/// Isolate the vocal of the song's media: a clean stereo wav goes through the
/// separator into work/vocals/<id>.wav, which is returned.
pub fn isolate(sid: &str, dry_run: bool) -> Result<PathBuf, String> {
    let media = crate::manifest::source(sid)?;
    let input = paths().work.join("vocals").join(format!("{sid}.input.wav"));
    if !dry_run {
        std::fs::create_dir_all(paths().work.join("vocals")).map_err(|error| error.to_string())?;
        let args: Vec<String> = ["-hide_banner", "-loglevel", "warning", "-y", "-i"]
            .iter()
            .map(|item| item.to_string())
            .chain(std::iter::once(media.display().to_string()))
            .chain(["-vn", "-ac", "2", "-ar", "44100"].iter().map(|item| item.to_string()))
            .chain(std::iter::once(input.display().to_string()))
            .collect();
        config::run("ffmpeg", &args, false, None)?;
    }
    separate(sid, &input, dry_run)
}

/// True when the file name is an audio stem (wav/flac/mp3/ogg).
fn is_audio(name: &str) -> bool {
    name.ends_with(".wav") || name.ends_with(".flac") || name.ends_with(".mp3") || name.ends_with(".ogg")
}

/// True when the stem name marks it as the instrumental (no-vocals) stem.
fn names_instrumental(name: &str) -> bool {
    name.contains("no_vocal") || name.contains("no vocal") || name.contains("instrumental") || name.contains("accompaniment") || name.contains("karaoke")
}

/// Find the instrumental (no-vocals) audio stem under `dir`.
fn find_instrumental_stem(dir: &Path) -> Option<PathBuf> {
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
                if is_audio(&name) && names_instrumental(&name) {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// Split the silent video stream out of the song's media into work/video/<id>.<ext>,
/// keeping the source container so the stream copies without re-encoding. Best-effort:
/// returns None when the media has no video (an audio-only download).
pub fn split_video(sid: &str, dry_run: bool) -> Result<Option<PathBuf>, String> {
    let media = crate::manifest::source(sid)?;
    if !crate::localmedia::probe(&media).video {
        return Ok(None);
    }
    let ext = media.extension().map(|ext| ext.to_string_lossy().to_lowercase()).unwrap_or_else(|| "mp4".into());
    let target = paths().work.join("video").join(format!("{sid}.{ext}"));
    if dry_run {
        return Ok(Some(target));
    }
    std::fs::create_dir_all(paths().work.join("video")).map_err(|error| error.to_string())?;
    let args: Vec<String> = ["-hide_banner", "-loglevel", "warning", "-y", "-i"]
        .iter()
        .map(|item| item.to_string())
        .chain(std::iter::once(media.display().to_string()))
        .chain(["-an", "-c", "copy"].iter().map(|item| item.to_string()))
        .chain(std::iter::once(target.display().to_string()))
        .collect();
    config::run("ffmpeg", &args, false, None)?;
    Ok(Some(target))
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
                if is_audio(&name) && name.contains("vocal") && !names_instrumental(&name) {
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
