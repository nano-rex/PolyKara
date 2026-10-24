//! The open dictionaries behind the readings, loaded once from the program's data folder.
//!
//! packaging/fetch-data.ps1 downloads them from pinned commits and checks their SHA-256:
//! pinyin-data and phrase-pinyin-data (MIT), ToJyutping's trie (BSD-2-Clause),
//! pykakasi's kakasi dictionaries (GPL-3.0) and OpenCC's TSCharacters (Apache-2.0).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::paths;

pub fn data_folder() -> PathBuf {
    if let Some(folder) = std::env::var_os("POLYKARA_DATA") {
        return PathBuf::from(folder);
    }
    paths().root.join("data")
}

fn read(name: &str) -> Option<String> {
    std::fs::read_to_string(data_folder().join(name)).ok().map(|text| text.trim_start_matches('\u{feff}').to_string())
}

/// A value loaded on first use and kept; None when its data file is missing.
pub struct Lazy<T> {
    cell: Mutex<Option<Option<Arc<T>>>>,
    load: fn() -> Option<T>,
}

impl<T> Lazy<T> {
    pub const fn new(load: fn() -> Option<T>) -> Lazy<T> {
        Lazy { cell: Mutex::new(None), load }
    }

    pub fn get(&self) -> Option<Arc<T>> {
        let mut cell = self.cell.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if cell.is_none() {
            *cell = Some((self.load)().map(Arc::new));
        }
        cell.as_ref().cloned().flatten()
    }
}

/// Mandarin: the usual reading of each character, and readings of whole words.
pub struct Pinyin {
    pub chars: HashMap<char, String>,
    pub phrases: HashMap<String, Vec<String>>,
    pub longest: usize,
}

fn load_pinyin() -> Option<Pinyin> {
    let mut chars = HashMap::new();
    for line in read("pinyin.txt")?.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let (code, readings) = match line.split_once(':') {
            Some(parts) => parts,
            None => continue,
        };
        let code = match code.trim().strip_prefix("U+").and_then(|hex| u32::from_str_radix(hex, 16).ok()).and_then(char::from_u32) {
            Some(c) => c,
            None => continue,
        };
        if let Some(first) = readings.split(',').next().map(str::trim).filter(|value| !value.is_empty()) {
            chars.insert(code, first.to_string());
        }
    }
    let mut phrases = HashMap::new();
    let mut longest = 1;
    if let Some(text) = read("phrases.txt") {
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if let Some((word, readings)) = line.split_once(':') {
                let word = word.trim().to_string();
                let readings: Vec<String> = readings.split_whitespace().map(str::to_string).collect();
                if readings.len() == word.chars().count() && !readings.is_empty() {
                    longest = longest.max(readings.len());
                    phrases.insert(word, readings);
                }
            }
        }
    }
    Some(Pinyin { chars, phrases, longest })
}

pub static PINYIN: Lazy<Pinyin> = Lazy::new(load_pinyin);

/// Cantonese: ToJyutping's trie of characters and words.
#[derive(Default)]
pub struct JyutpingNode {
    pub children: HashMap<char, usize>,
    /// Readings at this node: each is one syllable per character of the path (several for some).
    pub values: Vec<Vec<u32>>,
}

pub struct Jyutping {
    pub nodes: Vec<JyutpingNode>,
}

const ONSET: [&str; 20] = ["", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "ng", "gw", "kw", "w", "h", "z", "c", "s", "j"];
const NUCLEUS: [&str; 6] = ["aa", "a", "e", "i", "o", "u"];
const RHYME: [&str; 13] = ["oe", "oen", "oeng", "oet", "oek", "eoi", "eon", "eot", "yu", "yun", "yut", "m", "ng"];
const CODA: [&str; 9] = ["", "i", "u", "m", "n", "ng", "p", "t", "k"];

/// A syllable id from ToJyutping's data as Jyutping text, e.g. "ngo5".
pub fn jyutping_text(id: u32) -> String {
    let onset = ONSET.get((id / 402) as usize).copied().unwrap_or("");
    let rhyme_id = (id % 402) / 6;
    let rhyme = if rhyme_id >= 54 {
        RHYME.get((rhyme_id - 54) as usize).copied().unwrap_or("").to_string()
    } else {
        format!("{}{}", NUCLEUS.get((rhyme_id / 9) as usize).copied().unwrap_or(""), CODA[(rhyme_id % 9) as usize])
    };
    format!("{onset}{rhyme}{}", id % 6 + 1)
}

