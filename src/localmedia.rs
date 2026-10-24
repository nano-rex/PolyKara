//! Songs made from media files on this computer instead of a video URL.
//!
//! A local file is listed in songs.csv with its path in the url column. The
//! download step copies it into work/raw instead of running yt-dlp, and the
//! title and singer used for the lyrics search come from the file's own tags
//! or, when it has none, from its name ("Artist - Title.mp4", "01. Title.mp3", "歌手-歌名.mkv").

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::{expand_user, paths, MEDIA_SUFFIXES};
use crate::csv::Row;
use crate::json::Value;
use crate::manifest::{get, read_songs, write_songs, COLUMNS};
use crate::text::{is_han, is_hangul, is_kana};

const NOISE: &[&str] = &["official", "video", "audio", "lyric", "mv", "m/v", "karaoke", "ktv", "hd", "4k", "1080p", "720p", "live", "官方", "完整版", "高清", "伴奏", "歌词"];

/// True when the url column names a file rather than a web address.
pub fn is_local(url: &str) -> bool {
    let value = url.trim();
    !value.is_empty() && (value.starts_with("file://") || !value.contains("://"))
}

fn unquote(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() + 0 && index + 2 <= bytes.len() - 1 {
            if let Ok(value) = u8::from_str_radix(&text[index + 1..index + 3], 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn local_path(url: &str) -> PathBuf {
    let mut value = url.trim().to_string();
    if let Some(rest) = value.strip_prefix("file://") {
        let path = rest.find('/').map(|at| &rest[at..]).unwrap_or(rest);
        let mut decoded = unquote(path);
        // file:///C:/Music/song.mp3
        let bytes = decoded.as_bytes();
        if bytes.len() > 2 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
            decoded = decoded[1..].to_string();
        }
        value = decoded;
    }
    let path = expand_user(&value);
    if path.is_absolute() {
        path
    } else {
        paths().home.join(path)
    }
}

/// What ffprobe says about a media file.
#[derive(Clone, Debug, Default)]
pub struct Probe {
    pub duration: f64,
    pub tags: HashMap<String, String>,
    /// A real video track (not just cover art).
    pub video: bool,
    pub cover: bool,
    pub audio: bool,
}

pub fn probe(path: &Path) -> Probe {
    let path_text = path.to_string_lossy().into_owned();
    let output = crate::config::output("ffprobe", &["-v", "error", "-show_entries", "stream=codec_type:stream_disposition=attached_pic:format=duration:format_tags", "-of", "json", &path_text]);
    let data = output.ok().and_then(|output| Value::parse(&String::from_utf8_lossy(&output.stdout)).ok()).unwrap_or(Value::Null);
    let format = data.get("format");
    let tags = format.get("tags").pairs().iter().map(|(key, value)| (key.to_lowercase(), match value { Value::String(text) => text.clone(), other => other.dump() })).collect();
    let duration = match format.get("duration") {
        Value::String(text) => text.parse().unwrap_or(0.0),
        Value::Number(number) => *number,
        _ => 0.0,
    };
    let streams = data.get("streams").items();
    let picture = |stream: &Value| stream.get("disposition").get("attached_pic").as_i64().unwrap_or(0) != 0;
    Probe {
        duration,
        tags,
        video: streams.iter().any(|stream| stream.get("codec_type").str() == "video" && !picture(stream)),
        cover: streams.iter().any(|stream| stream.get("codec_type").str() == "video" && picture(stream)),
        audio: streams.iter().any(|stream| stream.get("codec_type").str() == "audio"),
    }
}

/// Remove "(Official Video)", "[KTV]", "【官方】" and the like from a file name.
fn without_noise(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    while index < chars.len() {
        if matches!(chars[index], '(' | '[' | '【' | '（') {
            if let Some(length) = chars[index + 1..].iter().position(|c| matches!(c, ')' | ']' | '】' | '）')) {
                let inside: String = chars[index + 1..index + 1 + length].iter().collect::<String>().to_lowercase();
                if NOISE.iter().any(|word| inside.contains(word)) {
                    index += length + 2;
                    continue;
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// "01. ", "02 - ", "07 " at the start; a longer number such as 1979 stays part of the title.
fn without_track_number(name: &str) -> &str {
    let trimmed = name.trim_start();
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return name;
    }
    let rest = &trimmed[digits..];
    let after_spaces = rest.trim_start();
    if let Some(mark) = after_spaces.chars().next() {
        if matches!(mark, '.' | '-' | '_' | ')' | ']') {
            return after_spaces[mark.len_utf8()..].trim_start();
        }
    }
    if digits <= 2 && rest.starts_with(char::is_whitespace) && after_spaces.chars().next().map_or(false, |c| !c.is_ascii_digit()) {
        return after_spaces;
    }
    name
}

fn split_separators(name: &str) -> Vec<String> {
    // " - ", " – ", " — ", " | " with spaces around, or "－" / "—" anywhere.
    let mut parts = vec![String::new()];
    let chars: Vec<char> = name.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if matches!(c, '－' | '—') {
            let last = parts.last_mut().unwrap();
            *last = last.trim_end().to_string();
            parts.push(String::new());
            index += 1;
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            continue;
        }
        if c.is_whitespace() {
            let mut next = index;
            while next < chars.len() && chars[next].is_whitespace() {
                next += 1;
            }
            if next < chars.len() && matches!(chars[next], '-' | '–' | '—' | '|') && next + 1 < chars.len() && chars[next + 1].is_whitespace() {
                parts.push(String::new());
                index = next + 1;
                while index < chars.len() && chars[index].is_whitespace() {
                    index += 1;
                }
                continue;
            }
        }
        parts.last_mut().unwrap().push(c);
        index += 1;
    }
    parts.into_iter().map(|part| part.trim().to_string()).filter(|part| !part.is_empty()).collect()
}

/// (artist, title) read from a file name; artist is empty when the name does not say.
pub fn guess_from_name(stem: &str) -> (String, String) {
    let name = stem.replace('_', " ");
    let name = without_track_number(&name).to_string();
    let name = without_noise(&name).split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim_matches(|c| c == ' ' || c == '-').to_string();
    let mut parts = split_separators(&name);
    if parts.len() < 2 && name.chars().any(is_han) && name.matches('-').count() == 1 {
        // Chinese file names often use a bare hyphen: 歌手-歌名.
        parts = name.split('-').map(|part| part.trim().to_string()).filter(|part| !part.is_empty()).collect();
    }
    if parts.len() >= 2 {
        return (parts[0].clone(), parts[1..].join(" - "));
    }
    (String::new(), if name.is_empty() { stem.to_string() } else { name })
}

pub fn guess_language(text: &str) -> String {
    if text.chars().any(is_kana) {
        "ja".into()
    } else if text.chars().any(is_hangul) {
        "ko".into()
    } else if text.chars().any(is_han) {
        "zh".into()
    } else {
        "en".into()
    }
}

/// The text in Latin letters, romanized when it is written in another script.
pub fn ascii_name(text: &str) -> String {
    let other_script = text.chars().any(|c| is_han(c) || is_kana(c) || is_hangul(c));
    if text.chars().any(|c| c.is_ascii_alphanumeric()) && !other_script {
        return text.to_string();
    }
    let settings = crate::toml::parse("enabled = true\nlanguages = \"auto\"\ntone = \"none\"\n").expect("valid settings");
    crate::romanize::romanize(text, &guess_language(text), &settings).unwrap_or_else(|| text.to_string())
}

pub fn new_id(title: &str, taken: &HashSet<String>) -> String {
    let lower = ascii_name(title).to_lowercase();
    let mut base = String::new();
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
        } else if !base.ends_with('-') {
            base.push('-');
        }
    }
    let base: String = base.trim_matches('-').chars().take(40).collect();
    let base = if base.is_empty() { "song".to_string() } else { base };
    let (mut candidate, mut number) = (base.clone(), 2);
    while taken.contains(&candidate) {
        candidate = format!("{base}-{number}");
        number += 1;
    }
    candidate
}

/// A path relative to the PolyKara folder when it is inside it, otherwise absolute.
pub fn relative_to_home(path: &Path) -> String {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let home = std::fs::canonicalize(&paths().home).unwrap_or_else(|_| paths().home.clone());
    let shown = match path.strip_prefix(&home) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => path,
    };
    let text = shown.to_string_lossy().into_owned();
    // canonicalize on Windows gives \\?\C:\...; songs.csv keeps the ordinary form.
    text.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(text)
}

/// A songs.csv row for a media file.
pub fn describe(path: &Path, taken: &HashSet<String>) -> Row {
    let info = probe(path);
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let (artist, title) = guess_from_name(&stem);
    let tag = |name: &str| info.tags.get(name).map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let title = tag("title").unwrap_or(title);
    let artist = tag("artist").or_else(|| tag("album_artist")).unwrap_or(artist);
    let mut row: Row = COLUMNS.iter().map(|name| (name.to_string(), String::new())).collect();
    let id = new_id(&if artist.is_empty() { title.clone() } else { format!("{artist} {title}") }, taken);
    for (key, value) in [("id", id), ("url", relative_to_home(path)), ("language", guess_language(&format!("{title}{artist}"))), ("title", title), ("artist", artist), ("template", "solo".into()), ("prompt_ms", "1000".into()), ("align_model", "auto".into())] {
        row.insert(key.to_string(), value);
    }
    row
}

/// Add media files to songs.csv; returns the rows that were added (files already listed are skipped).
pub fn add_files(files: &[PathBuf]) -> Result<Vec<Row>, String> {
    let (fields, mut songs) = read_songs();
    let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut listed: HashSet<PathBuf> = songs.iter().filter(|song| is_local(get(song, "url"))).map(|song| canonical(&local_path(get(song, "url")))).collect();
    let mut taken: HashSet<String> = songs.iter().map(|song| get(song, "id").to_string()).collect();
    let mut added = Vec::new();
    for path in files {
        let path = canonical(&expand_user(&path.to_string_lossy()));
        if !path.is_file() || listed.contains(&path) {
            continue;
        }
        let row = describe(&path, &taken);
        taken.insert(get(&row, "id").to_string());
        listed.insert(path);
        songs.push(row.clone());
        added.push(row);
    }
    if !added.is_empty() {
        write_songs(&fields, &songs)?;
    }
    Ok(added)
}

/// Place the song's local file in work/raw as <id><suffix>, linked when possible, copied otherwise.
pub fn bring_in(row: &Row, raw: &Path) -> Result<PathBuf, String> {
    let source = local_path(get(row, "url"));
    if !source.is_file() {
        return Err(format!("media file not found: {}", source.display()));
    }
    let suffix = source.extension().map(|extension| format!(".{}", extension.to_string_lossy().to_lowercase())).unwrap_or_default();
    if !MEDIA_SUFFIXES.contains(&suffix.as_str()) {
        return Err(format!("{} is not a supported media file ({})", source.display(), MEDIA_SUFFIXES.join(", ")));
    }
    std::fs::create_dir_all(raw).map_err(|error| error.to_string())?;
    let sid = get(row, "id");
    let target = raw.join(format!("{sid}{suffix}"));
    if target.exists() {
        std::fs::remove_file(&target).map_err(|error| error.to_string())?;
    }
    if std::fs::hard_link(&source, &target).is_err() {
        std::fs::copy(&source, &target).map_err(|error| format!("{} could not be copied ({error})", source.display()))?;
    }
    let info = probe(&target);
    // The lyrics search compares record lengths with this, as it does with yt-dlp's info JSON.
    let note = Value::object(vec![("duration", Value::Number(info.duration)), ("title", Value::from(get(row, "title"))), ("local_file", Value::from(source.to_string_lossy().into_owned()))]);
    std::fs::write(raw.join(format!("{sid}.info.json")), note.dump() + "\n").map_err(|error| error.to_string())?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_tell_the_singer_and_title() {
        assert_eq!(guess_from_name("Adele - Hello (Official Video)"), ("Adele".into(), "Hello".into()));
        assert_eq!(guess_from_name("01. Yesterday"), ("".into(), "Yesterday".into()));
        assert_eq!(guess_from_name("1979"), ("".into(), "1979".into()));
        assert_eq!(guess_from_name("周杰伦-晴天【官方MV】"), ("周杰伦".into(), "晴天".into()));
        assert_eq!(guess_from_name("07 My_Song"), ("".into(), "My Song".into()));
        assert_eq!(guess_language("晴天"), "zh");
        assert_eq!(guess_language("さくら"), "ja");
    }

    #[test]
    fn ids_are_unique_and_plain() {
        let taken: HashSet<String> = ["wanna".to_string()].into_iter().collect();
        assert_eq!(new_id("I Know You're Hurting", &HashSet::new()), "i-know-you-re-hurting");
        assert_eq!(new_id("Wanna", &taken), "wanna-2");
        assert_eq!(new_id("!!!", &HashSet::new()), "song");
        assert!(is_local("C:/Music/a.mp3"));
        assert!(is_local("file:///C:/Music/a.mp3"));
        assert!(!is_local("https://youtu.be/x"));
        // A drive letter is only absolute on Windows; elsewhere a file URL is a plain path.
        #[cfg(windows)]
        assert_eq!(local_path("file:///C:/Music/a%20b.mp3"), PathBuf::from("C:/Music/a b.mp3"));
        #[cfg(not(windows))]
        assert_eq!(local_path("file:///tmp/a%20b.mp3"), PathBuf::from("/tmp/a b.mp3"));
    }
}
