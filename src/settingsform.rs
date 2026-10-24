//! polykara.toml as form fields: which settings exist, how each is edited, and
//! in-place changes that keep the file's comments and layout.

use crate::config;
use crate::json::Value;
use crate::toml::{self, Item};

/// (tab, sections) in the order shown.
pub const TABS: &[(&str, &[&str])] = &[
    ("Lyrics", &["lyric", "karaoke", "singers"]),
    ("Readings", &["romanization"]),
    ("Layout", &["layout", "effects", "timing", "prompt"]),
    ("Title and credits", &["title", "credit"]),
    ("Video", &["video", "render", "logo", "watermark"]),
    ("Lyrics search", &["lyrics", "alignment", "charts"]),
    ("Player and live", &["player", "live"]),
];

pub fn section_name(name: &str) -> &str {
    match name {
        "lyric" => "Lyric text",
        "karaoke" => "Sung colours",
        "singers" => "Singers and voice colours",
        "romanization" => "Pinyin / romaji above the characters",
        "layout" => "Lines on screen",
        "effects" => "Karaoke effects",
        "player" => "Music player",
        "live" => "Live output",
        "countdown" => "Countdown dots",
        "timing" => "Pauses and timing",
        "prompt" => "Pause prompt",
        "card" => "Opening card (KTV layouts)",
        "title" => "Title",
        "credit" => "Credits",
        "video" => "Frame size",
        "render" => "Video encoding",
        "logo" => "Logo",
        "watermark" => "Watermark text",
        "lyrics" => "Lyric sources and accuracy check",
        "alignment" => "Automatic word timing (Whisper)",
        "charts" => "YouTube charts",
        other => other,
    }
}

fn choices(section: &str, key: &str) -> Option<&'static [&'static str]> {
    let list = match (section, key) {
        ("layout", "mode") => &["single", "alternate"][..],
        ("effects", "preset") => &["none", "fade", "pop", "glow"][..],
        ("live", "audio_output") => &["none", "pulse", "alsa"][..],
        ("render", "fit") => &["blur", "pad", "none"][..],
        ("render", "preset") => &["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow", ""][..],
        ("render", "video_codec") => &["libx264", "libx265", "h264_nvenc", "hevc_nvenc", "h264_qsv", "h264_amf"][..],
        ("render", "audio_codec") => &["aac", "libmp3lame", "copy"][..],
        ("logo", "position") => &["top-left", "top-right", "bottom-left", "bottom-right", "center"][..],
        ("romanization", "layout") => &["ruby", "rows"][..],
        ("romanization", "tone") => &["marks", "numbers", "none"][..],
        ("karaoke", "tag") => &["kf", "k", "ko"][..],
        ("alignment", "model") => &["auto", "tiny", "base", "small", "medium", "large-v3"][..],
        ("alignment", "device") => &["auto", "cpu", "cuda"][..],
        ("alignment", "compute_type") => &["int8", "int8_float16", "float16", "float32"][..],
        _ => match key {
            "horizontal" => &["left", "center", "right"][..],
            "vertical" => &["top", "middle", "bottom"][..],
            _ => return None,
        },
    };
    Some(list)
}

fn colour_words(section: &str, key: &str) -> Vec<String> {
    if (section, key) == ("card", "title_outline_color") {
        vec!["voice".into()]
    } else {
        Vec::new()
    }
}

pub fn label_for(key: &str) -> String {
    let text = key.replace("_ms", " (ms)").replace("color", "colour").replace('_', " ");
    match text.chars().next() {
        Some(first) => first.to_uppercase().collect::<String>() + &text[first.len_utf8()..],
        None => text,
    }
}

fn field_kind(section: &str, key: &str, value: &Item) -> (&'static str, Vec<String>) {
    match value {
        Item::Bool(_) => return ("bool", Vec::new()),
        Item::Array(_) => return ("list", Vec::new()),
        Item::Int(_) | Item::Float(_) => return ("number", Vec::new()),
        _ => {}
    }
    if let Some(list) = choices(section, key) {
        return ("choice", list.iter().map(|item| item.to_string()).collect());
    }
    if key == "font" {
        return ("font", Vec::new());
    }
    if key.ends_with("color") || ass_to_hex(&value.text()).is_some() {
        return ("colour", colour_words(section, key));
    }
    if key == "file" {
        return ("file", Vec::new());
    }
    ("text", Vec::new())
}

