//! Romanized sing-along lines for lyrics written in non-Latin scripts.
//!
//! A lyric line is split into pieces (a Chinese character, a Japanese word, a
//! Korean syllable, ...). Each piece keeps its position in the original text, so
//! the romanized row can be highlighted in step with the original row.

use std::collections::BTreeSet;
use std::sync::Mutex;

use crate::dicts::{JYUTPING, KAKASI, PINYIN, SIMPLIFIED};
use crate::readings;
use crate::subtitle::Word;
use crate::text::{halfwidth, is_han, is_hangul, is_kana, is_word_char, non_latin};
use crate::toml::Item;

pub const INDIC: &[&str] = &["ta", "hi", "bn", "gu", "kn", "ml", "mr", "ne", "pa", "sa", "te", "or"];

/// A piece of a line: start and end (character offsets) and its romanized text.
pub type Piece = (usize, usize, String);

/// Languages whose dictionary was needed but is not installed.
static MISSING: Mutex<Option<BTreeSet<String>>> = Mutex::new(None);

pub fn take_missing() -> Vec<String> {
    MISSING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take().unwrap_or_default().into_iter().collect()
}

fn missing(language: &str) {
    MISSING.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get_or_insert_with(BTreeSet::new).insert(language.to_string());
}

/// Which reading dictionaries are installed, for `check`.
pub fn dictionaries_status() -> String {
    let have = [("zh", PINYIN.get().is_some()), ("yue", JYUTPING.get().is_some()), ("ja", KAKASI.get().is_some())];
    let present: Vec<&str> = std::iter::once("ko").chain(have.iter().filter(|item| item.1).map(|item| item.0)).chain(["indic", "other scripts"]).collect();
    let absent: Vec<&str> = have.iter().filter(|item| !item.1).map(|item| item.0).collect();
    if absent.is_empty() {
        present.join(", ")
    } else {
        format!("{}; missing dictionaries for {} (reinstall PolyKara)", present.join(", "), absent.join(", "))
    }
}

/// Traditional Chinese written as Simplified (for comparing lyric files); unchanged without the table.
pub fn simplified(text: &str) -> String {
    match SIMPLIFIED.get() {
        Some(map) => text.chars().map(|c| map.get(&c).copied().unwrap_or(c)).collect(),
        None => text.to_string(),
    }
}

pub fn base_language(language: &str) -> String {
    let tag = language.to_lowercase().replace('_', "-");
    let tag = tag.trim();
    // Hong Kong Chinese is sung in Cantonese; other zh-* tags are Mandarin.
    if tag == "zh-hk" || tag == "zh-yue" {
        "yue".into()
    } else {
        tag.split('-').next().unwrap_or("").to_string()
    }
}

/// Pick the romanizer from the script of the line, using the song language to read Chinese characters.
pub fn engine_for(text: &str, language: &str) -> Option<String> {
    if !non_latin(text) {
        return None;
    }
    if text.chars().any(|c| is_kana(c) || matches!(c, '\u{31F0}'..='\u{31FF}')) {
        return Some("ja".into());
    }
    if text.chars().any(is_hangul) {
        return Some("ko".into());
    }
    if text.chars().any(is_han) {
        return Some(if language == "yue" || language == "ja" { language.to_string() } else { "zh".into() });
    }
    Some(if INDIC.contains(&language) { language.to_string() } else { "other".into() })
}

pub fn enabled(settings: &Item, language: &str, engine: &str) -> bool {
    if !settings.bool_or("enabled", true) {
        return false;
    }
    match settings.get("languages") {
        Item::Str(text) => text.eq_ignore_ascii_case("auto"),
        Item::Array(items) => {
            let wanted: Vec<String> = items.iter().map(|item| item.text().to_lowercase()).collect();
            wanted.iter().any(|item| item == language || item == engine || item == "auto")
        }
        // Missing: the default is every language.
        _ => !settings.has("languages"),
    }
}

/// (engine, romanized pieces covering the line), or None when the line needs no romanization.
pub fn pieces(text: &str, language: &str, settings: &Item) -> Option<(String, Vec<Piece>)> {
    let language = base_language(language);
    let engine = engine_for(text, &language)?;
    if !enabled(settings, &language, &engine) {
        return None;
    }
    let mut raw = if INDIC.contains(&engine.as_str()) { indic(text, &engine) } else { engine_pairs(&engine, text, settings)? };
    if readings::LANGUAGES.contains(&engine.as_str()) {
        raw = readings::apply(text, &engine, raw, settings);
    }
    let items = locate(text, &raw);
    if items.is_empty() {
        None
    } else {
        Some((engine, items))
    }
}