impl Jyutping {
    /// Decode trie.txt exactly as ToJyutping's Trie.py does.
    pub fn parse(text: &str) -> Option<Jyutping> {
        let s: Vec<char> = text.chars().collect();
        let mut nodes = vec![JyutpingNode::default()];
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        let mut i = 1;
        while let Some(&(parent, depth)) = stack.last() {
            let (mut node, mut depth) = (parent, depth);
            while (*s.get(i)? as u32) >= 256 {
                let c = s[i];
                node = match nodes[node].children.get(&c) {
                    Some(child) => *child,
                    None => {
                        nodes.push(JyutpingNode::default());
                        let child = nodes.len() - 1;
                        nodes[node].children.insert(c, child);
                        child
                    }
                };
                i += 1;
                depth += 1;
            }
            let mut values = Vec::new();
            while (*s.get(i)? as u32) < 123 {
                let mut count = 0;
                let mut word = Vec::new();
                while count < depth {
                    let id = (*s.get(i)? as u32 - 33) * 90 + (*s.get(i + 1)? as u32 - 33);
                    word.push(id);
                    i += 2;
                    if s.get(i) == Some(&'~') {
                        i += 1;
                    } else {
                        count += 1;
                    }
                }
                values.push(word);
            }
            if !values.is_empty() {
                nodes[node].values = values;
            }
            match s.get(i) {
                Some('{') => {
                    i += 1;
                    stack.push((node, depth));
                }
                Some('}') => {
                    i += 1;
                    stack.pop();
                }
                _ => {}
            }
        }
        Some(Jyutping { nodes })
    }

    /// (character, Jyutping) for each character: the longest word in the trie wins, as ToJyutping's get().
    pub fn readings(&self, text: &str) -> Vec<(char, Option<String>)> {
        let chars: Vec<char> = text.chars().collect();
        let mut result = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let mut node = 0;
            let mut best: Option<&Vec<u32>> = None;
            let mut reach = i;
            for (j, c) in chars.iter().enumerate().skip(i) {
                match self.nodes[node].children.get(c) {
                    Some(child) => node = *child,
                    None => break,
                }
                if let Some(first) = self.nodes[node].values.first() {
                    best = Some(first);
                    reach = j;
                }
            }
            if reach == i {
                // One character: its first reading (all its syllables, joined by spaces).
                let reading = best.map(|ids| ids.iter().map(|id| jyutping_text(*id)).collect::<Vec<_>>().join(" "));
                result.push((chars[i], reading));
                i += 1;
            } else if let Some(word) = best {
                let start = i;
                while i <= reach {
                    result.push((chars[i], word.get(i - start).map(|id| jyutping_text(*id))));
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
        result
    }
}

pub static JYUTPING: Lazy<Jyutping> = Lazy::new(|| Jyutping::parse(&read("jyutping-trie.txt")?));

/// Japanese: kakasi's kanji readings by first character, the kana-to-romaji table and variant kanji.
pub struct Kakasi {
    /// First character → [(kanji word, reading in hiragana, context)].
    pub kanwa: HashMap<char, Vec<(String, String, Option<Vec<String>>)>>,
    pub hepburn: HashMap<String, String>,
    pub hepburn_longest: usize,
    /// Variant kanji written as their usual form; None drops the character (variation selectors).
    pub itaiji: HashMap<char, Option<char>>,
}

/// Okurigana letters: a reading ending in "k" stands for every kana that starts with k.
fn okurigana(tail: char) -> &'static [&'static str] {
    match tail {
        'a' => &["あ", "ぁ", "っ", "わ", "ゎ"],
        'i' => &["い", "ぃ", "っ", "ゐ"],
        'u' => &["う", "ぅ", "っ"],
        'e' => &["え", "ぇ", "っ", "ゑ"],
        'o' => &["お", "ぉ", "っ"],
        'k' => &["か", "ゕ", "き", "く", "け", "ゖ", "こ", "っ"],
        'g' => &["が", "ぎ", "ぐ", "げ", "ご", "っ"],
        's' => &["さ", "し", "す", "せ", "そ", "っ"],
        'z' | 'j' => &["ざ", "じ", "ず", "ぜ", "ぞ", "っ"],
        't' => &["た", "ち", "つ", "て", "と", "っ"],
        'd' => &["だ", "ぢ", "づ", "で", "ど", "っ"],
        'c' => &["ち", "っ"],
        'n' => &["な", "に", "ぬ", "ね", "の", "ん"],
        'h' => &["は", "ひ", "ふ", "へ", "ほ", "っ"],
        'b' => &["ば", "び", "ぶ", "べ", "ぼ", "っ"],
        'f' => &["ふ", "っ"],
        'p' => &["ぱ", "ぴ", "ぷ", "ぺ", "ぽ", "っ"],
        'm' => &["ま", "み", "む", "め", "も"],
        'y' => &["や", "ゃ", "ゆ", "ゅ", "よ", "ょ"],
        'r' | 'l' => &["ら", "り", "る", "れ", "ろ"],
        'w' => &["わ", "ゐ", "ゑ", "ゎ", "を", "っ"],
        'v' => &["ゔ"],
        _ => &[],
    }
}

impl Kakasi {
    fn add(&mut self, kanji: String, yomi: String, context: Option<Vec<String>>) {
        let first = match kanji.chars().next() {
            Some(first) => first,
            None => return,
        };
        self.kanwa.entry(first).or_default().push((kanji, yomi, context));
    }

