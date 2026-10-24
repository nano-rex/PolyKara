//! Lyrics from the web: LRCLIB, NetEase Cloud Music, lyrics.ovh and lyric pages.

use std::path::PathBuf;

use crate::config::{load_config, paths};
use crate::csv::Row;
use crate::json::Value;
use crate::manifest::get;
use crate::text::is_word_char;
use crate::tools::{fetch, quote, urlencode, HttpError};
use crate::verify::Kind;

const NETEASE_HEADERS: &[(&str, &str)] = &[("Referer", "https://music.163.com/")];
const VOID_TAGS: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];
const TITLE_WORDS: &[&str] = &["official", "video", "mv", "m/v", "lyric", "lyrics", "audio", "visualizer", "visualiser", "hd", "4k", "remaster", "remastered"];
const TITLE_CJK: &[&str] = &["官方", "完整版", "高清", "歌词", "歌詞", "字幕"];

pub fn lyric_sources(row: &Row) -> Vec<String> {
    let configured = get(row, "lyric_sources").trim();
    if !configured.is_empty() {
        return configured.split(',').map(|item| item.trim().to_lowercase()).filter(|item| !item.is_empty()).collect();
    }
    load_config().get("lyrics").get("sources").items().iter().map(|item| item.text().trim().to_lowercase()).filter(|item| !item.is_empty()).collect()
}

pub fn fetch_json(url: &str, headers: &[(&str, &str)]) -> Result<Value, HttpError> {
    let body = fetch(url, headers)?;
    Value::parse(&String::from_utf8_lossy(&body)).map_err(|problem| HttpError::Network(format!("not JSON ({problem})")))
}

