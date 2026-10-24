//! The engine behind the desktop app: lyrics, readings, settings and the song
//! list, over JSON lines.
//!
//! Each request is one JSON line on stdin, {"id": 1, "cmd": "load", "args": {}},
//! and each answer one line on stdout, {"id": 1, "ok": true, "result": ...}.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::{self, paths};
use crate::csv::{self, Row};
use crate::json::Value;
use crate::localmedia;
use crate::manifest::{self, get};
use crate::model::{Line, Token};
use crate::readings;
use crate::romanize;
use crate::settingsform;
use crate::timing;

const LIBRARY_NAMES: &[(&str, &str)] = &[("zh", "pinyin"), ("yue", "Jyutping"), ("ja", "romaji")];
const LIBRARY_HINTS: &[(&str, &str)] = &[
    ("zh", "one syllable per character: 重来 = chóng lái (or chong2 lai2)"),
    ("yue", "one syllable per character: 銀行 = ngan4 hong4"),
    ("ja", "the whole word in kana or romaji: 君 = きみ"),
];

// ------------------------------------------------------------- the song list

pub fn read_songs() -> (Vec<String>, Vec<Row>) {
    let manifest = paths().manifest.clone();
    if !manifest.is_file() {
        return (manifest::COLUMNS.iter().map(|name| name.to_string()).collect(), Vec::new());
    }
    let text = match std::fs::read_to_string(&manifest) {
        Ok(text) => text.trim_start_matches('\u{feff}').to_string(),
        Err(_) => return (manifest::COLUMNS.iter().map(|name| name.to_string()).collect(), Vec::new()),
    };
    let (mut fields, songs) = csv::read(&text);
    for name in manifest::COLUMNS {
        if !fields.iter().any(|field| field == name) {
            fields.push((*name).to_string());
        }
    }
    (fields, songs)
}

fn find_row(sid: &str) -> Result<Row, String> {
    read_songs().1.into_iter().find(|song| get(song, "id") == sid).ok_or_else(|| format!("{sid} is not in songs.csv"))
}

fn row_json(row: &Row) -> Value {
    Value::Object(row.iter().map(|(key, value)| (key.clone(), Value::from(value))).collect())
}

fn rows_json(rows: &[Row]) -> Value {
    Value::Array(rows.iter().map(row_json).collect())
}

// ------------------------------------------------------------------- lines

