//! Readers for word-timed karaoke lyric formats (plain-text forms only).
//!
//! KSC  karaoke.add('mm:ss.xxx', 'mm:ss.xxx', 'text', 'd1,d2,...');
//! KRC  [line_start,line_dur]<offset,dur,0>word<offset,dur,0>word...
//! QRC  [line_start,line_dur]word(start,dur)word(start,dur)...  (optionally inside XML LyricContent="...")

use std::path::Path;

use crate::providers::unescape_html;
use crate::subtitle::{Entry, Word};
use crate::text::is_cjk_wide;

#[cfg(windows)]
fn decode_gb18030(data: &[u8]) -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn MultiByteToWideChar(code_page: u32, flags: u32, text: *const u8, length: i32, wide: *mut u16, wide_length: i32) -> i32;
    }
    const GB18030: u32 = 54936;
    const MB_ERR_INVALID_CHARS: u32 = 8;
    if data.is_empty() {
        return Some(String::new());
    }
    let length = unsafe { MultiByteToWideChar(GB18030, MB_ERR_INVALID_CHARS, data.as_ptr(), data.len() as i32, std::ptr::null_mut(), 0) };
    if length <= 0 {
        return None;
    }
    let mut wide = vec![0u16; length as usize];
    unsafe { MultiByteToWideChar(GB18030, MB_ERR_INVALID_CHARS, data.as_ptr(), data.len() as i32, wide.as_mut_ptr(), length) };
    String::from_utf16(&wide).ok()
}

#[cfg(not(windows))]
fn decode_gb18030(_data: &[u8]) -> Option<String> {
    None
}

/// A lyric file's text: UTF-16 with a byte-order mark, UTF-8, or GB18030.
pub fn read_text(path: &Path) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|error| format!("{} could not be read ({error})", path.display()))?;
    if data.starts_with(&[0xFF, 0xFE]) || data.starts_with(&[0xFE, 0xFF]) {
        let little = data[0] == 0xFF;
        let units: Vec<u16> = data[2..].chunks_exact(2).map(|pair| if little { u16::from_le_bytes([pair[0], pair[1]]) } else { u16::from_be_bytes([pair[0], pair[1]]) }).collect();
        return String::from_utf16(&units).map_err(|_| format!("{}: broken UTF-16 text", path.display()));
    }
    if data[..data.len().min(4096)].contains(&0) {
        return Err(format!("{} is a binary or encrypted lyric file; export it as plain text first", path.display()));
    }
    if let Ok(text) = std::str::from_utf8(&data) {
        return Ok(text.trim_start_matches('\u{feff}').to_string());
    }
    decode_gb18030(&data).ok_or_else(|| format!("{}: unrecognised text encoding", path.display()))
}

pub fn join_words(words: &[String]) -> String {
    if words.iter().any(|word| word.chars().any(is_cjk_wide)) {
        words.concat()
    } else {
        words.iter().map(|word| word.trim()).collect::<Vec<_>>().join(" ")
    }
}

pub fn entry(words: Vec<Word>, start: Option<i64>, end: Option<i64>) -> Option<Entry> {
    let words: Vec<Word> = words.into_iter().filter(|word| !word.2.trim().is_empty()).map(|(left, right, text)| (left, (left + 10).max(right), text.trim().to_string())).collect();
    let first = words.first()?;
    let start = start.unwrap_or(first.0);
    let end = end.unwrap_or(words[words.len() - 1].1);
    let text = join_words(&words.iter().map(|word| word.2.clone()).collect::<Vec<_>>());
    Some(Entry::new(start, (start + 300).max(end), text, words))
}

