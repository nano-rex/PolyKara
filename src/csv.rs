//! songs.csv: reading and writing CSV the way Python's csv module does (minimal quoting, \r\n lines).

use std::collections::HashMap;

pub type Row = HashMap<String, String>;

/// Every record of a CSV text as a list of fields.
pub fn records(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                any = false;
            }
            _ => field.push(c),
        }
    }
    if any {
        record.push(field);
        records.push(record);
    }
    records
}

/// (header, rows) with each row keyed by the header; short rows get empty values, extra values are dropped.
pub fn read(text: &str) -> (Vec<String>, Vec<Row>) {
    let mut all = records(text).into_iter();
    let header = all.next().unwrap_or_default();
    let rows = all
        .filter(|record| !(record.len() == 1 && record[0].is_empty()))
        .map(|record| header.iter().enumerate().map(|(index, name)| (name.clone(), record.get(index).cloned().unwrap_or_default())).collect())
        .collect();
    (header, rows)
}

fn field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn write(fields: &[String], rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str(&fields.iter().map(|name| field(name)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    for row in rows {
        out.push_str(&fields.iter().map(|name| field(row.get(name).map(String::as_str).unwrap_or(""))).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let text = "\u{feff}id,title,url\r\nsong,\"Hello, \"\"world\"\"\",https://x\r\nshort,T\r\n";
        let (fields, rows) = read(text);
        assert_eq!(fields, vec!["id", "title", "url"]);
        assert_eq!(rows[0]["title"], "Hello, \"world\"");
        assert_eq!(rows[1]["url"], "");
        let written = write(&fields, &rows);
        assert_eq!(read(&written).1, rows);
        assert!(written.starts_with("id,title,url\r\nsong,\"Hello, \"\"world\"\"\",https://x\r\n"));
    }

    #[test]
    fn multiline_fields_and_no_final_newline() {
        let (_, rows) = read("a,b\n\"x\ny\",2");
        assert_eq!(rows[0]["a"], "x\ny");
        assert_eq!(rows[0]["b"], "2");
    }
}