fn line_out(line: &Line) -> Value {
    let tokens = line
        .tokens
        .iter()
        .map(|token| Value::object(vec![("text", Value::from(&token.text)), ("start", token.start.map(Value::from).unwrap_or(Value::Null)), ("end", token.end.map(Value::from).unwrap_or(Value::Null))]))
        .collect();
    Value::object(vec![
        ("speaker", Value::from(&line.speaker)),
        ("start", line.start.map(Value::from).unwrap_or(Value::Null)),
        ("end", line.end.map(Value::from).unwrap_or(Value::Null)),
        ("tokens", Value::Array(tokens)),
    ])
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => Some(*number as i64),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn line_in(data: &Value) -> Result<Line, String> {
    let tokens = data
        .get("tokens")
        .items()
        .iter()
        .map(|token| Token { text: token.get("text").str().to_string(), start: integer(token.get("start")), end: integer(token.get("end")) })
        .collect();
    Ok(Line {
        tokens,
        speaker: data.get("speaker").str().to_string(),
        start: integer(data.get("start")),
        end: integer(data.get("end")),
    })
}

fn lines_in(args: &Value) -> Result<Vec<Line>, String> {
    args.get("lines").items().iter().map(line_in).collect()
}

/// The reading above each token: a string over the first token of a group, ""
/// over the rest, None for no reading.
fn token_readings(tokens: &[String], language: &str) -> Vec<Value> {
    let settings = config::load_config();
    let romanization = settings.get("romanization").clone();
    let words: Vec<(i64, i64, String)> = tokens.iter().map(|token| (0, 0, token.clone())).collect();
    let groups = romanize::ruby_groups(&words, language, &romanization).unwrap_or_default();
    let mut shown: Vec<Value> = Vec::new();
    for group in groups {
        shown.push(group.roman.map(Value::from).unwrap_or(Value::Null));
        for _ in 1..group.tokens.len() {
            shown.push(Value::from(""));
        }
    }
    if shown.len() == tokens.len() {
        shown
    } else {
        vec![Value::Null; tokens.len()]
    }
}

// ----------------------------------------------------------------- commands

fn hello() -> Value {
    let paths = paths();
    let tools = ["ffmpeg", "ffprobe", "ffplay", "yt-dlp"]
        .iter()
        .map(|name| (name.to_string(), config::find_tool(name).map(|path| Value::from(path.display().to_string())).unwrap_or(Value::Null)))
        .collect();
    Value::object(vec![
        ("version", Value::from(env!("CARGO_PKG_VERSION"))),
        ("home", Value::from(paths.home.display().to_string())),
        ("manifest", Value::from(paths.manifest.display().to_string())),
        ("config", Value::from(paths.config.display().to_string())),
        (
            "folders",
            Value::object(vec![
                ("raw", Value::from(paths.raw.display().to_string())),
                ("audio", Value::from(paths.audio.display().to_string())),
                ("ass", Value::from(paths.ass.display().to_string())),
                ("output", Value::from(paths.output.display().to_string())),
                ("lyrics", Value::from(paths.lyrics.display().to_string())),
            ]),
        ),
        ("media_suffixes", Value::Array(config::MEDIA_SUFFIXES.iter().map(|item| Value::from(*item)).collect())),
        ("tools", Value::Object(tools)),
        ("path", Value::from(std::env::var("PATH").unwrap_or_default())),
    ])
}

fn songs() -> Value {
    let (fields, songs) = read_songs();
    Value::object(vec![("fields", Value::Array(fields.into_iter().map(Value::from).collect())), ("songs", rows_json(&songs))])
}

fn save_songs(args: &Value) -> Result<Value, String> {
    let fields: Vec<String> = args.get("fields").items().iter().map(|item| item.str().to_string()).collect();
    let rows: Vec<Row> = args
        .get("songs")
        .items()
        .iter()
        .map(|song| song.pairs().iter().map(|(key, value)| (key.clone(), value.str().to_string())).collect())
        .collect();
    manifest::write_songs(&fields, &rows)?;
    Ok(songs())
}

fn taken(args: &Value) -> HashSet<String> {
    args.get("taken").items().iter().map(|item| item.str().to_string()).collect()
}

fn describe_file(args: &Value) -> Value {
    let path = PathBuf::from(args.get("path").str());
    row_json(&localmedia::describe(&path, &taken(args)))
}

fn add_media(args: &Value) -> Result<Value, String> {
    let paths_in: Vec<PathBuf> = args.get("paths").items().iter().map(|item| PathBuf::from(item.str())).collect();
    let added = localmedia::add_files(&paths_in)?;
    let mut pairs: Vec<(String, Value)> = vec![
        ("added".to_string(), Value::Array(added.iter().map(|row| Value::from(get(row, "id"))).collect())),
        ("skipped".to_string(), Value::from(paths_in.len().saturating_sub(added.len()))),
    ];
    if let Value::Object(more) = songs() {
        pairs.extend(more);
    }
    Ok(Value::Object(pairs))
}

fn make_id(args: &Value) -> Value {
    Value::object(vec![("id", Value::from(localmedia::new_id(args.get("title").str(), &taken(args))))])
}

fn relative(args: &Value) -> Value {
    Value::object(vec![("path", Value::from(localmedia::relative_to_home(&PathBuf::from(args.get("path").str()))))])
}

fn load(args: &Value) -> Result<Value, String> {
    let row = find_row(args.get("id").str())?;
    let sid = get(&row, "id").to_string();
    let (lines, label, problem) = match timing::load(&row) {
        Ok((lines, label)) => (lines, label, String::new()),
        Err(error) => (Vec::new(), String::new(), format!("The lyrics of {sid} could not be read ({error}). Paste them with Lyrics text….")),
    };
    let mut media = timing::media_for(&sid);
    if media.is_none() && localmedia::is_local(get(&row, "url")) {
        let local = localmedia::local_path(get(&row, "url"));
        if local.is_file() {
            media = Some(local);
        }
    }
    let language = romanize::base_language(get(&row, "language"));
    let karaoke = config::load_config();
    let sung = settingsform::ass_to_hex(&karaoke.get("karaoke").str_or("active_color", "")).unwrap_or_else(|| "#ff0000".into());
    Ok(Value::object(vec![
        ("row", row_json(&row)),
        ("lines", Value::Array(lines.iter().map(line_out).collect())),
        ("label", Value::from(label)),
        ("problem", Value::from(problem)),
        ("media", media.map(|path| Value::from(path.display().to_string())).unwrap_or(Value::Null)),
        ("language", Value::from(&language)),
        ("sung_colour", Value::from(sung)),
        ("edited_ass", Value::from(paths().ass.join(format!("{sid}.edited.ass")).display().to_string())),
        ("library", Value::Bool(readings::LANGUAGES.contains(&language.as_str()))),
    ]))
}

fn prepare_audio(args: &Value) -> Result<Value, String> {
    let sid = args.get("id").str();
    let mut media = PathBuf::from(args.get("media").str());
    let audio = paths().audio.join(format!("{sid}.flac"));
    if media != audio && localmedia::probe(&media).video {
        std::fs::create_dir_all(paths().audio.clone()).map_err(|error| error.to_string())?;
        let partial = paths().audio.join(format!("{sid}.partial.flac"));
        let mut command = Command::new(config::tool("ffmpeg"));
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&media)
            .args(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "flac"])
            .arg(&partial)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        crate::tools::quiet(&mut command);
        let _ = command.status();
        if partial.exists() && std::fs::metadata(&partial).map(|meta| meta.len() > 0).unwrap_or(false) {
            let _ = std::fs::rename(&partial, &audio);
            media = audio;
        }
        let _ = std::fs::remove_file(&partial);
    }
    let duration = (localmedia::probe(&media).duration * 1000.0) as i64;
    Ok(Value::object(vec![("media", Value::from(media.display().to_string())), ("duration", Value::from(duration))]))
}

