use crate::model::{Entry, Word};
use anyhow::{anyhow, Result};
use regex::Regex;
use std::fs;
use std::path::Path;

pub fn parse_file(path: &Path) -> Result<Vec<Entry>> {
    let text = fs::read_to_string(path)?;
    match path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "lrc" => parse_lrc(&text),
        "srt" | "vtt" => parse_timed(&text),
        "krc" => parse_krc(&text),
        "ksc" => parse_ksc(&text),
        _ => Ok(parse_plain(&text)),
    }
}

pub fn parse_lrc(text: &str) -> Result<Vec<Entry>> {
    let time = Regex::new(r"\[(\d+):(\d{2})(?:[.:](\d{1,3}))?\]")?;
    let word = Regex::new(r"<(\d+):(\d{2})(?:[.:](\d{1,3}))?>")?;
    let offset = Regex::new(r"(?i)^\[offset:([+-]?\d+)\]")?;
    let shift = text
        .lines()
        .find_map(|line| offset.captures(line).and_then(|c| c[1].parse::<i64>().ok()))
        .unwrap_or(0);
    let mut items = Vec::new();
    for line in text.lines() {
        let caps: Vec<_> = time.captures_iter(line).collect();
        if caps.is_empty() {
            continue;
        }
        let mut lyric = time.replace_all(line, "").to_string();
        let words = parse_inline_words(&lyric, &word, shift);
        lyric = word.replace_all(&lyric, "").to_string();
        if lyric.trim().is_empty() {
            continue;
        }
        for cap in caps {
            let start = timestamp(&cap)? + shift;
            items.push(Entry {
                start,
                end: start,
                text: lyric.clone(),
                words: words.clone(),
            });
        }
    }
    items.sort_by_key(|e| e.start);
    for i in 0..items.len() {
        items[i].end = items
            .get(i + 1)
            .map(|x| x.start)
            .unwrap_or(items[i].start + 2000)
            .max(items[i].start);
    }
    Ok(items)
}

fn parse_inline_words(text: &str, regex: &Regex, shift: i64) -> Vec<Word> {
    let marks: Vec<_> = regex.captures_iter(text).collect();
    if marks.is_empty() {
        return Vec::new();
    }
    let mut words = Vec::new();
    for (i, cap) in marks.iter().enumerate() {
        let start = timestamp(cap).unwrap_or(0) + shift;
        let end = marks
            .get(i + 1)
            .map(|next| timestamp(next).unwrap_or(start) + shift)
            .unwrap_or(start + 500);
        let left = cap.get(0).map(|m| m.end()).unwrap_or(0);
        let right = marks
            .get(i + 1)
            .and_then(|x| x.get(0))
            .map(|m| m.start())
            .unwrap_or(text.len());
        if right > left {
            words.push(Word {
                start,
                end: end.max(start),
                text: text[left..right].to_string(),
            });
        }
    }
    words
}

fn timestamp(caps: &regex::Captures<'_>) -> Result<i64> {
    let minute: i64 = caps[1].parse()?;
    let second: i64 = caps[2].parse()?;
    let fraction = caps.get(3).map(|x| x.as_str()).unwrap_or("0");
    let ms = match fraction.len() {
        1 => fraction.parse::<i64>()? * 100,
        2 => fraction.parse::<i64>()? * 10,
        _ => fraction.parse::<i64>()?,
    };
    Ok(minute * 60_000 + second * 1_000 + ms)
}

