//! Editable karaoke projects for the studio: JSON storage, ASS read-back, and lyric exporters.

use std::path::{Path, PathBuf};

use crate::ass::ass;
use crate::config::{paths, resolve};
use crate::csv::Row;
use crate::json::Value;
use crate::lyricformats::join_words;
use crate::manifest::{get, write_atomic};
use crate::subtitle::{lrc, timed_subtitle, Entry, Word};
use crate::text::is_cjk_wide;

pub fn project_path(sid: &str) -> PathBuf {
    paths().work.join("project").join(format!("{sid}.json"))
}

fn ass_time(value: &str) -> Option<i64> {
    let parts: Vec<&str> = value.trim().split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let (whole, fraction) = parts[2].split_once('.').unwrap_or((parts[2], ""));
    let fraction: i64 = format!("{fraction}00")[..2].parse().ok()?;
    Some(((parts[0].parse::<i64>().ok()? * 60 + parts[1].parse::<i64>().ok()?) * 60 + whole.parse::<i64>().ok()?) * 1000 + fraction * 10)
}

fn unescape(value: &str) -> String {
    value.replace("\\N", " ").replace("\\n", " ").replace("\\h", " ").replace("\\{", "{").replace("\\}", "}")
}

/// The last \k, \kf or \ko duration (centiseconds) in a block of override tags.
fn karaoke_duration(tags: &str) -> Option<i64> {
    let lower = tags.to_lowercase();
    let mut found = None;
    let mut search = 0;
    while let Some(at) = lower[search..].find("\\k") {
        let start = search + at + 2;
        let rest = &lower[start..];
        let rest = rest.strip_prefix('f').or_else(|| rest.strip_prefix('o')).unwrap_or(rest);
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            found = digits.parse().ok();
        }
        search = start;
    }
    found
}

/// Read the Lyric events of a PolyKara (or Aegisub \k) ASS file back into entries.
pub fn parse_ass(text: &str) -> Vec<Entry> {
    let padding = text
        .lines()
        .find_map(|line| line.strip_prefix("; PolyKaraIntroPaddingMs:"))
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let mut result = Vec::new();
    for line in text.lines() {
        let rest = match line.strip_prefix("Dialogue:") {
            Some(rest) => rest,
            None => continue,
        };
        let fields: Vec<&str> = rest.splitn(10, ',').collect();
        if fields.len() < 10 || !matches!(fields[3].trim().to_lowercase().as_str(), "lyric" | "default") {
            continue;
        }
        let (shown, end) = match (ass_time(fields[1]), ass_time(fields[2])) {
            (Some(shown), Some(end)) => (shown - padding, end - padding),
            _ => continue,
        };
        let body = fields[9];
        let mut words: Vec<Word> = Vec::new();
        let mut cursor = shown;
        let mut pending: Option<Word> = None;
        let body = if body.starts_with('{') { body.to_string() } else { format!("{{}}{body}") };
        let mut rest = body.as_str();
        while let Some(open) = rest.find('{') {
            let close = match rest[open..].find('}') {
                Some(close) => open + close,
                None => break,
            };
            let tags = &rest[open + 1..close];
            let after = &rest[close + 1..];
            let chunk_end = after.find('{').unwrap_or(after.len());
            let chunk = &after[..chunk_end];
            match karaoke_duration(tags) {
                Some(duration) => {
                    if let Some(word) = pending.take() {
                        words.push(word);
                    }
                    let duration = duration * 10;
                    if !chunk.trim().is_empty() {
                        pending = Some((cursor, cursor + duration, unescape(chunk).trim().to_string()));
                    }
                    cursor += duration;
                }
                None => {
                    if let Some(word) = pending.as_mut() {
                        if !chunk.trim().is_empty() {
                            word.2.push_str(unescape(chunk).trim());
                        }
                    }
                }
            }
            rest = &after[chunk_end..];
        }
        if let Some(word) = pending {
            words.push(word);
        }
        let mut plain = String::new();
        let mut depth = 0;
        for c in fields[9].chars() {
            match c {
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                c if depth == 0 => plain.push(c),
                _ => {}
            }
        }
        let lyric = if words.is_empty() { unescape(&plain).trim().to_string() } else { join_words(&words.iter().map(|word| word.2.clone()).collect::<Vec<_>>()) };
        if !lyric.is_empty() {
            let start = words.first().map(|word| word.0).unwrap_or(shown);
            result.push(Entry::new(start, end, lyric, words));
        }
    }
    result
}