fn save(args: &Value) -> Result<Value, String> {
    let row = find_row(args.get("id").str())?;
    let lines = lines_in(args)?;
    let (path, estimated) = timing::save(&row, &lines)?;
    Ok(Value::object(vec![
        ("path", Value::from(path.display().to_string())),
        ("name", Value::from(path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())),
        ("estimated", Value::from(estimated)),
    ]))
}

fn retext(args: &Value) -> Result<Value, String> {
    let lines = timing::retext(&lines_in(args)?, args.get("text").str());
    Ok(Value::object(vec![("lines", Value::Array(lines.iter().map(line_out).collect()))]))
}

fn lines_text(args: &Value) -> Result<Value, String> {
    Ok(Value::object(vec![("text", Value::from(timing::lines_to_text(&lines_in(args)?)))]))
}

fn line_readings(args: &Value) -> Result<Value, String> {
    let language = args.get("language").str();
    let readings: Vec<Value> = args
        .get("lines")
        .items()
        .iter()
        .map(|line| Value::Array(token_readings(&line.items().iter().map(|token| token.str().to_string()).collect::<Vec<_>>(), language)))
        .collect();
    Ok(Value::object(vec![("readings", Value::Array(readings)), ("stamp", Value::Array(Vec::new()))]))
}

fn library(args: &Value) -> Result<Value, String> {
    let language = args.get("language").str();
    if !readings::LANGUAGES.contains(&language) {
        return Err("The word library is for songs in Mandarin (zh), Cantonese (yue) and Japanese (ja).".into());
    }
    let name = LIBRARY_NAMES.iter().find(|item| item.0 == language).map(|item| item.1).unwrap_or(language);
    let hint = LIBRARY_HINTS.iter().find(|item| item.0 == language).map(|item| item.1).unwrap_or("");
    let path = readings::user_file(language);
    let builtin = readings::builtin_file(language);
    let text = if path.exists() {
        std::fs::read_to_string(&path).unwrap_or_default().trim_start_matches('\u{feff}').to_string()
    } else {
        format!("# Your {name} readings: one word per line, the word then its reading.\n")
    };
    let builtin_count = if builtin.exists() {
        std::fs::read_to_string(&builtin).map(|content| readings::parse(&content, language).0.len()).unwrap_or(0)
    } else {
        0
    };
    Ok(Value::object(vec![
        ("name", Value::from(name)),
        ("hint", Value::from(hint)),
        ("user_file", Value::from(path.display().to_string())),
        ("text", Value::from(text)),
        ("builtin_count", Value::from(builtin_count)),
    ]))
}

