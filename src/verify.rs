//! Collect every lyric file a song has and check that they agree with each other.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::{load_config, paths, resolve};
use crate::csv::Row;
use crate::difflib::Matcher;
use crate::json::Value;
use crate::manifest::get;
use crate::providers::provider_files;
use crate::subtitle::{downloaded_subtitle, parse_lrc, parse_timed_subtitle, read_text, Entry};
use crate::text::{split_speaker, token_key, tokenise};

/// A file with fewer words than this cannot confirm or contradict anything.
pub const MIN_TOKENS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Lrc,
    Subtitle,
    /// Untimed text.
    Plain,
}

#[derive(Clone, Debug)]
pub struct LyricFile {
    pub name: String,
    pub label: String,
    pub kind: Kind,
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub tokens: Vec<String>,
    pub score: f64,
    pub agrees: bool,
}

#[derive(Clone, Debug)]
pub struct LyricCheck {
    pub files: Vec<LyricFile>,
    primary: Option<usize>,
    pub min_sources: usize,
}

impl LyricCheck {
    pub fn primary(&self) -> Option<&LyricFile> {
        self.primary.map(|index| &self.files[index])
    }

    pub fn is_primary(&self, file: &LyricFile) -> bool {
        self.primary().map_or(false, |primary| primary.path == file.path && primary.name == file.name)
    }

    pub fn agreeing(&self) -> usize {
        self.files.iter().filter(|item| item.agrees).count()
    }

    pub fn verified(&self) -> bool {
        self.primary.is_some() && self.agreeing() >= self.min_sources
    }

    pub fn problem(&self) -> String {
        if self.primary.is_none() {
            return "no lyric file".into();
        }
        if self.files.len() < self.min_sources {
            return format!("only {} lyric file(s), {} needed for an accuracy check", self.files.len(), self.min_sources);
        }
        format!("only {} of {} lyric files agree, {} needed", self.agreeing(), self.files.len(), self.min_sources)
    }

    pub fn to_json(&self) -> Value {
        let files = self
            .files
            .iter()
            .map(|item| {
                Value::object(vec![
                    ("source", Value::from(&item.name)),
                    ("file", Value::from(item.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default())),
                    ("words", Value::from(item.tokens.len())),
                    ("agreement", Value::Number((item.score * 1000.0).round() / 1000.0)),
                    ("agrees", Value::from(item.agrees)),
                ])
            })
            .collect();
        Value::object(vec![
            ("verified", Value::from(self.verified())),
            ("min_sources", Value::from(self.min_sources)),
            ("agreeing", Value::from(self.agreeing())),
            ("primary", Value::from(self.primary().map(|item| item.name.clone()))),
            ("files", Value::Array(files)),
        ])
    }
}

/// Words (or CJK characters) of a lyric text in a form that ignores case, punctuation and script variant.
pub fn comparison_tokens(text: &str) -> Vec<String> {
    // Traditional and Simplified Chinese versions of the same lyrics agree.
    let text = crate::romanize::simplified(text);
    tokenise(&text).iter().map(|token| token_key(token)).filter(|key| !key.is_empty()).collect()
}

/// Share of the shorter text found, in order, in the longer one.
///
/// Measuring against the shorter text keeps a file that omits repeated choruses
/// from counting as a disagreement.
pub fn agreement(first: &[String], second: &[String]) -> f64 {
    if first.is_empty() || second.is_empty() {
        return 0.0;
    }
    let matched: usize = Matcher::new(first, second).matching_blocks().iter().map(|block| block.2).sum();
    matched as f64 / first.len().min(second.len()) as f64
}

/// Parse one lyric file; None when it has too little text to be usable.
pub fn load_file(name: &str, label: &str, kind: Kind, path: &std::path::Path) -> Option<LyricFile> {
    let content = read_text(path).ok()?;
    let entries = match kind {
        Kind::Lrc => parse_lrc(&content),
        Kind::Subtitle => parse_timed_subtitle(&content),
        Kind::Plain => Vec::new(),
    };
    let text = if kind == Kind::Plain { content } else { entries.iter().map(|entry| split_speaker(&entry.text).1).collect::<Vec<_>>().join("\n") };
    let tokens = comparison_tokens(&text);
    if tokens.len() < MIN_TOKENS || (kind != Kind::Plain && entries.is_empty()) {
        return None;
    }
    Some(LyricFile { name: name.into(), label: label.into(), kind, path: path.to_path_buf(), entries, tokens, score: 1.0, agrees: true })
}

