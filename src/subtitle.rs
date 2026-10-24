//! Lyric files with line timing (LRC, SRT, VTT) and matching their words to recognised speech.

use std::path::{Path, PathBuf};

use crate::config::{paths, resolve};
use crate::csv::Row;
use crate::difflib::{Matcher, Tag};
use crate::json::Value;
use crate::manifest::get;
use crate::text::{split_speaker, token_key, tokenise};

/// Estimated duration of a token nobody recognised, and how long one may be stretched.
pub const TYPICAL_TOKEN_MS: i64 = 400;
pub const MAX_TOKEN_MS: i64 = 1200;
pub const MIN_SPREAD_MS: i64 = 80;

/// One timed word (or CJK character): start and end in ms, and its text with any trailing space.
pub type Word = (i64, i64, String);
/// A recognised word from the aligner: start, end and its comparison key.
pub type Unit = (i64, i64, String);

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub start: i64,
    pub end: i64,
    /// The line, with a "[speaker:x] " prefix when a singer is named.
    pub text: String,
    pub words: Vec<Word>,
}

impl Entry {
    pub fn new(start: i64, end: i64, text: impl Into<String>, words: Vec<Word>) -> Entry {
        Entry { start, end, text: text.into(), words }
    }
}

fn find_json_files(folder: &Path, found: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                find_json_files(&path, found);
            } else if path.extension().map_or(false, |extension| extension == "json") {
                found.push(path);
            }
        }
    }
}

