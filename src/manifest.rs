use crate::model::Song;
use anyhow::Result;
use csv::{ReaderBuilder, WriterBuilder};
use std::path::Path;

pub const FIELDS: &[&str] = &[
    "id",
    "url",
    "title",
    "artist",
    "lyricist",
    "composer",
    "producer",
    "language",
    "voice",
    "lyrics_file",
    "word_timing_file",
    "subtitle_langs",
    "logo_file",
];
pub fn read(path: &Path) -> Result<Vec<Song>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut reader = ReaderBuilder::new().flexible(true).from_path(path)?;
    let headers = reader.headers()?.clone();
    let mut out = Vec::new();
    for row in reader.records() {
        let row = row?;
        let get = |key: &str| {
            headers
                .iter()
                .position(|x| x == key)
                .and_then(|i| row.get(i))
                .unwrap_or("")
                .to_string()
        };
        out.push(Song {
            id: get("id"),
            url: get("url"),
            title: get("title"),
            artist: get("artist"),
            lyricist: get("lyricist"),
            composer: get("composer"),
            producer: get("producer"),
            language: get("language"),
            voice: get("voice"),
            lyrics_file: get("lyrics_file"),
            word_timing_file: get("word_timing_file"),
            subtitle_langs: get("subtitle_langs"),
            logo_file: get("logo_file"),
        });
    }
    Ok(out)
}
pub fn write(path: &Path, songs: &[Song]) -> Result<()> {
    let mut w = WriterBuilder::new().from_path(path)?;
    w.write_record(FIELDS)?;
    for s in songs {
        w.write_record([
            &s.id,
            &s.url,
            &s.title,
            &s.artist,
            &s.lyricist,
            &s.composer,
            &s.producer,
            &s.language,
            &s.voice,
            &s.lyrics_file,
            &s.word_timing_file,
            &s.subtitle_langs,
            &s.logo_file,
        ])?;
    }
    w.flush()?;
    Ok(())
}
pub fn add_file(path: &Path, songs: &mut Vec<Song>) -> Result<Song> {
    let name = path.file_stem().and_then(|x| x.to_str()).unwrap_or("song");
    let (artist, title) = name
        .split_once(" - ")
        .map(|(a, t)| (a.to_string(), t.to_string()))
        .unwrap_or((String::new(), name.to_string()));
    let mut id = slug(&title);
    let base = id.clone();
    let mut n = 2;
    while songs.iter().any(|s| s.id == id) {
        id = format!("{}-{}", base, n);
        n += 1;
    }
    let song = Song {
        id,
        url: path.to_string_lossy().to_string(),
        title,
        artist,
        language: guess_language(name),
        ..Song::default()
    };
    songs.push(song.clone());
    Ok(song)
}
pub fn slug(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else if c.is_whitespace() || c == '-' {
                '-'
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
pub fn guess_language(s: &str) -> String {
    for c in s.chars() {
        let n = c as u32;
        if (0x4e00..=0x9fff).contains(&n) {
            return "zh".into();
        }
        if (0x3040..=0x30ff).contains(&n) {
            return "ja".into();
        }
        if (0xac00..=0xd7af).contains(&n) {
            return "ko".into();
        }
    }
    "en".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_slugged_and_scripts_are_detected() {
        assert_eq!(slug("I Know You're Hurting"), "i-know-you-re-hurting");
        assert_eq!(guess_language("こんにちは"), "ja");
        assert_eq!(guess_language("안녕"), "ko");
        assert_eq!(guess_language("你好"), "zh");
    }
}
