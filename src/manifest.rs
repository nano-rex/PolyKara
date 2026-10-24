//! songs.csv: the list of songs, and the media and tools they need.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::{self, paths, MEDIA_SUFFIXES};
use crate::csv::{self, Row};

pub const REQUIRED_COLUMNS: &[&str] = &["id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms"];
pub const COLUMNS: &[&str] = &["id", "url", "title", "artist", "lyricist", "composer", "producer", "template", "language", "subtitle_langs", "lyrics_file", "word_timing_file", "logo_file", "prompt_ms", "align_model", "lyric_sources", "lyric_pages", "voice"];

pub fn valid_id(sid: &str) -> bool {
    !sid.is_empty() && sid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn get<'a>(row: &'a Row, key: &str) -> &'a str {
    row.get(key).map(String::as_str).unwrap_or("")
}

/// songs.csv as (columns, rows), with every known column present. A missing file is an empty list.
pub fn read_songs() -> (Vec<String>, Vec<Row>) {
    let text = std::fs::read_to_string(&paths().manifest).unwrap_or_default();
    if text.trim().is_empty() {
        return (COLUMNS.iter().map(|name| name.to_string()).collect(), Vec::new());
    }
    let (mut fields, rows) = csv::read(&text);
    for name in COLUMNS {
        if !fields.iter().any(|field| field == name) {
            fields.push(name.to_string());
        }
    }
    (fields, rows)
}

/// Write songs.csv through a temporary file, so a crash cannot truncate it.
pub fn write_songs(fields: &[String], songs: &[Row]) -> Result<(), String> {
    let mut fields = fields.to_vec();
    let mut missing: Vec<&str> = REQUIRED_COLUMNS.iter().copied().filter(|name| !fields.iter().any(|field| field == name)).collect();
    missing.sort_unstable();
    fields.extend(missing.into_iter().map(str::to_string));
    let manifest = paths().manifest.clone();
    write_atomic(&manifest, &csv::write(&fields, songs))
}

pub fn write_atomic(path: &std::path::Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut partial = path.as_os_str().to_os_string();
    partial.push(".tmp");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, text).map_err(|error| format!("{} could not be written ({error})", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|error| format!("{} could not be replaced ({error})", path.display()))
}

/// The songs to work on: every valid row, or only the `selected` ids. Errors explain what is wrong with songs.csv.
pub fn rows(selected: &[String]) -> Result<Vec<Row>, String> {
    let manifest = paths().manifest.clone();
    let text = std::fs::read_to_string(&manifest).map_err(|_| format!("Manifest not found: {}", manifest.display()))?;
    let (fields, data) = csv::read(&text);
    let mut missing: Vec<&str> = REQUIRED_COLUMNS.iter().copied().filter(|name| !fields.iter().any(|field| field == name)).collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        return Err(format!("songs.csv is missing: {}", missing.join(", ")));
    }
    let (mut result, mut seen, mut urls) = (Vec::new(), HashSet::new(), HashSet::new());
    for row in data {
        let row: Row = row.into_iter().map(|(key, value)| (key, value.trim().to_string())).collect();
        let sid = get(&row, "id").to_string();
        if sid == "example" || row.values().all(|value| value.is_empty()) {
            continue;
        }
        if !valid_id(&sid) || seen.contains(&sid) {
            return Err(format!("Invalid or duplicate id: {sid:?}"));
        }
        let url = get(&row, "url").to_string();
        if url.is_empty() {
            return Err(format!("Missing url for {sid}"));
        }
        if urls.contains(&url) {
            return Err(format!("Duplicate url for {sid}; use one manifest row per source"));
        }
        seen.insert(sid);
        urls.insert(url);
        result.push(row);
    }
    let wanted: HashSet<&String> = selected.iter().filter(|item| !item.trim().is_empty()).collect();
    let mut unknown: Vec<&str> = wanted.iter().filter(|sid| !seen.contains(sid.as_str())).map(|sid| sid.as_str()).collect();
    if !unknown.is_empty() {
        unknown.sort_unstable();
        return Err(format!("Not in songs.csv: {}", unknown.join(", ")));
    }
    if wanted.is_empty() {
        Ok(result)
    } else {
        Ok(result.into_iter().filter(|row| wanted.contains(&row.get("id").cloned().unwrap_or_default())).collect())
    }
}