fn save_library(args: &Value) -> Result<Value, String> {
    let (language, content) = (args.get("language").str(), args.get("text").str());
    let problems = readings::parse(content, language).1;
    let path = readings::user_file(language);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&path, format!("{}\n", content.trim_end_matches('\n'))).map_err(|error| error.to_string())?;
    Ok(Value::object(vec![("problems", Value::Array(problems.into_iter().map(Value::from).collect()))]))
}

fn config_text() -> String {
    let path = paths().config.clone();
    if path.exists() {
        std::fs::read_to_string(&path).unwrap_or_default().trim_start_matches('\u{feff}').to_string()
    } else {
        String::new()
    }
}

fn settings_form(args: &Value) -> Value {
    let text = if args.get("text").is_null() { config_text() } else { args.get("text").str().to_string() };
    match settingsform::form(&text) {
        Ok(tabs) => Value::object(vec![("text", Value::from(text)), ("tabs", Value::Array(tabs)), ("error", Value::from(""))]),
        Err(problem) => Value::object(vec![
            ("text", Value::from(text)),
            ("tabs", Value::Array(Vec::new())),
            ("error", Value::from(format!("polykara.toml has a mistake, so only the file can be edited until it is fixed: {problem}"))),
        ]),
    }
}

fn settings_apply(args: &Value) -> Result<Value, String> {
    let text = settingsform::apply_changes(args.get("text").str(), args.get("changes").items())?;
    Ok(Value::object(vec![("text", Value::from(text))]))
}

fn settings_save(args: &Value) -> Result<Value, String> {
    let text = settingsform::apply_changes(args.get("text").str(), args.get("changes").items())?;
    crate::toml::parse(&text)?;
    let path = paths().config.clone();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let partial = path.with_file_name(format!("{}.tmp", path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()));
    std::fs::write(&partial, &text).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, &path).map_err(|error| error.to_string())?;
    config::clear_config_cache();
    Ok(settings_form(&Value::object(vec![("text", Value::from(text))])))
}

