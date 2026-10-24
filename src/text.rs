//! Text helpers shared by every part of PolyKara: lyric tokens, comparison keys, scripts.

/// Scripts written without spaces (kana, CJK ideographs, Hangul) are timed per character.
pub fn is_cjk_unit(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{AC00}'..='\u{D7AF}')
}

pub fn is_han(c: char) -> bool {
    matches!(c, '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}')
}

pub fn is_kana(c: char) -> bool {
    matches!(c, '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}')
}

pub fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}')
}

pub fn is_katakana(c: char) -> bool {
    matches!(c, '\u{30A0}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}')
}

pub fn is_hangul(c: char) -> bool {
    matches!(c, '\u{AC00}'..='\u{D7A3}')
}

/// CJK in the wide sense used to join words without spaces (punctuation, kana, ideographs, Hangul).
pub fn is_cjk_wide(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}' | '\u{F900}'..='\u{FAFF}')
}

/// Python's str.isalnum plus underscore: what \w matches.
pub fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn casefold(text: &str) -> String {
    text.to_lowercase()
}

/// Comparison form of a token: case-folded with punctuation removed.
pub fn token_key(token: &str) -> String {
    casefold(token).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Split a lyric line into karaoke syllables.
///
/// Tokens keep their punctuation and one trailing space where the source had
/// whitespace, so joining them with "" reproduces the line.
pub fn tokenise(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut prefix = String::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c.is_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        if is_cjk_unit(c) {
            index += 1;
        } else {
            while index < chars.len() && !chars[index].is_whitespace() && !is_cjk_unit(chars[index]) {
                index += 1;
            }
        }
        let mut piece: String = chars[start..index].iter().collect();
        if index < chars.len() && chars[index].is_whitespace() {
            piece.push(' ');
        }
        if !token_key(&piece).is_empty() {
            tokens.push(std::mem::take(&mut prefix) + &piece);
        } else if let Some(last) = tokens.last_mut() {
            last.push_str(&piece);
        } else {
            prefix.push_str(&piece);
        }
    }
    if !prefix.is_empty() {
        tokens.push(prefix);
    }
    if let Some(last) = tokens.last_mut() {
        let trimmed = last.trim_end().to_string();
        *last = trimmed;
    }
    tokens
}

/// The length in bytes of a leading "[singer: name]" style tag (with the spaces after it), and the name.
pub fn speaker_tag(text: &str) -> Option<(usize, String)> {
    let rest = text.strip_prefix('[')?;
    let lower = rest.to_lowercase();
    let word = ["singer", "speaker", "vocal", "role"].iter().find(|word| lower.starts_with(*word))?;
    let after = rest[word.len()..].trim_start();
    let after = after.strip_prefix(':')?;
    let close = after.find(']')?;
    let name = after[..close].trim();
    if name.is_empty() {
        return None;
    }
    let consumed_inner = text.len() - after.len() + close + 1;
    let tail = &text[consumed_inner..];
    let spaces = tail.len() - tail.trim_start().len();
    Some((consumed_inner + spaces, name.to_string()))
}

/// ("speaker", "lyric") from a line that may start with a [singer: x] tag; both trimmed.
pub fn split_speaker(text: &str) -> (String, String) {
    match speaker_tag(text) {
        Some((length, name)) => (name, text[length..].trim().to_string()),
        None => (String::new(), text.trim().to_string()),
    }
}

/// Full-width ASCII forms (，！？ and the ideographic space) as their ASCII characters, as NFKC does.
pub fn halfwidth(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            '\u{3000}' => ' ',
            other => other,
        })
        .collect()
}

/// Latin letters (and digits): scripts that need no reading above them.
pub fn is_latin_letter(c: char) -> bool {
    matches!(c, 'A'..='Z' | 'a'..='z' | '\u{00C0}'..='\u{024F}' | '\u{1E00}'..='\u{1EFF}' | '\u{2C60}'..='\u{2C7F}' | '\u{A720}'..='\u{A7FF}' | '\u{FF21}'..='\u{FF3A}' | '\u{FF41}'..='\u{FF5A}' | '\u{0250}'..='\u{02AF}')
}

/// Whether the text has letters in a script other than Latin.
pub fn non_latin(text: &str) -> bool {
    text.chars().any(|c| c.is_alphabetic() && !is_latin_letter(c) && !c.is_ascii())
}

/// Sort key for natural file order (Python sorted() on names): plain string order.
pub fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_keep_spaces_and_punctuation() {
        assert_eq!(tokenise("Hello, PolyKara world!"), vec!["Hello, ", "PolyKara ", "world!"]);
        assert_eq!(tokenise("唤不醒 hi"), vec!["唤", "不", "醒 ", "hi"]);
        assert_eq!(tokenise("「你好」"), vec!["「你", "好」"]);
        assert_eq!(tokenise("   "), Vec::<String>::new());
        assert_eq!(tokenise("...") , vec!["..."]);
    }

    #[test]
    fn keys_ignore_case_and_punctuation() {
        assert_eq!(token_key("Don't!"), "dont");
        assert_eq!(token_key("你,"), "你");
        assert_eq!(token_key("_"), "");
    }

    #[test]
    fn speaker_tags() {
        assert_eq!(split_speaker("[singer: 女] 唤不醒"), ("女".to_string(), "唤不醒".to_string()));
        assert_eq!(split_speaker("[Speaker:Bob]hi "), ("Bob".to_string(), "hi".to_string()));
        assert_eq!(split_speaker("[chorus] hi"), (String::new(), "[chorus] hi".to_string()));
        assert_eq!(speaker_tag("[singer: 女] 唤").map(|(length, _)| length), Some("[singer: 女] ".len()));
    }

    #[test]
    fn scripts() {
        assert!(non_latin("Привет"));
        assert!(non_latin("你好"));
        assert!(!non_latin("Héllo 123"));
        assert_eq!(halfwidth("你好，世界！"), "你好,世界!");
    }
}