/// Text with HTML character references (&amp; &#39; &#x4e2d;) replaced.
pub fn unescape_html(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest[1..].find(|c: char| c == ';' || c == '&' || c.is_whitespace()).map(|index| index + 1);
        let replaced = end.filter(|end| rest.as_bytes().get(*end) == Some(&b';')).and_then(|end| {
            let name = &rest[1..end];
            let value = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32),
                _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            value.map(|value| (value, end + 1))
        });
        match replaced {
            Some((value, length)) => {
                out.push(value);
                rest = &rest[length..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name) {
        let at = search + found;
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let after = lower[at + name.len()..].trim_start();
        if before_ok && after.starts_with('=') {
            let value_start = tag.len() - after.len() + 1;
            let value = tag[value_start..].trim_start();
            return Some(match value.chars().next() {
                Some(quote @ ('"' | '\'')) => value[1..].split(quote).next().unwrap_or("").to_string(),
                _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
            });
        }
        search = at + name.len();
    }
    None
}

/// The lyrics sections of a lyrics web page (Genius and similar), in page order.
pub fn extract_lyrics_page(html: &str) -> String {
    let mut depth = 0usize;
    let mut active: Option<usize> = None;
    let mut current = String::new();
    let mut candidates: Vec<String> = Vec::new();
    let finish = |current: &mut String, candidates: &mut Vec<String>| {
        let mut text = std::mem::take(current);
        while text.contains("\n\n\n") {
            text = text.replace("\n\n\n", "\n\n");
        }
        let text = text.trim().to_string();
        if !text.is_empty() {
            candidates.push(text);
        }
    };
    let mut rest = html;
    while !rest.is_empty() {
        let at = rest.find('<').unwrap_or(rest.len());
        if active.is_some() {
            current.push_str(&unescape_html(&rest[..at]));
        }
        rest = &rest[at..];
        if rest.is_empty() {
            break;
        }
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map(|end| &rest[end + 3..]).unwrap_or("");
            continue;
        }
        let close = match rest.find('>') {
            Some(close) => close,
            None => break,
        };
        let tag = &rest[1..close];
        rest = &rest[close + 1..];
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
        let self_closing = tag.trim_end().ends_with('/');
        if closing {
            if VOID_TAGS.contains(&name.as_str()) {
                continue;
            }
            if active == Some(depth) {
                finish(&mut current, &mut candidates);
                active = None;
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        if active.is_some() && ["br", "div", "p", "section"].contains(&name.as_str()) {
            current.push('\n');
        }
        // Void elements never get an end tag; counting them would unbalance the depth.
        if VOID_TAGS.contains(&name.as_str()) || self_closing {
            continue;
        }
        depth += 1;
        if ["script", "style"].contains(&name.as_str()) {
            let end_tag = format!("</{name}");
            let end = rest.to_lowercase().find(&end_tag).unwrap_or(rest.len());
            if active.is_some() {
                current.push_str(&rest[..end]);
            }
            rest = &rest[end..];
            continue;
        }
        let class = attribute(tag, "class").unwrap_or_default();
        let marker = attribute(tag, "data-lyrics-container").as_deref() == Some("true") || class.contains("Lyrics__Container") || class.contains("song_body-lyrics");
        if marker && active.is_none() {
            active = Some(depth);
            current.clear();
        }
    }
    if active.is_some() {
        finish(&mut current, &mut candidates);
    }
    candidates.join("\n")
}

/// Lyrics or a title reduced to words, for comparing names.
pub fn lyric_text(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while !rest.is_empty() {
        let skip = ['[', '<'].iter().find_map(|open| {
            let close = if *open == '[' { ']' } else { '>' };
            crate::subtitle::time_mark(rest, *open, close).map(|(_, length)| length)
        });
        if let Some(length) = skip {
            out.push(' ');
            rest = &rest[length..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        out.push(if is_word_char(c) || matches!(c, '\u{3000}'..='\u{9fff}' | '\u{ac00}'..='\u{d7af}') { c } else { ' ' });
        rest = &rest[c.len_utf8()..];
    }
    out.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn noisy(inside: &str) -> bool {
    let lower = inside.to_lowercase();
    if TITLE_CJK.iter().any(|word| lower.contains(word)) {
        return true;
    }
    let words: Vec<&str> = lower.split(|c: char| !(c.is_alphanumeric() || c == '/')).filter(|word| !word.is_empty()).collect();
    words.iter().any(|word| TITLE_WORDS.contains(word))
}

/// A video title without its decorations: (Official Video), [MV], 【官方】.
pub fn clean_title(title: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = title.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if matches!(chars[index], '(' | '[' | '【' | '（') {
            if let Some(length) = chars[index + 1..].iter().position(|c| matches!(c, ')' | ']' | '】' | '）')) {
                let inside: String = chars[index + 1..index + 1 + length].iter().collect();
                if !inside.contains(['(', '[', '【', '（']) && noisy(&inside) {
                    while out.ends_with(char::is_whitespace) {
                        out.pop();
                    }
                    index += length + 2;
                    continue;
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    let cleaned = out.trim().to_string();
    if cleaned.is_empty() { title.trim().to_string() } else { cleaned }
}

/// Length of the downloaded video in seconds, from yt-dlp's info JSON.
pub fn media_duration(row: &Row) -> Option<f64> {
    let text = std::fs::read_to_string(paths().raw.join(format!("{}.info.json", get(row, "id")))).ok()?;
    match Value::parse(&text).ok()?.get("duration") {
        Value::Number(number) => Some(*number),
        _ => None,
    }
}

fn artist_names(artist: &str) -> Vec<String> {
    let lower = artist.to_lowercase();
    let mut parts = vec![lower];
    for separator in [",", "&", "feat.", "feat", "ft.", "ft"] {
        parts = parts.iter().flat_map(|part| part.split(separator).map(str::to_string).collect::<Vec<_>>()).collect();
    }
    parts = parts.iter().flat_map(|part| part.split(" x ").map(str::to_string).collect::<Vec<_>>()).collect();
    parts.iter().map(|part| lyric_text(part)).collect()
}

/// Whether a lyrics record (trackName, artistName) is for this song.
pub fn metadata_matches(title: &str, artist: &str, candidate: &Value) -> bool {
    let found_title = lyric_text(&candidate.get("trackName").as_str().map(str::to_string).unwrap_or_else(|| text_of(candidate.get("trackName"))));
    let found_artist = lyric_text(&text_of(candidate.get("artistName")));
    let wanted = lyric_text(&clean_title(title));
    if wanted.is_empty() || found_title.is_empty() || !(found_title.contains(&wanted) || wanted.contains(&found_title)) {
        return false;
    }
    // Chart rows list several artists; one of them appearing on either side is enough.
    let names = artist_names(artist);
    if names.iter().all(String::is_empty) {
        // A song added from a file name without a singer: the title has to be enough.
        return true;
    }
    found_artist.is_empty() || names.iter().any(|name| !name.is_empty() && (found_artist.contains(name.as_str()) || name.contains(&found_artist)))
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.dump(),
    }
}

fn duration_of(value: &Value) -> Option<f64> {
    match value.get("duration") {
        Value::Number(number) => Some(*number),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

/// Records ordered by how close their length is to the video's.
fn closest(mut candidates: Vec<Value>, duration: Option<f64>) -> Vec<Value> {
    let distance = |item: &Value| match (duration, duration_of(item)) {
        (Some(duration), Some(length)) if duration != 0.0 => (length - duration).abs(),
        (Some(duration), None) if duration != 0.0 => 1e9,
        _ => 0.0,
    };
    candidates.sort_by(|a, b| distance(a).partial_cmp(&distance(b)).unwrap_or(std::cmp::Ordering::Equal));
    candidates
}

/// The LRCLIB record that best fits the song, preferring synced lyrics of the same length.
pub fn lrclib_lookup(title: &str, artist: &str, duration: Option<f64>) -> Result<Value, HttpError> {
    let mut candidates = Vec::new();
    let query = urlencode(&[("artist_name", artist.to_string()), ("track_name", clean_title(title))]);
    match fetch_json(&format!("https://lrclib.net/api/get?{query}"), &[]) {
        Ok(found) => candidates.push(found),
        Err(HttpError::Status(404)) => {}
        Err(error) => return Err(error),
    }
    let exact: Vec<&Value> = candidates.iter().filter(|item| !item.get("syncedLyrics").str().is_empty() && metadata_matches(title, artist, item)).collect();
    let far = match (exact.first(), duration) {
        (Some(first), Some(duration)) => (duration_of(first).unwrap_or(0.0) - duration).abs() > 2.0,
        _ => false,
    };
    if exact.is_empty() || far {
        // A different cut of the same song has shifted timestamps, so look for a closer one.
        let query = urlencode(&[("track_name", clean_title(title)), ("artist_name", artist.to_string())]);
        if let Value::Array(found) = fetch_json(&format!("https://lrclib.net/api/search?{query}"), &[])? {
            candidates.extend(found);
        }
    }
    let matching: Vec<Value> = candidates.into_iter().filter(|item| matches!(item, Value::Object(_)) && metadata_matches(title, artist, item)).collect();
    let synced = closest(matching.iter().filter(|item| !item.get("syncedLyrics").str().is_empty()).cloned().collect(), duration);
    Ok(synced.into_iter().next().or_else(|| matching.into_iter().next()).unwrap_or(Value::Object(Vec::new())))
}

/// NetEase credit lines such as "[00:00.000] 作词 : name" are not lyrics.
fn credit_line(line: &str) -> bool {
    let mut rest = line.trim_start();
    let mut marked = false;
    while rest.starts_with('[') {
        match rest.find(']') {
            Some(close) if rest[1..close].chars().all(|c| c.is_ascii_digit() || c == ':' || c == '.') => {
                rest = &rest[close + 1..];
                marked = true;
            }
            _ => break,
        }
    }
    if !marked {
        return false;
    }
    let rest = rest.trim_start();
    let colon = match rest.find([':', '：']) {
        Some(colon) => colon,
        None => return false,
    };
    let name = &rest[..colon];
    let after = rest[colon..].chars().nth(1);
    let count = name.chars().count();
    (2..=21).contains(&count) && name.ends_with(char::is_whitespace) && !name.contains(['[', ']', ':', '：']) && after.map_or(false, char::is_whitespace)
}

fn timed_line(line: &str) -> bool {
    let rest = line.trim_start();
    rest.starts_with('[') && rest[1..].split(':').next().map_or(false, |minutes| !minutes.is_empty() && minutes.chars().all(|c| c.is_ascii_digit())) && rest[1..].split(':').nth(1).map_or(false, |seconds| seconds.len() >= 2 && seconds[..2].chars().all(|c| c.is_ascii_digit()))
}

/// Synced lyrics from NetEase Cloud Music's public search: (lyrics, duration), or None.
pub fn netease_lookup(title: &str, artist: &str, duration: Option<f64>) -> Result<Option<(String, f64)>, HttpError> {
    let query = urlencode(&[("s", format!("{} {artist}", clean_title(title)).trim().to_string()), ("type", "1".into()), ("limit", "10".into()), ("offset", "0".into())]);
    let found = fetch_json(&format!("https://music.163.com/api/search/get?{query}"), NETEASE_HEADERS)?;
    let records: Vec<Value> = found
        .get("result")
        .get("songs")
        .items()
        .iter()
        .map(|song| {
            let artists = song.get("artists").items().iter().map(|item| text_of(item.get("name"))).collect::<Vec<_>>().join(", ");
            Value::object(vec![
                ("id", song.get("id").clone()),
                ("trackName", song.get("name").clone()),
                ("artistName", Value::from(artists)),
                ("duration", Value::Number(song.get("duration").as_i64().unwrap_or(0) as f64 / 1000.0)),
            ])
        })
        .filter(|record| !record.get("id").is_null() && metadata_matches(title, artist, record))
        .collect();
    for record in closest(records, duration).into_iter().take(3) {
        let id = record.get("id").as_i64().unwrap_or(0).to_string();
        let query = urlencode(&[("id", id), ("lv", "1".into()), ("kv", "1".into()), ("tv", "-1".into())]);
        let data = fetch_json(&format!("https://music.163.com/api/song/lyric?{query}"), NETEASE_HEADERS)?;
        let text = data.get("lrc").get("lyric").str().to_string();
        let lines: Vec<&str> = text.lines().filter(|line| !credit_line(line)).collect();
        if lines.iter().any(|line| timed_line(line)) {
            return Ok(Some((lines.join("\n"), duration_of(&record).unwrap_or(0.0))));
        }
    }
    Ok(None)
}

/// Lyric files already fetched for a song as (source, kind, path); kind is Lrc or Plain.
pub fn provider_files(row: &Row) -> Vec<(String, Kind, PathBuf)> {
    let sid = get(row, "id");
    let folder = paths().lyrics.clone();
    let mut found: Vec<(String, Kind, PathBuf)> = Vec::new();
    for name in ["lrclib", "netease"] {
        let path = folder.join(format!("{sid}.{name}.lrc"));
        // Earlier versions saved the LRCLIB result as <id>.lrc.
        let legacy = folder.join(format!("{sid}.lrc"));
        if path.exists() {
            found.push((name.into(), Kind::Lrc, path));
        } else if name == "lrclib" && legacy.exists() {
            found.push((name.into(), Kind::Lrc, legacy));
        }
    }
    for name in ["lrclib", "lyrics-ovh"] {
        let path = folder.join(format!("{sid}.{name}.txt"));
        if path.exists() && !found.iter().any(|item| item.0 == name) {
            found.push((name.into(), Kind::Plain, path));
        }
    }
    let mut pages: Vec<String> = std::fs::read_dir(&folder)
        .map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    pages.retain(|name| name.starts_with(&format!("{sid}.webpage")) && name.ends_with(".txt"));
    pages.sort();
    for name in pages {
        let source = name[sid.len() + 1..name.len() - 4].to_string();
        found.push((source, Kind::Plain, folder.join(name)));
    }
    found
}

fn webpage_path(sid: &str, index: usize) -> PathBuf {
    let folder = paths().lyrics.clone();
    if index == 0 {
        folder.join(format!("{sid}.webpage.txt"))
    } else {
        folder.join(format!("{sid}.webpage-{}.txt", index + 1))
    }
}

fn warn_duration(sid: &str, source: &str, found: Option<f64>, duration: Option<f64>) {
    if let (Some(found), Some(duration)) = (found, duration) {
        let difference = (found - duration).abs();
        if difference > 5.0 {
            println!("WARN {sid}: {source} lyrics are for a recording {difference:.0}s longer or shorter than this video; line timing may be offset");
        }
    }
}

/// Fetch lyrics from every configured source that has no file yet.
///
/// Each source is saved to its own file so the files can be compared with each
/// other. Returns false when a source could not be reached, so the lookup is
/// worth repeating.
pub fn fetch_external_lyrics(row: &Row) -> bool {
    let (sid, title, artist) = (get(row, "id"), get(row, "title").trim(), get(row, "artist").trim());
    let folder = paths().lyrics.clone();
    let _ = std::fs::create_dir_all(&folder);
    let have: Vec<String> = provider_files(row).into_iter().map(|item| item.0).collect();
    let duration = media_duration(row);
    let mut complete = true;
    let save = |name: &str, suffix: &str, text: &str| {
        let _ = std::fs::write(folder.join(format!("{sid}.{name}.{suffix}")), format!("{}\n", text.trim()));
        println!("{sid}: saved {} lyrics from {name}", if suffix == "lrc" { "synced" } else { "plain" });
    };
    for name in lyric_sources(row) {
        let result: Result<(), HttpError> = (|| {
            match name.as_str() {
                "lrclib" if !have.contains(&"lrclib".to_string()) => {
                    let record = lrclib_lookup(title, artist, duration)?;
                    if !record.get("syncedLyrics").str().trim().is_empty() {
                        warn_duration(sid, "LRCLIB", duration_of(&record), duration);
                        save("lrclib", "lrc", record.get("syncedLyrics").str());
                    } else if !record.get("plainLyrics").str().trim().is_empty() {
                        save("lrclib", "txt", record.get("plainLyrics").str());
                    } else {
                        println!("{sid}: lrclib has no lyrics for this song");
                    }
                }
                "netease" if !have.contains(&"netease".to_string()) => match netease_lookup(title, artist, duration)? {
                    Some((lyrics, found)) => {
                        warn_duration(sid, "NetEase", Some(found), duration);
                        save("netease", "lrc", &lyrics);
                    }
                    None => println!("{sid}: netease has no synced lyrics for this song"),
                },
                "lyrics.ovh" if !have.contains(&"lyrics-ovh".to_string()) => {
                    let result = fetch_json(&format!("https://api.lyrics.ovh/v1/{}/{}", quote(artist), quote(&clean_title(title))), &[])?;
                    if !result.get("lyrics").str().trim().is_empty() {
                        save("lyrics-ovh", "txt", result.get("lyrics").str());
                    } else {
                        println!("{sid}: lyrics.ovh has no lyrics for this song");
                    }
                }
                "webpage" => {
                    let pages: Vec<&str> = get(row, "lyric_pages").split('|').map(str::trim).filter(|page| !page.is_empty()).collect();
                    for (index, page) in pages.into_iter().enumerate() {
                        let target = webpage_path(sid, index);
                        if target.exists() {
                            continue;
                        }
                        let html = String::from_utf8_lossy(&fetch(page, &[])?).into_owned();
                        let extracted = extract_lyrics_page(&html);
                        if !extracted.is_empty() {
                            let _ = std::fs::write(&target, format!("{extracted}\n"));
                            println!("{sid}: extracted lyrics section from webpage");
                        }
                    }
                }
                "lrclib" | "netease" | "lyrics.ovh" => {}
                other => println!("{sid}: unknown lyric source {other:?}; expected lrclib, netease, lyrics.ovh, or webpage"),
            }
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(HttpError::Status(404)) => println!("{sid}: {name} has no lyrics for this song"),
            Err(error) => {
                complete = false;
                println!("{sid}: lyric source {name} unavailable ({error})");
            }
        }
    }
    complete
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyric_pages_are_extracted() {
        let html = r#"<html><body><div class="Header">Title</div><div data-lyrics-container="true">First line<br/>Second &amp; line<br><i>third</i></div><p>ads</p><div class="Lyrics__Container-x">Fourth</div></body></html>"#;
        assert_eq!(extract_lyrics_page(html), "First line\nSecond & line\nthird\nFourth");
    }

    #[test]
    fn titles_lose_their_decorations() {
        assert_eq!(clean_title("Hello (Official Video) [HD]"), "Hello");
        assert_eq!(clean_title("晴天【官方MV】"), "晴天");
        assert_eq!(clean_title("Song (feat. Someone)"), "Song (feat. Someone)");
    }

    #[test]
    fn records_match_the_song() {
        let record = Value::parse(r#"{"trackName": "Hello", "artistName": "Adele", "duration": 295}"#).unwrap();
        assert!(metadata_matches("Hello (Official Video)", "Adele", &record));
        assert!(metadata_matches("Hello", "Adele, Someone", &record));
        assert!(!metadata_matches("Goodbye", "Adele", &record));
        assert!(!metadata_matches("Hello", "Bob", &record));
        assert!(metadata_matches("Hello", "", &record));
        assert!(credit_line("[00:00.000] 作词 : 周杰伦"));
        assert!(!credit_line("[00:12.000]故事的小黄花"));
        assert_eq!(lyric_text("[00:01.00]Hello, World!"), "hello world");
        assert_eq!(unescape_html("a &lt;b&gt; &#20013;&#x6587; &unknown; &"), "a <b> 中文 &unknown; &");
    }
}