fn fonts() -> Value {
    let mut names: Vec<String> = Vec::new();
    if cfg!(windows) {
        for root in ["HKLM", "HKCU"] {
            let key = format!("{root}\\SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts");
            let output = Command::new("reg").args(["query", &key]).stdin(Stdio::null()).output();
            let text = match output {
                Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(),
                Err(_) => continue,
            };
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with("HKEY_") {
                    continue;
                }
                // "Arial (TrueType)    REG_SZ    arial.ttf"
                let name = line.split("    ").next().unwrap_or(line);
                let name = name.split(" (").next().unwrap_or(name);
                for part in name.split(" & ") {
                    let mut part = part.trim().to_string();
                    for style in [" Bold Italic", " Bold", " Italic", " Light", " Semibold", " Black", " Regular"] {
                        if part.ends_with(style) {
                            part.truncate(part.len() - style.len());
                        }
                    }
                    names.push(part.trim().to_string());
                }
            }
        }
    } else if config::find_tool("fc-list").is_some() {
        if let Ok(output) = Command::new(config::tool("fc-list")).args([":", "family"]).stdin(Stdio::null()).output() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some(family) = line.split(',').next() {
                    names.push(family.trim().to_string());
                }
            }
        }
    }
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    let names: Vec<String> = names.into_iter().filter(|name| !name.is_empty() && !name.starts_with('@')).collect();
    Value::object(vec![("fonts", Value::Array(names.into_iter().map(Value::from).collect()))])
}

fn command(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "hello" => Ok(hello()),
        "songs" => Ok(songs()),
        "save_songs" => save_songs(args),
        "describe" => Ok(describe_file(args)),
        "add_files" => add_media(args),
        "new_id" => Ok(make_id(args)),
        "relative" => Ok(relative(args)),
        "load" => load(args),
        "prepare_audio" => prepare_audio(args),
        "save" => save(args),
        "retext" => retext(args),
        "lines_text" => lines_text(args),
        "readings" => line_readings(args),
        "library" => library(args),
        "save_library" => save_library(args),
        "settings_form" => Ok(settings_form(args)),
        "settings_apply" => settings_apply(args),
        "settings_save" => settings_save(args),
        "fonts" => Ok(fonts()),
        other => Err(format!("unknown command {other}")),
    }
}

pub fn handle(request: &Value) -> Value {
    let id = request.get("id").clone();
    let empty = Value::object(Vec::<(String, Value)>::new());
    let args = if request.get("args").is_null() { &empty } else { request.get("args") };
    match command(request.get("cmd").str(), args) {
        Ok(result) => Value::object(vec![("id", id), ("ok", Value::Bool(true)), ("result", result)]),
        Err(error) => Value::object(vec![("id", id), ("ok", Value::Bool(false)), ("error", Value::from(error))]),
    }
}

/// Answer requests until stdin closes.
pub fn serve() -> i32 {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for raw in stdin.lock().lines() {
        let raw = match raw {
            Ok(raw) => raw,
            Err(_) => break,
        };
        if raw.trim().is_empty() {
            continue;
        }
        let answer = match Value::parse(&raw) {
            Ok(request) => handle(&request),
            Err(error) => Value::object(vec![("id", Value::Null), ("ok", Value::Bool(false)), ("error", Value::from(format!("not JSON: {error}")))]),
        };
        if writeln!(out, "{}", answer.dump()).is_err() {
            break;
        }
        let _ = out.flush();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip_through_json() {
        let line = Line {
            tokens: vec![Token { text: "Hello ".into(), start: Some(0), end: Some(200) }, Token { text: "world".into(), start: Some(200), end: Some(500) }],
            speaker: "女".into(),
            start: None,
            end: None,
        };
        let back = line_in(&line_out(&line)).unwrap();
        assert_eq!(back, line);
    }

    #[test]
    fn an_unknown_command_is_an_error_answer() {
        let request = Value::parse(r#"{"id": 7, "cmd": "nope"}"#).unwrap();
        let answer = handle(&request);
        assert_eq!(answer.get("id").as_i64(), Some(7));
        assert_eq!(answer.get("ok").as_bool(), Some(false));
        assert!(answer.get("error").str().contains("unknown command"));
    }

    #[test]
    fn readings_cover_every_token() {
        let tokens = vec!["重".to_string(), "来".to_string(), " ".to_string()];
        let shown = token_readings(&tokens, "zh");
        assert_eq!(shown.len(), tokens.len());
    }
}
