//! polykara.toml: a TOML reader for the settings file, and in-place edits that keep its comments.

use crate::json::Value as Json;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Array(Vec<Item>),
    /// Keys keep their order.
    Table(Vec<(String, Item)>),
}

pub static EMPTY: Item = Item::Table(Vec::new());

impl Item {
    pub fn get(&self, key: &str) -> &Item {
        match self {
            Item::Table(pairs) => pairs.iter().find(|(name, _)| name == key).map(|(_, value)| value).unwrap_or(&EMPTY),
            _ => &EMPTY,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        matches!(self, Item::Table(pairs) if pairs.iter().any(|(name, _)| name == key))
    }

    pub fn pairs(&self) -> &[(String, Item)] {
        match self {
            Item::Table(pairs) => pairs,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[Item] {
        match self {
            Item::Array(items) => items,
            _ => &[],
        }
    }

    pub fn is_table(&self) -> bool {
        matches!(self, Item::Table(_))
    }

    /// The value as text, as Python's str() would show it.
    pub fn text(&self) -> String {
        match self {
            Item::Str(text) => text.clone(),
            Item::Int(number) => number.to_string(),
            Item::Float(number) => float_text(*number),
            Item::Bool(value) => if *value { "True".into() } else { "False".into() },
            Item::Array(items) => format!("[{}]", items.iter().map(Item::literal).collect::<Vec<_>>().join(", ")),
            Item::Table(_) => String::new(),
        }
    }

    /// A string setting, or the default when it is missing.
    pub fn str_or(&self, key: &str, default: &str) -> String {
        match self.get(key) {
            Item::Table(pairs) if pairs.is_empty() && !self.has(key) => default.to_string(),
            item => item.text(),
        }
    }

    pub fn int_or(&self, key: &str, default: i64) -> i64 {
        match self.get(key) {
            Item::Int(number) => *number,
            Item::Float(number) => *number as i64,
            Item::Str(text) => text.trim().parse::<f64>().map(|number| number as i64).unwrap_or(default),
            Item::Bool(value) => *value as i64,
            _ => default,
        }
    }

    pub fn float_or(&self, key: &str, default: f64) -> f64 {
        match self.get(key) {
            Item::Int(number) => *number as f64,
            Item::Float(number) => *number,
            Item::Str(text) => text.trim().parse::<f64>().unwrap_or(default),
            _ => default,
        }
    }

    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        match self.get(key) {
            Item::Bool(value) => *value,
            Item::Int(number) => *number != 0,
            Item::Str(text) => !text.is_empty(),
            _ => default,
        }
    }

    /// The TOML literal for this value.
    pub fn literal(&self) -> String {
        match self {
            Item::Str(text) => {
                let mut out = String::from("\"");
                for c in text.chars() {
                    match c {
                        '\\' => out.push_str("\\\\"),
                        '"' => out.push_str("\\\""),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            Item::Int(number) => number.to_string(),
            Item::Float(number) => float_text(*number),
            Item::Bool(value) => if *value { "true".into() } else { "false".into() },
            Item::Array(items) => format!("[{}]", items.iter().map(Item::literal).collect::<Vec<_>>().join(", ")),
            Item::Table(pairs) => format!("{{{}}}", pairs.iter().map(|(key, value)| format!("{key} = {}", value.literal())).collect::<Vec<_>>().join(", ")),
        }
    }

    pub fn to_json(&self) -> Json {
        match self {
            Item::Str(text) => Json::String(text.clone()),
            Item::Int(number) => Json::Number(*number as f64),
            Item::Float(number) => Json::Number(*number),
            Item::Bool(value) => Json::Bool(*value),
            Item::Array(items) => Json::Array(items.iter().map(Item::to_json).collect()),
            Item::Table(pairs) => Json::Object(pairs.iter().map(|(key, value)| (key.clone(), value.to_json())).collect()),
        }
    }

    fn table_mut(&mut self, key: &str) -> Result<&mut Item, String> {
        let pairs = match self {
            Item::Table(pairs) => pairs,
            _ => return Err(format!("{key} is not a table")),
        };
        let index = match pairs.iter().position(|(name, _)| name == key) {
            Some(index) => index,
            None => {
                pairs.push((key.to_string(), Item::Table(Vec::new())));
                pairs.len() - 1
            }
        };
        match &mut pairs[index].1 {
            Item::Array(items) => match items.last_mut() {
                Some(last @ Item::Table(_)) => Ok(last),
                _ => Err(format!("{key} is not a table")),
            },
            item @ Item::Table(_) => Ok(item),
            _ => Err(format!("{key} is not a table")),
        }
    }

    fn insert(&mut self, key: String, value: Item) -> Result<(), String> {
        match self {
            Item::Table(pairs) => {
                if pairs.iter().any(|(name, _)| *name == key) {
                    return Err(format!("{key} is defined twice"));
                }
                pairs.push((key, value));
                Ok(())
            }
            _ => Err(format!("{key} cannot be set here")),
        }
    }
}

/// Python's repr() of a float: 1.0, 0.6, 1e-05.
pub fn float_text(number: f64) -> String {
    if number.is_finite() && number.fract() == 0.0 && number.abs() < 1e16 {
        format!("{number:.1}")
    } else {
        format!("{number}")
    }
}

/// Merge `other` into `base`: tables merge key by key, everything else replaces.
pub fn merge(base: &mut Item, other: &Item) {
    if let (Item::Table(pairs), Item::Table(others)) = (&mut *base, other) {
        for (key, value) in others {
            match pairs.iter_mut().find(|(name, _)| name == key) {
                Some((_, existing)) if existing.is_table() && value.is_table() => merge(existing, value),
                Some((_, existing)) => *existing = value.clone(),
                None => pairs.push((key.clone(), value.clone())),
            }
        }
    }
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    line: usize,
    _text: &'a str,
}

impl<'a> Parser<'a> {
    fn error<T>(&self, message: &str) -> Result<T, String> {
        Err(format!("line {}: {message}", self.line))
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn spaces(&mut self) {
        while matches!(self.peek(), Some(' ') | Some('\t')) {
            self.at += 1;
        }
    }

    /// Spaces, newlines and comments (inside arrays).
    fn blank(&mut self) {
        loop {
            match self.peek() {
                Some(' ') | Some('\t') | Some('\r') | Some('\n') => {
                    self.bump();
                }
                Some('#') => {
                    while !matches!(self.peek(), None | Some('\n')) {
                        self.at += 1;
                    }
                }
                _ => return,
            }
        }
    }

    fn line_end(&mut self) -> Result<(), String> {
        self.spaces();
        if self.peek() == Some('#') {
            while !matches!(self.peek(), None | Some('\n')) {
                self.at += 1;
            }
        }
        if self.peek() == Some('\r') {
            self.at += 1;
        }
        match self.peek() {
            None => Ok(()),
            Some('\n') => {
                self.bump();
                Ok(())
            }
            Some(other) => self.error(&format!("unexpected {other:?} after a value")),
        }
    }

    fn key_part(&mut self) -> Result<String, String> {
        self.spaces();
        match self.peek() {
            Some('"') => self.basic_string(),
            Some('\'') => self.literal_string(),
            _ => {
                let start = self.at;
                while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                    self.at += 1;
                }
                if start == self.at {
                    return self.error("expected a key");
                }
                Ok(self.chars[start..self.at].iter().collect())
            }
        }
    }

    fn key(&mut self) -> Result<Vec<String>, String> {
        let mut parts = vec![self.key_part()?];
        loop {
            self.spaces();
            if self.peek() == Some('.') {
                self.at += 1;
                parts.push(self.key_part()?);
            } else {
                return Ok(parts);
            }
        }
    }

    fn basic_string(&mut self) -> Result<String, String> {
        let multi = self.chars[self.at..].starts_with(&['"', '"', '"']);
        self.at += if multi { 3 } else { 1 };
        if multi && self.peek() == Some('\n') {
            self.bump();
        } else if multi && self.peek() == Some('\r') && self.chars.get(self.at + 1) == Some(&'\n') {
            self.at += 1;
            self.bump();
        }
        let mut out = String::new();
        loop {
            let c = match self.bump() {
                Some(c) => c,
                None => return self.error("unfinished string"),
            };
            match c {
                '"' if !multi => return Ok(out),
                '"' if self.chars[self.at..].starts_with(&['"', '"']) => {
                    self.at += 2;
                    return Ok(out);
                }
                '\n' if !multi => return self.error("unfinished string"),
                '\\' => {
                    let escape = match self.bump() {
                        Some(escape) => escape,
                        None => return self.error("unfinished escape"),
                    };
                    match escape {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        'u' | 'U' => {
                            let length = if escape == 'u' { 4 } else { 8 };
                            let digits: String = self.chars.get(self.at..self.at + length).map(|part| part.iter().collect()).unwrap_or_default();
                            self.at += length;
                            match u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32) {
                                Some(c) => out.push(c),
                                None => return self.error("bad unicode escape"),
                            }
                        }
                        '\n' | ' ' | '\t' | '\r' if multi => {
                            while matches!(self.peek(), Some(' ') | Some('\t') | Some('\n') | Some('\r')) {
                                self.bump();
                            }
                        }
                        other => return self.error(&format!("bad escape \\{other}")),
                    }
                }
                c => out.push(c),
            }
        }
    }

    fn literal_string(&mut self) -> Result<String, String> {
        let multi = self.chars[self.at..].starts_with(&['\'', '\'', '\'']);
        self.at += if multi { 3 } else { 1 };
        if multi && self.peek() == Some('\n') {
            self.bump();
        }
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return self.error("unfinished string"),
                Some('\'') if !multi => return Ok(out),
                Some('\'') if self.chars[self.at..].starts_with(&['\'', '\'']) => {
                    self.at += 2;
                    return Ok(out);
                }
                Some('\n') if !multi => return self.error("unfinished string"),
                Some(c) => out.push(c),
            }
        }
    }

    fn value(&mut self) -> Result<Item, String> {
        self.spaces();
        match self.peek() {
            Some('"') => self.basic_string().map(Item::Str),
            Some('\'') => self.literal_string().map(Item::Str),
            Some('[') => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.blank();
                    if self.peek() == Some(']') {
                        self.at += 1;
                        return Ok(Item::Array(items));
                    }
                    items.push(self.value()?);
                    self.blank();
                    match self.peek() {
                        Some(',') => self.at += 1,
                        Some(']') => {
                            self.at += 1;
                            return Ok(Item::Array(items));
                        }
                        _ => return self.error("expected , or ] in an array"),
                    }
                }
            }
            Some('{') => {
                self.at += 1;
                let mut table = Item::Table(Vec::new());
                self.spaces();
                if self.peek() == Some('}') {
                    self.at += 1;
                    return Ok(table);
                }
                loop {
                    let key = self.key()?;
                    self.spaces();
                    if self.peek() != Some('=') {
                        return self.error("expected = in an inline table");
                    }
                    self.at += 1;
                    let value = self.value()?;
                    set_path(&mut table, &key, value)?;
                    self.spaces();
                    match self.peek() {
                        Some(',') => self.at += 1,
                        Some('}') => {
                            self.at += 1;
                            return Ok(table);
                        }
                        _ => return self.error("expected , or } in an inline table"),
                    }
                }
            }
            Some(_) => {
                let start = self.at;
                while matches!(self.peek(), Some(c) if !matches!(c, ',' | ']' | '}' | '#' | '\n' | '\r' | ' ' | '\t')) {
                    self.at += 1;
                }
                let word: String = self.chars[start..self.at].iter().collect();
                match word.as_str() {
                    "true" => Ok(Item::Bool(true)),
                    "false" => Ok(Item::Bool(false)),
                    "inf" | "+inf" => Ok(Item::Float(f64::INFINITY)),
                    "-inf" => Ok(Item::Float(f64::NEG_INFINITY)),
                    "nan" | "+nan" | "-nan" => Ok(Item::Float(f64::NAN)),
                    _ => {
                        let clean = word.replace('_', "");
                        if let Some(hex) = clean.strip_prefix("0x") {
                            return i64::from_str_radix(hex, 16).map(Item::Int).or_else(|_| self.error(&format!("bad number {word}")));
                        }
                        if let Ok(number) = clean.parse::<i64>() {
                            Ok(Item::Int(number))
                        } else if let Ok(number) = clean.parse::<f64>() {
                            if clean.chars().any(|c| c.is_ascii_digit()) {
                                Ok(Item::Float(number))
                            } else {
                                self.error(&format!("bad value {word}"))
                            }
                        } else {
                            self.error(&format!("bad value {word:?}"))
                        }
                    }
                }
            }
            None => self.error("expected a value"),
        }
    }
}

