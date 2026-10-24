//! Japanese to romaji, following pykakasi's Kakasi.convert step by step (GPL-3.0 data and method).

use crate::dicts::Kakasi;

const SYMBOLS_3000: [&str; 33] = [
    " ", ",", ".", "\"", "(kigou)", "(kurikaesi)", "(sime)", "(maru)", "<", ">", "<<", ">>", "(", ")", "(", ")", "(", ")", "(kigou)", "(geta)", "(", ")", "(", ")", "(", ")", "(", ")", "~", "(kigou)", "\"", "(kigou)", "(kigou)",
];
const SYMBOLS_3030: [&str; 16] = ["-", "(kurikaesi)", "(kurikaesi)", "(kurikaesi)", "(kurikaesi)", "(kurikaesi)", "(kigou)", "XX", "", "", "", "", "(masu)", "(kurikaesi)", " ", " "];
const GREEK_UPPER: [&str; 25] = ["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta", "Iota", "Kappa", "Lambda", "Mu", "Nu", "Xi", "Omicron", "Pi", "Rho", "", "Sigma", "Tau", "Upsilon", "Phi", "Chi", "Psi", "Omega"];
const GREEK_LOWER: [&str; 25] = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "final sigma", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega"];
const FULLWIDTH_PUNCTUATION: [&str; 15] = ["!", "\"", "#", "$", "%", "&", "'", "(", ")", "*", "+", ",", "-", ".", "/"];
const CYRILLIC_UPPER: [&str; 32] = ["A", "B", "V", "G", "D", "E", "Zh", "Z", "I", "Y", "K", "L", "M", "N", "O", "P", "R", "S", "T", "U", "F", "H", "Ts", "Ch", "Sh", "Sch", "", "Y", "", "E", "Yu", "Ya"];
const LATIN1: [&str; 95] = [
    "!", "cent", "GBP", "currency", "yen", "|", "ss", "..", "(c)", "a", "<<", "not", "-", "(R)", "~", ".", "+-", "^2", "^3", "`", "u", "D", ".", ",", "^1", "", ">>", "1/4", "1/2", "3/4", "?", "A", "A", "A", "A", "A", "A", "AE", "C", "E", "E", "E", "E", "I", "I", "I", "I", "Eth", "N", "O", "O", "O", "O", "O", "x", "O", "U", "U", "U", "U", "Y", "", "", "a", "a", "a", "a", "a", "a", "ae", "c", "e", "e", "e", "e", "i", "i", "i", "i", "eth", "n", "o", "o", "o", "o", "o", "/", "o", "u", "u", "u", "u", "y", "", "y",
];

fn is_endmark(c: char) -> bool {
    ")]!,.\u{3001}\u{3002}\u{ff1f}\u{ff10}\u{ff1e}\u{ff1c}".contains(c)
}

fn is_long_symbol(c: char) -> bool {
    matches!(c, '\u{30FC}' | '\u{2015}' | '\u{2212}' | '\u{FF70}')
}

fn is_katakana(c: char) -> bool {
    let c = c as u32;
    (0x30A0 < c && c < 0x30FD) || (0xFF65 < c && c < 0xFF9F) || (0x1B164..=0x1B167).contains(&c)
}

fn is_hiragana(c: char) -> bool {
    let c = c as u32;
    (0x3040 < c && c < 0x3097) || (0x1B150..=0x1B152).contains(&c)
}

fn is_alpha(c: char) -> bool {
    (0x20..0x7F).contains(&(c as u32))
}

/// pykakasi's symbol region (its Greek range runs to 0x30A1, which this keeps as it is).
fn is_symbol(c: char) -> bool {
    let c = c as u32;
    (0x3000..=0x3020).contains(&c)
        || (0x3030..=0x303F).contains(&c)
        || (0x0391..=0x30A1).contains(&c)
        || (0x03B1..=0x03C9).contains(&c)
        || (0x0410..=0x044F).contains(&c)
        || (0xFF01..=0xFF1A).contains(&c)
        || (0x00A1..=0x00FF).contains(&c)
        || (0xFF20..=0xFF5E).contains(&c)
        || c == 0x0451
        || c == 0x0401
}