pub fn to_json(entries: &[Entry]) -> Value {
    let lines = entries
        .iter()
        .map(|entry| {
            let words = entry.words.iter().map(|(left, right, text)| Value::Array(vec![Value::from(*left), Value::from(*right), Value::from(text)])).collect();
            Value::object(vec![("start", Value::from(entry.start)), ("end", Value::from(entry.end)), ("text", Value::from(&entry.text)), ("words", Value::Array(words))])
        })
        .collect();
    Value::object(vec![("version", Value::from(1i64)), ("lines", Value::Array(lines))])
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => Some(*number as i64),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

pub fn from_json(data: &Value) -> Result<Vec<Entry>, String> {
    let mut result = Vec::new();
    for line in data.get("lines").items() {
        let mut words: Vec<Word> = Vec::new();
        for word in line.get("words").items() {
            let parts = word.items();
            if parts.len() != 3 {
                return Err("a word must be [start, end, text]".into());
            }
            let text = match &parts[2] {
                Value::String(text) => text.clone(),
                other => other.dump(),
            };
            if text.trim().is_empty() {
                continue;
            }
            words.push((integer(&parts[0]).ok_or("bad word start")?, integer(&parts[1]).ok_or("bad word end")?, text));
        }
        words.sort();
        let start = integer(line.get("start")).or_else(|| words.first().map(|word| word.0)).unwrap_or(0);
        let end = integer(line.get("end")).or_else(|| words.last().map(|word| word.1)).unwrap_or(start + 2000).max(start + 10);
        let given = line.get("text").str().trim().to_string();
        let text = if given.is_empty() { join_words(&words.iter().map(|word| word.2.clone()).collect::<Vec<_>>()) } else { given };
        if !text.is_empty() {
            result.push(Entry::new(start, end, text, words));
        }
    }
    result.sort_by_key(|entry| entry.start);
    Ok(result)
}

/// The entries of any lyric file the studio or player can read.
pub fn lyric_source_entries(path: &Path) -> Result<Vec<Entry>, String> {
    let suffix = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
    match suffix.as_str() {
        "lrc" => lrc(path),
        "srt" | "vtt" => timed_subtitle(path),
        "ass" => crate::subtitle::read_text(path).map(|text| parse_ass(&text)),
        _ => match crate::lyricformats::read(path) {
            Some(result) => result,
            None => {
                let text = crate::lyricformats::read_text(path)?;
                Ok(text.lines().map(str::trim).filter(|line| !line.is_empty()).enumerate().map(|(index, line)| Entry::new(index as i64 * 4000, index as i64 * 4000 + 3500, line, Vec::new())).collect())
            }
        },
    }
}

/// Best available timing for a song: saved project, edited ASS, automatic ASS, then lyrics_file.
pub fn load_entries(row: &Row) -> Result<(Vec<Entry>, String), String> {
    let sid = get(row, "id");
    let project = project_path(sid);
    if project.exists() {
        let text = std::fs::read_to_string(&project).map_err(|error| error.to_string())?;
        return Ok((from_json(&Value::parse(&text)?)?, "project".into()));
    }
    for (name, label) in [(format!("{sid}.edited.ass"), "edited ASS"), (format!("{sid}.auto.ass"), "automatic ASS")] {
        let path = paths().ass.join(name);
        if path.exists() {
            return Ok((parse_ass(&crate::subtitle::read_text(&path)?), label.into()));
        }
    }
    let raw = get(row, "lyrics_file").trim();
    if !raw.is_empty() {
        let path = resolve(raw);
        if path.exists() {
            return Ok((lyric_source_entries(&path)?, "lyrics_file".into()));
        }
    }
    Ok((Vec::new(), "empty".into()))
}

/// Keep the project and write the edited ASS that render and live prefer.
pub fn save_entries(row: &Row, entries: &[Entry]) -> Result<PathBuf, String> {
    let sid = get(row, "id");
    write_atomic(&project_path(sid), &format!("{}\n", to_json(entries).dump()))?;
    let edited = paths().ass.join(format!("{sid}.edited.ass"));
    write_atomic(&edited, &ass(row, entries)?)?;
    Ok(edited)
}

/// Timing units: one CJK character, or one Latin word; '|' splits a word into syllables.
pub fn split_units(text: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut word = String::new();
    for c in text.chars() {
        if is_cjk_wide(c) {
            if !word.is_empty() {
                units.push(std::mem::take(&mut word));
            }
            units.push(c.to_string());
        } else if c.is_whitespace() || c == '|' {
            if !word.is_empty() {
                units.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        units.push(word);
    }
    units
}

fn lrc_stamp(value: i64, open: char, close: char) -> String {
    let value = value.max(0);
    format!("{open}{:02}:{:02}.{:02}{close}", value / 60000, value / 1000 % 60, value % 1000 / 10)
}

fn separator(previous: &str, word: &str) -> &'static str {
    let joined = previous.is_empty() || (previous.chars().last().map_or(false, is_cjk_wide) && word.chars().next().map_or(false, is_cjk_wide));
    if joined { "" } else { " " }
}

pub fn export_lrc(entries: &[Entry]) -> String {
    let mut out = Vec::new();
    for entry in entries {
        if entry.words.is_empty() {
            out.push(format!("{}{}", lrc_stamp(entry.start, '[', ']'), entry.text));
            continue;
        }
        let mut body = String::new();
        let mut previous = String::new();
        for (left, _, word) in &entry.words {
            body.push_str(&format!("{}{}{word}", separator(&previous, word), lrc_stamp(*left, '<', '>')));
            previous = word.clone();
        }
        out.push(format!("{}{body}{}", lrc_stamp(entry.start, '[', ']'), lrc_stamp(entry.words[entry.words.len() - 1].1, '<', '>')));
    }
    out.join("\n") + "\n"
}

pub fn export_srt(entries: &[Entry]) -> String {
    let stamp = |value: i64| {
        let value = value.max(0);
        format!("{:02}:{:02}:{:02},{:03}", value / 3600000, value / 60000 % 60, value / 1000 % 60, value % 1000)
    };
    entries.iter().enumerate().map(|(index, entry)| format!("{}\n{} --> {}\n{}\n", index + 1, stamp(entry.start), stamp(entry.end), entry.text)).collect::<Vec<_>>().join("\n")
}

pub fn export_krc(entries: &[Entry]) -> String {
    let mut out = Vec::new();
    for entry in entries {
        let words = if entry.words.is_empty() { vec![(entry.start, entry.end, entry.text.clone())] } else { entry.words.clone() };
        let body: String = words.iter().map(|(left, right, word)| format!("<{},{},0>{word}", left - entry.start, right - left)).collect();
        out.push(format!("[{},{}]{body}", entry.start, entry.end - entry.start));
    }
    out.join("\n") + "\n"
}

pub fn export_ksc(entries: &[Entry]) -> String {
    let stamp = |value: i64| format!("{:02}:{:02}.{:03}", value / 60000, value / 1000 % 60, value % 1000);
    let mut out = vec!["karaoke := CreateKaraokeObject;".to_string(), "karaoke.rows := 2;".to_string(), "karaoke.clear;".to_string(), String::new()];
    for entry in entries {
        let words = if entry.words.is_empty() { vec![(entry.start, entry.end, entry.text.clone())] } else { entry.words.clone() };
        // KSC durations are back to back, so a gap is folded into the word before it.
        let units: String = words.iter().map(|(_, _, word)| if word.chars().count() == 1 && word.chars().all(is_cjk_wide) { word.clone() } else { format!("[{word}]") }).collect();
        let durations: Vec<String> = words.iter().enumerate().map(|(index, (left, right, _))| (words.get(index + 1).map(|next| next.0).unwrap_or(*right) - left).max(0).to_string()).collect();
        out.push(format!("karaoke.add('{}', '{}', '{}', '{}');", stamp(words[0].0), stamp(words[words.len() - 1].1), units.replace('\'', "''"), durations.join(",")));
    }
    out.join("\n") + "\n"
}

/// The exporter for a format name.
pub fn export(format: &str, entries: &[Entry]) -> Option<String> {
    Some(match format {
        "lrc" => export_lrc(entries),
        "srt" => export_srt(entries),
        "krc" => export_krc(entries),
        "ksc" => export_ksc(entries),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_reads_back() {
        let text = "; PolyKaraIntroPaddingMs: 1000\nDialogue: 0,0:00:02.00,0:00:04.00,Lyric,,0,0,150,,{\\kf50}Hello {\\k20}{\\kf30}world\nDialogue: 1,0:00:02.00,0:00:04.00,Romanization,,0,0,0,,x\nDialogue: 0,0:00:05.00,0:00:06.00,Lyric,,0,0,150,,plain line\n";
        let entries = parse_ass(text);
        assert_eq!(entries[0].words, vec![(1000, 1500, "Hello".to_string()), (1700, 2000, "world".to_string())]);
        assert_eq!(entries[0].text, "Hello world");
        assert_eq!((entries[1].start, entries[1].text.as_str()), (4000, "plain line"));
    }

    #[test]
    fn projects_round_trip_and_export() {
        let entries = vec![Entry::new(1000, 2000, "你好", vec![(1000, 1500, "你".into()), (1500, 2000, "好".into())])];
        assert_eq!(from_json(&to_json(&entries)).unwrap(), entries);
        assert_eq!(export_lrc(&entries), "[00:01.00]<00:01.00>你<00:01.50>好<00:02.00>\n");
        assert_eq!(export_krc(&entries), "[1000,1000]<0,500,0>你<500,500,0>好\n");
        assert!(export_ksc(&entries).contains("karaoke.add('00:01.000', '00:02.000', '你好', '500,500');"));
        assert_eq!(export_srt(&entries), "1\n00:00:01,000 --> 00:00:02,000\n你好\n");
        assert_eq!(split_units("Hel|lo 你好"), vec!["Hel", "lo", "你", "好"]);
    }
}