/// A value as it is shown in its field: lists as comma-separated text.
fn shown(value: &Item) -> Value {
    match value {
        Item::Bool(value) => Value::Bool(*value),
        Item::Array(items) => Value::from(items.iter().map(Item::text).collect::<Vec<_>>().join(", ")),
        Item::Int(number) => Value::from(*number),
        Item::Float(number) => Value::Number(*number),
        other => Value::from(other.text()),
    }
}

fn help_for(help: &[((String, String), String)], section: &str, key: &str) -> String {
    help.iter()
        .find(|((name, field), _)| name == section && field == key)
        .or_else(|| help.iter().find(|((name, field), _)| name == section && field.is_empty()))
        .map(|(_, text)| text.clone())
        .unwrap_or_default()
}

fn section_json(values: &Item, help: &[((String, String), String)], name: &str) -> Value {
    let mut fields = Vec::new();
    for (key, value) in values.get(name).pairs() {
        if value.is_table() {
            continue;
        }
        let (kind, offered) = field_kind(name, key, value);
        let mut note = help_for(help, name, key);
        if kind == "colour" {
            if !note.is_empty() {
                note.push(' ');
            }
            note.push_str("Colours are written &H<alpha><blue><green><red> in hex; alpha 00 is solid, FF invisible.");
        }
        fields.push(Value::object(vec![
            ("key", Value::from(key.as_str())),
            ("label", Value::from(label_for(key))),
            ("kind", Value::from(kind)),
            ("choices", Value::Array(offered.into_iter().map(Value::from).collect())),
            ("value", shown(value)),
            ("help", Value::from(note)),
            ("wide", Value::Bool(key == "text")),
        ]));
    }
    Value::object(vec![("id", Value::from(name)), ("title", Value::from(section_name(name))), ("fields", Value::Array(fields))])
}

/// Every known setting as tabs of sections of fields, with the file's comments as help.
pub fn form(text: &str) -> Result<Vec<Value>, String> {
    let values = config::merged(text)?;
    let help = toml::comments(text);
    let mut tabs = Vec::new();
    let mut placed: Vec<String> = Vec::new();
    for (name, sections) in TABS {
        let present: Vec<&str> = sections.iter().copied().filter(|item| values.has(item) && values.get(item).is_table()).collect();
        placed.extend(present.iter().map(|item| item.to_string()));
        let sections: Vec<Value> = present.into_iter().map(|item| section_json(&values, &help, item)).collect();
        tabs.push(Value::object(vec![("name", Value::from(*name)), ("sections", Value::Array(sections))]));
    }
    let others: Vec<&str> = values
        .pairs()
        .iter()
        .filter(|(name, value)| value.is_table() && !placed.contains(name))
        .map(|(name, _)| name.as_str())
        .collect();
    if !others.is_empty() {
        let sections: Vec<Value> = others.into_iter().map(|item| section_json(&values, &help, item)).collect();
        tabs.push(Value::object(vec![("name", Value::from("Other")), ("sections", Value::Array(sections))]));
    }
    Ok(tabs)
}

