//! Karaoke subtitles (ASS) from timed lyric lines.

use std::collections::HashMap;

use crate::config::{load_config, DOT_INTERVAL_MS, LONG_PAUSE_MS, MAX_SINGERS, SPEAKER_PALETTE};
use crate::csv::Row;
use crate::manifest::get;
use crate::romanize::romanize;
use crate::subtitle::{Entry, Word};
use crate::text::{is_cjk_wide, split_speaker};
use crate::toml::Item;

/// Per-syllable effect presets: (name, line template, syllable template, default params).
/// Templates are ASS override tags with {start} {mid} {end} {dur} {index} (ms from the event start) and params.
const PRESETS: &[(&str, &str, &str, &[(&str, i64)])] = &[
    ("none", "", "", &[]),
    ("fade", "\\fad({fade_in},{fade_out})", "", &[("fade_in", 200), ("fade_out", 200)]),
    ("pop", "", "\\fscx100\\fscy100\\t({start},{mid},\\fscx{scale}\\fscy{scale})\\t({mid},{end},\\fscx100\\fscy100)", &[("scale", 125)]),
    ("glow", "", "\\blur0\\t({start},{mid},\\blur{blur})\\t({mid},{end},\\blur0)", &[("blur", 6)]),
];

/// (line template, syllable template, params) from [effects].
pub fn effect_templates(config: &Item) -> Result<(String, String, HashMap<String, String>), String> {
    let values = config.get("effects");
    let name = values.str_or("preset", "none").to_lowercase();
    let preset = PRESETS.iter().find(|preset| preset.0 == name).ok_or_else(|| format!("effects.preset must be one of {}", PRESETS.iter().map(|preset| preset.0).collect::<Vec<_>>().join(", ")))?;
    let mut params: HashMap<String, String> = preset.3.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
    for (key, value) in values.get("params").pairs() {
        params.insert(key.clone(), value.text());
    }
    Ok((values.str_or("line", preset.1), values.str_or("syllable", preset.2), params))
}

/// Python's str.format with named fields; {{ and }} are literal braces.
pub fn format_fields(template: &str, fields: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => name.push(c),
                        None => return Err("unclosed {".into()),
                    }
                }
                let key = name.split([':', '!']).next().unwrap_or("").to_string();
                out.push_str(&fields(&key).ok_or_else(|| format!("'{key}'"))?);
            }
            c => out.push(c),
        }
    }
    Ok(out)
}

pub fn apply_template(template: &str, params: &HashMap<String, String>, timing: &[(&str, i64)]) -> Result<String, String> {
    if template.is_empty() {
        return Ok(String::new());
    }
    format_fields(template, &|key| timing.iter().find(|item| item.0 == key).map(|item| item.1.to_string()).or_else(|| params.get(key).cloned())).map_err(|field| format!("Unknown effect template field: {field}"))
}

/// A line's words as karaoke tags: times are whole centiseconds from the line start, so rounding never adds up.
pub fn karaoke_text(words: &[Word], event_start: i64, tag: &str, syllable_template: &str, params: &HashMap<String, String>) -> Result<String, String> {
    let mut parts = String::new();
    let mut cursor = 0i64;
    let mut previous = String::new();
    let round = |value: i64| ((value as f64) / 10.0).round() as i64;
    for (index, (left, right, word)) in words.iter().enumerate() {
        let begin = round(left - event_start).max(0);
        let finish = round(right - event_start).max(0);
        // Words already carry their own trailing space; CJK characters need none.
        let last = previous.chars().last();
        let first = word.chars().next();
        let joined = previous.is_empty() || last.map_or(false, char::is_whitespace) || first.map_or(false, char::is_whitespace) || (last.map_or(false, is_cjk_wide) && first.map_or(false, is_cjk_wide));
        let separator = if joined { "" } else { " " };
        if begin - cursor >= 1 {
            parts.push_str(&format!("{{\\k{}}}", begin - cursor));
            cursor = begin;
        }
        let duration = (finish - cursor).max(1);
        let (rel_start, rel_end) = ((left - event_start).max(0), (right - event_start).max(0));
        let effect = apply_template(syllable_template, params, &[("start", rel_start), ("mid", (rel_start + rel_end) / 2), ("end", rel_end), ("dur", rel_end - rel_start), ("index", index as i64)])?;
        parts.push_str(&format!("{separator}{{\\{tag}{duration}{effect}}}{}", esc(word)));
        cursor += duration;
        previous = word.clone();
    }
    Ok(parts)
}

