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

/// How much lyric text to prime whisper.cpp with. whisper truncates the initial
/// prompt to about half the context window, so a cap keeps it well within that.
const PROMPT_MAX_CHARS: usize = 700;

/// The lyrics to prime whisper.cpp with for this song, if any lyric file is present.
/// Prefers the file the accuracy check trusts most, falling back to the first found.
fn lyric_prompt(row: &Row) -> Option<String> {
    let check = crate::verify::check_lyrics(row).ok()?;
    let file = check.primary().or_else(|| check.files.first())?;
    prompt_from_lines(file.entries.iter().map(|entry| entry.text.as_str()), PROMPT_MAX_CHARS)
}

/// Join lyric lines into a single prompt, trimmed to `max_chars` characters.
fn prompt_from_lines<'a>(lines: impl Iterator<Item = &'a str>, max_chars: usize) -> Option<String> {
    let mut text = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
        if text.chars().count() >= max_chars {
            break;
        }
    }
    if text.is_empty() {
        return None;
    }
    Some(text.chars().take(max_chars).collect())
}

/// Run whisper.cpp on `audio`, writing its scratch JSON under `out_base`, and return
/// the grouped {"segments": [...]} word timing. `prompt` primes recognition toward
/// known words; pass None to keep the transcript an unbiased witness of what was sung.
/// Returns None on a dry run (the command is printed but not executed).
pub fn transcribe(row: &Row, audio: &Path, language: &str, out_base: &Path, prompt: Option<&str>, dry_run: bool) -> Result<Option<Value>, String> {
    if language.is_empty() {
        return Err("language is missing".into());
    }
    if !audio.exists() && !dry_run {
        return Err(format!("audio is missing: {}", audio.display()));
    }
    let whisper = find().ok_or_else(|| "whisper-cli is not installed: put it in the PolyKara bin folder or on PATH".to_string())?;
    let model = alignment_model(row);
    let model_file = model_path(&model).ok_or_else(|| {
        let name = if model.ends_with(".bin") { model.clone() } else { format!("ggml-{model}.bin") };
        format!("the whisper.cpp model {name} is missing: put it in {} or set alignment.model_path", paths().models.display())
    })?;
    let language = whisper_language(language, &model);
    // DTW token timestamps follow the audio much more closely than whisper's own
    // token offsets; whisper.cpp only computes them with flash attention off.
    let dtw = if config::load_config().get("alignment").bool_or("dtw", true) { dtw_preset(&model_file) } else { None };
    let mut args = vec![
        "-m".to_string(),
        model_file.display().to_string(),
        "-f".to_string(),
        audio.display().to_string(),
        "-l".to_string(),
        language,
        "-oj".to_string(),
        "-ojf".to_string(),
        "-of".to_string(),
        out_base.display().to_string(),
        "--split-on-word".to_string(),
        "-np".to_string(),
    ];
    if let Some(prompt) = prompt {
        args.push("--prompt".to_string());
        args.push(prompt.to_string());
    }
    let mut dtw_args = Vec::new();
    if let Some(preset) = dtw {
        dtw_args = vec!["--dtw".to_string(), preset.to_string(), "-nfa".to_string()];
    }
    if dry_run {
        show_command(&whisper, &args, &dtw_args);
        return Ok(None);
    }
    if let Some(folder) = out_base.parent() {
        std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    }
    let mut result = run_whisper(&whisper, &args, &dtw_args);
    if result.is_err() && !dtw_args.is_empty() {
        // A whisper.cpp build without DTW support still gives usable word timing.
        println!("WARN whisper.cpp failed with DTW timestamps; trying again without them");
        result = run_whisper(&whisper, &args, &[]);
    }
    result?;
    let raw = PathBuf::from(format!("{}.json", out_base.display()));
    let text = std::fs::read_to_string(&raw).map_err(|error| format!("whisper.cpp wrote no JSON ({}): {error}", raw.display()))?;
    let data = Value::parse(&text).map_err(|error| format!("whisper.cpp wrote invalid JSON: {error}"))?;
    let timing = convert(&data)?;
    let _ = std::fs::remove_file(&raw);
    Ok(Some(timing))
}

fn show_command(whisper: &Path, args: &[String], extra: &[String]) {
    let shown: Vec<&str> = args
        .iter()
        .enumerate()
        .map(|(index, value)| if index > 0 && args[index - 1] == "--prompt" { "<lyrics>" } else { value.as_str() })
        .chain(extra.iter().map(String::as_str))
        .collect();
    println!("$ {} {}", whisper.display(), shown.join(" "));
}