/// "mm:ss.xxx" or "h:mm:ss" in ms.
pub fn ksc_time(value: &str) -> Option<i64> {
    let value = value.trim();
    let (clock, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.chars().all(|c| c.is_ascii_digit()) || (value.contains('.') && fraction.is_empty()) {
        return None;
    }
    let parts: Vec<&str> = clock.split(':').collect();
    if !(2..=3).contains(&parts.len()) || parts.iter().any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let numbers: Vec<i64> = parts.iter().map(|part| part.parse().unwrap_or(0)).collect();
    let (hours, minutes, seconds) = if numbers.len() == 3 { (numbers[0], numbers[1], numbers[2]) } else { (0, numbers[0], numbers[1]) };
    let fraction: i64 = format!("{fraction}000")[..3].parse().unwrap_or(0);
    Some(((hours * 60 + minutes) * 60 + seconds) * 1000 + fraction)
}

/// KSC text in timing units: [bracketed] groups, else one CJK character or one Latin word.
pub fn ksc_units(text: &str, count: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut units = Vec::new();
    let mut buffer = String::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c == '[' {
            if let Some(length) = chars[index + 1..].iter().position(|item| *item == ']') {
                // As in the Python reader, a word being collected is not ended by a bracket group.
                units.push(chars[index + 1..index + 1 + length].iter().collect());
                index += length + 2;
                continue;
            }
        }
        if c.is_whitespace() {
            if !buffer.is_empty() {
                units.push(std::mem::take(&mut buffer));
            }
        } else if is_cjk_wide(c) {
            if !buffer.is_empty() {
                units.push(std::mem::take(&mut buffer));
            }
            units.push(c.to_string());
        } else {
            buffer.push(c);
        }
        index += 1;
    }
    if !buffer.is_empty() {
        units.push(buffer);
    }
    if units.len() != count {
        let plain: Vec<String> = text.chars().filter(|c| !c.is_whitespace() && *c != '[' && *c != ']').map(|c| c.to_string()).collect();
        if plain.len() == count {
            return plain;
        }
    }
    units
}

