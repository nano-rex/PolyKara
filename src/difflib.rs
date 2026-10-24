//! Python's difflib.SequenceMatcher (with autojunk off and no junk), which lyric matching relies on.

use std::collections::HashMap;
use std::hash::Hash;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Equal,
    Replace,
    Delete,
    Insert,
}

pub struct Matcher<'a, T: Eq + Hash> {
    a: &'a [T],
    b: &'a [T],
    b2j: HashMap<&'a T, Vec<usize>>,
}

impl<'a, T: Eq + Hash> Matcher<'a, T> {
    pub fn new(a: &'a [T], b: &'a [T]) -> Self {
        let mut b2j: HashMap<&T, Vec<usize>> = HashMap::new();
        for (index, item) in b.iter().enumerate() {
            b2j.entry(item).or_default().push(index);
        }
        Matcher { a, b, b2j }
    }

    fn longest_match(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut next: HashMap<usize, usize> = HashMap::new();
            if let Some(positions) = self.b2j.get(&self.a[i]) {
                for &j in positions {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j.checked_sub(1).and_then(|before| j2len.get(&before)).copied().unwrap_or(0) + 1;
                    next.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            j2len = next;
        }
        while besti > alo && bestj > blo && self.a[besti - 1] == self.b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && self.a[besti + bestsize] == self.b[bestj + bestsize] {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    /// (i, j, size) blocks where a[i..i+size] == b[j..j+size], ending with (len a, len b, 0).
    pub fn matching_blocks(&self) -> Vec<(usize, usize, usize)> {
        let (la, lb) = (self.a.len(), self.b.len());
        let mut queue = vec![(0, la, 0, lb)];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest_match(alo, ahi, blo, bhi);
            if k > 0 {
                blocks.push((i, j, k));
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        blocks.sort_unstable();
        let mut merged: Vec<(usize, usize, usize)> = Vec::new();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        for (i2, j2, k2) in blocks {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    merged.push((i1, j1, k1));
                }
                i1 = i2;
                j1 = j2;
                k1 = k2;
            }
        }
        if k1 > 0 {
            merged.push((i1, j1, k1));
        }
        merged.push((la, lb, 0));
        merged
    }

    pub fn opcodes(&self) -> Vec<(Tag, usize, usize, usize, usize)> {
        let (mut i, mut j) = (0, 0);
        let mut answer = Vec::new();
        for (ai, bj, size) in self.matching_blocks() {
            let tag = if i < ai && j < bj {
                Some(Tag::Replace)
            } else if i < ai {
                Some(Tag::Delete)
            } else if j < bj {
                Some(Tag::Insert)
            } else {
                None
            };
            if let Some(tag) = tag {
                answer.push((tag, i, ai, j, bj));
            }
            i = ai + size;
            j = bj + size;
            if size > 0 {
                answer.push((Tag::Equal, ai, i, bj, j));
            }
        }
        answer
    }

    pub fn ratio(&self) -> f64 {
        let matches: usize = self.matching_blocks().iter().map(|block| block.2).sum();
        let total = self.a.len() + self.b.len();
        if total == 0 {
            1.0
        } else {
            2.0 * matches as f64 / total as f64
        }
    }
}

/// Similarity of two strings by characters, as SequenceMatcher(None, a, b).ratio().
pub fn text_ratio(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    Matcher::new(&a, &b).ratio()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python() {
        // difflib.SequenceMatcher(None, "abxcd", "abcd", autojunk=False)
        let (a, b): (Vec<char>, Vec<char>) = ("abxcd".chars().collect(), "abcd".chars().collect());
        let matcher = Matcher::new(&a, &b);
        assert_eq!(matcher.matching_blocks(), vec![(0, 0, 2), (3, 2, 2), (5, 4, 0)]);
        assert_eq!(matcher.opcodes(), vec![(Tag::Equal, 0, 2, 0, 2), (Tag::Delete, 2, 3, 2, 2), (Tag::Equal, 3, 5, 2, 4)]);
        assert!((matcher.ratio() - 8.0 / 9.0).abs() < 1e-9);
        // "qabxcd" vs "abycdf": the Python documentation example.
        let (a, b): (Vec<char>, Vec<char>) = ("qabxcd".chars().collect(), "abycdf".chars().collect());
        let codes = Matcher::new(&a, &b).opcodes();
        assert_eq!(codes, vec![(Tag::Delete, 0, 1, 0, 0), (Tag::Equal, 1, 3, 0, 2), (Tag::Replace, 3, 4, 2, 3), (Tag::Equal, 4, 6, 3, 5), (Tag::Insert, 6, 6, 5, 6)]);
        assert_eq!(text_ratio("", ""), 1.0);
    }
}
