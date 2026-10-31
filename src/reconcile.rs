//! Compare the online lyrics with what whisper.cpp heard in the isolated vocal.
//!
//! The vocal transcript is an independent witness of the words actually sung. It
//! is not trusted to rewrite the lyrics on its own — automatic speech recognition
//! mishears sung words often — so PolyKara uses it to *judge* the online lyrics:
//! lines the transcript clearly confirms are trusted, and lines it contradicts are
//! flagged for a human to check in the editor. The transcript is also saved as a
//! low-trust lyric source, so a single wrong online source can be outvoted by the
//! accuracy check (see [`crate::verify`]).

use crate::difflib::Matcher;
use crate::verify::{agreement, comparison_tokens};

/// How one online lyric line fared against the transcript.
#[derive(Clone, Debug)]
pub struct LineCheck {
    pub text: String,
    /// Share of the line's words found, in order, in the transcript (0..1).
    pub confidence: f64,
    /// Whether the line is confirmed at the chosen threshold.
    pub ok: bool,
}

/// The outcome of reconciling a song's lyrics with its vocal transcript.
#[derive(Clone, Debug)]
pub struct Report {
    pub lines: Vec<LineCheck>,
    /// Overall word agreement between the lyrics and the transcript (0..1).
    pub agreement: f64,
}

impl Report {
    pub fn confirmed(&self) -> usize {
        self.lines.iter().filter(|line| line.ok).count()
    }

    pub fn flagged(&self) -> Vec<&LineCheck> {
        self.lines.iter().filter(|line| !line.ok).collect()
    }
}

/// The share of `needle` tokens that appear, in order, within `haystack`.
pub fn coverage(needle: &[String], haystack: &[String]) -> f64 {
    if needle.is_empty() {
        return 1.0;
    }
    if haystack.is_empty() {
        return 0.0;
    }
    let matched: usize = Matcher::new(needle, haystack).matching_blocks().iter().map(|block| block.2).sum();
    matched as f64 / needle.len() as f64
}

/// Judge each online lyric line against the heard transcript tokens. `heard` must
/// already be comparison tokens (see [`comparison_tokens`]); `threshold` is the
/// share of a line's words that must be heard for it to count as confirmed.
pub fn reconcile(online_lines: &[String], heard: &[String], threshold: f64) -> Report {
    let mut lines = Vec::new();
    let mut all_lyric_tokens: Vec<String> = Vec::new();
    for line in online_lines {
        let tokens = comparison_tokens(line);
        all_lyric_tokens.extend(tokens.iter().cloned());
        // A line with no comparable words (blank, punctuation, "[Chorus]") is not judged.
        let confidence = coverage(&tokens, heard);
        let ok = tokens.is_empty() || confidence >= threshold;
        lines.push(LineCheck { text: line.clone(), confidence, ok });
    }
    Report { lines, agreement: agreement(&all_lyric_tokens, heard) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<String> {
        comparison_tokens(text)
    }

    #[test]
    fn coverage_counts_in_order_matches() {
        let line = tokens("the quick brown fox");
        assert_eq!(coverage(&line, &tokens("the quick brown fox")), 1.0);
        assert_eq!(coverage(&line, &tokens("a quick lazy fox")), 0.5); // quick + fox of 4
        assert_eq!(coverage(&line, &[]), 0.0);
        assert_eq!(coverage(&[], &tokens("anything")), 1.0);
    }

    #[test]
    fn confirmed_and_flagged_lines_are_separated() {
        let online = vec![
            "Hello from the other side".to_string(),
            "A line the singer never sang".to_string(),
            "...".to_string(),
        ];
        let heard = tokens("hello from the other side and some more words here");
        let report = reconcile(&online, &heard, 0.6);
        assert!(report.lines[0].ok, "a well-heard line is confirmed");
        assert!(!report.lines[1].ok, "an unheard line is flagged");
        assert!(report.lines[2].ok, "a punctuation-only line has no words and is not judged");
        assert_eq!(report.confirmed(), 2);
        assert_eq!(report.flagged().len(), 1);
    }

    #[test]
    fn traditional_and_simplified_chinese_agree() {
        // comparison_tokens folds Traditional to Simplified, so the transcript confirms it.
        let online = vec!["中華人民".to_string()];
        let heard = tokens("中华人民");
        let report = reconcile(&online, &heard, 0.6);
        assert!(report.lines[0].ok);
    }
}
