//! The word library: readings for words the engines get wrong.
//!
//! Built-in lists live in the program's readings folder ({zh,yue,ja}.txt) and the
//! user's own words in <PolyKara folder>/readings/<language>.txt, which win.
//! Chinese words need one syllable per character (重来 chóng lái or chong2 lai2);
//! Japanese words take the whole reading in kana or romaji (君 きみ).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use crate::config::paths;
use crate::romanize::{engine_pairs, kana_to_romaji, tone_style};
use crate::text::is_han;
use crate::toml::Item;

pub const LANGUAGES: &[&str] = &["zh", "yue", "ja"];

pub fn builtin_file(language: &str) -> PathBuf {
    paths().root.join("readings").join(format!("{language}.txt"))
}

pub fn user_file(language: &str) -> PathBuf {
    paths().home.join("readings").join(format!("{language}.txt"))
}

const MARKED: &[(char, char, char)] = &[
    ('ā', 'a', '1'), ('á', 'a', '2'), ('ǎ', 'a', '3'), ('à', 'a', '4'),
    ('ē', 'e', '1'), ('é', 'e', '2'), ('ě', 'e', '3'), ('è', 'e', '4'),
    ('ī', 'i', '1'), ('í', 'i', '2'), ('ǐ', 'i', '3'), ('ì', 'i', '4'),
    ('ō', 'o', '1'), ('ó', 'o', '2'), ('ǒ', 'o', '3'), ('ò', 'o', '4'),
    ('ū', 'u', '1'), ('ú', 'u', '2'), ('ǔ', 'u', '3'), ('ù', 'u', '4'),
    ('ǖ', 'ü', '1'), ('ǘ', 'ü', '2'), ('ǚ', 'ü', '3'), ('ǜ', 'ü', '4'),
    ('ń', 'n', '2'), ('ň', 'n', '3'), ('ǹ', 'n', '4'), ('ḿ', 'm', '2'),
    ('ê', 'ê', '5'),
];

/// Pinyin in tone-number form: chóng → chong2, lǜ → lv4, de → de5.
pub fn numbered(syllable: &str) -> String {
    let value = syllable.trim().to_lowercase().replace("u:", "ü");
    let given = value.chars().last().filter(char::is_ascii_digit);
    let mut tone = given.unwrap_or('5');
    let mut letters = String::new();
    for c in value.trim_end_matches(|c: char| c.is_ascii_digit()).chars() {
        match MARKED.iter().find(|(marked, _, _)| *marked == c) {
            Some((_, base, mark)) => {
                if given.is_none() && *mark != '5' {
                    tone = *mark;
                }
                letters.push(*base);
            }
            None => letters.push(c),
        }
    }
    format!("{}{tone}", letters.replace('ü', "v"))
}

/// chong2 → chóng: the mark goes on a or e, on the o of ou, else on the last vowel.
pub fn marked(syllable: &str) -> String {
    let tone = match syllable.chars().last() {
        Some(tone) => tone,
        None => return String::new(),
    };
    let letters: Vec<char> = syllable[..syllable.len() - tone.len_utf8()].replace('v', "ü").chars().collect();
    if !matches!(tone, '1'..='4') {
        return letters.iter().collect();
    }
    let index = letters
        .iter()
        .position(|c| *c == 'a')
        .or_else(|| letters.iter().position(|c| *c == 'e'))
        .or_else(|| {
            let text: String = letters.iter().collect();
            if text.contains("ou") { letters.iter().position(|c| *c == 'o') } else { letters.iter().rposition(|c| matches!(c, 'i' | 'o' | 'u' | 'ü')) }
        });
    let mut out: Vec<char> = letters.clone();
    if let Some(index) = index {
        if let Some((marked, _, _)) = MARKED.iter().find(|(_, base, mark)| *base == letters[index] && *mark == tone) {
            out[index] = *marked;
        }
    }
    out.into_iter().collect()
}

/// A pinyin syllable (with tone marks, as the dictionary has it) in the given style.
pub fn pinyin_style(syllable: &str, style: &str) -> String {
    let value = numbered(syllable);
    match style {
        "numbers" => value,
        "none" => value.trim_end_matches(|c: char| c.is_ascii_digit()).to_string(),
        _ => {
            let shown = marked(&value);
            // Neutral tone: marks style shows the bare syllable with ü.
            if value.ends_with('5') { value[..value.len() - 1].replace('v', "ü") } else { shown }
        }
    }
}