/// The quoted arguments of each karaoke.add('...', ...) call: (start, end, text, durations).
fn ksc_calls(text: &str) -> Vec<(String, String, String, String)> {
    let lower = text.to_lowercase();
    let mut calls = Vec::new();
    let mut search = 0;
    while let Some(found) = lower[search..].find("karaoke.add") {
        let at = search + found + "karaoke.add".len();
        search = at;
        let mut rest = text[at..].trim_start();
        if !rest.starts_with('(') {
            continue;
        }
        rest = &rest[1..];
        let mut arguments = Vec::new();
        let mut ok = true;
        for number in 0..4 {
            rest = rest.trim_start();
            if !rest.starts_with('\'') {
                ok = false;
                break;
            }
            let mut value = String::new();
            let mut chars = rest[1..].char_indices();
            let mut end = None;
            while let Some((offset, c)) = chars.next() {
                if c == '\\' && number == 2 {
                    if let Some((_, escaped)) = chars.next() {
                        value.push('\\');
                        value.push(escaped);
                    }
                } else if c == '\'' {
                    if number == 2 && rest[1 + offset + 1..].starts_with('\'') {
                        chars.next();
                        value.push_str("''");
                    } else {
                        end = Some(1 + offset + 1);
                        break;
                    }
                } else {
                    value.push(c);
                }
            }
            match end {
                Some(end) => {
                    arguments.push(value);
                    rest = rest[end..].trim_start();
                    if number < 3 {
                        if !rest.starts_with(',') {
                            ok = false;
                            break;
                        }
                        rest = &rest[1..];
                    }
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && rest.trim_start().starts_with(')') && arguments.len() == 4 {
            calls.push((arguments[0].clone(), arguments[1].clone(), arguments[2].clone(), arguments[3].clone()));
        }
    }
    calls
}

pub fn parse_ksc(text: &str, name: &str) -> Vec<Entry> {
    let mut result = Vec::new();
    for (start_text, end_text, line, durations_text) in ksc_calls(text) {
        let (start, end) = match (ksc_time(&start_text), ksc_time(&end_text)) {
            (Some(start), Some(end)) => (start, end),
            _ => continue,
        };
        let line = line.replace("''", "'").replace("\\'", "'");
        let mut durations: Vec<i64> = Vec::new();
        let mut number = String::new();
        for c in durations_text.chars().chain(std::iter::once(',')) {
            if c.is_ascii_digit() || (c == '-' && number.is_empty()) {
                number.push(c);
            } else if !number.is_empty() {
                if let Ok(value) = number.parse() {
                    durations.push(value);
                }
                number.clear();
            }
        }
        let units = ksc_units(&line, durations.len());
        if units.len() != durations.len() {
            println!("WARN {name}: {} units vs {} durations at {start_text}; spreading evenly", units.len(), durations.len());
            durations = vec![10.max((end - start) / (units.len().max(1) as i64)); units.len()];
        }
        let mut cursor = start;
        let mut words = Vec::new();
        for (unit, duration) in units.into_iter().zip(durations) {
            words.push((cursor, cursor + duration.max(0), unit));
            cursor += duration.max(0);
        }
        if let Some(item) = entry(words, Some(start), Some(end)) {
            result.push(item);
        }
    }
    result
}

/// "[start,duration]rest" at the start of a line.
fn krc_line(line: &str) -> Option<(i64, i64, &str)> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    let (start, duration) = rest[..close].split_once(',')?;
    Some((start.trim().parse().ok()?, duration.trim().parse().ok()?, &rest[close + 1..]))
}

pub fn parse_krc(text: &str) -> Vec<Entry> {
    let mut result = Vec::new();
    for line in text.lines() {
        let (start, duration, body) = match krc_line(line.trim()) {
            Some(parts) => parts,
            None => continue,
        };
        let mut words = Vec::new();
        let mut rest = body;
        while let Some(open) = rest.find('<') {
            let close = match rest[open..].find('>') {
                Some(close) => open + close,
                None => break,
            };
            let numbers: Vec<i64> = rest[open + 1..close].split(',').filter_map(|part| part.trim().parse().ok()).collect();
            let after = &rest[close + 1..];
            let word_end = after.find('<').unwrap_or(after.len());
            if numbers.len() == 3 {
                words.push((start + numbers[0], start + numbers[0] + numbers[1], after[..word_end].to_string()));
            }
            rest = &after[word_end..];
        }
        if let Some(item) = entry(words, Some(start), Some(start + duration)) {
            result.push(item);
        }
    }
    result
}

pub fn parse_qrc(text: &str) -> Vec<Entry> {
    let mut content = text.to_string();
    if let Some(at) = text.find("LyricContent=\"") {
        let rest = &text[at + "LyricContent=\"".len()..];
        if let Some(end) = rest.find('"') {
            content = unescape_html(&rest[..end]);
        }
    }
    let mut result = Vec::new();
    for line in content.lines() {
        let (start, duration, body) = match krc_line(line.trim()) {
            Some(parts) => parts,
            None => continue,
        };
        let mut words = Vec::new();
        let mut rest = body;
        // word(start,duration) repeated; the word has no parentheses.
        while let Some(open) = rest.find('(') {
            let close = match rest[open..].find(')') {
                Some(close) => open + close,
                None => break,
            };
            let word = &rest[..open];
            let numbers: Vec<&str> = rest[open + 1..close].split(',').collect();
            if numbers.len() == 2 && !word.contains(')') {
                if let (Ok(left), Ok(length)) = (numbers[0].trim().parse::<i64>(), numbers[1].trim().parse::<i64>()) {
                    words.push((left, left + length, word.to_string()));
                }
            }
            rest = &rest[close + 1..];
        }
        if let Some(item) = entry(words, Some(start), Some(start + duration)) {
            result.push(item);
        }
    }
    result
}

/// Read a .ksc, .krc or .qrc file; None for other suffixes.
pub fn read(path: &Path) -> Option<Result<Vec<Entry>, String>> {
    let suffix = path.extension()?.to_string_lossy().to_lowercase();
    let reader: fn(&str, &str) -> Vec<Entry> = match suffix.as_str() {
        "ksc" => parse_ksc,
        "krc" => |text, _| parse_krc(text),
        "qrc" => |text, _| parse_qrc(text),
        _ => return None,
    };
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Some(read_text(path).map(|text| reader(&text, &name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksc_lines() {
        let text = "karaoke := CreateKaraokeObject;\nkaraoke.add('00:12.000', '00:13.000', '你好[Hi]', '300,400,300');\nkaraoke.add('00:14.000', '00:15.000', 'It''s me', '500,500');\n";
        let entries = parse_ksc(text, "t.ksc");
        assert_eq!(entries[0].words, vec![(12000, 12300, "你".to_string()), (12300, 12700, "好".to_string()), (12700, 13000, "Hi".to_string())]);
        assert_eq!(entries[0].text, "你好Hi");
        assert_eq!(entries[1].text, "It's me");
        assert_eq!(ksc_time("1:02:03.5"), Some(3_723_500));
    }

    #[test]
    fn krc_and_qrc_lines() {
        let krc = parse_krc("[ti:x]\n[12000,1000]<0,300,0>你<300,400,0>好\n");
        assert_eq!(krc[0].words, vec![(12000, 12300, "你".to_string()), (12300, 12700, "好".to_string())]);
        assert_eq!((krc[0].start, krc[0].end), (12000, 13000));
        let qrc = parse_qrc("<Lyric_1 LyricContent=\"[12000,1000]Hel(12000,300)lo (12300,400)\"/>");
        assert_eq!(qrc[0].words, vec![(12000, 12300, "Hel".to_string()), (12300, 12700, "lo".to_string())]);
        assert_eq!(join_words(&["Hel".into(), "lo".into()]), "Hel lo");
    }
}
