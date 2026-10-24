//! Lyric lines and the timing edits made to them, without any window code.
//!
//! The same shapes travel to and from the engine as JSON, so a line read from
//! a saved timing file comes back exactly as the editor left it.

use crate::json::Value;

/// The shortest a timed word may be, in milliseconds.
pub const MIN_TOKEN_MS: i64 = 30;
/// How long a word whose key was never released lasts.
pub const OPEN_TOKEN_MS: i64 = 400;
const HISTORY: usize = 200;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Token {
    pub text: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
}

impl Token {
    pub fn timed(&self) -> bool {
        self.start.is_some() && self.end.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub tokens: Vec<Token>,
    pub speaker: String,
    /// Line times for lines whose words are not timed yet (from subtitles or synced lyrics).
    pub start: Option<i64>,
    pub end: Option<i64>,
}

impl Line {
    pub fn text(&self) -> String {
        self.tokens.iter().map(|token| token.text.as_str()).collect()
    }

    /// (start, end) from the timed words, else from the line times.
    pub fn span(&self) -> Option<(i64, i64)> {
        let mut found: Option<(i64, i64)> = None;
        for token in &self.tokens {
            if let (Some(start), Some(end)) = (token.start, token.end) {
                found = Some(match found {
                    Some((low, high)) => (low.min(start), high.max(end)),
                    None => (start, end),
                });
            }
        }
        found.or_else(|| self.start.map(|start| (start, self.end.unwrap_or(start + 2000))))
    }

    /// Whether any word of this line was timed here (not only given line times by a lyric file).
    pub fn started(&self) -> bool {
        self.tokens.iter().any(|token| token.start.is_some())
    }
}

impl Line {
    pub fn to_json(&self) -> Value {
        let tokens = self.tokens.iter().map(|token| Value::object(vec![("text", Value::from(&token.text)), ("start", Value::from(token.start)), ("end", Value::from(token.end))])).collect();
        Value::object(vec![("speaker", Value::from(&self.speaker)), ("start", Value::from(self.start)), ("end", Value::from(self.end)), ("tokens", Value::Array(tokens))])
    }

