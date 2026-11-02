//! Lyric files with line timing (LRC, SRT, VTT) and matching their words to recognised speech.

use std::path::{Path, PathBuf};

use crate::config::{paths, resolve};
use crate::csv::Row;
use crate::json::Value;
use crate::manifest::get;
use crate::text::{split_speaker, token_key, tokenise};

/// Estimated duration of a token nobody recognised, and how long one may be stretched.
pub const TYPICAL_TOKEN_MS: i64 = 400;
pub const MAX_TOKEN_MS: i64 = 1200;
pub const MIN_SPREAD_MS: i64 = 80;

/// One timed word (or CJK character): start and end in ms, and its text with any trailing space.
pub type Word = (i64, i64, String);
/// A recognised word from the aligner: start, end and its comparison key.
pub type Unit = (i64, i64, String);

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub start: i64,
    pub end: i64,
    /// The line, with a "[speaker:x] " prefix when a singer is named.
    pub text: String,
    pub words: Vec<Word>,
}

impl Entry {
    pub fn new(start: i64, end: i64, text: impl Into<String>, words: Vec<Word>) -> Entry {
        Entry { start, end, text: text.into(), words }
    }
}

fn find_json_files(folder: &Path, found: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                find_json_files(&path, found);
            } else if path.extension().map_or(false, |extension| extension == "json") {
                found.push(path);
            }
        }
    }
}

/// The song's word timing file: word_timing_file from songs.csv, else work/align/<id>.json.
pub fn word_json(row: &Row) -> Option<PathBuf> {
    let configured = get(row, "word_timing_file").trim();
    let sid = get(row, "id");
    if !configured.is_empty() {
        let path = resolve(configured);
        if path.exists() {
            return Some(path);
        }
        println!("WARN {sid}: word_timing_file not found ({}); using automatic alignment", path.display());
    }
    let align = paths().align.clone();
    let exact = align.join(format!("{sid}.json"));
    if exact.exists() {
        return Some(exact);
    }
    let mut found = Vec::new();
    find_json_files(&align, &mut found);
    found.retain(|path| {
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = path.parent().and_then(|parent| parent.file_name()).map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        name.starts_with(&format!("{sid}.")) || parent == sid
    });
    found.sort();
    found.into_iter().next()
}

/// Recognised words as (start_ms, end_ms, key) units, one per lyric token.
pub fn timed_units(data: &Value) -> Vec<Unit> {
    let mut words: Vec<&Value> = Vec::new();
    for segment in data.get("segments").items() {
        let inner = if !segment.get("words").items().is_empty() { segment.get("words") } else { segment.get("word_segments") };
        words.extend(inner.items());
    }
    if words.is_empty() {
        words = data.get("word_segments").items().iter().collect();
    }
    let mut units = Vec::new();
    for word in words {
        let (start, end) = match (number(word.get("start")), number(word.get("end"))) {
            (Some(start), Some(end)) => ((start * 1000.0) as i64, (end * 1000.0) as i64),
            _ => continue,
        };
        let text = match word.get("word") {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            other => other.dump(),
        };
        let keys: Vec<String> = tokenise(&text).iter().map(|token| match_key(token)).filter(|key| !key.is_empty()).collect();
        let span = (end - start).max(0);
        let count = keys.len() as i64;
        // A recognised CJK "word" covers several lyric characters; share its time between them.
        for (index, key) in keys.into_iter().enumerate() {
            let index = index as i64;
            units.push((start + span * index / count, start + span * (index + 1) / count, key));
        }
    }
    units.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    units
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => Some(*number),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

pub fn load_units(path: &Path) -> Result<Vec<Unit>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{} could not be read ({error})", path.display()))?;
    let data = Value::parse(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let units = timed_units(&data);
    if units.is_empty() {
        return Err(format!("No word timings found in {}", path.display()));
    }
    Ok(units)
}

/// The key lyric tokens and recognised words are matched on: case, punctuation
/// and Traditional/Simplified Chinese do not matter.
pub fn match_key(token: &str) -> String {
    token_key(&crate::romanize::simplified(token))
}

/// Score of an exact match; near matches score less, unrelated words 0.
const EXACT: i32 = 10;
/// Bonus for a match that continues a run of matches, so runs beat scattered words.
const RUN_BONUS: i32 = 2;

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(ca != cb));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// How alike a lyric key and a heard key are: EXACT when equal, less for a
/// misheard spelling of the same word ("runnin" / "running"), 0 otherwise.
pub fn similarity(lyric: &str, heard: &str) -> i32 {
    if lyric == heard {
        return EXACT;
    }
    let (a, b): (Vec<char>, Vec<char>) = (lyric.chars().collect(), heard.chars().collect());
    // Short words and CJK carry too little spelling to compare; a different first
    // letter is usually a different word ("love" / "dove").
    if a.len() < 4 || b.len() < 4 || a[0] != b[0] || a.iter().chain(&b).any(|c| crate::text::is_cjk_unit(*c)) {
        return 0;
    }
    let longest = a.len().max(b.len());
    if a.len().min(b.len()) * 3 < longest * 2 {
        return 0;
    }
    let ratio = 1.0 - edit_distance(&a, &b) as f64 / longest as f64;
    if ratio < 0.65 {
        return 0;
    }
    (ratio * (EXACT - 1) as f64).floor() as i32
}