/// A field's text read back as a value of the setting's type.
fn field_value(section: &str, key: &str, raw: &Value, original: &Item) -> Result<Item, String> {
    if let Value::Bool(value) = raw {
        return Ok(Item::Bool(*value));
    }
    if let Value::Number(number) = raw {
        return Ok(if matches!(original, Item::Int(_)) { Item::Int(*number as i64) } else { Item::Float(*number) });
    }
    let text = raw.str().to_string();
    match original {
        Item::Bool(_) => Ok(Item::Bool(!text.trim().is_empty() && text.trim() != "false")),
        Item::Array(old) => {
            let all_int = !old.is_empty() && old.iter().all(|item| matches!(item, Item::Int(_)));
            let items = text
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| {
                    if all_int && is_integer(item) {
                        Item::Int(item.parse().unwrap_or(0))
                    } else {
                        Item::Str(item.to_string())
                    }
                })
                .collect();
            Ok(Item::Array(items))
        }
        Item::Int(_) => text.trim().parse::<i64>().map(Item::Int).map_err(|_| format!("[{section}] {} must be a number", label_for(key))),
        Item::Float(_) => text.trim().parse::<f64>().map(Item::Float).map_err(|_| format!("[{section}] {} must be a number", label_for(key))),
        _ => Ok(Item::Str(text)),
    }
}

fn is_integer(text: &str) -> bool {
    let rest = text.strip_prefix('-').unwrap_or(text);
    !rest.is_empty() && rest.chars().all(|character| character.is_ascii_digit())
}

/// The file text with each changed field, [section, key, shown value], written in.
pub fn apply_changes(text: &str, changes: &[Value]) -> Result<String, String> {
    let values = config::merged(text)?;
    let mut text = text.to_string();
    for change in changes {
        let parts = change.items();
        if parts.len() < 3 {
            return Err("a change is [section, key, value]".into());
        }
        let (section, key) = (parts[0].str(), parts[1].str());
        let original = values.get(section).get(key);
        let value = field_value(section, key, &parts[2], original)?;
        if value != *original {
            text = toml::set_value(&text, section, key, &value);
        }
    }
    Ok(text)
}

/// &HAABBGGRR as #rrggbb, or None when it is not an ASS colour.
pub fn ass_to_hex(value: &str) -> Option<String> {
    let text = value.trim();
    let digits = text.strip_prefix("&H")?;
    if digits.len() != 8 || !digits.chars().all(|character| character.is_ascii_hexdigit()) {
        return None;
    }
    let blue = &digits[2..4];
    let green = &digits[4..6];
    let red = &digits[6..8];
    Some(format!("#{}{}{}", red.to_lowercase(), green.to_lowercase(), blue.to_lowercase()))
}

/// #rrggbb as an ASS colour with the given alpha.
pub fn hex_to_ass(colour: &str, alpha: &str) -> String {
    let red = colour.get(1..3).unwrap_or("00");
    let green = colour.get(3..5).unwrap_or("00");
    let blue = colour.get(5..7).unwrap_or("00");
    format!("&H{}{}{}{}", alpha, blue, green, red).to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ass_colours_round_trip() {
        assert_eq!(ass_to_hex("&H00112233").as_deref(), Some("#332211"));
        assert_eq!(ass_to_hex("#332211"), None);
        assert_eq!(hex_to_ass("#332211", "00"), "&H00112233");
    }

    #[test]
    fn labels_read_like_the_settings_window() {
        assert_eq!(label_for("margin_v"), "Margin v");
        assert_eq!(label_for("lead_ms"), "Lead (ms)");
        assert_eq!(label_for("active_color"), "Active colour");
    }

    #[test]
    fn the_form_lists_the_tabs() {
        let tabs = form("").unwrap();
        assert_eq!(tabs.len(), 7);
        assert_eq!(tabs[0].get("name").str(), "Lyrics");
        assert_eq!(tabs[6].get("name").str(), "Player and live");
        assert!(tabs[0].get("sections").items().len() >= 3);
        let fields = tabs[0].get("sections").items()[0].get("fields").items();
        assert!(fields.iter().any(|field| field.get("key").str() == "font"));
        let colour = fields.iter().find(|field| field.get("key").str() == "color").unwrap();
        assert_eq!(colour.get("kind").str(), "colour");
    }

    #[test]
    fn changes_are_written_in_place() {
        let text = "[layout]\nmargin_v = 60\n";
        let changed = apply_changes(text, &[Value::Array(vec![Value::from("layout"), Value::from("margin_v"), Value::from("80")])]).unwrap();
        assert!(changed.contains("margin_v = 80"));
        assert!(changed.contains("[layout]"));
    }
}