/// (original, romanized) pairs from one engine; None when its dictionary is missing.
pub fn engine_pairs(engine: &str, text: &str, settings: &Item) -> Option<Vec<(String, String)>> {
    match engine {
        "zh" => mandarin(text, settings),
        "yue" => cantonese(text),
        "ja" => japanese(text),
        "ko" => Some(korean(text)),
        _ => Some(other(text)),
    }
}

/// Turn (original, romanized) pairs into offsets, merging punctuation into the piece before it.
fn locate(text: &str, raw: &[(String, String)]) -> Vec<Piece> {
    let chars: Vec<char> = text.chars().collect();
    let mut result: Vec<Piece> = Vec::new();
    let mut cursor = 0;
    for (original, roman) in raw {
        let needle: Vec<char> = original.chars().collect();
        let start = if needle.is_empty() { None } else { (cursor..=chars.len().saturating_sub(needle.len())).find(|at| chars[*at..].starts_with(&needle)) };
        let start = match start {
            Some(start) => start,
            None => {
                // The engine changed the text; show the line without per-piece timing.
                let joined = raw.iter().map(|item| item.1.trim()).filter(|item| !item.is_empty()).collect::<Vec<_>>().join(" ");
                return vec![(0, chars.len(), joined)];
            }
        };
        cursor = start + needle.len();
        // Text the engine passed through unchanged (Latin words, spaces) is split into words.
        let parts: Vec<Piece> = if roman == original {
            let mut parts = Vec::new();
            let mut index = 0;
            while index < needle.len() {
                if needle[index].is_whitespace() {
                    index += 1;
                    continue;
                }
                let begin = index;
                while index < needle.len() && !needle[index].is_whitespace() {
                    index += 1;
                }
                parts.push((start + begin, start + index, needle[begin..index].iter().collect()));
            }
            parts
        } else {
            vec![(start, cursor, roman.trim().to_string())]
        };
        for (begin, end, value) in parts {
            if value.is_empty() {
                continue;
            }
            match result.last_mut() {
                // Full-width marks such as ，and ！ read better as their ASCII forms here.
                Some(last) if !value.chars().any(is_word_char) => {
                    last.1 = end;
                    last.2.push_str(&halfwidth(&value));
                }
                _ => result.push((begin, end, value)),
            }
        }
    }
    result
}

/// Romanized text of each piece with the space that should follow it.
fn spaced(text: &str, items: &[Piece], engine: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    items
        .iter()
        .enumerate()
        .map(|(index, (_, end, roman))| {
            let following = items.get(index + 1).map(|item| item.0);
            // Korean syllables of one word are written together; other scripts get a space per piece.
            let space = match following {
                Some(next) => engine != "ko" || (*end < next && chars[*end..next].iter().all(|c| c.is_whitespace())),
                None => false,
            };
            format!("{roman}{}", if space { " " } else { "" })
        })
        .collect()
}

/// The whole line romanized, for lines without word timing.
pub fn romanize(text: &str, language: &str, settings: &Item) -> Option<String> {
    let (engine, items) = pieces(text, language, settings)?;
    Some(spaced(text, &items, &engine).concat().trim().to_string())
}

/// Romanized pieces timed from the original words they cover, so both rows highlight together.
pub fn romanize_words(words: &[Word], language: &str, settings: &Item) -> Option<Vec<Word>> {
    let text: String = words.iter().map(|word| word.2.as_str()).collect();
    let (engine, items) = pieces(&text, language, settings)?;
    let chars: Vec<char> = text.chars().collect();
    let mut owner = Vec::new();
    for (index, word) in words.iter().enumerate() {
        owner.extend(std::iter::repeat(index).take(word.2.chars().count()));
    }
    let mut timed: Vec<Word> = Vec::new();
    for ((start, end, _), label) in items.iter().zip(spaced(&text, &items, &engine)) {
        let covered: Vec<usize> = (*start..*end).filter(|offset| !chars[*offset].is_whitespace()).map(|offset| owner[offset]).collect();
        if let (Some(first), Some(last)) = (covered.first(), covered.last()) {
            timed.push((words[*first].0, words[*last].1, label));
        }
    }
    if let Some(last) = timed.last_mut() {
        last.2 = last.2.trim_end().to_string();
    }
    (!timed.is_empty()).then(|| timed)
}