/// A library syllable in the tone style of [romanization] tone ("marks", "numbers" or "none").
pub fn styled(syllable: &str, language: &str, tone: &str) -> String {
    match language {
        "zh" => {
            let value = numbered(syllable);
            match tone {
                "numbers" => value,
                "none" => value.trim_end_matches(|c: char| c.is_ascii_digit()).replace('v', "ü"),
                _ => marked(&value),
            }
        }
        "yue" if tone == "none" => syllable.chars().filter(|c| !c.is_ascii_digit()).collect(),
        _ => syllable.to_string(),
    }
}

/// ({word: readings}, problems). Chinese words must have one syllable per character.
pub fn parse(text: &str, language: &str) -> (HashMap<String, Vec<String>>, Vec<String>) {
    let mut words = HashMap::new();
    let mut problems = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (word, reading) = line.split_once(' ').unwrap_or((line, ""));
        let word = word.trim();
        let parts: Vec<String> = reading.split_whitespace().map(str::to_string).collect();
        if parts.is_empty() {
            problems.push(format!("line {}: {word} has no reading", number + 1));
            continue;
        }
        let parts = if language == "zh" || language == "yue" {
            let count = word.chars().count();
            if parts.len() != count || !word.chars().all(is_han) {
                problems.push(format!("line {}: {word} needs one syllable for each of its {count} characters", number + 1));
                continue;
            }
            parts
        } else {
            vec![parts.concat()]
        };
        words.insert(word.to_string(), parts);
    }
    (words, problems)
}

pub fn stamp(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

type Library = (Option<SystemTime>, Option<SystemTime>, HashMap<String, Vec<String>>);
static LIBRARIES: Mutex<Option<HashMap<String, Library>>> = Mutex::new(None);

/// The merged library; reread when either file changes.
pub fn library(language: &str) -> HashMap<String, Vec<String>> {
    if !LANGUAGES.contains(&language) {
        return HashMap::new();
    }
    let (builtin, user) = (builtin_file(language), user_file(language));
    let (builtin_stamp, user_stamp) = (stamp(&builtin), stamp(&user));
    let mut cache = LIBRARIES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some((first, second, words)) = cache.get(language) {
        if *first == builtin_stamp && *second == user_stamp {
            return words.clone();
        }
    }
    let mut words = HashMap::new();
    for path in [builtin, user] {
        if let Ok(text) = std::fs::read_to_string(&path) {
            words.extend(parse(text.trim_start_matches('\u{feff}'), language).0);
        }
    }
    cache.insert(language.to_string(), (builtin_stamp, user_stamp, words.clone()));
    words
}

/// Library words found in the text (character offsets), longest first at each position, without overlaps.
pub fn matches(text: &str, words: &HashMap<String, Vec<String>>) -> Vec<(usize, usize, Vec<String>)> {
    if words.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let longest = words.keys().map(|word| word.chars().count()).max().unwrap_or(0);
    let mut found = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let hit = (1..=longest.min(chars.len() - index)).rev().find_map(|size| {
            let word: String = chars[index..index + size].iter().collect();
            words.get(&word).map(|reading| (size, reading.clone()))
        });
        match hit {
            Some((size, reading)) => {
                found.push((index, index + size, reading));
                index += size;
            }
            None => index += 1,
        }
    }
    found
}

