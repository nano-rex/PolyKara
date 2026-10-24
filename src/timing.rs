//! Word timing made or corrected by hand in the timing editor.
//!
//! The editor saves an enhanced LRC file, work/lyrics/<id>.timed.lrc, with a
//! time mark before every word (or CJK character). When that file exists it is
//! the song's lyric source ahead of everything else, and because it already
//! carries word timing, no automatic alignment is needed for it.

use std::path::PathBuf;

use crate::config::paths;
use crate::csv::Row;
use crate::difflib::{text_ratio, Matcher, Tag};
use crate::manifest::{find_source, get};
use crate::model::{Line, Token};
use crate::subtitle::{fill_gaps, load_units, parse_lrc, plain_entries, read_text, time_entries, word_json, Entry};
use crate::text::{speaker_tag, split_speaker, tokenise};
use crate::verify::{check_lyrics, Kind};

/// Untimed lines placed after the last timed one are spaced this far apart.
pub const ESTIMATED_LINE_MS: i64 = 3000;

pub fn edited_path(sid: &str) -> PathBuf {
    paths().lyrics.join(format!("{sid}.timed.lrc"))
}

/// The file to play: the normalized audio when it is still there, else the downloaded media.
pub fn media_for(sid: &str) -> Option<PathBuf> {
    let audio = paths().audio.join(format!("{sid}.flac"));
    if audio.exists() {
        Some(audio)
    } else {
        find_source(sid)
    }
}

pub fn lines_from_entries(entries: &[Entry]) -> Vec<Line> {
    let mut lines = Vec::new();
    for entry in entries {
        let (speaker, lyric) = split_speaker(&entry.text);
        let tokens: Vec<Token> = if entry.words.is_empty() {
            tokenise(&lyric).into_iter().map(|text| Token { text, start: None, end: None }).collect()
        } else {
            let mut tokens: Vec<Token> = entry.words.iter().map(|(left, right, word)| Token { text: word.clone(), start: Some(*left), end: Some(*right) }).collect();
            // Keep a speaker tag that was timed as part of the first word out of the lyric (and only the tag:
            // the space after an English first word separates it from the next).
            if let Some((length, _)) = speaker_tag(&tokens[0].text) {
                if length < tokens[0].text.len() {
                    tokens[0].text = tokens[0].text[length..].to_string();
                }
            }
            tokens
        };
        if !tokens.is_empty() {
            lines.push(Line { tokens, speaker, start: Some(entry.start), end: Some(entry.end) });
        }
    }
    lines
}

/// Untimed lines from lyric text, one per non-empty line; a [singer: x] tag at the start is kept.
pub fn lines_from_text(text: &str) -> Vec<Line> {
    text.lines()
        .filter_map(|raw| {
            let (speaker, lyric) = split_speaker(raw.trim());
            let tokens: Vec<Token> = tokenise(&lyric).into_iter().map(|text| Token { text, start: None, end: None }).collect();
            (!tokens.is_empty()).then(|| Line { tokens, speaker, start: None, end: None })
        })
        .collect()
}