/// Every lyric file of a song as (name, label, kind, path), most trusted first.
pub fn candidates(row: &Row) -> Result<Vec<(String, String, Kind, PathBuf)>, String> {
    let sid = get(row, "id");
    let mut found = Vec::new();
    let edited = paths().lyrics.join(format!("{sid}.timed.lrc"));
    if edited.exists() {
        // Timing made in the timing editor outranks every other source.
        found.push(("editor".to_string(), "timing editor".to_string(), Kind::Lrc, edited));
    }
    let raw = get(row, "lyrics_file");
    if !raw.is_empty() {
        let path = resolve(raw);
        if !path.exists() {
            return Err(format!("lyrics_file not found: {}", path.display()));
        }
        let suffix = path.extension().map(|extension| extension.to_string_lossy().to_lowercase()).unwrap_or_default();
        let kind = match suffix.as_str() {
            "lrc" => Kind::Lrc,
            "srt" | "vtt" => Kind::Subtitle,
            "txt" => Kind::Plain,
            _ => return Err(format!("lyrics_file must be .lrc, .srt, .vtt, or .txt: {}", path.display())),
        };
        found.push(("manifest".into(), "manifest lyrics_file".into(), kind, path));
    }
    if let Some(manual) = downloaded_subtitle(row, false) {
        found.push(("subtitle".into(), "downloaded subtitle".into(), Kind::Subtitle, manual));
    }
    let external = provider_files(row);
    for (name, kind, path) in &external {
        if *kind == Kind::Lrc {
            found.push((name.clone(), format!("{name} synced lyrics"), Kind::Lrc, path.clone()));
        }
    }
    if let Some(automatic) = downloaded_subtitle(row, true) {
        if load_config().get("lyrics").bool_or("auto_captions", true) {
            found.push(("auto-captions".into(), "automatic captions".into(), Kind::Subtitle, automatic));
        }
    }
    for (name, kind, path) in external {
        if kind == Kind::Plain {
            found.push((name.clone(), format!("{name} lyrics"), Kind::Plain, path));
        }
    }
    Ok(found)
}

/// Choose the lyric file to render from and mark which other files agree with it.
pub fn check_files(mut files: Vec<LyricFile>, min_sources: usize, threshold: f64, fixed_primary: bool) -> LyricCheck {
    if files.is_empty() {
        return LyricCheck { files, primary: None, min_sources };
    }
    let count = files.len();
    let mut scores = vec![vec![0.0; count]; count];
    for a in 0..count {
        for b in a + 1..count {
            let score = agreement(&files[a].tokens, &files[b].tokens);
            scores[a][b] = score;
            scores[b][a] = score;
        }
    }
    let supporters = |index: usize| 1 + (0..count).filter(|other| *other != index && scores[index][*other] >= threshold).count();
    let timed: Vec<usize> = (0..count).filter(|index| files[*index].kind != Kind::Plain).collect();
    let chosen = if fixed_primary || timed.is_empty() {
        0
    } else {
        // The most trusted timed file that enough other files confirm, else the best-confirmed one.
        match timed.iter().copied().find(|index| supporters(*index) >= min_sources) {
            Some(index) => index,
            None => timed.iter().copied().max_by_key(|index| (supporters(*index), std::cmp::Reverse(*index))).unwrap_or(0),
        }
    };
    for (index, item) in files.iter_mut().enumerate() {
        item.score = if index == chosen { 1.0 } else { scores[index][chosen] };
        item.agrees = item.score >= threshold;
    }
    LyricCheck { files, primary: Some(chosen), min_sources }
}

static CACHE: Mutex<Option<HashMap<String, LyricCheck>>> = Mutex::new(None);

pub fn clear_cache() {
    *CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// Accuracy check for one song. Results are kept until a lyric file or the settings change.
pub fn check_lyrics(row: &Row) -> Result<LyricCheck, String> {
    let config = load_config();
    let settings = config.get("lyrics");
    let min_sources = settings.int_or("min_sources", 3).max(1) as usize;
    let threshold = settings.float_or("agreement_threshold", 0.6);
    let found = candidates(row)?;
    let fixed = !get(row, "lyrics_file").is_empty() || found.first().map_or(false, |item| item.0 == "editor");
    let mut key = format!("{min_sources}|{threshold}|{fixed}");
    for (name, _, _, path) in &found {
        let stamp = std::fs::metadata(path).and_then(|meta| meta.modified()).ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|age| age.as_nanos()).unwrap_or(0);
        key.push_str(&format!("|{name}:{}:{stamp}", path.display()));
    }
    if let Some(cached) = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().and_then(|cache| cache.get(&key)) {
        return Ok(cached.clone());
    }
    let files: Vec<LyricFile> = found.iter().filter_map(|(name, label, kind, path)| load_file(name, label, *kind, path)).collect();
    // A manifest or editor file that turned out unusable must not hand its priority to the next file.
    let fixed = fixed && files.first().map_or(false, |item| item.name == "editor" || item.name == "manifest");
    let check = check_files(files, min_sources, threshold, fixed);
    let mut cache = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() > 64 {
        cache.clear();
    }
    cache.insert(key, check.clone());
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, kind: Kind, words: &str) -> LyricFile {
        LyricFile { name: name.into(), label: name.into(), kind, path: PathBuf::from(name), entries: Vec::new(), tokens: comparison_tokens(words), score: 1.0, agrees: true }
    }

    #[test]
    fn agreement_measures_against_the_shorter_text() {
        let long = comparison_tokens("one two three four five one two three four five");
        let short = comparison_tokens("one two three four five");
        assert_eq!(agreement(&long, &short), 1.0);
        assert_eq!(agreement(&long, &[]), 0.0);
    }

    #[test]
    fn the_confirmed_timed_file_is_chosen() {
        let words = "a b c d e f g h i j k l";
        let files = vec![file("subtitle", Kind::Subtitle, "x y z w v u t s r q p o"), file("lrclib", Kind::Lrc, words), file("plain", Kind::Plain, words)];
        let check = check_files(files, 2, 0.6, false);
        assert_eq!(check.primary().unwrap().name, "lrclib");
        assert!(check.verified());
        assert!(!check.files[0].agrees);
        assert!(check.problem().contains("lyric files"));
    }
}