/// A symbol in Latin letters, or "" when it has none.
fn symbol_text(c: char) -> String {
    let code = c as u32;
    let pick = |table: &[&str], index: u32| table.get(index as usize).copied().unwrap_or("").to_string();
    match code {
        0x3000..=0x3020 => pick(&SYMBOLS_3000, code - 0x3000),
        0x3030..=0x303F => pick(&SYMBOLS_3030, code - 0x3030),
        0x0391..=0x03A9 => pick(&GREEK_UPPER, code - 0x0391),
        0x03B1..=0x03C9 => pick(&GREEK_LOWER, code - 0x03B1),
        0x0410..=0x042F => pick(&CYRILLIC_UPPER, code - 0x0410),
        0x0430..=0x044F => pick(&CYRILLIC_UPPER, code - 0x0430).to_lowercase(),
        0x0401 => "E".into(),
        0x0451 => "e".into(),
        0xFF01..=0xFF0F => pick(&FULLWIDTH_PUNCTUATION, code - 0xFF01),
        0xFF10..=0xFF1A => char::from_u32(code - 0xFF10 + '0' as u32).map(String::from).unwrap_or_default(),
        0xFF20..=0xFF40 => char::from_u32(0x41 + code - 0xFF21).map(String::from).unwrap_or_default(),
        0xFF41..=0xFF5E => char::from_u32(0x61 + code - 0xFF41).map(String::from).unwrap_or_default(),
        0x00A1..=0x00FF => pick(&LATIN1, code - 0x00A1),
        _ => String::new(),
    }
}