/// Lines for edited lyric text, keeping the timing of every line whose words did not change.
///
/// A changed line keeps its line times, and the timing of its words when it
/// still has the same number of them (a corrected typo).
/// The lyrics text of the lines, one per line, with the singer prefix.
pub fn lines_to_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| if line.speaker.is_empty() { line.text() } else { format!("[singer: {}] {}", line.speaker, line.text()) })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn retext(lines: &[Line], text: &str) -> Vec<Line> {
    let mut fresh = lines_from_text(text);
    let old: Vec<String> = lines.iter().map(Line::text).collect();
    let new: Vec<String> = fresh.iter().map(Line::text).collect();
    for (tag, i1, i2, j1, j2) in Matcher::new(&old, &new).opcodes() {
        match tag {
            Tag::Equal => {
                for offset in 0..i2 - i1 {
                    let previous = &lines[i1 + offset];
                    let target = &mut fresh[j1 + offset];
                    target.tokens = previous.tokens.clone();
                    target.start = previous.start;
                    target.end = previous.end;
                }
            }
            Tag::Replace => {
                // Pair each changed line with the most similar old line, in order.
                let mut cursor = i1;
                for index in j1..j2 {
                    let best = (cursor..i2).map(|candidate| (text_ratio(&old[candidate], &new[index]), candidate)).fold(None, |best: Option<(f64, usize)>, item| match best {
                        Some(best) if best.0 > item.0 => Some(best),
                        _ => Some(item),
                    });
                    let (score, candidate) = match best {
                        Some(best) => best,
                        None => continue,
                    };
                    if score < 0.5 {
                        continue;
                    }
                    cursor = candidate + 1;
                    let previous = &lines[candidate];
                    let target = &mut fresh[index];
                    target.start = previous.start;
                    target.end = previous.end;
                    if previous.tokens.len() == target.tokens.len() {
                        for (before, after) in previous.tokens.iter().zip(target.tokens.iter_mut()) {
                            after.start = before.start;
                            after.end = before.end;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    fresh
}

/// The song's lines with the best timing already known, and where they came from.
pub fn load(row: &Row) -> Result<(Vec<Line>, String), String> {
    let sid = get(row, "id");
    let edited = edited_path(sid);
    if edited.exists() {
        let name = edited.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        return Ok((lines_from_entries(&parse_lrc(&read_text(&edited)?)), format!("your saved timing ({name})")));
    }
    let report = check_lyrics(row)?;
    let primary = match report.primary() {
        Some(primary) => primary,
        None => return Ok((Vec::new(), String::new())),
    };
    let timing = word_json(row);
    if primary.kind == Kind::Plain {
        let text = read_text(&primary.path)?;
        if let Some(timing) = &timing {
            let entries = plain_entries(&text, &load_units(timing)?);
            if !entries.is_empty() {
                return Ok((lines_from_entries(&entries), format!("{}, timed automatically", primary.label)));
            }
        }
        return Ok((lines_from_text(&text), format!("{} (not timed yet)", primary.label)));
    }
    if let Some(timing) = &timing {
        if primary.entries.iter().any(|entry| entry.words.is_empty()) {
            let mut estimated = Vec::new();
            let entries = time_entries(&primary.entries, &load_units(timing)?, 0, 1000, &mut estimated);
            return Ok((lines_from_entries(&entries), format!("{}, timed automatically", primary.label)));
        }
    }
    Ok((lines_from_entries(&primary.entries), primary.label.clone()))
}

/// Entries ready to save, and how many lines had no timing and were placed by estimate.
///
/// Words left untimed inside a timed line are spread between their timed
/// neighbours; a line with no times at all is placed in the gap between the
/// lines around it.
pub fn finished(lines: &[Line]) -> (Vec<Entry>, usize) {
    let mut spans: Vec<Option<(i64, i64)>> = lines.iter().map(Line::span).collect();
    let mut estimated = 0;
    let mut index = 0;
    while index < lines.len() {
        if spans[index].is_some() {
            index += 1;
            continue;
        }
        let mut stop = index;
        while stop < lines.len() && spans[stop].is_none() {
            stop += 1;
        }
        let count = (stop - index) as i64;
        let low = if index > 0 { spans[index - 1].map(|span| span.1).unwrap_or(0) } else { 0 };
        let high = if stop < lines.len() { spans[stop].map(|span| span.0).unwrap_or(low) } else { low + ESTIMATED_LINE_MS * count };
        let step = 300.max((high - low) / count);
        for offset in 0..count {
            let begin = low + step * offset;
            spans[index + offset as usize] = Some((begin, begin + step));
        }
        estimated += count as usize;
        index = stop;
    }
    let mut entries = Vec::new();
    for (line, span) in lines.iter().zip(spans) {
        let (start, end) = span.unwrap_or((0, 0));
        let times: Vec<Option<(i64, i64)>> = line.tokens.iter().map(|token| match (token.start, token.end) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => None,
        }).collect();
        let filled = fill_gaps(&times, start, end.max(start + crate::model::MIN_TOKEN_MS * line.tokens.len() as i64));
        let words: Vec<(i64, i64, String)> = filled.into_iter().zip(&line.tokens).map(|((left, right), token)| (left, right.max(left + 10), token.text.clone())).collect();
        let body = line.text();
        let text = if line.speaker.is_empty() { body.trim().to_string() } else { format!("[speaker:{}] {}", line.speaker, body.trim()) };
        let (first, last) = (words[0].0, words[words.len() - 1].1);
        entries.push(Entry::new(first, last.max(first + 300), text, words));
    }
    (entries, estimated)
}

pub fn stamp(value: i64) -> String {
    let value = value.max(0);
    format!("{:02}:{:02}.{:03}", value / 60000, value / 1000 % 60, value % 1000)
}

/// Enhanced LRC: [line start] <word start>word ... <last word end>, plus an empty [mark] where a line ends early.
pub fn to_lrc(entries: &[Entry], title: &str, artist: &str) -> String {
    let mut out = vec!["[re:PolyKara timing editor]".to_string()];
    if !title.is_empty() {
        out.push(format!("[ti:{title}]"));
    }
    if !artist.is_empty() {
        out.push(format!("[ar:{artist}]"));
    }
    for (index, entry) in entries.iter().enumerate() {
        let (speaker, _) = split_speaker(&entry.text);
        let mut body = String::new();
        for (number, (left, right, word)) in entry.words.iter().enumerate() {
            body.push_str(&format!("<{}>{word}", stamp(*left)));
            // An empty mark ends a word before the next one starts (a pause in the singing).
            if number + 1 == entry.words.len() || entry.words[number + 1].0 > *right {
                body.push_str(&format!("<{}>", stamp(*right)));
            }
        }
        let tag = if speaker.is_empty() { String::new() } else { format!("[speaker:{speaker}] ") };
        out.push(format!("[{}]{tag}{body}", stamp(entry.start)));
        let following = entries.get(index + 1).map(|next| next.start);
        if following.map_or(true, |next| next > entry.end) {
            out.push(format!("[{}]", stamp(entry.end)));
        }
    }
    out.join("\n") + "\n"
}

/// Save the timing; returns the file and how many lines were placed by estimate.
pub fn save(row: &Row, lines: &[Line]) -> Result<(PathBuf, usize), String> {
    let (entries, estimated) = finished(lines);
    let target = edited_path(get(row, "id"));
    crate::manifest::write_atomic(&target, &to_lrc(&entries, get(row, "title"), get(row, "artist")))?;
    Ok((target, estimated))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed_line(start: i64, pieces: &[&str], speaker: &str) -> Line {
        Line {
            tokens: pieces.iter().enumerate().map(|(index, piece)| Token { text: piece.to_string(), start: Some(start + 300 * index as i64), end: Some(start + 300 * index as i64 + 250) }).collect(),
            speaker: speaker.into(),
            start: None,
            end: None,
        }
    }

    #[test]
    fn saved_timing_reads_back_word_for_word() {
        let lines = vec![timed_line(1000, &["Hello ", "PolyKara ", "world"], ""), timed_line(5000, &["唤", "不", "醒"], "女")];
        let (entries, estimated) = finished(&lines);
        let back = parse_lrc(&to_lrc(&entries, "Title", "Singer"));
        assert_eq!(estimated, 0);
        assert_eq!(back.iter().map(|entry| entry.words.clone()).collect::<Vec<_>>(), entries.iter().map(|entry| entry.words.clone()).collect::<Vec<_>>());
        assert_eq!(back[0].text, "Hello PolyKara world");
        assert_eq!(back[1].text, "[speaker:女] 唤不醒");
        // The empty mark after a line gives its real end, not the next line's start.
        assert_eq!(back[0].end, 1850);
        let read = lines_from_entries(&back);
        assert_eq!(read[1].speaker, "女");
        assert_eq!(read[0].tokens[0].text, "Hello ");
    }

    #[test]
    fn untimed_words_and_lines_are_filled_in() {
        let mut partial = timed_line(1000, &["一", "二", "三", "四"], "");
        partial.tokens[1].start = None;
        partial.tokens[1].end = None;
        let untimed = Line { tokens: vec![Token { text: "没".into(), start: None, end: None }, Token { text: "有".into(), start: None, end: None }], ..Default::default() };
        let (entries, estimated) = finished(&[partial, untimed, timed_line(9000, &["五", "六"], "")]);
        assert_eq!(estimated, 1);
        let first = &entries[0].words;
        assert_eq!((first[1].0, first[1].1), (first[0].1, first[2].0));
        assert!(entries[0].end <= entries[1].start && entries[1].start < entries[2].start);
    }

    #[test]
    fn retext_keeps_timing() {
        let lines = vec![timed_line(1000, &["one ", "two"], ""), timed_line(3000, &["three ", "four"], "")];
        let fresh = retext(&lines, "one two\nnew line here\n[singer: 男] three fore");
        assert_eq!(fresh[0].tokens[1].start, Some(1300));
        assert!(fresh[1].tokens.iter().all(|token| token.start.is_none()));
        // Same number of words: a corrected typo keeps its times.
        assert_eq!((fresh[2].speaker.as_str(), fresh[2].tokens[1].text.as_str(), fresh[2].tokens[1].start), ("男", "fore", Some(3300)));
    }
}