/// Pairs (lyric index, unit index) of the best in-order alignment: the most
/// similar words, preferring unbroken runs. `allowed` rules out pairs whose
/// recognised word lies outside the lyric token's line.
fn align_pairs(keys: &[String], units: &[Unit], allowed: &dyn Fn(usize, usize) -> bool) -> Vec<(usize, usize)> {
    let (n, m) = (keys.len(), units.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    // Score each distinct pair of words once.
    let mut ids = std::collections::HashMap::new();
    let lyric_ids: Vec<usize> = keys.iter().map(|key| { let next = ids.len(); *ids.entry(key.as_str()).or_insert(next) }).collect();
    let lyric_count = ids.len();
    let mut heard_ids = std::collections::HashMap::new();
    let unit_ids: Vec<usize> = units.iter().map(|unit| { let next = heard_ids.len(); *heard_ids.entry(unit.2.as_str()).or_insert(next) }).collect();
    let mut lyric_words = vec![""; lyric_count];
    for (word, id) in &ids {
        lyric_words[*id] = word;
    }
    let mut heard_words = vec![""; heard_ids.len()];
    for (word, id) in &heard_ids {
        heard_words[*id] = word;
    }
    let scores: Vec<i32> = lyric_words.iter().flat_map(|a| heard_words.iter().map(move |b| similarity(a, b))).collect();
    let heard_count = heard_words.len();
    // best: the best score of keys[..i] against units[..j]; run: the same, ending in the match (i-1, j-1).
    // Steps: 0 from above, 1 from the left, 2 a match after `best`, 3 a match continuing `run`.
    const NONE: i32 = i32::MIN / 2;
    let width = m + 1;
    let mut best_step = vec![0u8; (n + 1) * width];
    let mut run_step = vec![0u8; (n + 1) * width];
    let mut best_prev = vec![0i32; width];
    let mut run_prev = vec![NONE; width];
    let mut best_row = vec![0i32; width];
    let mut run_row = vec![NONE; width];
    for j in 1..=m {
        best_step[j] = 1;
    }
    for i in 1..=n {
        best_row[0] = 0;
        run_row[0] = NONE;
        best_step[i * width] = 0;
        for j in 1..=m {
            let mut run = NONE;
            let mut step = 2;
            let score = scores[lyric_ids[i - 1] * heard_count + unit_ids[j - 1]];
            if score > 0 && allowed(i - 1, j - 1) {
                run = best_prev[j - 1] + score;
                if run_prev[j - 1] > NONE && run_prev[j - 1] + score + RUN_BONUS > run {
                    run = run_prev[j - 1] + score + RUN_BONUS;
                    step = 3;
                }
            }
            run_row[j] = run;
            run_step[i * width + j] = step;
            let (mut best, mut how) = (best_prev[j], 0u8);
            if best_row[j - 1] > best {
                best = best_row[j - 1];
                how = 1;
            }
            if run > best {
                best = run;
                how = step;
            }
            best_row[j] = best;
            best_step[i * width + j] = how;
        }
        std::mem::swap(&mut best_prev, &mut best_row);
        std::mem::swap(&mut run_prev, &mut run_row);
    }
    let mut pairs = Vec::new();
    let (mut i, mut j, mut in_run) = (n, m, false);
    while i > 0 && j > 0 {
        let step = if in_run { run_step[i * width + j] } else { best_step[i * width + j] };
        match step {
            0 => i -= 1,
            1 => j -= 1,
            _ => {
                pairs.push((i - 1, j - 1));
                in_run = step == 3;
                i -= 1;
                j -= 1;
            }
        }
    }
    pairs.reverse();
    pairs
}

/// Time the unmatched lyric tokens `run` from the unmatched recognised words
/// `heard` between the same two matches: one to one when the counts agree
/// (misheard words or homophones in place), else spread over their time.
fn fill_run(times: &mut [Option<(i64, i64)>], run: &[usize], heard: &[usize], units: &[Unit]) {
    if run.is_empty() || heard.is_empty() {
        return;
    }
    if run.len() == heard.len() {
        for (token, unit) in run.iter().zip(heard) {
            times[*token] = Some((units[*unit].0, units[*unit].1));
        }
        return;
    }
    let (low, high) = (units[heard[0]].0, units[heard[heard.len() - 1]].1);
    let count = run.len() as i64;
    if MIN_SPREAD_MS * count <= high - low && high - low <= MAX_TOKEN_MS * count {
        for (offset, token) in run.iter().enumerate() {
            let offset = offset as i64;
            times[*token] = Some((low + (high - low) * offset / count, low + (high - low) * (offset + 1) / count));
        }
    }
}

/// How far a recognised word may be from its lyric line's time and still belong to it.
pub const LINE_LEAD_MS: i64 = 1000;
pub const LINE_TAIL_MS: i64 = 1000;

/// Give each lyric token the time of the recognised word it corresponds to, or None.
///
/// `line_of` gives each token's line and `windows` each line's (start, end) in ms;
/// with no windows the tokens are aligned over the whole recording.
pub fn match_lines(keys: &[String], line_of: &[usize], windows: Option<&[(i64, i64)]>, units: &[Unit]) -> Vec<Option<(i64, i64)>> {
    let mut times = vec![None; keys.len()];
    let fits = |line: usize, unit: usize| match windows {
        Some(windows) => units[unit].0 >= windows[line].0 - LINE_LEAD_MS && units[unit].0 <= windows[line].1 + LINE_TAIL_MS,
        None => true,
    };
    let pairs = align_pairs(keys, units, &|token, unit| fits(line_of[token], unit));
    for (token, unit) in &pairs {
        times[*token] = Some((units[*unit].0, units[*unit].1));
    }
    // Between two matches, give each line's unmatched tokens the unmatched words heard in its time.
    let mut previous: (usize, usize) = (0, 0);
    for anchor in pairs.iter().map(|(token, unit)| (*token, *unit)).chain(std::iter::once((keys.len(), units.len()))) {
        let tokens: Vec<usize> = (previous.0..anchor.0).filter(|token| times[*token].is_none()).collect();
        let mut start = 0;
        while start < tokens.len() {
            let line = line_of[tokens[start]];
            let mut stop = start;
            while stop < tokens.len() && line_of[tokens[stop]] == line {
                stop += 1;
            }
            let next_line_start = match (windows, tokens.get(stop)) {
                (Some(windows), Some(token)) => windows[line_of[*token]].0,
                _ => i64::MAX,
            };
            let first_line = start == 0;
            let heard: Vec<usize> = (previous.1..anchor.1)
                .filter(|unit| fits(line, *unit) && units[*unit].0 < next_line_start && (first_line || windows.map_or(true, |windows| units[*unit].0 >= windows[line].0)))
                .collect();
            fill_run(&mut times, &tokens[start..stop], &heard, units);
            start = stop;
        }
        previous = (anchor.0 + 1, anchor.1 + 1);
    }
    times
}

/// Give each lyric token the time of the recognised word it corresponds to, or None.
pub fn match_tokens(keys: &[String], units: &[Unit]) -> Vec<Option<(i64, i64)>> {
    match_lines(keys, &vec![0; keys.len()], None, units)
}

/// The constant offset (ms) between a timed lyric file and the recording, such as
/// an album LRC on a video with a longer intro, or 0 when the lines already fit
/// or the evidence is weak. Measured from lines whose first word was recognised.
pub fn line_offset(keys: &[String], line_of: &[usize], starts: &[i64], units: &[Unit]) -> i64 {
    let pairs = align_pairs(keys, units, &|_, _| true);
    let mut deltas: Vec<i64> = Vec::new();
    for (token, unit) in pairs {
        if token == 0 || line_of[token - 1] != line_of[token] {
            deltas.push(units[unit].0 - starts[line_of[token]]);
        }
    }
    if deltas.len() < 3 || deltas.len() * 4 < starts.len() {
        return 0;
    }
    let median = |values: &mut Vec<i64>| {
        values.sort_unstable();
        values[values.len() / 2]
    };
    let middle = median(&mut deltas);
    let mut close: Vec<i64> = deltas.iter().copied().filter(|delta| (delta - middle).abs() <= 700).collect();
    if close.len() * 5 < deltas.len() * 3 {
        return 0;
    }
    let offset = median(&mut close);
    if offset.abs() < 400 {
        0
    } else {
        offset
    }
}

/// Estimate unmatched tokens from their matched neighbours and keep the sequence monotonic.
pub fn fill_gaps(times: &[Option<(i64, i64)>], low: i64, high: i64) -> Vec<(i64, i64)> {
    let mut filled: Vec<Option<(i64, i64)>> = times.to_vec();
    let mut index = 0;
    while index < filled.len() {
        if filled[index].is_some() {
            index += 1;
            continue;
        }
        let mut stop = index;
        while stop < filled.len() && filled[stop].is_none() {
            stop += 1;
        }
        let count = (stop - index) as i64;
        let left = if index > 0 { filled[index - 1].map(|time| time.1) } else { None };
        let right = if stop < filled.len() { filled[stop].map(|time| time.0) } else { None };
        let (left, right) = match (left, right) {
            (None, None) => (low, high.min(low + count * MAX_TOKEN_MS)),
            (None, Some(right)) => (low.max(right - count * TYPICAL_TOKEN_MS), right),
            (Some(left), None) => (left, high.min(left + count * TYPICAL_TOKEN_MS)),
            (Some(left), Some(right)) => (left, right.min(left + count * MAX_TOKEN_MS)),
        };
        let right = right.max(left);
        for offset in 0..count {
            filled[index + offset as usize] = Some((left + (right - left) * offset / count, left + (right - left) * (offset + 1) / count));
        }
        index = stop;
    }
    let mut cursor = low;
    filled
        .into_iter()
        .map(|time| {
            let (left, right) = time.unwrap_or((cursor, cursor));
            let left = left.max(cursor).min(high);
            let right = right.max(left).min(high);
            cursor = right;
            (left, right)
        })
        .collect()
}

/// Attach word timing to each line by matching lyric tokens against recognised words.
///
/// Lines that already carry word timing are kept. Start times of lines where
/// nothing was recognised are added to `estimated`.
pub fn time_entries(entries: &[Entry], units: &[Unit], max_tail_ms: i64, tail_hold_ms: i64, estimated: &mut Vec<i64>) -> Vec<Entry> {
    time_entries_with_offset(entries, units, max_tail_ms, tail_hold_ms, estimated).0
}

/// [`time_entries`], also returning the offset (ms) the lines were moved by to fit the recording.
///
/// The whole song is aligned at once, so a line's words can be found slightly
/// before or after its time in the file, and a repeated chorus cannot claim the
/// words of the next one.
pub fn time_entries_with_offset(entries: &[Entry], units: &[Unit], max_tail_ms: i64, tail_hold_ms: i64, estimated: &mut Vec<i64>) -> (Vec<Entry>, i64) {
    let lines: Vec<Vec<String>> = entries
        .iter()
        .map(|entry| if entry.words.is_empty() { tokenise(&split_speaker(&entry.text).1) } else { Vec::new() })
        .collect();
    let todo: Vec<usize> = (0..entries.len()).filter(|index| !lines[*index].is_empty()).collect();
    let mut keys = Vec::new();
    let mut line_of = Vec::new();
    for (line, index) in todo.iter().enumerate() {
        for token in &lines[*index] {
            keys.push(match_key(token));
            line_of.push(line);
        }
    }
    let starts: Vec<i64> = todo.iter().map(|index| entries[*index].start).collect();
    let offset = if units.is_empty() { 0 } else { line_offset(&keys, &line_of, &starts, units) };
    let windows: Vec<(i64, i64)> = todo.iter().map(|index| (entries[*index].start + offset, entries[*index].end + offset)).collect();
    let times = match_lines(&keys, &line_of, Some(&windows), units);
    let mut result = Vec::new();
    let mut cursor = 0;
    let mut line = 0;
    let mut previous_end = i64::MIN;
    for (index, entry) in entries.iter().enumerate() {
        if !entry.words.is_empty() {
            previous_end = entry.words.last().map_or(entry.end, |word| word.1);
            result.push(entry.clone());
            continue;
        }
        if lines[index].is_empty() {
            result.push(Entry::new(entry.start + offset, entry.end + offset, entry.text.clone(), Vec::new()));
            continue;
        }
        let count = lines[index].len();
        let line_times = &times[cursor..cursor + count];
        cursor += count;
        let (start, end) = windows[line];
        line += 1;
        let first = line_times.iter().flatten().next().map(|time| time.0);
        let last = line_times.iter().flatten().last().map(|time| time.1);
        let recognised = first.is_some();
        if !recognised {
            estimated.push(start);
        }
        // Words heard a little before or after the line's time still belong to it.
        let low = first.map_or(start, |first| first.min(start.max(previous_end)));
        let high = last.map_or(end, |last| last.max(end)).max(low);
        let timed: Vec<Word> = fill_gaps(line_times, low, high).into_iter().zip(lines[index].iter().cloned()).map(|((left, right), token)| (left, right, token)).collect();
        let mut end = high;
        // A line that runs into a long instrumental should leave the screen after its last word.
        let last_end = timed.last().map(|word| word.1).unwrap_or(end);
        previous_end = last_end;
        if recognised && max_tail_ms > 0 && end - last_end > max_tail_ms {
            end = (low + 300).max(last_end + tail_hold_ms.max(0));
        }
        result.push(Entry::new(low, end, entry.text.clone(), timed));
    }
    (result, offset)
}

/// Time untimed lyric lines by aligning the whole text against the recognised words.
pub fn plain_entries(text: &str, units: &[Unit]) -> Vec<Entry> {
    let lines: Vec<Vec<String>> = text.lines().map(|line| tokenise(line.trim())).filter(|tokens| !tokens.is_empty()).collect();
    let keys: Vec<String> = lines.iter().flatten().map(|token| match_key(token)).collect();
    if keys.is_empty() || units.is_empty() {
        return Vec::new();
    }
    let times = fill_gaps(&match_tokens(&keys, units), units[0].0, units[units.len() - 1].1);
    let mut result = Vec::new();
    let mut cursor = 0;
    for tokens in lines {
        let timed: Vec<Word> = times[cursor..cursor + tokens.len()].iter().zip(&tokens).map(|((left, right), token)| (*left, *right, token.clone())).collect();
        cursor += tokens.len();
        let start = timed[0].0;
        let end = (start + 300).max(timed[timed.len() - 1].1);
        result.push(Entry::new(start, end, tokens.concat(), timed));
    }
    result
}

/// "[mm:ss.xx]" (or "<mm:ss.xx>" with `open` '<') at the start of `text`: (ms, length in bytes).
pub fn time_mark(text: &str, open: char, close: char) -> Option<(i64, usize)> {
    let rest = text.strip_prefix(open)?;
    let end = rest.find(close)?;
    let inside = &rest[..end];
    let (minutes, after) = inside.split_once(':')?;
    if minutes.is_empty() || !minutes.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (seconds, fraction) = match after.find(['.', ':']) {
        Some(at) => (&after[..at], &after[at + 1..]),
        None => (after, ""),
    };
    if seconds.len() != 2 || !seconds.chars().all(|c| c.is_ascii_digit()) || fraction.len() > 3 || !fraction.chars().all(|c| c.is_ascii_digit()) || (after.contains(['.', ':']) && fraction.is_empty()) {
        return None;
    }
    let fraction: i64 = format!("{fraction}000")[..3].parse().ok()?;
    let value = (minutes.parse::<i64>().ok()? * 60 + seconds.parse::<i64>().ok()?) * 1000 + fraction;
    Some((value, open.len_utf8() + end + close.len_utf8()))
}

/// Split an enhanced-LRC line into its plain text and (start, end, word) timings.
pub fn inline_words(lyric: &str, start: i64, end: i64) -> (String, Vec<Word>) {
    let mut marks: Vec<(usize, usize, i64)> = Vec::new();
    let mut search = 0;
    while let Some(found) = lyric[search..].find('<') {
        let at = search + found;
        match time_mark(&lyric[at..], '<', '>') {
            Some((value, length)) => {
                marks.push((at, at + length, value));
                search = at + length;
            }
            None => search = at + 1,
        }
    }
    if marks.is_empty() {
        return (lyric.trim().to_string(), Vec::new());
    }
    let mut pieces: Vec<(i64, &str)> = vec![(start, &lyric[..marks[0].0])];
    for (index, (_, after, value)) in marks.iter().enumerate() {
        let stop = marks.get(index + 1).map(|mark| mark.0).unwrap_or(lyric.len());
        pieces.push((*value, &lyric[*after..stop]));
    }
    let mut words: Vec<Word> = Vec::new();
    for (index, (left, piece)) in pieces.iter().enumerate() {
        if piece.trim().is_empty() {
            continue;
        }
        let following = pieces.get(index + 1);
        let right = following.map(|item| item.0).unwrap_or(end);
        let spaced = piece.chars().last().map_or(false, char::is_whitespace) || following.map_or(false, |item| item.1.chars().next().map_or(false, char::is_whitespace));
        words.push(((*left).max(start), (left + 10).max(right), format!("{}{}", piece.trim(), if spaced { " " } else { "" })));
    }
    if let Some(last) = words.last_mut() {
        last.2 = last.2.trim_end().to_string();
    }
    (words.iter().map(|word| word.2.as_str()).collect(), words)
}

fn offset_tag(line: &str) -> Option<i64> {
    let rest = line.trim_start().strip_prefix('[')?;
    let lower = rest.to_lowercase();
    let after = lower.strip_prefix("offset")?.trim_start().strip_prefix(':')?.trim_start();
    let close = after.find(']')?;
    after[..close].trim().parse().ok()
}

pub fn parse_lrc(content: &str) -> Vec<Entry> {
    let mut offset = 0;
    let mut items: Vec<(i64, String)> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if let Some(value) = offset_tag(line) {
            // A positive LRC offset makes the lyrics appear earlier.
            offset = -value;
            continue;
        }
        let mut marks = Vec::new();
        let mut position = 0;
        while let Some((value, length)) = time_mark(&line[position..], '[', ']') {
            marks.push(value);
            position += length;
            position += line[position..].len() - line[position..].trim_start().len();
        }
        if marks.is_empty() {
            continue;
        }
        // A timestamp without text marks where the previous line ends.
        let text = line[position..].trim().to_string();
        items.extend(marks.into_iter().map(|value| (value, text.clone())));
    }
    items.sort_by_key(|item| item.0);
    let mut result = Vec::new();
    for (index, (start, raw)) in items.iter().enumerate() {
        let (speaker, lyric) = split_speaker(raw);
        if lyric.is_empty() {
            continue;
        }
        // Lines sharing a timestamp (original plus translation) end together at the next later mark.
        let fallback = start + 2000i64.max(lyric.chars().count() as i64 * 180);
        let end = items[index + 1..].iter().map(|item| item.0).find(|time| time > start).unwrap_or(fallback);
        let end = end.max(start + 300);
        let (clean, words) = inline_words(&lyric, *start, end);
        if clean.is_empty() {
            continue;
        }
        let text = if speaker.is_empty() { clean } else { format!("[speaker:{speaker}] {clean}") };
        let shift = (-start).max(offset);
        result.push(Entry::new(start + shift, end + shift, text, words.into_iter().map(|(left, right, word)| (left + shift, right + shift, word)).collect()));
    }
    result
}