/// Katakana as hiragana (other characters unchanged).
pub fn to_hiragana(text: &str) -> String {
    text.chars()
        .map(|c| {
            let code = c as u32;
            if 0x30A0 < code && code < 0x30F7 {
                char::from_u32(code - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

fn hepburn(kakasi: &Kakasi, hiragana: &str) -> String {
    let chars: Vec<char> = hiragana.chars().collect();
    let mut kana_done = String::new();
    let mut i = 0;
    // Longest kana sequence in the Hepburn table.
    while i < chars.len() {
        let mut best: Option<(usize, &String)> = None;
        for length in 1..=kakasi.hepburn_longest.min(chars.len() - i) {
            let key: String = chars[i..i + length].iter().collect();
            if let Some(value) = kakasi.hepburn.get(&key) {
                best = Some((length, value));
            }
        }
        match best {
            Some((length, value)) => {
                kana_done.push_str(value);
                i += length;
            }
            None => {
                kana_done.push(chars[i]);
                i += 1;
            }
        }
    }
    // Symbols in letters; a long-vowel mark repeats the letter before it.
    let mut result = String::new();
    for c in kana_done.chars() {
        let symbol = symbol_text(c);
        if !symbol.is_empty() {
            result.push_str(&symbol);
        } else if is_long_symbol(c) {
            match result.chars().last() {
                Some(last) => result.push(last),
                None => result.push('-'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// The reading of the kanji word at the start of `text`, and how many characters it covers.
fn kanji_reading(kakasi: &Kakasi, text: &[char], before: &str) -> (String, usize) {
    let mut translated: Vec<char> = Vec::new();
    for c in text {
        match kakasi.itaiji.get(c) {
            Some(Some(usual)) => translated.push(*usual),
            Some(None) => {}
            None => translated.push(*c),
        }
    }
    let dropped = text.len() - translated.len();
    if translated.is_empty() {
        return (String::new(), dropped);
    }
    let table = match kakasi.kanwa.get(&translated[0]) {
        Some(table) => table,
        None => return (String::new(), 0),
    };
    let (mut best, mut length) = (String::new(), 0);
    for (word, yomi, context) in table {
        let count = word.chars().count();
        if count <= translated.len() && word.chars().zip(&translated).all(|(a, b)| a == *b) {
            let fits = context.as_ref().map_or(true, |context| context.iter().any(|item| item == before));
            if fits && length < count {
                best = yomi.clone();
                length = count;
            }
        }
    }
    (best, (length + dropped).min(text.len()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Type {
    Kanji,
    Kana,
    Hiragana,
    Symbol,
    Alpha,
}

/// (original, romaji) pieces of Japanese text, the way pykakasi's convert() splits and reads them.
pub fn convert(kakasi: &Kakasi, text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut result: Vec<(String, String)> = Vec::new();
    let (mut original, mut kana) = (String::new(), String::new());
    let mut previous = Type::Kanji;
    let mut i = 0;
    let output = |result: &mut Vec<(String, String)>, original: &str, kana: &str| {
        result.push((original.to_string(), hepburn(kakasi, &to_hiragana(kana))));
    };
    while i < chars.len() {
        let c = chars[i];
        enum Action {
            Flush,
            Put,
            FlushThenAlone,
            Skip,
            Nothing,
        }
        let action;
        if is_endmark(c) {
            previous = Type::Symbol;
            action = Action::Flush;
        } else if is_long_symbol(c) {
            action = Action::Put;
        } else if is_katakana(c) {
            action = if previous == Type::Kana { Action::Put } else { Action::Flush };
            previous = Type::Kana;
        } else if is_hiragana(c) {
            action = if previous == Type::Hiragana { Action::Put } else { Action::Flush };
            previous = Type::Hiragana;
        } else if is_alpha(c) {
            action = if previous == Type::Alpha { Action::Put } else { Action::Flush };
            previous = Type::Alpha;
        } else if is_symbol(c) {
            action = if previous != Type::Symbol { Action::Flush } else { Action::FlushThenAlone };
            previous = Type::Symbol;
        } else if (0x3400..0xE000).contains(&(c as u32)) || kakasi.itaiji.contains_key(&c) {
            if !original.is_empty() {
                output(&mut result, &original, &kana);
            }
            let (reading, length) = kanji_reading(kakasi, &chars[i..], &original);
            previous = Type::Kanji;
            if length > 0 {
                original = chars[i..i + length].iter().collect();
                kana = reading;
                i += length;
                action = Action::Nothing;
            } else {
                // An unknown kanji is left out of the reading.
                original = c.to_string();
                kana.clear();
                i += 1;
                action = Action::Skip;
            }
        } else if (0xF000..=0xFFFD).contains(&(c as u32)) || (0x10000..=0x10FFFD).contains(&(c as u32)) {
            previous = Type::Symbol;
            if !original.is_empty() {
                output(&mut result, &original, &kana);
                original.clear();
                kana.clear();
            }
            i += 1;
            action = Action::Nothing;
        } else {
            if !original.is_empty() {
                output(&mut result, &original, &kana);
                original.clear();
                kana.clear();
            }
            output(&mut result, &c.to_string(), "");
            i += 1;
            action = Action::Nothing;
        }
        match action {
            Action::Flush => {
                if !original.is_empty() {
                    output(&mut result, &original, &kana);
                }
                original = chars[i].to_string();
                kana = original.clone();
                i += 1;
            }
            Action::Put => {
                original.push(chars[i]);
                kana.push(chars[i]);
                i += 1;
            }
            Action::FlushThenAlone => {
                if !original.is_empty() {
                    output(&mut result, &original, &kana);
                }
                output(&mut result, &chars[i].to_string(), &chars[i].to_string());
                original.clear();
                kana.clear();
                i += 1;
            }
            Action::Skip => {
                if !original.is_empty() {
                    output(&mut result, &original, &kana);
                }
                original.clear();
                kana.clear();
            }
            Action::Nothing => {}
        }
    }
    if !original.is_empty() {
        output(&mut result, &original, &kana);
    }
    result
}

/// Kana (hiragana or katakana) in romaji.
pub fn kana_to_romaji(kakasi: &Kakasi, reading: &str) -> String {
    hepburn(kakasi, &to_hiragana(reading))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn small() -> Kakasi {
        let mut kanwa: HashMap<char, Vec<(String, String, Option<Vec<String>>)>> = HashMap::new();
        kanwa.insert('私', vec![("私".into(), "わたし".into(), None)]);
        kanwa.insert('君', vec![("君".into(), "くん".into(), None)]);
        let hepburn: HashMap<String, String> = [("わ", "wa"), ("た", "ta"), ("し", "shi"), ("は", "ha"), ("く", "ku"), ("ん", "n"), ("カ", "ka"), ("か", "ka"), ("ら", "ra"), ("お", "o"), ("け", "ke")].iter().map(|(kana, romaji)| (kana.to_string(), romaji.to_string())).collect();
        Kakasi { kanwa, hepburn, hepburn_longest: 1, itaiji: HashMap::new() }
    }

    #[test]
    fn words_split_by_script() {
        let kakasi = small();
        assert_eq!(convert(&kakasi, "私はカラオケ!"), vec![
            ("私".to_string(), "watashi".to_string()),
            ("は".to_string(), "ha".to_string()),
            ("カラオケ".to_string(), "karaoke".to_string()),
            ("!".to_string(), "!".to_string()),
        ]);
        assert_eq!(kana_to_romaji(&kakasi, "カー"), "kaa");
    }
}
