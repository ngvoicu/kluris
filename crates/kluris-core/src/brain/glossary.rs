//! Glossary entries: v2's `search.parse_glossary_entries`.
//!
//! Each line of `glossary.md`'s body (Python `str.splitlines`, then
//! `str.strip`) that is not blank and does not start with `#` is tried as:
//! 1. `**term** - definition` (any run of `-`, `–`, `—`; the term ends at the
//!    first `**` that lets the rest match). A bold line is kept unless its
//!    term or definition is empty or the term is a header word; either way it
//!    is not tried as a table row.
//! 2. a table row with exactly two cells, `| term | definition |`, skipped
//!    when the term or the definition is a header word or the term holds only
//!    `-`, `:` and spaces.
//!
//! Header words compare lowercased. Order and duplicates are kept. `\s` and
//! `strip` use Python's whitespace (which adds `\x1c`-`\x1f` to Unicode's).

use std::sync::LazyLock;

use regex::Regex;

use crate::frontmatter::py_splitlines;
use crate::yaml::is_py_space;

/// A glossary term and its definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryEntry {
    pub term: String,
    pub definition: String,
}

/// Python's `\s` for `str` patterns (`str.isspace`).
const PY_SPACE: &str = r"[\t\n\x0B\x0C\r\x1C-\x1F \x{85}\x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}]";

static BOLD_DASH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\A\*\*(.+?)\*\*{s}*[-\x{{2013}}\x{{2014}}]+{s}*(.+?){s}*\z",
        s = PY_SPACE
    ))
    .unwrap_or_else(|e| panic!("glossary bold pattern: {e}"))
});

static TABLE_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\A\|{s}*([^|]+?){s}*\|{s}*([^|]+?){s}*\|{s}*\z",
        s = PY_SPACE
    ))
    .unwrap_or_else(|e| panic!("glossary table pattern: {e}"))
});

const HEADER_TERMS: [&str; 7] = [
    "term",
    "meaning",
    "definition",
    ":------",
    ":---",
    "---",
    "------",
];

fn is_header(text: &str) -> bool {
    HEADER_TERMS.contains(&text.to_lowercase().as_str())
}

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// The `(term, definition)` entries of a glossary body, in order.
pub fn parse_glossary_entries(body: &str) -> Vec<GlossaryEntry> {
    let mut entries = Vec::new();
    for raw in py_splitlines(body) {
        let line = py_strip(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(m) = BOLD_DASH.captures(line) {
            let term = py_strip(m.get(1).map_or("", |g| g.as_str()));
            let definition = py_strip(m.get(2).map_or("", |g| g.as_str()));
            if !term.is_empty() && !definition.is_empty() && !is_header(term) {
                entries.push(GlossaryEntry {
                    term: term.to_string(),
                    definition: definition.to_string(),
                });
            }
            continue;
        }
        if let Some(m) = TABLE_ROW.captures(line) {
            let term = py_strip(m.get(1).map_or("", |g| g.as_str()));
            let definition = py_strip(m.get(2).map_or("", |g| g.as_str()));
            if is_header(term)
                || is_header(definition)
                || term.chars().all(|c| matches!(c, '-' | ':' | ' '))
            {
                continue;
            }
            if !definition.is_empty() {
                entries.push(GlossaryEntry {
                    term: term.to_string(),
                    definition: definition.to_string(),
                });
            }
        }
    }
    entries
}