/// An ASS time, h:mm:ss.cc.
pub fn at(value: i64) -> String {
    let cs = value.max(0) / 10;
    format!("{}:{:02}:{:02}.{:02}", cs / 360000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

pub fn esc(value: &str) -> String {
    // A literal backslash would start an ASS override sequence, so it is shown as a slash.
    value.replace('\\', "/").replace('{', "\\{").replace('}', "\\}").replace('\n', "\\N")
}

pub fn alignment(values: &Item) -> i64 {
    if values.has("alignment") {
        return values.int_or("alignment", 2);
    }
    let horizontal = match values.str_or("horizontal", "center").to_lowercase().as_str() {
        "left" => 0,
        "right" => 2,
        _ => 1,
    };
    let vertical = match values.str_or("vertical", "bottom").to_lowercase().as_str() {
        "middle" | "center" => 3,
        "top" => 6,
        _ => 0,
    };
    vertical + horizontal + 1
}

pub fn intro_padding_ms(config: &Item, entries: &[Entry]) -> i64 {
    if !config.get("video").bool_or("extend_intro", true) || entries.is_empty() {
        return 0;
    }
    let first_lyric = entries.iter().map(|entry| entry.start).min().unwrap_or(0);
    let mut card_end = 0;
    for name in ["title", "credit"] {
        let values = config.get(name);
        if values.bool_or("enabled", true) {
            card_end = card_end.max(values.int_or("start_ms", 0) + values.int_or("duration_ms", 6000).max(0));
        }
    }
    (card_end - first_lyric).max(0)
}

fn style(name: &str, values: &Item, primary: &str, secondary: &str, outline: &str, back: &str) -> String {
    format!(
        "Style: {name},{},{},{},{},{},{},{},{},{},{},100,100,0,0,1,{},{},{},{},{},{},1",
        values.str_or("font", "Arial"),
        values.get("size").text(),
        values.str_or("color", primary),
        values.str_or("secondary_color", secondary),
        values.str_or("outline_color", outline),
        values.str_or("back_color", back),
        values.bool_or("bold", true) as i32,
        values.bool_or("italic", false) as i32,
        values.bool_or("underline", false) as i32,
        values.bool_or("strikeout", false) as i32,
        if values.has("outline") { values.get("outline").text() } else { "2".into() },
        if values.has("shadow") { values.get("shadow").text() } else { "1".into() },
        alignment(values),
        values.get("margin_l").text(),
        values.get("margin_r").text(),
        values.get("margin_v").text(),
    )
}

fn set(table: &mut Item, key: &str, value: Item) {
    if let Item::Table(pairs) = table {
        match pairs.iter_mut().find(|(name, _)| name == key) {
            Some(pair) => pair.1 = value,
            None => pairs.push((key.to_string(), value)),
        }
    }
}

/// The karaoke subtitle file for a song.
pub fn ass(row: &Row, entries: &[Entry]) -> Result<String, String> {
    let config = load_config();
    ass_with(&config, row, entries)
}

pub fn ass_with(config: &Item, row: &Row, entries: &[Entry]) -> Result<String, String> {
    let video = config.get("video");
    let (title_style, credit_style) = (config.get("title"), config.get("credit"));
    let mut lyric_style = config.get("lyric").clone();
    let karaoke = config.get("karaoke");
    let active = karaoke.str_or("active_color", &lyric_style.str_or("color", "&H00FF0000"));
    let inactive = karaoke.str_or("inactive_color", &lyric_style.str_or("secondary_color", "&H00FFFFFF"));
    set(&mut lyric_style, "color", Item::Str(active));
    set(&mut lyric_style, "secondary_color", Item::Str(inactive));
    let (prompt_style, watermark_style, romanization_style) = (config.get("prompt"), config.get("watermark"), config.get("romanization"));
    let text_template = |values: &Item, fallback: &str| {
        format_fields(&values.str_or("text", fallback), &|key| row.get(key).cloned()).map_err(|field| format!("Unknown title/credit template field: {}", field.trim_matches('\'')))
    };
    let title = esc(&text_template(title_style, "{title}  |  {artist}")?);
    let credit = esc(&text_template(credit_style, "作词：{lyricist}    作曲：{composer}    字幕制作：{producer}")?);
    let mut out: Vec<String> = vec![
        "[Script Info]".into(),
        "ScriptType: v4.00+".into(),
        format!("PlayResX: {}", video.get("play_res_x").text()),
        format!("PlayResY: {}", video.get("play_res_y").text()),
        "WrapStyle: 2".into(),
        "ScaledBorderAndShadow: yes".into(),
        String::new(),
        "[V4+ Styles]".into(),
        "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding".into(),
        style("Header", title_style, "&H00FFFFFF", "&H00FFFFFF", "&H80000000", "&H50000000"),
        style("Credit", credit_style, "&H00FFFFFF", "&H00FFFFFF", "&H80000000", "&H50000000"),
        style("Lyric", &lyric_style, "&H00FF0000", "&H00FFFFFF", "&H80000000", "&H50000000"),
        style("Romanization", romanization_style, "&H00FFFFFF", "&H00FFFFFF", "&H80000000", "&H50000000"),
        style("Prompt", prompt_style, "&H00FFFFFF", "&H00FFFFFF", "&H80000000", "&H50000000"),
        style("Watermark", watermark_style, "&H80FFFFFF", "&H00FFFFFF", "&H80000000", "&H50000000"),
        String::new(),
        "[Events]".into(),
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text".into(),
    ];
    let card_event = |style_name: &str, values: &Item, text: &str| -> Option<String> {
        let start = values.int_or("start_ms", 0).max(0);
        let end = start + values.int_or("duration_ms", 6000).max(0);
        if end <= start {
            return None;
        }
        let fade_in = values.int_or("fade_in_ms", 0).max(0).min(end - start);
        let fade_out = values.int_or("fade_out_ms", 0).max(0).min(end - start - fade_in);
        let animation = if fade_in > 0 || fade_out > 0 { format!("{{\\fad({fade_in},{fade_out})}}") } else { String::new() };
        Some(format!("Dialogue: 0,{},{},{style_name},,0,0,0,,{animation}{text}", at(start), at(end)))
    };
    if title_style.bool_or("enabled", true) {
        out.extend(card_event("Header", title_style, &title));
    }
    if credit_style.bool_or("enabled", true) {
        out.extend(card_event("Credit", credit_style, &credit));
    }
    if watermark_style.bool_or("enabled", false) && !watermark_style.str_or("text", "").is_empty() {
        out.push(format!("Dialogue: 1,0:00:00.00,9:59:59.99,Watermark,,0,0,0,,{}", esc(&watermark_style.str_or("text", ""))));
    }
    let intro_padding = intro_padding_ms(config, entries);
    out.insert(1, format!("; PolyKaraIntroPaddingMs: {intro_padding}"));
    let mut speakers: Vec<String> = Vec::new();
    for entry in entries {
        let (speaker, _) = split_speaker(&entry.text);
        if !speaker.is_empty() && !speakers.iter().any(|item| item.to_lowercase() == speaker.to_lowercase()) {
            speakers.push(speaker);
        }
    }
    let max_singers = config.get("singers").int_or("max", MAX_SINGERS as i64).max(0) as usize;
    if speakers.len() > max_singers {
        return Err(format!("{} has {} singers; maximum is {max_singers}", get(row, "id"), speakers.len()));
    }
    let slots: HashMap<String, &str> = speakers.iter().enumerate().map(|(index, speaker)| (speaker.to_lowercase(), SPEAKER_PALETTE[index % SPEAKER_PALETTE.len()])).collect();
    let timing = config.get("timing");
    let long_pause_ms = timing.int_or("long_pause_ms", LONG_PAUSE_MS);
    let dot_interval_ms = timing.int_or("dot_interval_ms", DOT_INTERVAL_MS);
    let karaoke_tag = karaoke.str_or("tag", "kf").to_lowercase();
    if karaoke_tag != "k" && karaoke_tag != "kf" {
        return Err("karaoke.tag must be 'k' or 'kf'".into());
    }
    let (line_template, syllable_template, effect_params) = effect_templates(config)?;
    let layout = config.get("layout");
    let mode = layout.str_or("mode", "single").to_lowercase();
    // "pairs" came from the KTV layout of 0.3.x settings files; alternate is the closest.
    let mode = if mode == "pairs" { "alternate".to_string() } else { mode };
    if mode != "single" && mode != "alternate" {
        return Err("layout.mode must be 'single' or 'alternate'".into());
    }
    let alternate = mode == "alternate";
    let lead_in_ms = if alternate { layout.int_or("lead_in_ms", 1500).max(0) } else { 0 };
    let lyric_margin_v = lyric_style.int_or("margin_v", 150);
    let roman_margin = romanization_style.get("margin_v");
    let romanization_gap = if roman_margin.text().eq_ignore_ascii_case("auto") { 75 } else { romanization_style.int_or("margin_v", 225) - lyric_margin_v };
    let rows_layout = [
        (layout.int_or("top_alignment", 1), layout.int_or("top_margin_v", lyric_margin_v + 130)),
        (layout.int_or("bottom_alignment", 3), layout.int_or("bottom_margin_v", lyric_margin_v)),
    ];
    let mut row_free = [0i64, 0];
    let mut previous_end = 0;
    let semantic = |speaker: &str| match speaker.to_lowercase().as_str() {
        "duet" | "both" | "shared" => Some("&H00FFFFFF"),
        "backing" => Some("&H0000FF00"),
        _ => None,
    };
    for (index, entry) in entries.iter().enumerate() {
        let start = entry.start + intro_padding;
        let end = entry.end + intro_padding;
        let words: Vec<Word> = entry.words.iter().map(|(left, right, word)| (left + intro_padding, right + intro_padding, word.clone())).collect();
        let (speaker, lyric_text) = split_speaker(&entry.text);
        if start - previous_end >= long_pause_ms {
            for count in 1..4 {
                let dot_start = start - (4 - count) * dot_interval_ms;
                out.push(format!("Dialogue: 2,{},{},Prompt,,0,0,220,,{}", at(dot_start), at(dot_start + dot_interval_ms), ".".repeat(count as usize)));
            }
        }
        let slot = if alternate { index % 2 } else { 0 };
        // Alternate mode shows a line early, but never before its row is free.
        let shown = if alternate { start.min(row_free[slot].max(start - lead_in_ms)) } else { start };
        row_free[slot] = end;
        let mut visible = if words.is_empty() { esc(&lyric_text) } else { karaoke_text(&words, shown, &karaoke_tag, &syllable_template, &effect_params)? };
        if !speaker.is_empty() {
            let colour = semantic(&speaker).or_else(|| slots.get(&speaker.to_lowercase()).copied()).unwrap_or("&H00FFFFFF");
            visible = format!("{{\\c{colour}}}{visible}");
        }
        let line_effect = apply_template(&line_template, &effect_params, &[("start", 0), ("mid", (end - shown) / 2), ("end", end - shown), ("dur", end - shown), ("index", index as i64)])?;
        let (align_tag, margin_v, prefix) = if alternate {
            let (align_tag, margin_v) = rows_layout[slot];
            (align_tag, margin_v, format!("{{\\an{align_tag}{line_effect}}}"))
        } else {
            (0, lyric_margin_v, if line_effect.is_empty() { String::new() } else { format!("{{{line_effect}}}") })
        };
        if let Some(romanized) = romanize(&lyric_text, get(row, "language"), romanization_style) {
            let roman_prefix = if alternate { format!("{{\\an{align_tag}}}") } else { String::new() };
            let roman_margin = if alternate { margin_v + romanization_gap } else { 0 };
            out.push(format!("Dialogue: 1,{},{},Romanization,,0,0,{roman_margin},,{roman_prefix}{}", at(shown), at(end), esc(&romanized)));
        }
        out.push(format!("Dialogue: 0,{},{},Lyric,,0,0,{margin_v},,{prefix}{visible}", at(shown), at(end)));
        previous_end = previous_end.max(end);
    }
    Ok(out.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Row {
        [("id", "song"), ("title", "T"), ("artist", "A"), ("lyricist", "L"), ("composer", "C"), ("producer", "P"), ("language", "en")].iter().map(|(key, value)| (key.to_string(), value.to_string())).collect()
    }

    #[test]
    fn karaoke_tags_count_centiseconds() {
        let words: Vec<Word> = vec![(1000, 1300, "Hello ".into()), (1400, 1800, "world".into())];
        assert_eq!(karaoke_text(&words, 1000, "kf", "", &HashMap::new()).unwrap(), "{\\kf30}Hello {\\k10}{\\kf40}world");
        let cjk: Vec<Word> = vec![(0, 100, "你".into()), (100, 200, "好".into())];
        assert_eq!(karaoke_text(&cjk, 0, "k", "", &HashMap::new()).unwrap(), "{\\k10}你{\\k10}好");
        assert_eq!(at(3_723_456), "1:02:03.45");
        assert_eq!(esc("a\\b{c}\nd"), "a/b\\{c\\}\\Nd");
    }

    #[test]
    fn subtitle_file_has_styles_cards_and_lines() {
        let config = crate::config::merged("[title]\nenabled = false\n[credit]\nenabled = false\n").unwrap();
        let entries = vec![Entry::new(1000, 2000, "[speaker:女] Hi there", vec![(1000, 1500, "Hi ".into()), (1500, 2000, "there".into())])];
        let text = ass_with(&config, &row(), &entries).unwrap();
        assert!(text.contains("; PolyKaraIntroPaddingMs: 0"));
        assert!(text.contains("Style: Lyric,Arial,58,&H00FF0000,&H00FFFFFF,&H80000000,&H50000000,1,0,0,0,100,100,0,0,1,3,1,2,80,80,150,1"));
        assert!(text.contains("Dialogue: 0,0:00:01.00,0:00:02.00,Lyric,,0,0,150,,{\\c&H00B4771F}{\\kf50}Hi {\\kf50}there"));
        let with_cards = crate::config::merged("").unwrap();
        let padded = ass_with(&with_cards, &row(), &entries).unwrap();
        assert!(padded.contains("; PolyKaraIntroPaddingMs: 5000"));
        assert!(padded.contains("Dialogue: 0,0:00:00.00,0:00:06.00,Header,,0,0,0,,{\\fad(300,300)}T  |  A"));
    }

    #[test]
    fn templates_and_effects() {
        assert_eq!(format_fields("{title}  |  {{x}}", &|key| (key == "title").then(|| "T".to_string())).unwrap(), "T  |  {x}");
        assert!(format_fields("{nope}", &|_| None).is_err());
        let config = crate::config::merged("[effects]\npreset = \"pop\"\n[effects.params]\nscale = 130\n").unwrap();
        let (_, syllable, params) = effect_templates(&config).unwrap();
        assert!(apply_template(&syllable, &params, &[("start", 0), ("mid", 5), ("end", 10), ("dur", 10), ("index", 0)]).unwrap().contains("\\fscx130"));
        assert!(effect_templates(&crate::config::merged("[effects]\npreset = \"x\"\n").unwrap()).is_err());
    }
}
