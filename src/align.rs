//! Automatic word timing, delegated to the bundled aligner.
//!
//! Whisper needs Python (faster-whisper) or the whisperx program, so this step
//! calls the aligner that ships with PolyKara (`polykara-align`) for one song
//! and leaves everything else to the Rust pipeline. The aligner writes
//! work/align/<id>.json, the same file the editor and the ASS step read.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{paths, tool};
use crate::csv::Row;
use crate::manifest::get;
use crate::subtitle::word_json;

/// The aligner program: POLYKARA_ALIGNER, next to this program, or on PATH.
pub fn find() -> Option<PathBuf> {
    if let Some(program) = std::env::var_os("POLYKARA_ALIGNER") {
        return Some(program.into());
    }
    let name = if cfg!(windows) { "polykara-align.exe" } else { "polykara-align" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(folder) = exe.parent() {
            for place in folder.ancestors() {
                let candidate = place.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    crate::config::find_tool("polykara-align")
}

pub fn available() -> bool {
    find().is_some()
}

/// True when word timing exists and is not older than the normalized audio.
pub fn is_aligned(row: &Row) -> bool {
    let timing = word_json(row);
    let audio = paths().audio.join(format!("{}.flac", get(row, "id")));
    match timing {
        None => false,
        Some(path) => !audio.exists() || modified(&path) >= modified(&audio),
    }
}

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path).and_then(|meta| meta.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

/// Time one song's words. The aligner reads songs.csv itself, so it needs only
/// the id; its own model and device settings apply.
pub fn align_row(row: &Row, dry_run: bool) -> Result<(), String> {
    let sid = get(row, "id");
    if get(row, "language").trim().is_empty() {
        return Err("language is missing".into());
    }
    let audio = paths().audio.join(format!("{sid}.flac"));
    if !audio.exists() && !dry_run {
        return Err(format!("normalized audio is missing: {}", audio.display()));
    }
    let aligner = find().ok_or_else(|| "no aligner is installed: install whisperx or the PolyKara aligner".to_string())?;
    let args = ["align".to_string(), "--only".to_string(), sid.to_string(), "--reprocess".to_string()];
    println!("$ {} {}", aligner.display(), args.join(" "));
    if dry_run {
        return Ok(());
    }
    let mut command = Command::new(&aligner);
    command.args(&args).stdin(Stdio::null());
    crate::tools::quiet(&mut command);
    let status = command.status().map_err(|error| format!("{} could not be started ({error})", aligner.display()))?;
    if !status.success() {
        return Err(format!("{} exited with status {}", aligner.display(), status.code().unwrap_or(-1)));
    }
    if word_json(row).is_none() {
        return Err("the aligner produced no word-level timing".into());
    }
    Ok(())
}

/// The aligner's own path, for the self-test to report.
pub fn shown() -> String {
    find().map(|path| path.display().to_string()).unwrap_or_else(|| "MISSING".into())
}

/// Kept for callers that only want the tool name.
pub fn program() -> String {
    tool("polykara-align").display().to_string()
}
