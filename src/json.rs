//! JSON for the engine's requests and answers: a small parser and writer, no dependencies.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    /// Keys keep their order.
    Object(Vec<(String, Value)>),
}

pub static NULL: Value = Value::Null;

impl Value {
    pub fn object<K: Into<String>>(pairs: Vec<(K, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(key, value)| (key.into(), value)).collect())
    }

    /// The value under `key`, or null.
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Object(pairs) => pairs.iter().find(|(name, _)| name == key).map(|(_, value)| value).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    pub fn str(&self) -> &str {
        match self {
            Value::String(text) => text,
            _ => "",
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(number) if number.is_finite() => Some(number.round() as i64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            _ => &[],
        }
    }

    pub fn pairs(&self) -> &[(String, Value)] {
        match self {
            Value::Object(pairs) => pairs,
            _ => &[],
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn strings(&self) -> Vec<String> {
        self.items().iter().map(|item| item.str().to_string()).collect()
    }

    pub fn dump(&self) -> String {
        let mut out = String::new();
        write_value(self, &mut out);
        out
    }

    pub fn parse(text: &str) -> Result<Value, String> {
        let mut parser = Parser { bytes: text.as_bytes(), at: 0 };
        let value = parser.value()?;
        parser.space();
        if parser.at != parser.bytes.len() {
            return Err(format!("unexpected text at {}", parser.at));
        }
        Ok(value)
    }
}

impl From<&str> for Value {
    fn from(text: &str) -> Value {
        Value::String(text.to_string())
    }
}

impl From<String> for Value {
    fn from(text: String) -> Value {
        Value::String(text)
    }
}

impl From<&String> for Value {
    fn from(text: &String) -> Value {
        Value::String(text.clone())
    }
}

impl From<i64> for Value {
    fn from(number: i64) -> Value {
        Value::Number(number as f64)
    }
}

impl From<usize> for Value {
    fn from(number: usize) -> Value {
        Value::Number(number as f64)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Value {
        Value::Bool(value)
    }
}

impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Value {
        value.map(Into::into).unwrap_or(Value::Null)
    }
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(items: Vec<T>) -> Value {
        Value::Array(items.into_iter().map(Into::into).collect())
    }
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) if !number.is_finite() => out.push_str("null"),
        Value::Number(number) if number.fract() == 0.0 && number.abs() < 1e15 => {
            let _ = write!(out, "{}", *number as i64);
        }
        Value::Number(number) => {
            let _ = write!(out, "{number}");
        }
        Value::String(text) => write_string(text, out),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(pairs) => {
            out.push('{');
            for (index, (key, item)) in pairs.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(item, out);
            }
            out.push('}');
        }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn space(&mut self) {
        while self.at < self.bytes.len() && matches!(self.bytes[self.at], b' ' | b'\t' | b'\n' | b'\r') {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn expect(&mut self, word: &str) -> Result<(), String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(())
        } else {
            Err(format!("expected {word} at {}", self.at))
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.space();
        match self.peek() {
            Some(b'n') => self.expect("null").map(|_| Value::Null),
            Some(b't') => self.expect("true").map(|_| Value::Bool(true)),
            Some(b'f') => self.expect("false").map(|_| Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.peek() == Some(b']') {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value()?);
                    self.space();
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err(format!("expected , or ] at {}", self.at)),
                    }
                }
            }
            Some(b'{') => {
                self.at += 1;
                let mut pairs = Vec::new();
                self.space();
                if self.peek() == Some(b'}') {
                    self.at += 1;
                    return Ok(Value::Object(pairs));
                }
                loop {
                    self.space();
                    if self.peek() != Some(b'"') {
                        return Err(format!("expected a key at {}", self.at));
                    }
                    let key = self.string()?;
                    self.space();
                    if self.peek() != Some(b':') {
                        return Err(format!("expected : at {}", self.at));
                    }
                    self.at += 1;
                    pairs.push((key, self.value()?));
                    self.space();
                    match self.peek() {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Value::Object(pairs));
                        }
                        _ => return Err(format!("expected , or }} at {}", self.at)),
                    }
                }
            }
            Some(b'-') | Some(b'0'..=b'9') => {
                let start = self.at;
                while self.at < self.bytes.len() && matches!(self.bytes[self.at], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.at += 1;
                }
                let text = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|error| error.to_string())?;
                text.parse::<f64>().map(Value::Number).map_err(|_| format!("bad number at {start}"))
            }
            _ => Err(format!("unexpected character at {}", self.at)),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.bytes.get(self.at..self.at + 4).ok_or("short \\u escape")?;
        let text = std::str::from_utf8(digits).map_err(|error| error.to_string())?;
        self.at += 4;
        u32::from_str_radix(text, 16).map_err(|_| "bad \\u escape".to_string())
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while self.at < self.bytes.len() && self.bytes[self.at] != b'"' && self.bytes[self.at] != b'\\' {
                self.at += 1;
            }
            out.push_str(std::str::from_utf8(&self.bytes[start..self.at]).map_err(|error| error.to_string())?);
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    let escape = self.peek().ok_or("unfinished escape")?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) && self.bytes[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(format!("bad escape at {}", self.at)),
                    }
                }
                _ => return Err("unfinished string".into()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = r#"{"id":3,"ok":true,"result":{"lines":[{"text":"唤 \"x\"\n","start":1000,"end":null}],"r":-1.5e2,"e":[]}}"#;
        let value = Value::parse(text).unwrap();
        assert_eq!(value.get("id").as_i64(), Some(3));
        let token = &value.get("result").get("lines").items()[0];
        assert_eq!(token.get("text").str(), "唤 \"x\"\n");
        assert!(token.get("end").is_null());
        assert_eq!(value.get("result").get("r").as_i64(), Some(-150));
        assert_eq!(Value::parse(&value.dump()).unwrap(), value);
    }

    #[test]
    fn escapes_and_surrogates() {
        assert_eq!(Value::parse(r#""é🎤\t""#).unwrap().str(), "é🎤\t");
        assert_eq!(Value::from("a\u{1}b").dump(), r#""a\u0001b""#);
        assert!(Value::parse("{\"a\":}").is_err());
        assert!(Value::parse("[1,2] x").is_err());
    }

    #[test]
    fn builders() {
        let value = Value::object(vec![("n", Value::from(5i64)), ("s", Value::from(Some("x"))), ("none", Value::from(None::<i64>)), ("list", Value::from(vec!["a", "b"]))]);
        assert_eq!(value.dump(), r#"{"n":5,"s":"x","none":null,"list":["a","b"]}"#);
    }
}
