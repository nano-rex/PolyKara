//! Local music library for the player: scanning, tags and lyric lookup
//! (polykara/library.py ported). The studio server serves this over HTTP.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::{self, paths};
use crate::json::Value;
use crate::project;
use crate::providers;
use crate::subtitle::Entry;
use crate::tools;

pub const AUDIO_SUFFIXES: &[&str] = &[".mp3", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".wav", ".wma", ".ape", ".mp4", ".mkv", ".webm", ".mov"];
pub const LYRIC_SUFFIXES: &[&str] = &[".krc", ".qrc", ".ksc", ".lrc", ".ass", ".srt", ".vtt", ".txt"];

fn cache_file() -> PathBuf {
    paths().work.join("library.json")
}

fn fetched_folder() -> PathBuf {
    paths().work.join("player-lyrics")
}

/// The folders the player scans: [player] library in polykara.toml.
pub fn library_roots(root: &Path) -> Vec<PathBuf> {
    let config = config::load_config();
    let configured = config.get("player").get("library");
    let mut result = Vec::new();
    for item in configured.items() {
        let path = config::expand_user(item.text().trim());
        let path = if path.is_absolute() { path } else { root.join(path) };
        if path.is_dir() {
            result.push(std::fs::canonicalize(&path).unwrap_or(path));
        }
    }
    result
}

/// Tags and length of one file, as the player shows them.
pub fn probe(path: &Path) -> Value {
    let info = crate::localmedia::probe(path);
    let tags = &info.tags;
    let tag = |name: &str| tags.get(name).map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let title = tag("title").or_else(|| tag("TITLE")).unwrap_or_else(|| path.file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
    let artist = tag("artist").or_else(|| tag("ARTIST")).or_else(|| tag("album_artist")).or_else(|| tag("ALBUMARTIST")).unwrap_or_default();
    let album = tag("album").or_else(|| tag("ALBUM")).unwrap_or_default();
    Value::object(vec![
        ("title", Value::from(title)),
        ("artist", Value::from(artist)),
        ("album", Value::from(album)),
        ("duration", Value::Number(info.duration)),
        ("video", Value::Bool(info.video)),
    ])
}

/// A stable id for a track path (the first 16 hex digits of its SHA-1).
pub fn track_id(path: &Path) -> String {
    sha1_hex(path.to_string_lossy().as_bytes())[..16].to_string()
}

fn audio(path: &Path) -> bool {
    path.extension().map(|suffix| {
        let suffix = format!(".{}", suffix.to_string_lossy().to_lowercase());
        AUDIO_SUFFIXES.contains(&suffix.as_str())
    }).unwrap_or(false)
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|age| age.as_secs() as i64).unwrap_or(0)
}

fn size(path: &Path) -> i64 {
    std::fs::metadata(path).map(|meta| meta.len() as i64).unwrap_or(0)
}

/// Tracks under the configured folders; ffprobe tags are cached by path, size and mtime.
pub fn scan(root: &Path, refresh: bool) -> Vec<Value> {
    let mut cache: HashMap<String, Value> = HashMap::new();
    if !refresh {
        if let Ok(text) = std::fs::read_to_string(cache_file()) {
            if let Ok(Value::Array(items)) = Value::parse(&text) {
                for item in items {
                    cache.insert(item.get("path").str().to_string(), item);
                }
            }
        }
    }
    let mut tracks = Vec::new();
    for folder in library_roots(root) {
        let mut found = Vec::new();
        walk(&folder, &mut found);
        found.sort();
        for path in found {
            if !audio(&path) || !path.is_file() {
                continue;
            }
            let key = path.to_string_lossy().into_owned();
            let (now_size, now_mtime) = (size(&path), mtime(&path));
            let item = cache.get(&key).filter(|item| item.get("size").as_i64() == Some(now_size) && item.get("mtime").as_i64() == Some(now_mtime)).cloned();
            let mut item = match item {
                Some(item) => item,
                None => {
                    let mut pairs: Vec<(String, Value)> = vec![("path".into(), Value::from(&key)), ("size".into(), Value::from(now_size)), ("mtime".into(), Value::from(now_mtime))];
                    if let Value::Object(more) = probe(&path) {
                        pairs.extend(more);
                    }
                    Value::Object(pairs)
                }
            };
            if let Value::Object(pairs) = &mut item {
                let set = |pairs: &mut Vec<(String, Value)>, name: &str, value: Value| match pairs.iter_mut().find(|(key, _)| key == name) {
                    Some(pair) => pair.1 = value,
                    None => pairs.push((name.to_string(), value)),
                };
                set(pairs, "id", Value::from(track_id(&path)));
                set(pairs, "lyrics", Value::Bool(sidecar(&path).is_some()));
                let output = std::fs::canonicalize(paths().output.clone()).unwrap_or_else(|_| paths().output.clone());
                set(pairs, "burned", Value::Bool(path.ancestors().any(|parent| parent == output.as_path())));
            }
            tracks.push(item);
        }
    }
    let _ = std::fs::create_dir_all(paths().work.clone());
    let _ = std::fs::write(cache_file(), Value::Array(tracks.clone()).dump() + "\n");
    tracks
}

fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else {
                found.push(path);
            }
        }
    }
}

/// The lyric file next to a track, PolyKara's own ASS, or lyrics kept in work/.
pub fn sidecar(path: &Path) -> Option<PathBuf> {
    for suffix in LYRIC_SUFFIXES {
        for candidate in [path.with_extension(suffix.trim_start_matches('.')), path.with_extension(suffix.trim_start_matches('.').to_uppercase())] {
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    let stem = path.file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    for name in [format!("{stem}.edited.ass"), format!("{stem}.auto.ass")] {
        let candidate = paths().ass.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    stored_lyrics(&track_id(path))
}

pub fn stored_lyrics(tid: &str) -> Option<PathBuf> {
    LYRIC_SUFFIXES.iter().map(|suffix| fetched_folder().join(format!("{tid}{suffix}"))).find(|candidate| candidate.exists())
}

/// Keep user-supplied or fetched lyrics in work/, never next to the user's music files.
pub fn store_lyrics(tid: &str, suffix: &str, data: &[u8]) -> Result<PathBuf, String> {
    let suffix = suffix.to_lowercase();
    let suffix = if LYRIC_SUFFIXES.contains(&suffix.as_str()) { suffix } else { ".lrc".into() };
    let folder = fetched_folder();
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    if let Ok(entries) = std::fs::read_dir(&folder) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("{tid}.")) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let output = folder.join(format!("{tid}{suffix}"));
    std::fs::write(&output, data).map_err(|error| error.to_string())?;
    Ok(output)
}

/// Synced lyrics from LRCLIB for a track, kept in work/.
pub fn fetch_lrclib(track: &Value) -> Option<PathBuf> {
    let mut pairs = vec![("artist_name", track.get("artist").str().to_string()), ("track_name", track.get("title").str().to_string())];
    if let Some(duration) = track.get("duration").as_i64().filter(|value| *value > 0) {
        pairs.push(("duration", duration.to_string()));
    }
    let url = format!("https://lrclib.net/api/get?{}", tools::urlencode(&pairs));
    let data = match providers::fetch_json(&url, &[]) {
        Ok(data) => data,
        Err(problem) => {
            println!("player: LRCLIB lookup failed for {} ({problem})", track.get("title").str());
            return None;
        }
    };
    let synced = data.get("syncedLyrics").str().trim();
    if synced.is_empty() {
        return None;
    }
    store_lyrics(track.get("id").str(), ".lrc", format!("{synced}\n").as_bytes()).ok()
}

pub fn guess_language(text: &str) -> String {
    if text.chars().any(|character| ('\u{3040}'..='\u{30ff}').contains(&character)) {
        return "ja".into();
    }
    if text.chars().any(|character| ('\u{ac00}'..='\u{d7af}').contains(&character)) {
        return "ko".into();
    }
    if text.chars().any(|character| ('\u{4e00}'..='\u{9fff}').contains(&character)) {
        return "zh".into();
    }
    String::new()
}

/// A plain lyric line without the karaoke word codes, such as `<00:12.34>`.
fn plain_text(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '<' {
            let mut end = index + 1;
            while end < chars.len() && (chars[end].is_ascii_digit() || matches!(chars[end], ':' | '.' | '-')) {
                end += 1;
            }
            if end < chars.len() && chars[end] == '>' && chars[index + 1..end].iter().any(char::is_ascii_digit) {
                index = end + 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out.trim().to_string()
}

/// Two LRC lines with the same timestamp are an original plus its translation.
pub fn merge_translations(entries: &[Entry]) -> Vec<Value> {
    let mut lines: Vec<(i64, i64, String, Vec<(i64, i64, String)>, String)> = Vec::new();
    let mut merged: Vec<usize> = Vec::new();
    for entry in entries {
        let text = plain_text(&entry.text);
        if let Some(last) = lines.last() {
            if last.0 == entry.start && entry.words.is_empty() {
                let index = lines.len() - 1;
                lines[index].4 = text;
                merged.push(index);
                continue;
            }
        }
        lines.push((entry.start, entry.end, text, entry.words.clone(), String::new()));
    }
    for index in merged {
        let following = lines.get(index + 1).map(|line| line.0).unwrap_or(lines[index].0 + 4000);
        lines[index].1 = lines[index].1.max(following);
    }
    lines
        .into_iter()
        .map(|(start, end, text, words, translation)| {
            let words: Vec<Value> = words.iter().map(|(start, end, text)| Value::Array(vec![Value::from(*start), Value::from(*end), Value::from(text.as_str())])).collect();
            Value::object(vec![
                ("start", Value::from(start)),
                ("end", Value::from(end)),
                ("text", Value::from(text)),
                ("words", Value::Array(words)),
                ("translation", Value::from(translation)),
            ])
        })
        .collect()
}

/// A track's lyrics: the sidecar file, else LRCLIB, with readings for the player.
pub fn track_lyrics(track: &Value, online: bool) -> Value {
    let path = PathBuf::from(track.get("path").str());
    let mut source = sidecar(&path);
    let mut origin = "sidecar";
    if source.is_none() && online {
        source = fetch_lrclib(track);
        origin = "lrclib";
    }
    let source = match source {
        Some(source) => source,
        None => return Value::object(vec![("source", Value::from("none")), ("lines", Value::Array(Vec::new()))]),
    };
    let entries = match project::lyric_source_entries(&source) {
        Ok(entries) => entries,
        Err(problem) => return Value::object(vec![("source", Value::from("error")), ("error", Value::from(problem)), ("lines", Value::Array(Vec::new()))]),
    };
    let mut lines = merge_translations(&entries);
    let sample: String = lines.iter().take(20).map(|line| line.get("text").str()).collect();
    let language = guess_language(&sample);
    if !language.is_empty() {
        let config = config::load_config();
        let mut settings = config.get("romanization").clone();
        if let crate::toml::Item::Table(pairs) = &mut settings {
            match pairs.iter_mut().find(|(key, _)| key == "enabled") {
                Some(pair) => pair.1 = crate::toml::Item::Bool(true),
                None => pairs.push(("enabled".into(), crate::toml::Item::Bool(true))),
            }
        }
        for line in &mut lines {
            let text = line.get("text").str().trim_start_matches(|character: char| character == '[' || character == ']' || character.is_ascii_digit() || character == ':' || character == '.' || character == ' ').to_string();
            if let Value::Object(pairs) = line {
                let roman = crate::romanize::romanize(&text, &language, &settings).unwrap_or_default();
                pairs.push(("roman".into(), Value::from(roman)));
            }
        }
    }
    Value::object(vec![
        ("source", Value::from(origin)),
        ("file", Value::from(source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())),
        ("language", Value::from(language)),
        ("lines", Value::Array(lines)),
    ])
}

// ------------------------------------------------------------------- SHA-1

/// A small SHA-1 for track ids, so ids stay the same as the Python player's.
fn sha1_hex(data: &[u8]) -> String {
    let mut state: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut words = [0u32; 80];
        for index in 0..16 {
            words[index] = u32::from_be_bytes([block[index * 4], block[index * 4 + 1], block[index * 4 + 2], block[index * 4 + 3]]);
        }
        for index in 16..80 {
            words[index] = (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (state[0], state[1], state[2], state[3], state[4]);
        for (index, word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
    state.iter().map(|word| format!("{word:08x}")).collect::<Vec<_>>().concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_sha1_prefixes() {
        assert_eq!(&sha1_hex(b"abc")[..16], "a9993e364706816a");
        assert_eq!(&sha1_hex(b"")[..16], "da39a3ee5e6b4b0d");
    }

    #[test]
    fn languages_are_guessed_from_scripts() {
        assert_eq!(guess_language("こんにちは"), "ja");
        assert_eq!(guess_language("안녕하세요"), "ko");
        assert_eq!(guess_language("晴天"), "zh");
        assert_eq!(guess_language("hello"), "");
    }

    #[test]
    fn word_codes_are_removed() {
        assert_eq!(plain_text("<00:12.34>晴天"), "晴天");
        assert_eq!(plain_text("a < b"), "a < b");
    }

    #[test]
    fn translations_merge_into_the_line_above() {
        let entries = vec![
            Entry::new(1000, 2000, "Hello", Vec::new()),
            Entry::new(1000, 2000, "你好", Vec::new()),
            Entry::new(3000, 4000, "World", Vec::new()),
        ];
        let lines = merge_translations(&entries);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].get("translation").str(), "你好");
        assert!(lines[0].get("end").as_i64().unwrap() >= 3000);
    }
}