/// A group of timed words under the romanized piece that reads them.
#[derive(Clone, Debug, PartialEq)]
pub struct RubyGroup {
    pub start: usize,
    /// None for text such as English words that needs no reading above it.
    pub roman: Option<String>,
    pub tokens: Vec<Word>,
    pub space_before: bool,
}

pub fn ruby_groups(words: &[Word], language: &str, settings: &Item) -> Option<Vec<RubyGroup>> {
    let text: String = words.iter().map(|word| word.2.as_str()).collect();
    let chars: Vec<char> = text.chars().collect();
    let (_, items) = pieces(&text, language, settings)?;
    let mut groups: Vec<RubyGroup> = items
        .iter()
        .enumerate()
        .map(|(index, (start, end, roman))| {
            let original: String = chars[*start..*end].iter().collect();
            let space_before = index > 0 && *start > items[index - 1].1 && chars[items[index - 1].1..*start].iter().all(|c| c.is_whitespace());
            RubyGroup { start: *start, roman: if original == *roman { None } else { Some(roman.clone()) }, tokens: Vec::new(), space_before }
        })
        .collect();
    let mut offset = 0;
    for (left, right, word) in words {
        let first = offset + word.chars().count() - word.trim_start().chars().count();
        offset += word.chars().count();
        let owner = groups.iter().enumerate().filter(|(_, group)| group.start <= first).max_by_key(|(_, group)| group.start).map(|(index, _)| index).unwrap_or(0);
        groups[owner].tokens.push((*left, *right, word.trim().to_string()));
    }
    groups.retain(|group| !group.tokens.is_empty());
    (!groups.is_empty()).then(|| groups)
}

/// The tone style from [romanization] tone: "marks", "numbers" or "none".
pub fn tone_style(settings: &Item) -> &'static str {
    match settings.str_or("tone", "marks").to_lowercase().as_str() {
        "numbers" | "number" | "tone3" => "numbers",
        "none" | "plain" | "normal" => "none",
        _ => "marks",
    }
}

/// Mandarin: phrases first (longest match), then each character's usual reading.
fn mandarin(text: &str, settings: &Item) -> Option<Vec<(String, String)>> {
    let pinyin = match PINYIN.get() {
        Some(pinyin) => pinyin,
        None => {
            missing("zh");
            return None;
        }
    };
    let style = tone_style(settings);
    let chars: Vec<char> = text.chars().collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !is_han(chars[index]) {
            let start = index;
            while index < chars.len() && !is_han(chars[index]) {
                index += 1;
            }
            let run: String = chars[start..index].iter().collect();
            result.push((run.clone(), run));
            continue;
        }
        let mut run_end = index;
        while run_end < chars.len() && is_han(chars[run_end]) {
            run_end += 1;
        }
        let mut found: Option<(usize, &Vec<String>)> = None;
        for size in (2..=pinyin.longest.min(run_end - index)).rev() {
            let word: String = chars[index..index + size].iter().collect();
            if let Some(readings) = pinyin.phrases.get(&word) {
                found = Some((size, readings));
                break;
            }
        }
        match found {
            Some((size, readings)) => {
                for (offset, reading) in readings.iter().enumerate() {
                    result.push((chars[index + offset].to_string(), readings::pinyin_style(reading, style)));
                }
                index += size;
            }
            None => {
                let c = chars[index];
                let reading = pinyin.chars.get(&c).map(|reading| readings::pinyin_style(reading, style)).unwrap_or_else(|| c.to_string());
                result.push((c.to_string(), reading));
                index += 1;
            }
        }
    }
    Some(result)
}

fn cantonese(text: &str) -> Option<Vec<(String, String)>> {
    let trie = match JYUTPING.get() {
        Some(trie) => trie,
        None => {
            missing("yue");
            return None;
        }
    };
    let mut result: Vec<(String, String)> = Vec::new();
    for (c, reading) in trie.readings(text) {
        match reading {
            Some(reading) => result.push((c.to_string(), reading)),
            None => match result.last_mut() {
                Some(last) if last.0 == last.1 => {
                    last.0.push(c);
                    last.1.push(c);
                }
                _ => result.push((c.to_string(), c.to_string())),
            },
        }
    }
    Some(result)
}

