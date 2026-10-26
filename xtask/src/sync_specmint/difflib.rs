//! Python 3.11's `difflib.unified_diff` over lists of lines, as
//! `scripts/sync-specmint.py` calls it (`n=3`, `lineterm="\n"`, no dates):
//! `SequenceMatcher(None, a, b)` with its `autojunk` heuristic (in a `b` of
//! 200 or more lines, a line seen more than `len(b) // 100 + 1` times is
//! "popular" and never starts a match), `get_matching_blocks`,
//! `get_opcodes` and `get_grouped_opcodes`. Translated from CPython 3.11
//! `Lib/difflib.py` (`SequenceMatcher`, `unified_diff`,
//! `_format_range_unified`); see LICENSE-PSF.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    Replace,
    Delete,
    Insert,
    Equal,
}

type Opcode = (Tag, usize, usize, usize, usize);

struct Matcher<'a> {
    a: &'a [String],
    b: &'a [String],
    /// Where each non-popular line of `b` occurs, ascending.
    b2j: HashMap<&'a str, Vec<usize>>,
}

impl<'a> Matcher<'a> {
    fn new(a: &'a [String], b: &'a [String]) -> Self {
        let mut b2j: HashMap<&str, Vec<usize>> = HashMap::new();
        for (j, line) in b.iter().enumerate() {
            b2j.entry(line.as_str()).or_default().push(j);
        }
        let n = b.len();
        if n >= 200 {
            let ntest = n / 100 + 1;
            b2j.retain(|_, idxs| idxs.len() <= ntest);
        }
        Matcher { a, b, b2j }
    }

    /// `find_longest_match(alo, ahi, blo, bhi)` with no junk function.
    fn longest_match(
        &self,
        alo: usize,
        ahi: usize,
        blo: usize,
        bhi: usize,
    ) -> (usize, usize, usize) {
        let (a, b) = (self.a, self.b);
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for (i, line) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut newj2len = HashMap::new();
            if let Some(js) = self.b2j.get(line.as_str()) {
                for &j in js {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j
                        .checked_sub(1)
                        .and_then(|p| j2len.get(&p))
                        .copied()
                        .unwrap_or(0)
                        + 1;
                    newj2len.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            j2len = newj2len;
        }
        // Popular lines are not junk: the match grows over them.
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi
            && bestj + bestsize < bhi
            && a[besti + bestsize] == b[bestj + bestsize]
        {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    fn matching_blocks(&self) -> Vec<(usize, usize, usize)> {
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
        let mut out = Vec::new();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        for (i2, j2, k2) in blocks {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    out.push((i1, j1, k1));
                }
                (i1, j1, k1) = (i2, j2, k2);
            }
        }
        if k1 > 0 {
            out.push((i1, j1, k1));
        }
        out.push((la, lb, 0));
        out
    }

    fn opcodes(&self) -> Vec<Opcode> {
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
            (i, j) = (ai + size, bj + size);
            if size > 0 {
                answer.push((Tag::Equal, ai, i, bj, j));
            }
        }
        answer
    }

    fn grouped_opcodes(&self, n: usize) -> Vec<Vec<Opcode>> {
        let mut codes = self.opcodes();
        if codes.is_empty() {
            codes.push((Tag::Equal, 0, 1, 0, 1));
        }
        if let Some(first) = codes.first_mut()
            && first.0 == Tag::Equal
        {
            let (tag, i1, i2, j1, j2) = *first;
            *first = (
                tag,
                i1.max(i2.saturating_sub(n)),
                i2,
                j1.max(j2.saturating_sub(n)),
                j2,
            );
        }
        if let Some(last) = codes.last_mut()
            && last.0 == Tag::Equal
        {
            let (tag, i1, i2, j1, j2) = *last;
            *last = (tag, i1, i2.min(i1 + n), j1, j2.min(j1 + n));
        }
        let nn = n + n;
        let mut groups = Vec::new();
        let mut group = Vec::new();
        for (tag, mut i1, i2, mut j1, j2) in codes {
            if tag == Tag::Equal && i2 - i1 > nn {
                group.push((tag, i1, i2.min(i1 + n), j1, j2.min(j1 + n)));
                groups.push(std::mem::take(&mut group));
                i1 = i1.max(i2.saturating_sub(n));
                j1 = j1.max(j2.saturating_sub(n));
            }
            group.push((tag, i1, i2, j1, j2));
        }
        if !group.is_empty() && !(group.len() == 1 && group[0].0 == Tag::Equal) {
            groups.push(group);
        }
        groups
    }
}