/// The engine's (original, reading) pieces with library readings put in.
///
/// Chinese pieces are single characters, so a library word replaces the
/// readings of its characters and the engine keeps its context for the rest.
/// Japanese words are cut out of the line and the engine reads what is between them.
pub fn apply(text: &str, language: &str, raw: Vec<(String, String)>, settings: &Item) -> Vec<(String, String)> {
    let tone = tone_style(settings);
    let found = matches(text, &library(language));
    let chars: Vec<char> = text.chars().collect();
    if language == "ja" {
        if found.is_empty() {
            return raw;
        }
        let mut result: Vec<(String, String)> = Vec::new();
        let mut cursor = 0;
        let read_gap = |start: usize, end: usize, result: &mut Vec<(String, String)>| {
            let gap: String = chars[start..end].iter().collect();
            result.extend(engine_pairs("ja", &gap, settings).unwrap_or_else(|| vec![(gap.clone(), gap.clone())]));
        };
        for (start, end, reading) in &found {
            if *start > cursor {
                read_gap(cursor, *start, &mut result);
            }
            result.push((chars[*start..*end].iter().collect(), kana_to_romaji(&reading[0])));
            cursor = *end;
        }
        if cursor < chars.len() {
            read_gap(cursor, chars.len(), &mut result);
        }
        // The engine read each gap on its own, so particles after a library word are fixed here.
        return result
            .into_iter()
            .enumerate()
            .map(|(index, (original, reading))| {
                let particle = match original.as_str() {
                    "は" => Some("wa"),
                    "へ" => Some("e"),
                    "を" => Some("o"),
                    _ => None,
                };
                match particle {
                    Some(particle) if index > 0 => (original, particle.to_string()),
                    _ => (original, reading),
                }
            })
            .collect();
    }
    let mut readings: HashMap<usize, String> = HashMap::new();
    for (start, _, parts) in &found {
        for (offset, part) in parts.iter().enumerate() {
            readings.insert(start + offset, styled(part, language, tone));
        }
    }
    if language == "zh" {
        // A reduplicated word before 地 (慢慢地, 轻轻地) makes it the particle de.
        for index in 2..chars.len() {
            if chars[index] == '地' && is_han(chars[index - 2]) && chars[index - 2] == chars[index - 1] {
                readings.entry(index).or_insert_with(|| styled("de5", language, tone));
            }
        }
    }
    if readings.is_empty() {
        return raw;
    }
    let mut offset = 0;
    raw.into_iter()
        .map(|(original, reading)| {
            let length = original.chars().count();
            let reading = if length == 1 { readings.get(&offset).cloned().unwrap_or(reading) } else { reading };
            offset += length;
            (original, reading)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinyin_tone_forms() {
        assert_eq!(numbered("chóng"), "chong2");
        assert_eq!(numbered("lǜ"), "lv4");
        assert_eq!(numbered("de"), "de5");
        assert_eq!(numbered("nǚ3"), "nv3");
        assert_eq!(marked("chong2"), "chóng");
        assert_eq!(marked("lv4"), "lǜ");
        assert_eq!(marked("gou3"), "gǒu");
        assert_eq!(marked("liu2"), "liú");
        assert_eq!(marked("de5"), "de");
        assert_eq!(styled("chong2", "zh", "none"), "chong");
        assert_eq!(styled("lv4", "zh", "none"), "lü");
        assert_eq!(styled("ngan4", "yue", "none"), "ngan");
        assert_eq!(pinyin_style("lǜ", "numbers"), "lv4");
        assert_eq!(pinyin_style("lǜ", "none"), "lv");
        assert_eq!(pinyin_style("le", "marks"), "le");
    }

    #[test]
    fn library_files_are_checked() {
        let (words, problems) = parse("# comment\n重来 chóng lái\n睡不着 shui4 bu4\n君 きみ\n", "zh");
        assert_eq!(words["重来"], vec!["chóng", "lái"]);
        // 君 きみ passes the Chinese check too: one character, one syllable.
        assert_eq!(problems.len(), 1);
        let (japanese, _) = parse("君 きみ\n温もり ぬく もり\n", "ja");
        assert_eq!(japanese["温もり"], vec!["ぬくもり"]);
        let found = matches("我重来了重", &words);
        assert_eq!(found, vec![(1, 3, vec!["chóng".to_string(), "lái".to_string()])]);
    }

    #[test]
    fn chinese_library_readings_replace_characters() {
        let settings = crate::toml::parse("tone = \"marks\"\n").unwrap();
        let raw = vec![("慢".to_string(), "màn".to_string()), ("慢".to_string(), "màn".to_string()), ("地".to_string(), "dì".to_string())];
        assert_eq!(apply("慢慢地", "zh", raw, &settings)[2].1, "de");
    }
}
