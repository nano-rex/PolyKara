use crate::lyricformats::{export_lrc, export_srt, parse_file};
use crate::model::{Entry, Song};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

pub fn project_path(work: &Path, id: &str) -> PathBuf {
    work.join("project").join(format!("{}.json", id))
}
pub fn save(work: &Path, song: &Song, entries: &[Entry]) -> Result<()> {
    let path = project_path(work, &song.id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(entries)?)?;
    Ok(())
}
pub fn load(work: &Path, song: &Song) -> Result<Option<Vec<Entry>>> {
    let path = project_path(work, &song.id);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice(&fs::read(path)?)?))
}
pub fn load_or_lyrics(work: &Path, song: &Song) -> Result<(Vec<Entry>, String)> {
    if let Some(e) = load(work, song)? {
        return Ok((e, "project".into()));
    }
    if !song.lyrics_file.is_empty() {
        let p = PathBuf::from(&song.lyrics_file);
        if p.exists() {
            return Ok((parse_file(&p)?, "lyrics".into()));
        }
    }
    Err(anyhow::anyhow!("no project or lyrics file for {}", song.id))
}
pub fn to_json(entries: &[Entry]) -> Result<String> {
    Ok(serde_json::to_string_pretty(entries)?)
}
pub fn from_json(text: &str) -> Result<Vec<Entry>> {
    Ok(serde_json::from_str(text)?)
}
pub fn export(entries: &[Entry], format: &str) -> Result<String> {
    match format {
        "lrc" => Ok(export_lrc(entries)),
        "srt" => Ok(export_srt(entries)),
        "json" => to_json(entries),
        _ => Err(anyhow::anyhow!("unsupported export format: {}", format)),
    }
}