/// `difflib._format_range_unified`.
fn range(start: usize, stop: usize) -> String {
    let mut beginning = start + 1;
    let length = stop - start;
    if length == 1 {
        return beginning.to_string();
    }
    if length == 0 {
        beginning -= 1;
    }
    format!("{beginning},{length}")
}

/// `list(difflib.unified_diff(a, b, fromfile, tofile))`: each item is one
/// output line (a content line keeps its own ending, or none).
pub fn unified_diff(a: &[String], b: &[String], fromfile: &str, tofile: &str) -> Vec<String> {
    let matcher = Matcher::new(a, b);
    let mut out = Vec::new();
    for (index, group) in matcher.grouped_opcodes(3).iter().enumerate() {
        if index == 0 {
            out.push(format!("--- {fromfile}\n"));
            out.push(format!("+++ {tofile}\n"));
        }
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        out.push(format!(
            "@@ -{} +{} @@\n",
            range(first.1, last.2),
            range(first.3, last.4)
        ));
        for &(tag, i1, i2, j1, j2) in group {
            if tag == Tag::Equal {
                out.extend(a[i1..i2].iter().map(|line| format!(" {line}")));
                continue;
            }
            if matches!(tag, Tag::Replace | Tag::Delete) {
                out.extend(a[i1..i2].iter().map(|line| format!("-{line}")));
            }
            if matches!(tag, Tag::Replace | Tag::Insert) {
                out.extend(b[j1..j2].iter().map(|line| format!("+{line}")));
            }
        }
    }
    out
}

/// Python's `str.splitlines()`: lines end at `\n`, `\r`, `\r\n`, `\v`,
/// `\f`, `\x1c`, `\x1d`, `\x1e`, `\x85`, ` ` and ` `; with
/// `keepends` each line keeps its ending.
pub fn splitlines(text: &str, keepends: bool) -> Vec<String> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let is_break = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !is_break {
            continue;
        }
        let mut end = i + c.len_utf8();
        if c == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
            chars.next();
            end += 1;
        }
        lines.push(text[start..if keepends { end } else { i }].to_string());
        start = end;
    }
    if start < text.len() {
        lines.push(text[start..].to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        splitlines(text, true)
    }

    #[test]
    fn splitlines_like_python() {
        assert_eq!(splitlines("a\r\nb\rc\n", true), ["a\r\n", "b\r", "c\n"]);
        assert_eq!(splitlines("a\r\nb\rc\n", false), ["a", "b", "c"]);
        assert_eq!(splitlines("", true), Vec::<String>::new());
        assert_eq!(splitlines("x", true), ["x"]);
    }

    #[test]
    fn identical_inputs_give_no_diff() {
        assert!(unified_diff(&lines("a\nb\n"), &lines("a\nb\n"), "x", "y").is_empty());
        assert!(unified_diff(&[], &[], "x", "y").is_empty());
    }

    #[test]
    fn hunks_are_split_past_twice_the_context() {
        // difflib.unified_diff("a b c d e f g h i j k".split(), ...) with
        // `b` and `j` changed: two hunks.
        let a: Vec<String> = "abcdefghijk".chars().map(|c| format!("{c}\n")).collect();
        let mut b = a.clone();
        b[1] = "B\n".into();
        b[9] = "J\n".into();
        let diff = unified_diff(&a, &b, "o", "n").concat();
        assert_eq!(
            diff,
            "--- o\n+++ n\n@@ -1,5 +1,5 @@\n a\n-b\n+B\n c\n d\n e\n@@ -7,5 +7,5 @@\n g\n h\n i\n-j\n+J\n k\n"
        );
    }

    #[test]
    fn empty_sides_use_zero_ranges() {
        let diff = unified_diff(&[], &lines("a\n"), "o", "n").concat();
        assert_eq!(diff, "--- o\n+++ n\n@@ -0,0 +1 @@\n+a\n");
    }
}