fn set_path(table: &mut Item, key: &[String], value: Item) -> Result<(), String> {
    let mut current = table;
    for part in &key[..key.len() - 1] {
        current = current.table_mut(part)?;
    }
    current.insert(key[key.len() - 1].clone(), value)
}

/// Read a TOML document.
pub fn parse(text: &str) -> Result<Item, String> {
    let mut parser = Parser { chars: text.chars().collect(), at: 0, line: 1, _text: text };
    let mut root = Item::Table(Vec::new());
    let mut path: Vec<String> = Vec::new();
    loop {
        parser.blank();
        match parser.peek() {
            None => return Ok(root),
            Some('[') => {
                parser.at += 1;
                let array = parser.peek() == Some('[');
                if array {
                    parser.at += 1;
                }
                let key = parser.key()?;
                parser.spaces();
                let closing = if array { "]]" } else { "]" };
                for c in closing.chars() {
                    if parser.peek() != Some(c) {
                        return parser.error("expected ] after a table name");
                    }
                    parser.at += 1;
                }
                parser.line_end()?;
                if array {
                    let mut current = &mut root;
                    for part in &key[..key.len() - 1] {
                        current = current.table_mut(part)?;
                    }
                    let last = key[key.len() - 1].clone();
                    match current {
                        Item::Table(pairs) => match pairs.iter_mut().find(|(name, _)| *name == last) {
                            Some((_, Item::Array(items))) => items.push(Item::Table(Vec::new())),
                            Some(_) => return parser.error(&format!("{last} is not an array of tables")),
                            None => pairs.push((last, Item::Array(vec![Item::Table(Vec::new())]))),
                        },
                        _ => return parser.error("not a table"),
                    }
                } else {
                    let mut current = &mut root;
                    for part in &key {
                        current = current.table_mut(part)?;
                    }
                }
                path = key;
            }
            Some(_) => {
                let key = parser.key()?;
                parser.spaces();
                if parser.peek() != Some('=') {
                    return parser.error("expected = after a key");
                }
                parser.at += 1;
                let value = parser.value()?;
                parser.line_end()?;
                let mut current = &mut root;
                for part in &path {
                    current = current.table_mut(part)?;
                }
                set_path(current, &key, value).map_err(|message| format!("line {}: {message}", parser.line))?;
            }
        }
    }
}

