//! Automatic word timing with whisper.cpp.
//!
//! `whisper-cli` transcribes the normalized audio with token timestamps; this
//! module groups the tokens into words and writes work/align/<id>.json in the
//! shape the editor and the ASS step read: {"segments": [{"start", "end", "text", "words": [...]}]}.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::{self, paths};
use crate::csv::Row;
use crate::json::Value;
use crate::manifest::get;
use crate::subtitle::word_json;

/// The whisper.cpp program: POLYKARA_WHISPER, next to this program, or on PATH.
pub fn find() -> Option<PathBuf> {
    if let Some(program) = std::env::var_os("POLYKARA_WHISPER") {
        return Some(program.into());
    }
    let name = if cfg!(windows) { "whisper-cli.exe" } else { "whisper-cli" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(folder) = exe.parent() {
            for place in folder.ancestors() {
                let candidate = place.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    config::find_tool("whisper-cli")
}

pub fn available() -> bool {
    find().is_some()
}

/// True when word timing exists and is not older than the normalized audio.
pub fn is_aligned(row: &Row) -> bool {
    let timing = word_json(row);
    let audio = paths().audio.join(format!("{}.flac", get(row, "id")));
    match timing {
        None => false,
        Some(path) => !audio.exists() || modified(&path) >= modified(&audio),
    }
}

fn modified(path: &Path) -> std::time::SystemTime {
    std::fs::metadata(path).and_then(|meta| meta.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

/// The model for a song: its align_model column, else [alignment] model.
/// "auto" picks by installed memory, as the Python aligner did.
pub fn alignment_model(row: &Row) -> String {
    let config = config::load_config();
    let settings = config.get("alignment");
    let configured = {
        let own = get(row, "align_model").trim().to_lowercase();
        if own.is_empty() { settings.str_or("model", "auto").to_lowercase() } else { own }
    };
    if configured != "auto" {
        return configured;
    }
    let total_gib = config::total_memory_bytes() as f64 / (1024.0 * 1024.0 * 1024.0);
    let reserve_gib = settings.float_or("reserve_memory_gib", 2.0).max(0.0);
    let budget_gib = (total_gib - reserve_gib).max(0.0);
    let preferred = if budget_gib >= settings.float_or("large_min_budget_gib", 12.0) {
        "large-v3"
    } else if budget_gib >= settings.float_or("medium_min_budget_gib", 8.0) {
        "medium"
    } else if budget_gib >= settings.float_or("small_min_budget_gib", 4.0) {
        "small"
    } else if budget_gib >= settings.float_or("base_min_budget_gib", 2.0) {
        "base"
    } else {
        "tiny"
    };
    // Use the best model that is actually installed, so a machine with only the
    // bundled small model still works while a larger one is picked up when added.
    let order = ["large-v3", "medium", "small", "base", "tiny"];
    let start = order.iter().position(|name| *name == preferred).unwrap_or(order.len() - 1);
    let pick = order[start..]
        .iter()
        .copied()
        .find(|name| model_path(name).is_some())
        .or_else(|| order[..start].iter().rev().copied().find(|name| model_path(name).is_some()))
        .unwrap_or(preferred);
    println!("{}: detected {total_gib:.1} GiB RAM; reserving {reserve_gib:.1} GiB for the system, using {budget_gib:.1} GiB for alignment -> {pick}", get(row, "id"));
    pick.to_string()
}

/// The ggml model file: alignment.model_path, POLYKARA_WHISPER_MODEL, or the
/// usual places under the PolyKara folder.
pub fn model_path(model: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("POLYKARA_WHISPER_MODEL") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let configured = config::load_config().get("alignment").str_or("model_path", "");
    if !configured.trim().is_empty() {
        let path = config::resolve(configured.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    let name = if model.ends_with(".bin") { model.to_string() } else { format!("ggml-{model}.bin") };
    let path = config::resolve(&name);
    if path.is_file() {
        return Some(path);
    }
    for folder in [paths().models.clone(), paths().root.join("data").join("models"), paths().root.join("bin"), paths().root.join("models")] {
        let candidate = folder.join(&name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Whisper's language code: yue only exists in large-v3, other tags lose their region.
fn whisper_language(language: &str, model: &str) -> String {
    let tag = language.trim().to_lowercase().replace('_', "-");
    let mut code = if matches!(tag.as_str(), "yue" | "zh-hk" | "zh-yue") { "yue".to_string() } else { tag.split('-').next().unwrap_or("").to_string() };
    if code == "yue" && !model.starts_with("large-v3") {
        code = "zh".into();
    }
    code
}

/// Time one song's words with whisper.cpp.
pub fn align_row(row: &Row, dry_run: bool) -> Result<(), String> {
    let sid = get(row, "id");
    let language = get(row, "language").trim().to_string();
    if language.is_empty() {
        return Err("language is missing".into());
    }
    let audio = paths().audio.join(format!("{sid}.flac"));
    if !audio.exists() && !dry_run {
        return Err(format!("normalized audio is missing: {}", audio.display()));
    }
    let whisper = find().ok_or_else(|| "whisper-cli is not installed: put it in the PolyKara bin folder or on PATH".to_string())?;
    let model = alignment_model(row);
    let model_file = model_path(&model).ok_or_else(|| {
        let name = if model.ends_with(".bin") { model.clone() } else { format!("ggml-{model}.bin") };
        format!("the whisper.cpp model {name} is missing: put it in {} or set alignment.model_path", paths().models.display())
    })?;
    let language = whisper_language(&language, &model);
    let base = paths().work.join("whisper").join(sid);
    let args = vec![
        "-m".to_string(),
        model_file.display().to_string(),
        "-f".to_string(),
        audio.display().to_string(),
        "-l".to_string(),
        language,
        "-oj".to_string(),
        "-ojf".to_string(),
        "-of".to_string(),
        base.display().to_string(),
        "--split-on-word".to_string(),
        "-np".to_string(),
    ];
    println!("$ {} {}", whisper.display(), args.join(" "));
    if dry_run {
        return Ok(());
    }
    std::fs::create_dir_all(paths().work.join("whisper")).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(paths().align.clone()).map_err(|error| error.to_string())?;
    let mut command = Command::new(&whisper);
    command.args(&args).stdin(Stdio::null());
    crate::tools::quiet(&mut command);
    let status = command.status().map_err(|error| format!("{} could not be started ({error})", whisper.display()))?;
    if !status.success() {
        return Err(format!("{} exited with status {}", whisper.display(), status.code().unwrap_or(-1)));
    }
    let raw = PathBuf::from(format!("{}.json", base.display()));
    let text = std::fs::read_to_string(&raw).map_err(|error| format!("whisper.cpp wrote no JSON ({}): {error}", raw.display()))?;
    let data = Value::parse(&text).map_err(|error| format!("whisper.cpp wrote invalid JSON: {error}"))?;
    let timing = convert(&data)?;
    std::fs::write(paths().align.join(format!("{sid}.json")), format!("{}\n", timing.dump())).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&raw);
    Ok(())
}

/// Group whisper.cpp tokens into words and build the aligner JSON.
fn convert(data: &Value) -> Result<Value, String> {
    let mut segments = Vec::new();
    let mut total = 0usize;
    for segment in data.get("transcription").items() {
        let mut words: Vec<(f64, f64, String)> = Vec::new();
        for token in segment.get("tokens").items() {
            let raw = token.get("text").str();
            if raw.is_empty() || raw.starts_with("[_") || raw.starts_with("<|") {
                continue;
            }
            let piece = raw.trim();
            if piece.is_empty() {
                continue;
            }
            let start = token.get("offsets").get("from").as_i64().unwrap_or(0) as f64 / 1000.0;
            let end = token.get("offsets").get("to").as_i64().unwrap_or(0) as f64 / 1000.0;
            if end <= start {
                continue;
            }
            let starts_word = raw.starts_with(' ')
                || words.last().map(|(_, _, text)| has_cjk(text) || has_cjk(piece)).unwrap_or(true);
            let mut appended = false;
            if !starts_word {
                if let Some(word) = words.last_mut() {
                    word.1 = end;
                    word.2.push_str(piece);
                    appended = true;
                }
            }
            if !appended {
                words.push((start, end, piece.to_string()));
            }
        }
        if words.is_empty() {
            continue;
        }
        total += words.len();
        let segment_start = segment.get("offsets").get("from").as_i64().map(|value| value as f64 / 1000.0).unwrap_or(words[0].0);
        let segment_end = segment.get("offsets").get("to").as_i64().map(|value| value as f64 / 1000.0).unwrap_or(words[words.len() - 1].1);
        let text = segment.get("text").str().trim().to_string();
        let words: Vec<Value> = words
            .into_iter()
            .map(|(start, end, word)| Value::object(vec![("start", Value::Number(start)), ("end", Value::Number(end)), ("word", Value::from(word))]))
            .collect();
        segments.push(Value::object(vec![
            ("start", Value::Number(segment_start)),
            ("end", Value::Number(segment_end)),
            ("text", Value::from(text)),
            ("words", Value::Array(words)),
        ]));
    }
    if total == 0 {
        return Err("whisper.cpp returned no word timestamps".into());
    }
    Ok(Value::object(vec![("segments", Value::Array(segments))]))
}

fn has_cjk(text: &str) -> bool {
    text.chars().any(|character| matches!(character as u32, 0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xac00..=0xd7af | 0x20000..=0x2fa1f))
}

/// The program path, for the self-test to report.
pub fn shown() -> String {
    find().map(|path| path.display().to_string()).unwrap_or_else(|| "MISSING".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin_tokens_join_into_words() {
        let data = Value::parse(
            r#"{"transcription": [{"offsets": {"from": 0, "to": 1300}, "text": " Hello world!", "tokens": [
                {"text": " Hello", "offsets": {"from": 0, "to": 500}},
                {"text": " world", "offsets": {"from": 600, "to": 1200}},
                {"text": "!", "offsets": {"from": 1200, "to": 1300}},
                {"text": "[_BEG_]", "offsets": {"from": 0, "to": 0}}
            ]}]}"#,
        )
        .unwrap();
        let timing = convert(&data).unwrap();
        let words = timing.get("segments").items()[0].get("words").items();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].get("word").str(), "Hello");
        assert_eq!(words[1].get("word").str(), "world!");
        assert_eq!(words[1].get("start"), &Value::Number(0.6));
        assert_eq!(words[1].get("end"), &Value::Number(1.3));
    }

    #[test]
    fn chinese_tokens_stay_one_word_each() {
        let data = Value::parse(
            r#"{"transcription": [{"offsets": {"from": 0, "to": 900}, "text": " 你好", "tokens": [
                {"text": " 你", "offsets": {"from": 0, "to": 400}},
                {"text": "好", "offsets": {"from": 400, "to": 900}}
            ]}]}"#,
        )
        .unwrap();
        let timing = convert(&data).unwrap();
        let words = timing.get("segments").items()[0].get("words").items();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].get("word").str(), "你");
        assert_eq!(words[1].get("word").str(), "好");
    }

    #[test]
    fn languages_follow_whisper_rules() {
        assert_eq!(whisper_language("zh-HK", "large-v3"), "yue");
        assert_eq!(whisper_language("zh-HK", "small"), "zh");
        assert_eq!(whisper_language("en-US", "medium"), "en");
        assert_eq!(whisper_language("yue", "large-v3-turbo"), "yue");
    }

    #[test]
    fn empty_transcripts_are_reported() {
        let data = Value::parse(r#"{"transcription": [{"text": " ", "tokens": []}]}"#).unwrap();
        assert!(convert(&data).is_err());
    }
}