fn japanese(text: &str) -> Option<Vec<(String, String)>> {
    let kakasi = match KAKASI.get() {
        Some(kakasi) => kakasi,
        None => {
            missing("ja");
            return None;
        }
    };
    let mut result: Vec<(String, String)> = Vec::new();
    for (original, roman) in crate::kakasi::convert(&kakasi, text) {
        let roman = if roman.is_empty() { original.clone() } else { roman };
        // Particles are written は/へ/を but sung wa/e/o.
        let roman = match original.as_str() {
            "は" if !result.is_empty() => "wa".to_string(),
            "へ" if !result.is_empty() => "e".to_string(),
            "を" if !result.is_empty() => "o".to_string(),
            _ => roman,
        };
        result.push((original, roman));
    }
    Some(result)
}

/// Kana in romaji (for word library readings written in kana).
pub fn kana_to_romaji(reading: &str) -> String {
    if reading.chars().all(|c| c.is_ascii_alphabetic() || matches!(c, '\'' | ' ' | '-')) {
        return reading.to_string();
    }
    match KAKASI.get() {
        Some(kakasi) => crate::kakasi::kana_to_romaji(&kakasi, reading),
        None => reading.to_string(),
    }
}

// Revised Romanization of Korean, per syllable.
const INITIALS: [&str; 19] = ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
const VOWELS: [&str; 21] = ["a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i"];
const FINALS: [&str; 28] = ["", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p", "t", "t", "ng", "t", "t", "k", "t", "p", "t"];

/// A final consonant followed by a silent ㅇ is sung at the start of the next syllable.
fn carried(final_index: u32) -> Option<&'static str> {
    Some(match final_index {
        1 => "g",
        2 => "kk",
        4 => "n",
        7 => "d",
        8 => "r",
        16 => "m",
        17 => "b",
        19 => "s",
        20 => "ss",
        22 => "j",
        23 => "ch",
        24 => "k",
        25 => "t",
        26 => "p",
        27 => "",
        _ => return None,
    })
}

fn korean(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut result: Vec<(String, String)> = Vec::new();
    let mut carry: Option<&str> = None;
    for (index, c) in chars.iter().enumerate() {
        if !is_hangul(*c) {
            carry = None;
            match result.last_mut() {
                Some(last) if last.0 == last.1 && !last.0.chars().last().map_or(false, is_hangul) => {
                    last.0.push(*c);
                    last.1.push(*c);
                }
                _ => result.push((c.to_string(), c.to_string())),
            }
            continue;
        }
        let code = *c as u32 - 0xAC00;
        let (initial, vowel, final_index) = (code / 588, code % 588 / 28, code % 28);
        let start = carry.unwrap_or(INITIALS[initial as usize]);
        let silent_next = chars.get(index + 1).map_or(false, |next| is_hangul(*next) && (*next as u32 - 0xAC00) / 588 == 11);
        let ending = match carried(final_index) {
            // 사랑이 → sa-rang-i but 먹어 → meo-geo: the final moves to the next syllable.
            Some(moved) if silent_next => {
                carry = Some(moved);
                ""
            }
            _ => {
                carry = None;
                FINALS[final_index as usize]
            }
        };
        result.push((c.to_string(), format!("{start}{}{ending}", VOWELS[vowel as usize])));
    }
    result
}