    pub fn from_json(value: &Value) -> Line {
        Line {
            tokens: value.get("tokens").items().iter().map(|token| Token { text: token.get("text").str().to_string(), start: token.get("start").as_i64(), end: token.get("end").as_i64() }).collect(),
            speaker: value.get("speaker").str().to_string(),
            start: value.get("start").as_i64(),
            end: value.get("end").as_i64(),
        }
    }
}

pub fn lines_json(lines: &[Line]) -> Value {
    Value::Array(lines.iter().map(Line::to_json).collect())
}

pub fn lines_from_json(value: &Value) -> Vec<Line> {
    value.items().iter().map(Line::from_json).collect()
}

pub type Place = (usize, usize);

pub fn clock(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{:02}:{:02}.{:03}", ms / 60000, ms / 1000 % 60, ms % 1000)
}

pub fn lines_to_text(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| if line.speaker.is_empty() { line.text() } else { format!("[singer: {}] {}", line.speaker, line.text()) })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn token(lines: &[Line], place: Option<Place>) -> Option<&Token> {
    let (li, ti) = place?;
    lines.get(li)?.tokens.get(ti)
}

pub fn token_mut(lines: &mut [Line], place: Option<Place>) -> Option<&mut Token> {
    let (li, ti) = place?;
    lines.get_mut(li)?.tokens.get_mut(ti)
}

pub fn first_untimed(lines: &[Line]) -> Option<Place> {
    for (li, line) in lines.iter().enumerate() {
        for (ti, token) in line.tokens.iter().enumerate() {
            if !token.timed() {
                return Some((li, ti));
            }
        }
    }
    if lines.is_empty() { None } else { Some((0, 0)) }
}

pub fn following(lines: &[Line], (li, ti): Place) -> Option<Place> {
    if ti + 1 < lines.get(li)?.tokens.len() {
        Some((li, ti + 1))
    } else if li + 1 < lines.len() {
        Some((li + 1, 0))
    } else {
        None
    }
}

pub fn previous(lines: &[Line], place: Option<Place>) -> Option<Place> {
    match place {
        None => {
            let last = lines.len().checked_sub(1)?;
            Some((last, lines[last].tokens.len().checked_sub(1)?))
        }
        Some((li, ti)) if ti > 0 => Some((li, ti - 1)),
        Some((li, _)) if li > 0 => Some((li - 1, lines[li - 1].tokens.len().checked_sub(1)?)),
        _ => None,
    }
}

/// Give a word whose key was never released an end: the next word's start, or 400 ms.
pub fn close_open(lines: &mut [Line]) {
    for line in lines.iter_mut() {
        for ti in 0..line.tokens.len() {
            if let (Some(start), None) = (line.tokens[ti].start, line.tokens[ti].end) {
                let after = line.tokens.get(ti + 1).and_then(|token| token.start);
                line.tokens[ti].end = Some(match after {
                    Some(after) if after > start => after,
                    _ => start + OPEN_TOKEN_MS,
                });
            }
        }
    }
}

/// The word being sung at ms.
pub fn sung_at(lines: &[Line], ms: i64) -> Option<Place> {
    let mut place = None;
    for (li, line) in lines.iter().enumerate() {
        for (ti, token) in line.tokens.iter().enumerate() {
            if let Some(start) = token.start {
                if start <= ms && ms < token.end.unwrap_or(start + OPEN_TOKEN_MS) {
                    place = Some((li, ti));
                }
            }
        }
    }
    place
}

/// The line being sung (or coming next) and the one after it, for the preview.
pub fn stage_lines(lines: &[Line], ms: i64, cursor: Option<Place>) -> Vec<usize> {
    if lines.is_empty() {
        return Vec::new();
    }
    // Only words timed here count: line times from other sources may not fit this recording.
    let mut current = None;
    for (li, line) in lines.iter().enumerate() {
        if let Some(first) = line.tokens.iter().filter_map(|token| token.start).min() {
            if first - 1500 <= ms {
                current = Some(li);
            }
        }
    }
    let waiting = cursor.map(|(li, _)| li);
    let current = match current {
        None => waiting.unwrap_or(0),
        Some(current) => {
            let line = &lines[current];
            let last_end = line.tokens.iter().filter_map(|token| token.end).max();
            let finished = matches!(last_end, Some(end) if ms > end + 800) && line.tokens.iter().all(Token::timed);
            if finished && current + 1 < lines.len() {
                match waiting {
                    Some(waiting) if waiting > current => waiting,
                    _ => current + 1,
                }
            } else {
                current
            }
        }
    };
    [current, current + 1].into_iter().filter(|&li| li < lines.len()).collect()
}

/// How far a word may move: up to its timed neighbours.
pub fn limits(lines: &[Line], place: Place) -> (i64, i64) {
    let low = token(lines, previous(lines, Some(place))).and_then(|token| token.end).unwrap_or(0);
    let high = token(lines, following(lines, place)).and_then(|token| token.start).unwrap_or(i64::MAX / 4);
    (low, high)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyMode {
    /// Press marks where a word starts, release where it ends.
    Hold,
    /// Each press starts the next word and ends the one before.
    Tap,
}

/// The lines of one song while they are being timed, with undo.
#[derive(Default)]
pub struct Session {
    pub lines: Vec<Line>,
    pub cursor: Option<Place>,
    pub selected: Option<Place>,
    pub holding: Option<Place>,
    pub dirty: bool,
    /// Counts edits, so a save that finishes after another edit leaves the song marked as changed.
    pub version: u64,
    history: Vec<(Vec<Line>, Option<Place>)>,
}

impl Session {
    pub fn new(lines: Vec<Line>) -> Self {
        let cursor = first_untimed(&lines);
        Session { lines, cursor, ..Default::default() }
    }

    pub fn remember(&mut self) {
        self.history.push((self.lines.clone(), self.cursor));
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.dirty = true;
        self.version += 1;
    }

    pub fn undo(&mut self) -> bool {
        match self.history.pop() {
            Some((lines, cursor)) => {
                self.lines = lines;
                self.cursor = cursor;
                self.holding = None;
                self.selected = None;
                true
            }
            None => false,
        }
    }

    pub fn replace_lines(&mut self, lines: Vec<Line>) {
        self.remember();
        self.lines = lines;
        self.cursor = first_untimed(&self.lines);
        self.selected = None;
        self.holding = None;
    }

    /// The timing key went down at `at` ms of the song.
    pub fn press(&mut self, at: i64, mode: KeyMode) {
        let (li, ti) = match self.cursor {
            Some(place) if token(&self.lines, Some(place)).is_some() => place,
            _ => return,
        };
        self.remember();
        let line = &mut self.lines[li];
        if mode == KeyMode::Tap && ti > 0 {
            let before = &mut line.tokens[ti - 1];
            if let (Some(start), None) = (before.start, before.end) {
                before.end = Some(at.max(start + MIN_TOKEN_MS));
            }
        }
        // Later words of this line timed after this point are from an earlier take.
        for later in line.tokens.iter_mut().skip(ti + 1) {
            if matches!(later.start, Some(start) if start < at) {
                later.start = None;
                later.end = None;
            }
        }
        line.tokens[ti].start = Some(at);
        line.tokens[ti].end = None;
        let last = ti + 1 == line.tokens.len();
        if mode == KeyMode::Hold || last {
            self.holding = Some((li, ti));
        } else {
            self.cursor = following(&self.lines, (li, ti));
        }
        self.selected = Some((li, ti));
    }

    /// The timing key came up (or playback stopped) at `at` ms.
    pub fn release(&mut self, at: i64) -> bool {
        let place = match self.holding.take() {
            Some(place) => place,
            None => return false,
        };
        if let Some(token) = token_mut(&mut self.lines, Some(place)) {
            if let Some(start) = token.start {
                token.end = Some(at.max(start + MIN_TOKEN_MS));
            }
        }
        self.cursor = following(&self.lines, place);
        true
    }

    /// Undo the last timed word and go back to it; returns where it started.
    pub fn step_back(&mut self) -> Option<Option<i64>> {
        if self.holding.is_some() {
            return None;
        }
        let place = previous(&self.lines, self.cursor);
        token(&self.lines, place)?;
        self.remember();
        let token = token_mut(&mut self.lines, place)?;
        let start = token.start;
        token.start = None;
        token.end = None;
        self.cursor = place;
        Some(start)
    }

    pub fn nudge(&mut self, delta: i64) -> bool {
        if !matches!(token(&self.lines, self.selected), Some(token) if token.timed()) {
            return false;
        }
        self.remember();
        let token = token_mut(&mut self.lines, self.selected).expect("checked above");
        token.start = token.start.map(|value| value + delta);
        token.end = token.end.map(|value| value + delta);
        true
    }

    pub fn clear_line(&mut self) -> bool {
        let li = match self.selected.or(self.cursor) {
            Some((li, _)) if li < self.lines.len() => li,
            _ => return false,
        };
        self.remember();
        let line = &mut self.lines[li];
        for token in &mut line.tokens {
            token.start = None;
            token.end = None;
        }
        line.start = None;
        line.end = None;
        self.cursor = Some((li, 0));
        true
    }

    pub fn anything_timed(&self) -> bool {
        self.lines.iter().any(|line| line.span().is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &[&str]) -> Line {
        Line { tokens: text.iter().map(|piece| Token { text: piece.to_string(), ..Default::default() }).collect(), ..Default::default() }
    }

    #[test]
    fn hold_marks_start_and_end_then_moves_on() {
        let mut session = Session::new(vec![line(&["a", "b"]), line(&["c"])]);
        session.press(1000, KeyMode::Hold);
        assert_eq!(session.holding, Some((0, 0)));
        session.release(1200);
        assert_eq!((session.lines[0].tokens[0].start, session.lines[0].tokens[0].end), (Some(1000), Some(1200)));
        assert_eq!(session.cursor, Some((0, 1)));
        // A release right after the press still gives the word its shortest length.
        session.press(1300, KeyMode::Hold);
        session.release(1301);
        assert_eq!(session.lines[0].tokens[1].end, Some(1300 + MIN_TOKEN_MS));
        assert_eq!(session.cursor, Some((1, 0)));
        assert!(session.dirty);
    }

    #[test]
    fn tap_ends_the_word_before() {
        let mut session = Session::new(vec![line(&["a", "b", "c"])]);
        session.press(1000, KeyMode::Tap);
        session.press(1500, KeyMode::Tap);
        assert_eq!(session.lines[0].tokens[0].end, Some(1500));
        assert_eq!(session.cursor, Some((0, 2)));
        // The last word of a line is held, so its end comes from the release.
        session.press(2000, KeyMode::Tap);
        assert_eq!(session.holding, Some((0, 2)));
        session.release(2400);
        assert_eq!(session.lines[0].tokens[2].end, Some(2400));
    }

    #[test]
    fn retiming_a_word_clears_later_words_of_an_earlier_take() {
        let mut session = Session::new(vec![line(&["a", "b"])]);
        session.lines[0].tokens[1] = Token { text: "b".into(), start: Some(900), end: Some(1100) };
        session.press(1000, KeyMode::Hold);
        assert_eq!(session.lines[0].tokens[1].start, None);
    }

    #[test]
    fn step_back_and_undo() {
        let mut session = Session::new(vec![line(&["a", "b"])]);
        session.press(1000, KeyMode::Hold);
        session.release(1200);
        assert_eq!(session.step_back(), Some(Some(1000)));
        assert_eq!(session.cursor, Some((0, 0)));
        assert!(!session.lines[0].tokens[0].timed());
        assert!(session.undo());
        assert!(session.lines[0].tokens[0].timed());
        assert_eq!(session.cursor, Some((0, 1)));
    }

    #[test]
    fn open_words_are_closed_by_the_next_start() {
        let mut lines = vec![line(&["a", "b", "c"])];
        lines[0].tokens[0].start = Some(1000);
        lines[0].tokens[1].start = Some(1300);
        lines[0].tokens[2].start = Some(1200);
        close_open(&mut lines);
        assert_eq!(lines[0].tokens[0].end, Some(1300));
        assert_eq!(lines[0].tokens[1].end, Some(1300 + OPEN_TOKEN_MS));
        assert_eq!(lines[0].tokens[2].end, Some(1200 + OPEN_TOKEN_MS));
    }

    #[test]
    fn neighbours_and_limits() {
        let mut lines = vec![line(&["a", "b"]), line(&["c"])];
        assert_eq!(following(&lines, (0, 1)), Some((1, 0)));
        assert_eq!(following(&lines, (1, 0)), None);
        assert_eq!(previous(&lines, Some((1, 0))), Some((0, 1)));
        assert_eq!(previous(&lines, None), Some((1, 0)));
        assert_eq!(previous(&lines, Some((0, 0))), None);
        lines[0].tokens[0].end = Some(500);
        lines[1].tokens[0].start = Some(3000);
        assert_eq!(limits(&lines, (0, 1)), (500, 3000));
    }

    #[test]
    fn preview_shows_the_sung_line_then_moves_on() {
        let mut lines = vec![line(&["a"]), line(&["b"]), line(&["c"])];
        assert_eq!(stage_lines(&lines, 0, Some((0, 0))), vec![0, 1]);
        lines[0].tokens[0] = Token { text: "a".into(), start: Some(2000), end: Some(2500) };
        assert_eq!(stage_lines(&lines, 1000, Some((1, 0))), vec![0, 1]);
        // Well after the first line ends, the preview waits on the line being timed.
        assert_eq!(stage_lines(&lines, 4000, Some((2, 0))), vec![2]);
        assert_eq!(sung_at(&lines, 2100), Some((0, 0)));
        assert_eq!(sung_at(&lines, 2600), None);
    }

    #[test]
    fn spans_and_text() {
        let mut first = line(&["Hello ", "world"]);
        first.speaker = "女".into();
        first.start = Some(100);
        assert_eq!(first.span(), Some((100, 2100)));
        first.tokens[1].start = Some(900);
        first.tokens[1].end = Some(1200);
        assert_eq!(first.span(), Some((900, 1200)));
        assert_eq!(lines_to_text(&[first, line(&["x"])]), "[singer: 女] Hello world\nx");
        assert_eq!(clock(83_456), "01:23.456");
        assert_eq!(clock(-5), "00:00.000");
    }

    #[test]
    fn lines_read_the_engine_json() {
        let parsed = Line::from_json(&Value::parse(r#"{"speaker": "", "start": null, "end": null, "tokens": [{"text": "a", "start": 5, "end": null}]}"#).unwrap());
        assert_eq!(parsed.tokens[0].start, Some(5));
        assert_eq!(parsed.tokens[0].end, None);
        assert_eq!(Line::from_json(&parsed.to_json()), parsed);
    }
}