/// The comment lines just above each key (or above its section), as help text: ((section, key), help).
pub fn comments(text: &str) -> Vec<((String, String), String)> {
    let mut found: Vec<((String, String), String)> = Vec::new();
    let mut section_help: Vec<(String, String)> = Vec::new();
    let (mut section, mut pending) = (String::new(), Vec::<String>::new());
    for line in text.lines() {
        let stripped = line.trim();
        if let Some(comment) = stripped.strip_prefix('#') {
            pending.push(comment.trim_start_matches('#').trim().to_string());
            continue;
        }
        if stripped.starts_with('[') {
            if let Some(close) = stripped.find(']') {
                section = stripped[1..close].trim().trim_start_matches('[').to_string();
                if !pending.is_empty() {
                    section_help.push((section.clone(), pending.join(" ")));
                }
            }
        } else if let Some(equals) = stripped.find('=') {
            let key = stripped[..equals].trim();
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !pending.is_empty() {
                found.push(((section.clone(), key.to_string()), pending.join(" ")));
            }
        }
        if !stripped.is_empty() {
            pending.clear();
        }
    }
    for (section, help) in section_help {
        if !found.iter().any(|((name, key), _)| *name == section && key.is_empty()) {
            found.push(((section, String::new()), help));
        }
    }
    found
}