/// The ITRANS-style letters of a character in an Indic block, by its offset in the block.
fn indic_letter(offset: u32) -> Option<(&'static str, char)> {
    // ('v' vowel, 'c' consonant, 'm' vowel sign, 'x' virama, 's' sign or other)
    Some(match offset {
        0x01 => (".N", 's'),
        0x02 => ("M", 's'),
        0x03 => ("H", 's'),
        0x05 => ("a", 'v'),
        0x06 => ("A", 'v'),
        0x07 => ("i", 'v'),
        0x08 => ("I", 'v'),
        0x09 => ("u", 'v'),
        0x0A => ("U", 'v'),
        0x0B => ("RRi", 'v'),
        0x0C => ("LLi", 'v'),
        0x0D | 0x0E | 0x0F => ("e", 'v'),
        0x10 => ("ai", 'v'),
        0x11 | 0x12 | 0x13 => ("o", 'v'),
        0x14 => ("au", 'v'),
        0x15 => ("k", 'c'),
        0x16 => ("kh", 'c'),
        0x17 => ("g", 'c'),
        0x18 => ("gh", 'c'),
        0x19 => ("~N", 'c'),
        0x1A => ("ch", 'c'),
        0x1B => ("Ch", 'c'),
        0x1C => ("j", 'c'),
        0x1D => ("jh", 'c'),
        0x1E => ("~n", 'c'),
        0x1F => ("T", 'c'),
        0x20 => ("Th", 'c'),
        0x21 => ("D", 'c'),
        0x22 => ("Dh", 'c'),
        0x23 => ("N", 'c'),
        0x24 => ("t", 'c'),
        0x25 => ("th", 'c'),
        0x26 => ("d", 'c'),
        0x27 => ("dh", 'c'),
        0x28 | 0x29 => ("n", 'c'),
        0x2A => ("p", 'c'),
        0x2B => ("ph", 'c'),
        0x2C => ("b", 'c'),
        0x2D => ("bh", 'c'),
        0x2E => ("m", 'c'),
        0x2F => ("y", 'c'),
        0x30 => ("r", 'c'),
        0x31 => ("R", 'c'),
        0x32 => ("l", 'c'),
        0x33 => ("L", 'c'),
        0x34 => ("zh", 'c'),
        0x35 => ("v", 'c'),
        0x36 => ("sh", 'c'),
        0x37 => ("Sh", 'c'),
        0x38 => ("s", 'c'),
        0x39 => ("h", 'c'),
        0x3C => ("", 'n'),
        0x3D => (".a", 's'),
        0x3E => ("A", 'm'),
        0x3F => ("i", 'm'),
        0x40 => ("I", 'm'),
        0x41 => ("u", 'm'),
        0x42 => ("U", 'm'),
        0x43 => ("RRi", 'm'),
        0x44 => ("RRI", 'm'),
        0x45 | 0x46 | 0x47 => ("e", 'm'),
        0x48 => ("ai", 'm'),
        0x49 | 0x4A | 0x4B => ("o", 'm'),
        0x4C => ("au", 'm'),
        0x4D => ("", 'x'),
        0x50 => ("OM", 's'),
        0x57 => ("au", 'm'),
        0x58 => ("q", 'c'),
        0x59 => ("K", 'c'),
        0x5A => ("G", 'c'),
        0x5B => ("z", 'c'),
        0x5C => (".D", 'c'),
        0x5D => (".Dh", 'c'),
        0x5E => ("f", 'c'),
        0x5F => ("Y", 'c'),
        0x60 => ("RRI", 'v'),
        0x61 => ("LLI", 'v'),
        0x62 => ("LLi", 'm'),
        0x63 => ("LLI", 'm'),
        0x64 => (".", 's'),
        0x65 => ("..", 's'),
        0x66 => ("0", 's'),
        0x67 => ("1", 's'),
        0x68 => ("2", 's'),
        0x69 => ("3", 's'),
        0x6A => ("4", 's'),
        0x6B => ("5", 's'),
        0x6C => ("6", 's'),
        0x6D => ("7", 's'),
        0x6E => ("8", 's'),
        0x6F => ("9", 's'),
        0x70 => ("M", 's'),
        0x71 => ("", 's'),
        0x7A => ("N", 'c'),
        0x7B => ("n", 'c'),
        0x7C => ("r", 'c'),
        0x7D => ("l", 'c'),
        0x7E => ("L", 'c'),
        0x7F => ("k", 'c'),
        _ => return None,
    })
}

fn indic_block(language: &str) -> u32 {
    match language {
        "bn" => 0x0980,
        "pa" => 0x0A00,
        "gu" => 0x0A80,
        "or" => 0x0B00,
        "ta" => 0x0B80,
        "te" => 0x0C00,
        "kn" => 0x0C80,
        "ml" => 0x0D00,
        _ => 0x0900,
    }
}

/// An Indic word in ITRANS letters: consonants carry the vowel a unless a vowel sign or virama follows.
pub fn indic_word(word: &str, language: &str) -> String {
    let base = indic_block(language);
    let mut out = String::new();
    let mut pending_a = false;
    for c in word.chars() {
        let code = c as u32;
        // Every Indic block has the same layout, so any of them is read by its offset.
        let letter = if (0x0900..0x0D80).contains(&code) { indic_letter((code - 0x0900) % 0x80) } else { None };
        let _ = base;
        match letter {
            Some((text, kind)) => match kind {
                'm' => {
                    out.push_str(text);
                    pending_a = false;
                }
                'x' => pending_a = false,
                'n' => {}
                _ => {
                    if pending_a {
                        out.push('a');
                    }
                    out.push_str(text);
                    pending_a = kind == 'c';
                }
            },
            None => {
                if pending_a {
                    out.push('a');
                    pending_a = false;
                }
                out.push(c);
            }
        }
    }
    if pending_a {
        out.push('a');
    }
    out
}