pub fn parse_timed(text: &str) -> Result<Vec<Entry>> {
    let re = Regex::new(
        r"(?m)(\d{1,2}):(\d{2}):(\d{2})[,.](\d{3})\s*-->\s*(\d{1,2}):(\d{2}):(\d{2})[,.](\d{3})[^\n]*\n([^\n]*(?:\n(?!\s*$|\d+\s*$|\d{1,2}:\d{2}:\d{2})[^\n]*)*)",
    )?;
    let mut out = Vec::new();
    for c in re.captures_iter(text) {
        let start = clock(&c[1], &c[2], &c[3], &c[4]);
        let end = clock(&c[5], &c[6], &c[7], &c[8]);
        let body = c[9]
            .replace("<i>", "")
            .replace("</i>", "")
            .replace("<b>", "")
            .replace("</b>", "")
            .trim()
            .to_string();
        if !body.is_empty() {
            out.push(Entry {
                start,
                end,
                text: body,
                words: Vec::new(),
            });
        }
    }
    Ok(out)
}
fn clock(h: &str, m: &str, s: &str, ms: &str) -> i64 {
    h.parse::<i64>().unwrap_or(0) * 3_600_000
        + m.parse::<i64>().unwrap_or(0) * 60_000
        + s.parse::<i64>().unwrap_or(0) * 1_000
        + ms.parse::<i64>().unwrap_or(0)
}
fn parse_plain(text: &str) -> Vec<Entry> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| Entry {
            start: i as i64 * 3000,
            end: i as i64 * 3000 + 2500,
            text: l.trim().to_string(),
            words: Vec::new(),
        })
        .collect()
}
fn parse_krc(text: &str) -> Result<Vec<Entry>> {
    let re = Regex::new(r"\[(\d+),(\d+)\](.*)")?;
    Ok(re
        .captures_iter(text)
        .map(|c| Entry {
            start: c[1].parse().unwrap_or(0),
            end: c[1].parse::<i64>().unwrap_or(0) + c[2].parse::<i64>().unwrap_or(0),
            text: c[3].to_string(),
            words: Vec::new(),
        })
        .collect())
}
fn parse_ksc(text: &str) -> Result<Vec<Entry>> {
    let re = Regex::new(r"(?m)^\[(\d+)\](.*)$")?;
    Ok(re
        .captures_iter(text)
        .map(|c| Entry {
            start: c[1].parse().unwrap_or(0),
            end: c[1].parse::<i64>().unwrap_or(0) + 2000,
            text: c[2].trim().to_string(),
            words: Vec::new(),
        })
        .collect())
}

pub fn export_lrc(entries: &[Entry]) -> String {
    entries
        .iter()
        .map(|e| {
            format!(
                "[{:02}:{:05.2}]{}\n",
                e.start / 60000,
                (e.start % 60000) as f64 / 1000.0,
                e.text
            )
        })
        .collect()
}
pub fn export_srt(entries: &[Entry]) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                srt_time(e.start),
                srt_time(e.end),
                e.text
            )
        })
        .collect()
}
fn srt_time(ms: i64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}
pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(|x| x.chars().filter(|c| c.is_alphanumeric()).collect())
        .filter(|x: &String| !x.is_empty())
        .collect()
}

pub fn time_entries(entries: &mut [Entry], recognition: &[Word]) {
    for entry in entries {
        if !entry.words.is_empty() || recognition.is_empty() {
            continue;
        }
        let duration = (entry.end - entry.start).max(0);
        let parts: Vec<_> = entry.text.split_whitespace().collect();
        let step = if parts.is_empty() {
            duration
        } else {
            duration / parts.len() as i64
        };
        entry.words = parts
            .iter()
            .enumerate()
            .map(|(i, w)| Word {
                start: entry.start + i as i64 * step,
                end: entry.start + (i as i64 + 1) * step,
                text: (if i + 1 < parts.len() {
                    format!("{} ", w)
                } else {
                    w.to_string()
                }),
            })
            .collect();
    }
}

#[allow(dead_code)]
fn _validate() -> Result<()> {
    Err(anyhow!("unsupported lyric format"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_uses_next_timestamp_as_line_end() {
        let entries = parse_lrc("[00:01.00]First\n[00:05.50]Second\n").unwrap();
        assert_eq!(
            (entries[0].start, entries[0].end, entries[0].text.as_str()),
            (1000, 5500, "First")
        );
        assert_eq!(entries[1].start, 5500);
    }

    #[test]
    fn enhanced_lrc_keeps_word_marks() {
        let entries = parse_lrc("[00:12.00]<00:12.00>Hello <00:12.80>world<00:13.50>\n").unwrap();
        assert_eq!(entries[0].text, "Hello world");
        assert_eq!(entries[0].words[0].text, "Hello ");
        assert_eq!(
            (entries[0].words[0].start, entries[0].words[1].start),
            (12000, 12800)
        );
    }

    #[test]
    fn tokenization_ignores_case_and_punctuation() {
        assert_eq!(
            tokenize("Amazing grace, how sweet!"),
            tokenize("amazing GRACE how sweet")
        );
    }
}