/// The song's word timing file: word_timing_file from songs.csv, else work/align/<id>.json.
pub fn word_json(row: &Row) -> Option<PathBuf> {
    let configured = get(row, "word_timing_file").trim();
    let sid = get(row, "id");
    if !configured.is_empty() {
        let path = resolve(configured);
        if path.exists() {
            return Some(path);
        }
        println!("WARN {sid}: word_timing_file not found ({}); using automatic alignment", path.display());
    }
    let align = paths().align.clone();
    let exact = align.join(format!("{sid}.json"));
    if exact.exists() {
        return Some(exact);
    }
    let mut found = Vec::new();
    find_json_files(&align, &mut found);
    found.retain(|path| {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = path.parent().and_then(|parent| parent.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        name.starts_with(&format!("{sid}.")) || parent == sid
    });
    found.sort();
    found.into_iter().next()
}

/// Recognised words as (start_ms, end_ms, key) units, one per lyric token.
pub fn timed_units(data: &Value) -> Vec<Unit> {
    let mut words: Vec<&Value> = Vec::new();
    for segment in data.get("segments").items() {
        let inner = if !segment.get("words").items().is_empty() { segment.get("words") } else { segment.get("word_segments") };
        words.extend(inner.items());
    }
    if words.is_empty() {
        words = data.get("word_segments").items().iter().collect();
    }
    let mut units = Vec::new();
    for word in words {
        let (start, end) = match (number(word.get("start")), number(word.get("end"))) {
            (Some(start), Some(end)) => ((start * 1000.0) as i64, (end * 1000.0) as i64),
            _ => continue,
        };
        let text = match word.get("word") {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.dump(),
        };
        let keys: Vec<String> = tokenise(&text).iter().map(|token| token_key(token)).filter(|key| !key.is_empty()).collect();
        let span = (end - start).max(0);
        let count = keys.len() as i64;
        // A recognised CJK "word" covers several lyric characters; share its time between them.
        for (index, key) in keys.into_iter().enumerate() {
            let index = index as i64;
            units.push((start + span * index / count, start + span * (index + 1) / count, key));
        }
    }
    units.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    units
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => Some(*number),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub fn load_units(path: &Path) -> Result<Vec<Unit>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{} could not be read ({error})", path.display()))?;
    let data = Value::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let units = timed_units(&data);
    if units.is_empty() {
        return Err(format!("No word timings found in {}", path.display()));
    }
    Ok(units)
}

/// Give each lyric token the time of the recognised word it corresponds to, or None.
pub fn match_tokens(keys: &[String], units: &[Unit]) -> Vec<Option<(i64, i64)>> {
    let mut times = vec![None; keys.len()];
    if keys.is_empty() || units.is_empty() {
        return times;
    }
    let heard: Vec<String> = units.iter().map(|unit| unit.2.clone()).collect();
    for (tag, i1, i2, j1, j2) in Matcher::new(keys, &heard).opcodes() {
        let count = i2 - i1;
        if tag == Tag::Equal || (tag == Tag::Replace && count == j2 - j1) {
            // Equal-length mismatches are misheard words or homophones in the same position.
            for offset in 0..count {
                times[i1 + offset] = Some((units[j1 + offset].0, units[j1 + offset].1));
            }
        } else if tag == Tag::Replace {
            let (low, high) = (units[j1].0, units[j2 - 1].1);
            let count = count as i64;
            if MIN_SPREAD_MS * count <= high - low && high - low <= MAX_TOKEN_MS * count {
                for offset in 0..count {
                    times[i1 + offset as usize] = Some((low + (high - low) * offset / count, low + (high - low) * (offset + 1) / count));
                }
            }
        }
    }
    times
}

/// Estimate unmatched tokens from their matched neighbours and keep the sequence monotonic.
pub fn fill_gaps(times: &[Option<(i64, i64)>], low: i64, high: i64) -> Vec<(i64, i64)> {
    let mut filled: Vec<Option<(i64, i64)>> = times.to_vec();
    let mut index = 0;
    while index < filled.len() {
        if filled[index].is_some() {
            index += 1;
            continue;
        }
        let mut stop = index;
        while stop < filled.len() && filled[stop].is_none() {
            stop += 1;
        }
        let count = (stop - index) as i64;
        let left = if index > 0 { filled[index - 1].map(|time| time.1) } else { None };
        let right = if stop < filled.len() { filled[stop].map(|time| time.0) } else { None };
        let (left, right) = match (left, right) {
            (None, None) => (low, high.min(low + count * MAX_TOKEN_MS)),
            (None, Some(right)) => (low.max(right - count * TYPICAL_TOKEN_MS), right),
            (Some(left), None) => (left, high.min(left + count * TYPICAL_TOKEN_MS)),
            (Some(left), Some(right)) => (left, right.min(left + count * MAX_TOKEN_MS)),
        };
        let right = right.max(left);
        for offset in 0..count {
            filled[index + offset as usize] = Some((left + (right - left) * offset / count, left + (right - left) * (offset + 1) / count));
        }
        index = stop;
    }
    let mut cursor = low;
    filled
        .into_iter()
        .map(|time| {
            let (left, right) = time.unwrap_or((cursor, cursor));
            let left = left.max(cursor).min(high);
            let right = right.max(left).min(high);
            cursor = right;
            (left, right)
        })
        .collect()
}

/// Attach word timing to each line by matching lyric tokens against recognised words.
///
/// Lines that already carry word timing are kept. Start times of lines where
/// nothing was recognised are added to `estimated`.
pub fn time_entries(entries: &[Entry], units: &[Unit], max_tail_ms: i64, tail_hold_ms: i64, estimated: &mut Vec<i64>) -> Vec<Entry> {
    let mut result = Vec::new();
    let mut used = 0;
    for entry in entries {
        if !entry.words.is_empty() {
            result.push(entry.clone());
            continue;
        }
        let tokens = tokenise(&split_speaker(&entry.text).1);
        if tokens.is_empty() {
            result.push(entry.clone());
            continue;
        }
        let mut first = used;
        while first < units.len() && units[first].1 < entry.start {
            first += 1;
        }
        let mut last = first;
        while last < units.len() && units[last].0 <= entry.end {
            last += 1;
        }
        used = used.max(last);
        let keys: Vec<String> = tokens.iter().map(|token| token_key(token)).collect();
        let times = match_tokens(&keys, &units[first..last]);
        let recognised = times.iter().any(Option::is_some);
        if !recognised {
            estimated.push(entry.start);
        }
        let timed: Vec<Word> = fill_gaps(&times, entry.start, entry.end).into_iter().zip(tokens).map(|((left, right), token)| (left, right, token)).collect();
        let mut end = entry.end;
        // A line that runs into a long instrumental should leave the screen after its last word.
        let last_end = timed.last().map(|word| word.1).unwrap_or(end);
        if recognised && max_tail_ms > 0 && end - last_end > max_tail_ms {
            end = (entry.start + 300).max(last_end + tail_hold_ms.max(0));
        }
        result.push(Entry::new(entry.start, end, entry.text.clone(), timed));
    }
    result
}

/// Time untimed lyric lines by aligning the whole text against the recognised words.
pub fn plain_entries(text: &str, units: &[Unit]) -> Vec<Entry> {
    let lines: Vec<Vec<String>> = text.lines().map(|line| tokenise(line.trim())).filter(|tokens| !tokens.is_empty()).collect();
    let keys: Vec<String> = lines.iter().flatten().map(|token| token_key(token)).collect();
    if keys.is_empty() || units.is_empty() {
        return Vec::new();
    }
    let times = fill_gaps(&match_tokens(&keys, units), units[0].0, units[units.len() - 1].1);
    let mut result = Vec::new();
    let mut cursor = 0;
    for tokens in lines {
        let timed: Vec<Word> = times[cursor..cursor + tokens.len()].iter().zip(&tokens).map(|((left, right), token)| (*left, *right, token.clone())).collect();
        cursor += tokens.len();
        let start = timed[0].0;
        let end = (start + 300).max(timed[timed.len() - 1].1);
        result.push(Entry::new(start, end, tokens.concat(), timed));
    }
    result
}

/// "[mm:ss.xx]" (or "<mm:ss.xx>" with `open` '<') at the start of `text`: (ms, length in bytes).
pub fn time_mark(text: &str, open: char, close: char) -> Option<(i64, usize)> {
    let rest = text.strip_prefix(open)?;
    let end = rest.find(close)?;
    let inside = &rest[..end];
    let (minutes, after) = inside.split_once(':')?;
    if minutes.is_empty() || !minutes.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = match after.find(['.', ':']) {
        Some(at) => (&after[..at], &after[at + 1..]),
        None => (after, ""),
    };
    if seconds.len() != 2 || !seconds.chars().all(|c| c.is_ascii_digit()) || fraction.len() > 3 || !fraction.chars().all(|c| c.is_ascii_digit()) || (after.contains(['.', ':']) && fraction.is_empty()) {
        return None;
    }
    let fraction: i64 = format!("{fraction}000")[..3].parse().ok()?;
    let value = (minutes.parse::<i64>().ok()? * 60 + seconds.parse::<i64>().ok()?) * 1000 + fraction;
    Some((value, open.len_utf8() + end + close.len_utf8()))
}

/// Split an enhanced-LRC line into its plain text and (start, end, word) timings.
pub fn inline_words(lyric: &str, start: i64, end: i64) -> (String, Vec<Word>) {
    let mut marks: Vec<(usize, usize, i64)> = Vec::new();
    let mut search = 0;
    while let Some(found) = lyric[search..].find('<') {
        let at = search + found;
        match time_mark(&lyric[at..], '<', '>') {
            Some((value, length)) => {
                marks.push((at, at + length, value));
                search = at + length;
            }
            None => search = at + 1,
        }
    }
    if marks.is_empty() {
        return (lyric.trim().to_string(), Vec::new());
    }
    let mut pieces: Vec<(i64, &str)> = vec![(start, &lyric[..marks[0].0])];
    for (index, (_, after, value)) in marks.iter().enumerate() {
        let stop = marks.get(index + 1).map(|mark| mark.0).unwrap_or(lyric.len());
        pieces.push((*value, &lyric[*after..stop]));
    }
    let mut words: Vec<Word> = Vec::new();
    for (index, (left, piece)) in pieces.iter().enumerate() {
        if piece.trim().is_empty() {
            continue;
        }
        let following = pieces.get(index + 1);
        let right = following.map(|item| item.0).unwrap_or(end);
        let spaced = piece.chars().last().map_or(false, char::is_whitespace) || following.map_or(false, |item| item.1.chars().next().map_or(false, char::is_whitespace));
        words.push(((*left).max(start), (left + 10).max(right), format!("{}{}", piece.trim(), if spaced { " " } else { "" })));
    }
    if let Some(last) = words.last_mut() {
        last.2 = last.2.trim_end().to_string();
    }
    (words.iter().map(|word| word.2.as_str()).collect(), words)
}

fn offset_tag(line: &str) -> Option<i64> {
    let rest = line.trim_start().strip_prefix('[')?;
    let lower = rest.to_lowercase();
    let after = lower.strip_prefix("offset")?.trim_start().strip_prefix(':')?.trim_start();
    let close = after.find(']')?;
    after[..close].trim().parse().ok()
}

pub fn parse_lrc(content: &str) -> Vec<Entry> {
    let mut offset = 0;
    let mut items: Vec<(i64, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = offset_tag(line) {
            // A positive LRC offset makes the lyrics appear earlier.
            offset = -value;
            continue;
        }
        let mut marks = Vec::new();
        let mut position = 0;
        while let Some((value, length)) = time_mark(&line[position..], '[', ']') {
            marks.push(value);
            position += length;
            position += line[position..].len() - line[position..].trim_start().len();
        }
        if marks.is_empty() {
            continue;
        }
        // A timestamp without text marks where the previous line ends.
        let text = line[position..].trim().to_string();
        items.extend(marks.into_iter().map(|value| (value, text.clone())));
    }
    items.sort_by_key(|item| item.0);
    let mut result = Vec::new();
    for (index, (start, raw)) in items.iter().enumerate() {
        let (speaker, lyric) = split_speaker(raw);
        if lyric.is_empty() {
            continue;
        }
        // Lines sharing a timestamp (original plus translation) end together at the next later mark.
        let fallback = start + 2000i64.max(lyric.chars().count() as i64 * 180);
        let end = items[index + 1..].iter().map(|item| item.0).find(|time| time > start).unwrap_or(fallback);
        let end = end.max(start + 300);
        let (clean, words) = inline_words(&lyric, *start, end);
        if clean.is_empty() {
            continue;
        }
        let text = if speaker.is_empty() { clean } else { format!("[speaker:{speaker}] {clean}") };
        let shift = (-start).max(offset);
        result.push(Entry::new(start + shift, end + shift, text, words.into_iter().map(|(left, right, word)| (left + shift, right + shift, word)).collect()));
    }
    result
}

/// "01:02:03,456" or "02:03.456" in ms.
pub fn subtitle_time(value: &str) -> Option<i64> {
    let value = value.replace(',', ".");
    let parts: Vec<&str> = value.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => ("0", *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let fraction: i64 = format!("{fraction}000").get(..3)?.parse().ok()?;
    Some((hours.parse::<i64>().ok()? * 3600 + minutes.parse::<i64>().ok()? * 60 + whole.parse::<i64>().ok()?) * 1000 + fraction)
}

fn time_like(value: &str) -> bool {
    let parts: Vec<&str> = value.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return false;
    }
    let last = parts[parts.len() - 1];
    let (seconds, fraction) = match last.find([',', '.']) {
        Some(at) => (&last[..at], &last[at + 1..]),
        None => return false,
    };
    parts[..parts.len() - 1].iter().all(|part| (1..=2).contains(&part.len()) && part.chars().all(|c| c.is_ascii_digit()))
        && seconds.len() == 2
        && seconds.chars().all(|c| c.is_ascii_digit())
        && (1..=3).contains(&fraction.len())
        && fraction.chars().all(|c| c.is_ascii_digit())
}

/// The start and end of a "00:01.000 --> 00:02.000" cue line.
fn cue_times(line: &str) -> Option<(i64, i64)> {
    let arrow = line.find("-->")?;
    let before = line[..arrow].trim_end();
    let after = line[arrow + 3..].trim_start();
    let start = before.rsplit(char::is_whitespace).next()?;
    let end = after.split(char::is_whitespace).next()?;
    // The time may be glued to other text before it; take its longest time-like tail.
    let start = (0..start.len()).filter(|at| start.is_char_boundary(*at)).map(|at| &start[at..]).find(|tail| time_like(tail))?;
    let end = (1..=end.len()).rev().filter(|at| end.is_char_boundary(*at)).map(|at| &end[..at]).find(|head| time_like(head))?;
    Some((subtitle_time(start)?, subtitle_time(end)?))
}

fn strip_markup(line: &str) -> String {
    let mut out = String::new();
    let mut skip: Option<char> = None;
    for c in line.chars() {
        match skip {
            Some(close) if c == close => skip = None,
            Some(_) => {}
            None if c == '<' => skip = Some('>'),
            None if c == '{' => skip = Some('}'),
            None if matches!(c, '♪' | '♫' | '♬') => {}
            None => out.push(c),
        }
    }
    out.trim().to_string()
}

pub fn parse_timed_subtitle(content: &str) -> Vec<Entry> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut index = 0;
    let mut previous = String::new();
    while index < lines.len() {
        let (start, end) = match cue_times(lines[index]) {
            Some(times) => times,
            None => {
                index += 1;
                continue;
            }
        };
        index += 1;
        let mut text_lines: Vec<String> = Vec::new();
        // YouTube captions contain whitespace-only lines inside a cue, so only a truly
        // empty line, or a blank one followed by the next cue, ends the text block.
        while index < lines.len() && !lines[index].is_empty() && !(lines[index].trim().is_empty() && lines[index + 1..lines.len().min(index + 3)].iter().any(|line| cue_times(line).is_some())) {
            let cleaned = strip_markup(lines[index]);
            if !cleaned.is_empty() {
                text_lines.push(cleaned);
            }
            index += 1;
        }
        // YouTube automatic captions repeat the previous line above each new one and
        // insert near-zero-length cues that only restate it.
        if end - start <= 50 {
            continue;
        }
        if text_lines.len() > 1 && text_lines[0] == previous {
            text_lines.remove(0);
        }
        if text_lines.is_empty() {
            continue;
        }
        previous = text_lines[text_lines.len() - 1].clone();
        let text = text_lines.join(" ");
        let (speaker, lyric) = split_speaker(&text);
        // Skip sound descriptions such as [Music] or [Applause].
        let description = lyric.starts_with('[') && lyric.ends_with(']') && !lyric[1..lyric.len() - 1].contains(']');
        if !lyric.is_empty() && !description {
            let tagged = if speaker.is_empty() { lyric } else { format!("[speaker:{speaker}] {lyric}") };
            result.push(Entry::new(start, (start + 300).max(end), tagged, Vec::new()));
        }
    }
    result
}