fn indic(text: &str, language: &str) -> Vec<(String, String)> {
    text.split_whitespace().map(|word| (word.to_string(), indic_word(word, language))).collect()
}

/// Letters of other scripts (Cyrillic, Greek, Hebrew, Arabic, Thai) in Latin letters.
fn other_letter(c: char) -> Option<&'static str> {
    let lower = c.to_lowercase().next().unwrap_or(c);
    Some(match lower {
        'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'ґ' => "g", 'д' => "d", 'е' => "e", 'ё' => "yo", 'є' => "ye", 'ж' => "zh", 'з' => "z", 'и' => "i", 'і' => "i", 'ї' => "yi", 'й' => "y",
        'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ў' => "w", 'ф' => "f", 'х' => "kh", 'ц' => "ts", 'ч' => "ch",
        'ш' => "sh", 'щ' => "shch", 'ъ' => "", 'ы' => "y", 'ь' => "", 'э' => "e", 'ю' => "yu", 'я' => "ya", 'ђ' => "dj", 'ј' => "j", 'љ' => "lj", 'њ' => "nj", 'ћ' => "c", 'џ' => "dz",
        'α' | 'ά' => "a", 'β' => "v", 'γ' => "g", 'δ' => "d", 'ε' | 'έ' => "e", 'ζ' => "z", 'η' | 'ή' => "i", 'θ' => "th", 'ι' | 'ί' | 'ϊ' | 'ΐ' => "i", 'κ' => "k", 'λ' => "l", 'μ' => "m",
        'ν' => "n", 'ξ' => "x", 'ο' | 'ό' => "o", 'π' => "p", 'ρ' => "r", 'σ' | 'ς' => "s", 'τ' => "t", 'υ' | 'ύ' | 'ϋ' | 'ΰ' => "y", 'φ' => "f", 'χ' => "ch", 'ψ' => "ps", 'ω' | 'ώ' => "o",
        'א' => "'", 'ב' => "b", 'ג' => "g", 'ד' => "d", 'ה' => "h", 'ו' => "v", 'ז' => "z", 'ח' => "ch", 'ט' => "t", 'י' => "y", 'כ' | 'ך' => "k", 'ל' => "l", 'מ' | 'ם' => "m", 'נ' | 'ן' => "n",
        'ס' => "s", 'ע' => "'", 'פ' | 'ף' => "p", 'צ' | 'ץ' => "ts", 'ק' => "k", 'ר' => "r", 'ש' => "sh", 'ת' => "t",
        'ا' | 'أ' | 'آ' | 'ى' => "a", 'إ' => "i", 'ب' => "b", 'ت' => "t", 'ث' => "th", 'ج' => "j", 'ح' => "h", 'خ' => "kh", 'د' => "d", 'ذ' => "dh", 'ر' => "r", 'ز' => "z", 'س' => "s",
        'ش' => "sh", 'ص' => "s", 'ض' => "d", 'ط' => "t", 'ظ' => "z", 'ع' => "'", 'غ' => "gh", 'ف' => "f", 'ق' => "q", 'ك' => "k", 'ل' => "l", 'م' => "m", 'ن' => "n", 'ه' => "h", 'ة' => "h",
        'و' => "w", 'ي' => "y", 'ء' | 'ؤ' | 'ئ' => "'", 'پ' => "p", 'چ' => "ch", 'ژ' => "zh", 'گ' => "g", 'ک' => "k", 'ی' => "y",
        'ก' => "k", 'ข' | 'ฃ' | 'ค' | 'ฅ' | 'ฆ' => "kh", 'ง' => "ng", 'จ' | 'ฉ' | 'ช' | 'ฌ' => "ch", 'ซ' | 'ศ' | 'ษ' | 'ส' => "s", 'ญ' | 'ย' => "y", 'ฎ' | 'ด' => "d", 'ฏ' | 'ต' => "t",
        'ฐ' | 'ฑ' | 'ฒ' | 'ถ' | 'ท' | 'ธ' => "th", 'ณ' | 'น' => "n", 'บ' => "b", 'ป' => "p", 'ผ' | 'พ' | 'ภ' => "ph", 'ฝ' | 'ฟ' => "f", 'ม' => "m", 'ร' => "r", 'ล' | 'ฬ' => "l", 'ว' => "w",
        'ห' | 'ฮ' => "h", 'อ' => "", 'ะ' | 'ั' | 'า' => "a", 'ำ' => "am", 'ิ' | 'ี' => "i", 'ึ' | 'ื' => "ue", 'ุ' | 'ู' => "u", 'เ' => "e", 'แ' => "ae", 'โ' => "o", 'ใ' | 'ไ' => "ai",
        '็' | '่' | '้' | '๊' | '๋' | '์' | 'ๆ' | 'ฯ' => "",
        _ => return None,
    })
}