/// "01:02:03,456" or "02:03.456" in ms.
pub fn subtitle_time(value: &str) -> Option<i64> {
    let value = value.replace(',', ".");
    let parts: Vec<&str> = value.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => ("0", *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    let (whole, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let fraction: i64 = format!("{fraction}000").get(..3)?.parse().ok()?;
    Some((hours.parse::<i64>().ok()? * 3600 + minutes.parse::<i64>().ok()? * 60 + whole.parse::<i64>().ok()?) * 1000 + fraction)
}

fn time_like(value: &str) -> bool {
    let parts: Vec<&str> = value.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return false;
    }
    let last = parts[parts.len() - 1];
    let (seconds, fraction) = match last.find([',', '.']) {
        Some(at) => (&last[..at], &last[at + 1..]),
        None => return false,
    };
    parts[..parts.len() - 1].iter().all(|part| (1..=2).contains(&part.len()) && part.chars().all(|c| c.is_ascii_digit()))
        && seconds.len() == 2
        && seconds.chars().all(|c| c.is_ascii_digit())
        && (1..=3).contains(&fraction.len())
        && fraction.chars().all(|c| c.is_ascii_digit())
}

/// The start and end of a "00:01.000 --> 00:02.000" cue line.
fn cue_times(line: &str) -> Option<(i64, i64)> {
    let arrow = line.find("-->")?;
    let before = line[..arrow].trim_end();
    let after = line[arrow + 3..].trim_start();
    let start = before.rsplit(char::is_whitespace).next()?;
    let end = after.split(char::is_whitespace).next()?;
    // The time may be glued to other text before it; take its longest time-like tail.
    let start = (0..start.len()).filter(|at| start.is_char_boundary(*at)).map(|at| &start[at..]).find(|tail| time_like(tail))?;
    let end = (1..=end.len()).rev().filter(|at| end.is_char_boundary(*at)).map(|at| &end[..at]).find(|head| time_like(head))?;
    Some((subtitle_time(start)?, subtitle_time(end)?))
}

fn strip_markup(line: &str) -> String {
    let mut out = String::new();
    let mut skip: Option<char> = None;
    for c in line.chars() {
        match skip {
            Some(close) if c == close => skip = None,
            Some(_) => {}
            None if c == '<' => skip = Some('>'),
            None if c == '{' => skip = Some('}'),
            None if matches!(c, '♪' | '♫' | '♬') => {}
            None => out.push(c),
        }
    }
    out.trim().to_string()
}

pub fn parse_timed_subtitle(content: &str) -> Vec<Entry> {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut index = 0;
    let mut previous = String::new();
    while index < lines.len() {
        let (start, end) = match cue_times(lines[index]) {
            Some(times) => times,
            None => {
                index += 1;
                continue;
            }
        };
        index += 1;
        let mut text_lines: Vec<String> = Vec::new();
        // YouTube captions contain whitespace-only lines inside a cue, so only a truly
        // empty line, or a blank one followed by the next cue, ends the text block.
        while index < lines.len() && !lines[index].is_empty() && !(lines[index].trim().is_empty() && lines[index + 1..lines.len().min(index + 3)].iter().any(|line| cue_times(line).is_some())) {
            let cleaned = strip_markup(lines[index]);
            if !cleaned.is_empty() {
                text_lines.push(cleaned);
            }
            index += 1;
        }
        // YouTube automatic captions repeat the previous line above each new one and
        // insert near-zero-length cues that only restate it.
        if end - start <= 50 {
            continue;
        }
        if text_lines.len() > 1 && text_lines[0] == previous {
            text_lines.remove(0);
        }
        if text_lines.is_empty() {
            continue;
        }
        previous = text_lines[text_lines.len() - 1].clone();
        let text = text_lines.join(" ");
        let (speaker, lyric) = split_speaker(&text);
        // Skip sound descriptions such as [Music] or [Applause].
        let description = lyric.starts_with('[') && lyric.ends_with(']') && !lyric[1..lyric.len() - 1].contains(']');
        if !lyric.is_empty() && !description {
            let tagged = if speaker.is_empty() { lyric } else { format!("[speaker:{speaker}] {lyric}") };
            result.push(Entry::new(start, (start + 300).max(end), tagged, Vec::new()));
        }
    }
    result
}

pub fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{} could not be read ({error})", path.display()))?;
    String::from_utf8(bytes).map(|text| text.trim_start_matches('\u{feff}').to_string()).map_err(|_| format!("{} is not UTF-8 text", path.display()))
}