pub fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{} could not be read ({error})", path.display()))?;
    String::from_utf8(bytes).map(|text| text.trim_start_matches('\u{feff}').to_string()).map_err(|_| format!("{} is not UTF-8 text", path.display()))
}

pub fn lrc(path: &Path) -> Result<Vec<Entry>, String> {
    read_text(path).map(|text| parse_lrc(&text))
}

pub fn timed_subtitle(path: &Path) -> Result<Vec<Entry>, String> {
    read_text(path).map(|text| parse_timed_subtitle(&text))
}

/// The best downloaded subtitle: creator-made by default, automatic captions with `auto`.
pub fn downloaded_subtitle(row: &Row, auto: bool) -> Option<PathBuf> {
    let sid = get(row, "id");
    let folder = paths().subtitles.clone();
    let prefix = format!("{sid}.");
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.retain(|name| {
        let lower = name.to_lowercase();
        name.starts_with(&prefix) && (lower.ends_with(".vtt") || lower.ends_with(".srt")) && lower.contains(".auto.") == auto
    });
    if names.is_empty() {
        return None;
    }
    let language = get(row, "language").trim().to_lowercase();
    let preferred: Vec<String> = get(row, "subtitle_langs")
        .split(',')
        .map(|item| item.trim().to_lowercase())
        .filter(|item| !item.is_empty() && item != "all")
        .map(|item| item.trim_end_matches(['*', '.']).to_string())
        .collect();
    let tag = |name: &str| {
        let lower = name.to_lowercase();
        let rest = &lower[sid.len() + 1..];
        let rest = rest.strip_prefix("auto.").unwrap_or(rest);
        rest.rsplit_once('.').map(|(head, _)| head.to_string()).unwrap_or_default()
    };
    names.sort_by_key(|name| {
        let tag = tag(name);
        (if !language.is_empty() && tag.starts_with(&language) { 0 } else { 1 }, if preferred.iter().any(|item| !item.is_empty() && tag.starts_with(item.as_str())) { 0 } else { 1 }, name.clone())
    });
    Some(folder.join(&names[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_lrc_with_offsets_and_speakers() {
        let entries = parse_lrc("[offset:+500]\n[00:05.000]<00:05.000>Hello <00:05.500>PolyKara <00:06.200>world<00:06.800>\n[00:08.00][singer: 女] 唤醒\n[00:10.000]\n");
        assert_eq!(entries[0].words, vec![(4500, 5000, "Hello ".to_string()), (5000, 5700, "PolyKara ".to_string()), (5700, 6300, "world".to_string())]);
        assert_eq!(entries[0].text, "Hello PolyKara world");
        assert_eq!(entries[1].text, "[speaker:女] 唤醒");
        assert_eq!((entries[1].start, entries[1].end), (7500, 9500));
    }

    #[test]
    fn lines_sharing_a_timestamp_end_together() {
        let entries = parse_lrc("[00:01.00]原文\n[00:01.00]Translation\n[00:04.00]next\n");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].end, 4000);
        assert_eq!(entries[1].end, 4000);
    }

    #[test]
    fn youtube_captions() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000 align:start\nfirst <c>line</c>\n\n00:00:03.000 --> 00:00:03.010\nfirst line\n\n00:00:03.010 --> 00:00:05.000\nfirst line\nsecond line\n\n00:00:06.000 --> 00:00:07.000\n[Music]\n";
        let entries = parse_timed_subtitle(vtt);
        assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>(), vec!["first line", "second line"]);
        assert_eq!(subtitle_time("01:02:03,456"), Some(3_723_456));
    }

    #[test]
    fn words_are_matched_to_what_was_heard() {
        let units: Vec<Unit> = vec![(1000, 1300, "hello".into()), (1300, 1800, "world".into()), (2000, 2200, "again".into())];
        let entries = vec![Entry::new(900, 2500, "Hello big world again", vec![])];
        let mut estimated = Vec::new();
        let timed = time_entries(&entries, &units, 0, 1000, &mut estimated);
        let starts: Vec<i64> = timed[0].words.iter().map(|word| word.0).collect();
        assert_eq!(timed[0].words[0], (1000, 1300, "Hello ".to_string()));
        assert_eq!(starts[2], 1300);
        assert!(starts.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(estimated.is_empty());
        assert_eq!(fill_gaps(&[None, Some((500, 600)), None], 0, 1000), vec![(100, 500), (500, 600), (600, 1000)]);
    }

    #[test]
    fn aligner_json_becomes_units() {
        let data = Value::parse(r#"{"segments": [{"words": [{"start": 1.0, "end": 1.6, "word": " 唤不醒"}, {"start": 2.0, "end": 2.5, "word": "Hi!"}, {"start": null, "end": 3, "word": "x"}]}]}"#).unwrap();
        let units = timed_units(&data);
        assert_eq!(units, vec![(1000, 1200, "唤".into()), (1200, 1400, "不".into()), (1400, 1600, "醒".into()), (2000, 2500, "hi".into())]);
        let plain = plain_entries("唤不醒\nhi", &units);
        assert_eq!(plain[1].words[0].0, 2000);
    }
}