pub fn is_media(name: &str) -> bool {
    let lower = name.to_lowercase();
    MEDIA_SUFFIXES.iter().any(|suffix| lower.ends_with(suffix))
}

/// The downloaded media of a song: <id>.mp4, else <id>.<media suffix> (not yt-dlp leftovers like id.f137.mp4).
pub fn find_source(sid: &str) -> Option<PathBuf> {
    let raw = paths().raw.clone();
    let exact = raw.join(format!("{sid}.mp4"));
    if exact.is_file() {
        return Some(exact);
    }
    let prefix = format!("{sid}.");
    let mut names: Vec<String> = std::fs::read_dir(&raw)
        .map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names.into_iter().find(|name| match name.strip_prefix(&prefix) {
        Some(extension) => !extension.contains('.') && is_media(name),
        None => false,
    }).map(|name| raw.join(name))
}

pub fn source(sid: &str) -> Result<PathBuf, String> {
    find_source(sid).ok_or_else(|| format!("No downloaded media found for {sid}"))
}

/// Free space in GiB on the drive of `path` (None when unknown).
pub fn free_gib(path: &std::path::Path) -> Option<f64> {
    crate::tools::free_bytes(path).map(|bytes| bytes as f64 / 1024f64.powi(3))
}

/// Print the tools and songs; an error when a required tool is missing.
pub fn check(need_ytdlp: bool) -> Result<(), String> {
    println!("Songs: {}", rows(&[])?.len());
    let tools = ["yt-dlp", "ffmpeg", "ffprobe"];
    for tool in tools {
        println!("{tool}: {}", config::find_tool(tool).map(|path| path.display().to_string()).unwrap_or_else(|| "MISSING".into()));
    }
    println!("aligner: {}", crate::align::shown());
    println!("romanization: {}", crate::romanize::dictionaries_status());
    let base = if paths().work.exists() { paths().work.clone() } else { paths().home.clone() };
    if let Some(free) = free_gib(&base) {
        println!("free disk space: {free:.1} GiB in {}", base.display());
    }
    let required: &[&str] = if need_ytdlp { &tools } else { &tools[1..] };
    let missing: Vec<&str> = required.iter().copied().filter(|tool| config::find_tool(tool).is_none()).collect();
    if !missing.is_empty() {
        return Err(format!("Required tools are missing: {}", missing.join(", ")));
    }
    if need_ytdlp {
        config::require_current_ytdlp()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_check_the_song_list() {
        let home = std::env::temp_dir().join(format!("polykara-manifest-{}", std::process::id()));
        std::fs::create_dir_all(home.join("work/raw")).unwrap();
        let header = REQUIRED_COLUMNS.join(",");
        let blank = ",".repeat(REQUIRED_COLUMNS.len() - 2);
        config::with_home(&home, || {
            std::fs::write(home.join("songs.csv"), format!("{header}\nexample,https://x{blank}\na,https://a{blank}\nb,https://b{blank}\n")).unwrap();
            assert_eq!(rows(&[]).unwrap().len(), 2);
            assert_eq!(rows(&["b".to_string()]).unwrap()[0]["id"], "b");
            assert!(rows(&["zzz".to_string()]).unwrap_err().contains("zzz"));
            std::fs::write(home.join("songs.csv"), format!("{header}\na,https://a{blank}\na,https://b{blank}\n")).unwrap();
            assert!(rows(&[]).unwrap_err().contains("duplicate"));
            for name in ["a.f137.mp4", "a.info.json", "a.webm", "ab.mp4"] {
                std::fs::write(home.join("work/raw").join(name), "").unwrap();
            }
            assert_eq!(find_source("a").unwrap().file_name().unwrap(), "a.webm");
        });
        let _ = std::fs::remove_dir_all(&home);
    }
}