fn run_whisper(whisper: &Path, args: &[String], extra: &[String]) -> Result<(), String> {
    show_command(whisper, args, extra);
    let mut command = Command::new(whisper);
    command.args(args).args(extra).stdin(Stdio::null());
    crate::tools::quiet(&mut command);
    let status = command.status().map_err(|error| format!("{} could not be started ({error})", whisper.display()))?;
    if !status.success() {
        return Err(format!("{} exited with status {}", whisper.display(), status.code().unwrap_or(-1)));
    }
    Ok(())
}

/// whisper.cpp's DTW alignment-heads preset for a model file such as
/// ggml-large-v3-turbo-q5_0.bin, or None when the model has no preset.
fn dtw_preset(model_file: &Path) -> Option<&'static str> {
    let name = model_file.file_name()?.to_string_lossy().to_lowercase();
    let name = name.strip_suffix(".bin").unwrap_or(&name);
    let name = name.strip_prefix("ggml-").unwrap_or(name);
    // Quantized models keep their architecture: drop a "-q5_0"-style suffix.
    let name = match name.rfind("-q") {
        Some(at) if name[at + 2..].chars().next().map_or(false, |c| c.is_ascii_digit()) => &name[..at],
        _ => name,
    };
    Some(match name {
        "tiny" => "tiny",
        "tiny.en" => "tiny.en",
        "base" => "base",
        "base.en" => "base.en",
        "small" => "small",
        "small.en" => "small.en",
        "medium" => "medium",
        "medium.en" => "medium.en",
        "large-v1" => "large.v1",
        "large-v2" => "large.v2",
        "large-v3" => "large.v3",
        "large-v3-turbo" => "large.v3.turbo",
        _ => return None,
    })
}

/// Time one song's words with whisper.cpp, priming it with the known lyrics.
pub fn align_row(row: &Row, dry_run: bool) -> Result<(), String> {
    let sid = get(row, "id");
    let language = get(row, "language").trim().to_string();
    let audio = paths().audio.join(format!("{sid}.flac"));
    let base = paths().work.join("whisper").join(sid);
    // Prime recognition with the lyrics we already have, so whisper.cpp hears the
    // actual words — especially names and CJK readings it would otherwise guess.
    let prompt = lyric_prompt(row);
    if prompt.is_some() {
        println!("{sid}: priming word timing with the known lyrics");
    }
    let timing = match transcribe(row, &audio, &language, &base, prompt.as_deref(), dry_run)? {
        Some(timing) => timing,
        None => return Ok(()),
    };
    std::fs::create_dir_all(paths().align.clone()).map_err(|error| error.to_string())?;
    std::fs::write(paths().align.join(format!("{sid}.json")), format!("{}\n", timing.dump())).map_err(|error| error.to_string())?;
    Ok(())
}

/// Group whisper.cpp tokens into words and build the aligner JSON.
fn convert(data: &Value) -> Result<Value, String> {
    let mut segments = Vec::new();
    let mut total = 0usize;
    for segment in data.get("transcription").items() {
        // (start, end, text, DTW onset) per word, in ms.
        let mut words: Vec<(f64, f64, String, Option<f64>)> = Vec::new();
        for token in segment.get("tokens").items() {
            let raw = token.get("text").str();
            if raw.is_empty() || raw.starts_with("[_") || raw.starts_with("<|") {
                continue;
            }
            let piece = raw.trim();
            if piece.is_empty() {
                continue;
            }
            let start = token.get("offsets").get("from").as_i64().unwrap_or(0) as f64;
            let end = token.get("offsets").get("to").as_i64().unwrap_or(0) as f64;
            if end <= start {
                continue;
            }
            // t_dtw is in centiseconds, -1 when DTW was not run.
            let onset = match token.get("t_dtw") {
                Value::Number(value) if *value >= 0.0 => Some(value * 10.0),
                _ => None,
            };
            let starts_word = raw.starts_with(' ')
                || words.last().map(|word| has_cjk(&word.2) || has_cjk(piece)).unwrap_or(true);
            let mut appended = false;
            if !starts_word {
                if let Some(word) = words.last_mut() {
                    word.1 = end;
                    word.2.push_str(piece);
                    appended = true;
                }
            }
            if !appended {
                words.push((start, end, piece.to_string(), onset));
            }
        }
        if words.is_empty() {
            continue;
        }
        let segment_start = segment.get("offsets").get("from").as_i64().map(|value| value as f64).unwrap_or(words[0].0);
        let segment_end = segment.get("offsets").get("to").as_i64().map(|value| value as f64).unwrap_or(words[words.len() - 1].1);
        let timed = if words.iter().all(|word| word.3.is_some()) { dtw_times(&words, segment_start, segment_end) } else { words.iter().map(|word| (word.0, word.1)).collect() };
        total += words.len();
        let text = segment.get("text").str().trim().to_string();
        let words: Vec<Value> = words
            .into_iter()
            .zip(timed)
            .map(|((_, _, word, _), (start, end))| Value::object(vec![("start", Value::Number(start / 1000.0)), ("end", Value::Number(end / 1000.0)), ("word", Value::from(word))]))
            .collect();
        segments.push(Value::object(vec![
            ("start", Value::Number(segment_start / 1000.0)),
            ("end", Value::Number(segment_end / 1000.0)),
            ("text", Value::from(text)),
            ("words", Value::Array(words)),
        ]));
    }
    if total == 0 {
        return Err("whisper.cpp returned no word timestamps".into());
    }
    Ok(Value::object(vec![("segments", Value::Array(segments))]))
}