fn is_section_line(line: &str, section: &str) -> bool {
    let trimmed = line.trim();
    match trimmed.strip_prefix('[').and_then(|rest| rest.find(']').map(|close| (rest, close))) {
        Some((rest, close)) => rest[..close].trim() == section && { let after = rest[close + 1..].trim(); after.is_empty() || after.starts_with('#') },
        None => false,
    }
}

fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let equals = trimmed.find('=')?;
    let key = trimmed[..equals].trim();
    (!key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then(|| key)
}

/// The file text with one setting changed in place, added to its section, or added with a new section.
pub fn set_value(text: &str, section: &str, key: &str, value: &Item) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let entry = format!("{key} = {}", value.literal());
    let start = match lines.iter().position(|line| is_section_line(line, section)) {
        Some(start) => start,
        None => return format!("{}\n\n[{section}]\n{entry}\n", text.trim_end_matches('\n')),
    };
    let end = (start + 1..lines.len()).find(|index| lines[*index].trim_start().starts_with('[')).unwrap_or(lines.len());
    let mut last_key = start;
    for index in start + 1..end {
        if key_of(&lines[index]) == Some(key) {
            lines[index] = entry;
            return lines.join("\n") + "\n";
        }
        if key_of(&lines[index]).is_some() {
            last_key = index;
        }
    }
    lines.insert(last_key + 1, entry);
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# PolyKara settings\n[layout]\n# How lines are shown.\nmode = \"pairs\"\nlead_ms = 2000\n\n[lyric]\nsize = 120\n";

    #[test]
    fn reads_the_settings_file() {
        let text = "a = 1\n[video]\nplay_res_x = 1_920 # wide\nratio = 0.5\non = true\nname = 'C:\\path'\nlist = [\n  \"my\",\n  \"jp\", # comment\n]\n[effects.params]\nscale = 125\n[x]\ninline = {a = 1, b = \"two\"}\nmulti = \"\"\"\nline\\nnext\"\"\"\n";
        let doc = parse(text).unwrap();
        assert_eq!(doc.get("video").get("play_res_x"), &Item::Int(1920));
        assert_eq!(doc.get("video").get("ratio"), &Item::Float(0.5));
        assert_eq!(doc.get("video").get("name"), &Item::Str("C:\\path".into()));
        assert_eq!(doc.get("video").get("list").items().len(), 2);
        assert_eq!(doc.get("effects").get("params").get("scale"), &Item::Int(125));
        assert_eq!(doc.get("x").get("inline").get("b"), &Item::Str("two".into()));
        assert_eq!(doc.get("x").get("multi"), &Item::Str("line\nnext".into()));
        assert!(parse("[layout\n").is_err());
        assert!(parse("a = 1\na = 2\n").is_err());
    }

    #[test]
    fn values_change_in_place_and_comments_stay() {
        let text = set_value(SAMPLE, "layout", "mode", &Item::Str("alternate".into()));
        let text = set_value(&text, "lyric", "bold", &Item::Bool(true));
        let text = set_value(&text, "card", "title_size", &Item::Int(100));
        assert!(text.contains("# How lines are shown.\nmode = \"alternate\"\nlead_ms = 2000"));
        assert!(text.contains("[lyric]\nsize = 120\nbold = true"));
        assert!(text.ends_with("[card]\ntitle_size = 100\n"));
        assert_eq!(Item::Array(vec![Item::Str("my".into()), Item::Str("jp".into())]).literal(), "[\"my\", \"jp\"]");
        assert_eq!(Item::Str("say \"hi\"".into()).literal(), "\"say \\\"hi\\\"\"");
        assert_eq!(Item::Float(0.6).literal(), "0.6");
        assert_eq!(Item::Float(1.0).literal(), "1.0");
    }

    #[test]
    fn comments_become_help() {
        let help = comments(SAMPLE);
        assert!(help.contains(&(("layout".into(), "mode".into()), "How lines are shown.".into())));
    }

    #[test]
    fn merging_keeps_unset_defaults() {
        let mut base = parse("[a]\nx = 1\ny = 2\n").unwrap();
        merge(&mut base, &parse("[a]\ny = 3\n[b]\nz = true\n").unwrap());
        assert_eq!(base.get("a").get("x"), &Item::Int(1));
        assert_eq!(base.get("a").get("y"), &Item::Int(3));
        assert_eq!(base.get("b").get("z"), &Item::Bool(true));
    }
}
