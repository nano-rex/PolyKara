use crate::lyricformats::tokenize;
use std::collections::HashSet;

pub fn agreement(left: &[String], right: &[String]) -> f64 {
    let a: HashSet<_> = left.iter().collect();
    let b: HashSet<_> = right.iter().collect();
    if a.is_empty() && b.is_empty() {
        1.0
    } else {
        a.intersection(&b).count() as f64 / a.union(&b).count() as f64
    }
}
pub fn compare(texts: &[String], min_sources: usize, threshold: f64) -> bool {
    if texts.len() < min_sources {
        return false;
    }
    let tokens: Vec<_> = texts.iter().map(|x| tokenize(x)).collect();
    let mut best = 0;
    for i in 0..tokens.len() {
        let count = tokens
            .iter()
            .filter(|x| agreement(&tokens[i], x) >= threshold)
            .count();
        best = best.max(count);
    }
    best >= min_sources
}