/// The longest a word may be held before the next one starts; longer gaps are a pause.
const MAX_HELD_MS: f64 = 3000.0;

/// Word times from DTW onsets: each word starts at its onset and lasts until the
/// next word starts (a held note), unless the gap is a pause, where it keeps
/// whisper's own duration. Onsets are kept in order and inside the segment.
fn dtw_times(words: &[(f64, f64, String, Option<f64>)], segment_start: f64, segment_end: f64) -> Vec<(f64, f64)> {
    let mut starts = Vec::with_capacity(words.len());
    let mut cursor = segment_start;
    for word in words {
        let onset = word.3.unwrap_or(word.0).max(cursor).min(segment_end);
        starts.push(onset);
        cursor = onset;
    }
    (0..words.len())
        .map(|index| {
            let start = starts[index];
            let own = (words[index].1 - words[index].0).clamp(100.0, MAX_HELD_MS);
            let end = match starts.get(index + 1) {
                Some(next) if next - start <= MAX_HELD_MS => *next,
                Some(next) => (start + own).min(*next),
                None => (start + own).min(segment_end.max(start + 100.0)),
            };
            (start, end.max(start))
        })
        .collect()
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
    fn dtw_onsets_set_word_boundaries() {
        // whisper's own offsets put "world" at 0.6s; DTW heard it start at 0.8s.
        let data = Value::parse(
            r#"{"transcription": [{"offsets": {"from": 0, "to": 9000}, "text": " Hello world again", "tokens": [
                {"text": " Hello", "offsets": {"from": 0, "to": 500}, "t_dtw": 12},
                {"text": " world", "offsets": {"from": 600, "to": 1200}, "t_dtw": 80},
                {"text": " again", "offsets": {"from": 1200, "to": 1500}, "t_dtw": 500}
            ]}]}"#,
        )
        .unwrap();
        let words = convert(&data).unwrap().get("segments").items()[0].get("words").items().to_vec();
        let times: Vec<(f64, f64)> = words.iter().map(|word| (word.get("start").as_f64().unwrap(), word.get("end").as_f64().unwrap())).collect();
        assert_eq!(times[0], (0.12, 0.8), "held until the next word starts");
        assert_eq!(times[1], (0.8, 1.4), "a pause before the next word keeps whisper's duration");
        assert_eq!(times[2], (5.0, 5.3));
    }

    #[test]
    fn missing_dtw_keeps_whisper_offsets() {
        let data = Value::parse(
            r#"{"transcription": [{"offsets": {"from": 0, "to": 1300}, "text": " Hi there", "tokens": [
                {"text": " Hi", "offsets": {"from": 0, "to": 500}, "t_dtw": -1},
                {"text": " there", "offsets": {"from": 600, "to": 1200}, "t_dtw": -1}
            ]}]}"#,
        )
        .unwrap();
        let words = convert(&data).unwrap().get("segments").items()[0].get("words").items().to_vec();
        assert_eq!(words[1].get("start"), &Value::Number(0.6));
    }

    #[test]
    fn dtw_presets_follow_model_files() {
        assert_eq!(dtw_preset(Path::new("models/ggml-small.bin")), Some("small"));
        assert_eq!(dtw_preset(Path::new("ggml-large-v3-turbo-q5_0.bin")), Some("large.v3.turbo"));
        assert_eq!(dtw_preset(Path::new("ggml-medium.en-q8_0.bin")), Some("medium.en"));
        assert_eq!(dtw_preset(Path::new("my-finetune.bin")), None);
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

    #[test]
    fn prompt_joins_non_empty_lines() {
        let lines = ["  Hello world ", "", "   ", "second line"];
        assert_eq!(prompt_from_lines(lines.into_iter(), 700).unwrap(), "Hello world second line");
    }

    #[test]
    fn prompt_is_capped_and_none_when_blank() {
        let long = "a".repeat(1000);
        let capped = prompt_from_lines([long.as_str()].into_iter(), 700).unwrap();
        assert_eq!(capped.chars().count(), 700);
        assert!(prompt_from_lines(["", "   "].into_iter(), 700).is_none());
    }
}
