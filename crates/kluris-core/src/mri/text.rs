//! The MRI's text rules: v2's `neuron_excerpt.extract` (title and first-line
//! excerpt) and `mri._build_content_preview`, with Python's string semantics
//! (`splitlines`, `strip`, slicing by code point) and v2's two anchor
//! patterns written out by hand.

use crate::frontmatter::py_splitlines;
use crate::search::items::stem_title;
use crate::yaml::is_py_space;

/// The excerpt's cap, in characters.
const EXCERPT_CHARS: usize = 220;
/// The preview's caps.
const PREVIEW_LINES: usize = 48;
const PREVIEW_CHARS: usize = 2800;

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

fn py_rstrip(text: &str) -> &str {
    text.trim_end_matches(is_py_space)
}

/// The first `n` characters of `text` (Python's `text[:n]`).
fn take_chars(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

/// Python's `str.splitlines()`: the lines without their line breaks.
pub(crate) fn py_lines(text: &str) -> Vec<&str> {
    py_splitlines(text)
        .into_iter()
        .map(|line| {
            if let Some(rest) = line.strip_suffix("\r\n") {
                rest
            } else {
                match line.char_indices().next_back() {
                    Some((at, c)) if is_line_break(c) => &line[..at],
                    _ => line,
                }
            }
        })
        .collect()
}

fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\u{b}'
            | '\u{c}'
            | '\u{1c}'
            | '\u{1d}'
            | '\u{1e}'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// `re.sub(pattern, "", text)` for a pattern whose match at a position, if
/// any, is decided without backtracking: `matcher` returns the length of the
/// match starting at the beginning of its argument.
fn remove_matches(text: &str, matcher: impl Fn(&str) -> Option<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some(len) = matcher(rest) {
            at += len;
            continue;
        }
        let c = rest.chars().next().map_or(1, char::len_utf8);
        out.push_str(&rest[..c]);
        at += c;
    }
    out
}

/// The length of the run of Python whitespace (`\s`) at the start of `text`.
fn spaces(text: &str) -> usize {
    text.len() - text.trim_start_matches(is_py_space).len()
}

/// The runtime's `<a\s+[^>]*></a>` (case-sensitive): `<a`, whitespace, then
/// everything up to the first `>`, which must open `></a>`.
fn empty_anchor(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("<a")?;
    if spaces(rest) == 0 {
        return None;
    }
    let close = rest.find('>')?;
    rest[close..]
        .starts_with("></a>")
        .then_some(2 + close + "></a>".len())
}

/// One character matched case-insensitively the way Python's `re`
/// (IGNORECASE, Unicode) matches the pattern letter `letter`.
fn ci(text: &str, letter: char) -> Option<usize> {
    let c = text.chars().next()?;
    let matches = match letter {
        'i' => matches!(c, 'i' | 'I' | '\u{131}' | '\u{130}'),
        _ => c.eq_ignore_ascii_case(&letter),
    };
    matches.then(|| c.len_utf8())
}

/// The MRI's `<a\s+id=["'][^"']*["']\s*>\s*</a>` (IGNORECASE).
fn id_anchor(text: &str) -> Option<usize> {
    let mut at = text.starts_with('<').then_some(1)?;
    at += ci(&text[at..], 'a')?;
    let ws = spaces(&text[at..]);
    if ws == 0 {
        return None;
    }
    at += ws;
    at += ci(&text[at..], 'i')?;
    at += ci(&text[at..], 'd')?;
    at += text[at..].starts_with('=').then_some(1)?;
    at += text[at..].starts_with(['"', '\'']).then_some(1)?;
    at += text[at..].find(['"', '\''])? + 1;
    at += spaces(&text[at..]);
    at += text[at..].starts_with('>').then_some(1)?;
    at += spaces(&text[at..]);
    at += text[at..].starts_with("</").then_some(2)?;
    at += ci(&text[at..], 'a')?;
    at += text[at..].starts_with('>').then_some(1)?;
    Some(at)
}

/// v2's `neuron_excerpt.extract(path, body)`: the first `# ` heading (or the
/// file's stem title) and the first line that is not blank, a heading, a list
/// item, a fence, a ruler or a navigation line, capped at 220 characters.
/// Empty `<a ...></a>` tags are removed from each line first.
pub fn excerpt(file_name: &str, body: &str) -> (String, String) {
    let mut title = String::new();
    let mut found = String::new();
    for raw in py_lines(body) {
        let cleaned = remove_matches(raw, empty_anchor);
        let line = py_strip(&cleaned);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ")
            && title.is_empty()
        {
            title = py_strip(rest).to_string();
            continue;
        }
        if ["## ", "- ", "* ", "```", "---", "up ", "sideways "]
            .iter()
            .any(|p| line.starts_with(p))
        {
            continue;
        }
        found = line.to_string();
        break;
    }
    if title.is_empty() {
        title = stem_title(file_name);
    }
    (title, take_chars(&found, EXCERPT_CHARS).to_string())
}

/// v2's `_build_content_preview(content)`: `(full body, preview, truncated)`.
/// The first `# ` line and the blank lines before the body are dropped and
/// `<a id=...></a>` anchors removed; the preview keeps at most 48 lines and
/// 2800 characters (cut back to a line end) and ends in `\n\n...` when cut.
pub fn content_preview(content: &str) -> (String, String, bool) {
    if py_strip(content).is_empty() {
        return (String::new(), String::new(), false);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut skipped_title = false;
    for raw in py_lines(content) {
        let cleaned = remove_matches(raw, id_anchor);
        let line = py_rstrip(&cleaned);
        if !skipped_title && py_strip(line).starts_with("# ") {
            skipped_title = true;
            continue;
        }
        if lines.is_empty() && py_strip(line).is_empty() {
            continue;
        }
        lines.push(line.to_string());
    }
    let full = py_strip(&lines.join("\n")).to_string();
    if full.is_empty() {
        return (String::new(), String::new(), false);
    }
    let truncated = lines.len() > PREVIEW_LINES || full.chars().count() > PREVIEW_CHARS;
    let head = lines[..lines.len().min(PREVIEW_LINES)].join("\n");
    let mut preview = py_strip(&head).to_string();
    if preview.chars().count() > PREVIEW_CHARS {
        let cut = py_rstrip(take_chars(&preview, PREVIEW_CHARS));
        let cut = match cut.rsplit_once('\n') {
            Some((before, _)) => py_rstrip(before),
            None => cut,
        };
        preview = cut.to_string();
    }
    if truncated {
        preview = format!("{}\n\n...", py_rstrip(&preview));
    }
    (full, preview, truncated)
}