/// A word of another script in Latin letters, keeping capitals.
pub fn other_word(word: &str) -> String {
    let mut out = String::new();
    for c in word.chars() {
        match other_letter(c) {
            Some(letters) if c.is_uppercase() && !letters.is_empty() => {
                let mut chars = letters.chars();
                if let Some(first) = chars.next() {
                    out.extend(first.to_uppercase());
                    out.extend(chars);
                }
            }
            Some(letters) => out.push_str(letters),
            None => out.push(c),
        }
    }
    out
}

fn other(text: &str) -> Vec<(String, String)> {
    text.split_whitespace().map(|word| (word.to_string(), other_word(word))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(tone: &str) -> Item {
        crate::toml::parse(&format!("enabled = true\nlanguages = \"auto\"\ntone = \"{tone}\"\n")).unwrap()
    }

    #[test]
    fn korean_syllables_carry_their_final_consonant() {
        let pairs = korean("사랑이 먹어");
        let romans: Vec<&str> = pairs.iter().map(|pair| pair.1.as_str()).collect();
        assert_eq!(romans, vec!["sa", "rang", "i", " ", "meo", "geo"]);
        assert_eq!(romanize("사랑이 먹어", "ko", &settings("none")).unwrap(), "sarangi meogeo");
    }

    #[test]
    fn scripts_choose_their_engine() {
        assert_eq!(engine_for("Hello", "en"), None);
        assert_eq!(engine_for("你好", "zh-HK").as_deref(), Some("zh"));
        assert_eq!(engine_for("你好", "yue").as_deref(), Some("yue"));
        assert_eq!(engine_for("こんにちは世界", "zh").as_deref(), Some("ja"));
        assert_eq!(engine_for("नमस्ते", "hi").as_deref(), Some("hi"));
        assert_eq!(engine_for("Привет", "ru").as_deref(), Some("other"));
        assert_eq!(base_language("zh_HK"), "yue");
        assert_eq!(base_language("pt-BR"), "pt");
    }

    #[test]
    fn indic_and_other_scripts() {
        assert_eq!(indic_word("नमस्ते", "hi"), "namaste");
        assert_eq!(indic_word("வணக்கம்", "ta"), "vaNakkam");
        assert_eq!(other_word("Привет"), "Privet");
        assert_eq!(other_word("καλημέρα"), "kalimera");
        assert_eq!(romanize("Привет мир", "ru", &settings("none")).unwrap(), "Privet mir");
    }

    #[test]
    fn punctuation_joins_the_piece_before() {
        let raw = vec![("你".to_string(), "ni".to_string()), ("，".to_string(), "，".to_string()), ("好".to_string(), "hao".to_string())];
        assert_eq!(locate("你，好", &raw), vec![(0, 2, "ni,".to_string()), (2, 3, "hao".to_string())]);
        let groups_words: Vec<Word> = vec![(0, 10, "Hi ".into()), (10, 20, "你".into())];
        let raw_groups = locate("Hi 你", &[("Hi ".to_string(), "Hi ".to_string()), ("你".to_string(), "ni".to_string())]);
        assert_eq!(raw_groups, vec![(0, 2, "Hi".to_string()), (3, 4, "ni".to_string())]);
        let _ = groups_words;
    }

    #[test]
    fn settings_choose_languages() {
        let list = crate::toml::parse("languages = [\"zh\", \"ja\"]\n").unwrap();
        assert!(enabled(&list, "zh", "zh"));
        assert!(!enabled(&list, "ko", "ko"));
        let off = crate::toml::parse("enabled = false\n").unwrap();
        assert!(!enabled(&off, "zh", "zh"));
    }
}