pub fn lrc(path: &Path) -> Result<Vec<Entry>, String> {
    read_text(path).map(|text| parse_lrc(&text))
}

pub fn timed_subtitle(path: &Path) -> Result<Vec<Entry>, String> {
    read_text(path).map(|text| parse_timed_subtitle(&text))
}

/// The best downloaded subtitle: creator-made by default, automatic captions with `auto`.
pub fn downloaded_subtitle(row: &Row, auto: bool) -> Option<PathBuf> {
    let sid = get(row, "id");
    let folder = paths().subtitles.clone();
    let prefix = format!("{sid}.");
    let mut names: Vec<String> = std::fs::read_dir(&folder)
        .map(|entries| entries.filter_map(|entry| entry.ok()).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.retain(|name| {
        let lower = name.to_lowercase();
        name.starts_with(&prefix) && (lower.ends_with(".vtt") || lower.ends_with(".srt")) && lower.contains(".auto.") == auto
    });
    if names.is_empty() {
        return None;
    }
    let language = get(row, "language").trim().to_lowercase();
    let preferred: Vec<String> = get(row, "subtitle_langs")
        .split(',')
        .map(|item| item.trim().to_lowercase())
        .filter(|item| !item.is_empty() && item != "all")
        .map(|item| item.trim_end_matches(['*', '.']).to_string())
        .collect();
    let tag = |name: &str| {
        let lower = name.to_lowercase();
        let rest = &lower[sid.len() + 1..];
        let rest = rest.strip_prefix("auto.").unwrap_or(rest);
        rest.rsplit_once('.').map(|(head, _)| head.to_string()).unwrap_or_default()
    };
    names.sort_by_key(|name| {
        let tag = tag(name);
        (if !language.is_empty() && tag.starts_with(&language) { 0 } else { 1 }, if preferred.iter().any(|item| !item.is_empty() && tag.starts_with(item.as_str())) { 0 } else { 1 }, name.clone())
    });
    Some(folder.join(&names[0]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_lrc_with_offsets_and_speakers() {
        let entries = parse_lrc("[offset:+500]\n[00:05.000]<00:05.000>Hello <00:05.500>PolyKara <00:06.200>world<00:06.800>\n[00:08.00][singer: 女] 唤醒\n[00:10.000]\n");
        assert_eq!(entries[0].words, vec![(4500, 5000, "Hello ".to_string()), (5000, 5700, "PolyKara ".to_string()), (5700, 6300, "world".to_string())]);
        assert_eq!(entries[0].text, "Hello PolyKara world");
        assert_eq!(entries[1].text, "[speaker:女] 唤醒");
        assert_eq!((entries[1].start, entries[1].end), (7500, 9500));
    }

    #[test]
    fn lines_sharing_a_timestamp_end_together() {
        let entries = parse_lrc("[00:01.00]原文\n[00:01.00]Translation\n[00:04.00]next\n");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].end, 4000);
        assert_eq!(entries[1].end, 4000);
    }

    #[test]
    fn youtube_captions() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.000 align:start\nfirst <c>line</c>\n\n00:00:03.000 --> 00:00:03.010\nfirst line\n\n00:00:03.010 --> 00:00:05.000\nfirst line\nsecond line\n\n00:00:06.000 --> 00:00:07.000\n[Music]\n";
        let entries = parse_timed_subtitle(vtt);
        assert_eq!(entries.iter().map(|entry| entry.text.as_str()).collect::<Vec<_>>(), vec!["first line", "second line"]);
        assert_eq!(subtitle_time("01:02:03,456"), Some(3_723_456));
    }

    #[test]
    fn words_are_matched_to_what_was_heard() {
        let units: Vec<Unit> = vec![(1000, 1300, "hello".into()), (1300, 1800, "world".into()), (2000, 2200, "again".into())];
        let entries = vec![Entry::new(900, 2500, "Hello big world again", vec![])];
        let mut estimated = Vec::new();
        let timed = time_entries(&entries, &units, 0, 1000, &mut estimated);
        let starts: Vec<i64> = timed[0].words.iter().map(|word| word.0).collect();
        assert_eq!(timed[0].words[0], (1000, 1300, "Hello ".to_string()));
        assert_eq!(starts[2], 1300);
        assert!(starts.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(estimated.is_empty());
        assert_eq!(fill_gaps(&[None, Some((500, 600)), None], 0, 1000), vec![(100, 500), (500, 600), (600, 1000)]);
    }

    fn heard(words: &[(i64, &str)]) -> Vec<Unit> {
        words.iter().map(|(start, word)| (*start, start + 200, match_key(word))).collect()
    }

    #[test]
    fn misheard_spellings_still_match() {
        assert_eq!(similarity("running", "running"), EXACT);
        assert!(similarity("runnin", "running") > 0);
        assert_eq!(similarity("love", "dove"), 0, "short words must match exactly");
        assert_eq!(similarity("你", "妳"), 0);
        // "tonight" was heard as "tonite": still the anchor for its line, so "stay" is not pulled onto it.
        let units = heard(&[(1000, "stay"), (1400, "with"), (1800, "me"), (2200, "tonite")]);
        let entries = vec![Entry::new(900, 3000, "Stay with me tonight", vec![])];
        let timed = time_entries(&entries, &units, 0, 1000, &mut Vec::new());
        assert_eq!(timed[0].words.iter().map(|word| word.0).collect::<Vec<_>>(), vec![1000, 1400, 1800, 2200]);
    }

    #[test]
    fn a_repeated_line_keeps_to_its_own_time() {
        // The same words are sung twice; each line takes the words heard in its own time.
        let units = heard(&[(1000, "oh"), (1300, "baby"), (5000, "oh"), (5300, "baby")]);
        let entries = vec![Entry::new(4800, 6000, "oh baby", vec![]), Entry::new(9000, 10000, "something else here", vec![])];
        let timed = time_entries(&entries, &units, 0, 1000, &mut Vec::new());
        assert_eq!(timed[0].words[0].0, 5000);
        assert_eq!(timed[0].words[1].0, 5300);
    }

    #[test]
    fn words_slightly_before_the_line_belong_to_it() {
        let units = heard(&[(1000, "one"), (1500, "two"), (2400, "three"), (2800, "four")]);
        let entries = vec![Entry::new(900, 2600, "one two", vec![]), Entry::new(2600, 4000, "three four", vec![])];
        let timed = time_entries(&entries, &units, 0, 1000, &mut Vec::new());
        assert_eq!(timed[1].start, 2400, "the line starts when its first word is sung");
        assert_eq!(timed[1].words[0].0, 2400);
        assert!(timed[0].end <= timed[1].words[0].0 || timed[0].words[1].1 <= timed[1].words[0].0);
    }

    #[test]
    fn a_shifted_lyric_file_is_moved_onto_the_recording() {
        // An album LRC on a video whose intro is 4 s longer: every line is heard 4 s late.
        let lines = ["first line of the song", "second line goes here", "third line follows on", "and the fourth line ends"];
        let mut entries = Vec::new();
        let mut words = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let start = 10_000 + index as i64 * 5000;
            entries.push(Entry::new(start, start + 5000, *line, vec![]));
            for (offset, word) in line.split(' ').enumerate() {
                words.push((start + 4000 + 50 + offset as i64 * 400, word));
            }
        }
        let units = heard(&words);
        let mut estimated = Vec::new();
        let (timed, offset) = time_entries_with_offset(&entries, &units, 0, 1000, &mut estimated);
        assert_eq!(offset, 4050);
        assert!(estimated.is_empty());
        for (index, entry) in timed.iter().enumerate() {
            assert_eq!(entry.words[0].0, 14_050 + index as i64 * 5000);
            assert_eq!(entry.words.last().unwrap().0, 14_050 + index as i64 * 5000 + 400 * (entry.words.len() as i64 - 1));
        }
        // Lines already in time are left alone.
        let (_, none) = time_entries_with_offset(&timed, &units, 0, 1000, &mut Vec::new());
        assert_eq!(none, 0);
    }

    #[test]
    fn a_misheard_first_character_takes_its_place() {
        // whisper wrote a homophone for the first character: same count, so it keeps the time.
        let units = heard(&[(1000, "您"), (1200, "好"), (1400, "吗")]);
        let entries = vec![Entry::new(900, 2000, "你好吗", vec![])];
        let timed = time_entries(&entries, &units, 0, 1000, &mut Vec::new());
        assert_eq!(timed[0].words.iter().map(|word| word.0).collect::<Vec<_>>(), vec![1000, 1200, 1400]);
    }

    #[test]
    fn aligner_json_becomes_units() {
        let data = Value::parse(r#"{"segments": [{"words": [{"start": 1.0, "end": 1.6, "word": " 唤不醒"}, {"start": 2.0, "end": 2.5, "word": "Hi!"}, {"start": null, "end": 3, "word": "x"}]}]}"#).unwrap();
        let units = timed_units(&data);
        assert_eq!(units, vec![(1000, 1200, "唤".into()), (1200, 1400, "不".into()), (1400, 1600, "醒".into()), (2000, 2500, "hi".into())]);
        let plain = plain_entries("唤不醒\nhi", &units);
        assert_eq!(plain[1].words[0].0, 2000);
    }
}