    /// One dictionary line, "よみ 漢字 [context...]", as pykakasi's dictionary builder reads it.
    fn add_line(&mut self, line: &str) {
        if line.starts_with(";;") || line.trim().is_empty() {
            return;
        }
        let tokens: Vec<&str> = line.split(' ').collect();
        if tokens.len() < 2 {
            return;
        }
        let (mut yomi, kanji) = (tokens[0].to_string(), tokens[1].to_string());
        let context = (tokens.len() > 2).then(|| tokens[2..].iter().map(|item| item.to_string()).collect());
        match yomi.chars().last() {
            Some(tail) if (tail as u32) <= 'z' as u32 => {
                yomi.pop();
                for kana in okurigana(tail) {
                    self.add(format!("{kanji}{kana}"), format!("{yomi}{kana}"), context.clone());
                }
            }
            _ => self.add(kanji, yomi, context),
        }
    }
}

fn load_kakasi() -> Option<Kakasi> {
    let mut kakasi = Kakasi { kanwa: HashMap::new(), hepburn: HashMap::new(), hepburn_longest: 1, itaiji: HashMap::new() };
    for name in ["kakasidict.utf8", "unidict_noun.utf8", "unidict_adj.utf8"] {
        let text = read(name)?;
        for line in text.lines() {
            kakasi.add_line(line.trim());
        }
    }
    for line in read("hepburnhira.utf8")?.lines() {
        let line = line.trim();
        if line.starts_with(";;") || line.is_empty() {
            continue;
        }
        if let Some((romaji, kana)) = line.split_once(' ') {
            kakasi.hepburn_longest = kakasi.hepburn_longest.max(kana.chars().count());
            kakasi.hepburn.insert(kana.to_string(), romaji.to_string());
        }
    }
    for line in read("itaijidict.utf8").unwrap_or_default().lines() {
        let line = line.trim();
        if line.starts_with(";;") || line.is_empty() {
            continue;
        }
        if let Some((usual, variant)) = line.split_once(' ') {
            if let (Some(usual), Some(variant)) = (usual.chars().next(), variant.chars().next()) {
                kakasi.itaiji.insert(variant, Some(usual));
            }
        }
    }
    for code in (0xFE00..0xFE02).chain(0xE0100..0xE01EF) {
        if let Some(c) = char::from_u32(code) {
            kakasi.itaiji.insert(c, None);
        }
    }
    Some(kakasi)
}

pub static KAKASI: Lazy<Kakasi> = Lazy::new(load_kakasi);

/// Traditional Chinese characters to Simplified (OpenCC TSCharacters), for comparing lyric files.
fn load_simplified() -> Option<HashMap<char, char>> {
    let mut map = HashMap::new();
    for line in read("TSCharacters.txt")?.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(['\t', ' ']).filter(|part| !part.is_empty());
        if let (Some(from), Some(to)) = (parts.next(), parts.next()) {
            let (mut from_chars, mut to_chars) = (from.chars(), to.chars());
            if let (Some(from), None, Some(to), None) = (from_chars.next(), from_chars.next(), to_chars.next(), to_chars.next()) {
                if from != to {
                    map.insert(from, to);
                }
            }
        }
    }
    Some(map)
}

pub static SIMPLIFIED: Lazy<HashMap<char, char>> = Lazy::new(load_simplified);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jyutping_ids_decode() {
        // ngo5 = onset ng (11), rhyme o (nucleus 4, coda 0 → 36), tone 5 (id 4).
        assert_eq!(jyutping_text(11 * 402 + 36 * 6 + 4), "ngo5");
        assert_eq!(jyutping_text(0 * 402 + 65 * 6 + 3), "m4");
    }

    #[test]
    fn jyutping_trie_text_is_decoded() {
        // {銀 ngan4 {行 ngan4 hong4}} : a root child 銀 with one reading, and the word 銀行.
        let code = |id: u32| format!("{}{}", char::from_u32(id / 90 + 33).unwrap(), char::from_u32(id % 90 + 33).unwrap());
        let ngan4 = 11 * 402 + 13 * 6 + 3;
        let hong4 = 15 * 402 + 41 * 6 + 3;
        let text = format!("{{銀{}{{行{}{}}}}}", code(ngan4), code(ngan4), code(hong4));
        let trie = Jyutping::parse(&text).unwrap();
        assert_eq!(trie.readings("銀行x"), vec![('銀', Some("ngan4".into())), ('行', Some("hong4".into())), ('x', None)]);
    }

    #[test]
    fn okurigana_lines_expand() {
        let mut kakasi = Kakasi { kanwa: HashMap::new(), hepburn: HashMap::new(), hepburn_longest: 1, itaiji: HashMap::new() };
        kakasi.add_line("むずかしk 難し");
        let words: Vec<&str> = kakasi.kanwa[&'難'].iter().map(|item| item.0.as_str()).collect();
        assert!(words.contains(&"難しく"));
        assert_eq!(kakasi.kanwa[&'難'][0].1, "むずかしか");
    }
}
