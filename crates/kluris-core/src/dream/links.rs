//! The links dream checks in a body: v2's `parse_markdown_links` with the
//! decided fixes (research-03 §5 #32, interview-01 #5, PAR-041):
//! - links inside fenced code blocks (```` ``` ````, `~~~`) and inline code
//!   spans are not links;
//! - a target with a URI scheme (`mailto:`, `ftp:`, `tel:`, `file:`, ...) is
//!   not a file (v2 skipped only `http://` and `https://`); a one-letter
//!   "scheme" is a Windows drive and stays a path;
//! - a link title is not part of the path: `[a](b.md "t")`, `'t'`, `(t)`;
//! - the path may be URL-encoded: [`decoded`] gives the other reading, and a
//!   link is fine when either names a file.
//!
//! As in v2, the target is cut at `#` and then at `?`, an empty one is
//! dropped, and image links (`![x](y.png)`) are checked like any link.

use std::sync::LazyLock;

use regex::Regex;

/// v2's `LINK_PATTERN`.
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").unwrap_or_else(|e| panic!("LINK is valid: {e}"))
});

/// A fence line (up to 3 spaces of indentation, then 3 or more backticks or
/// tildes): its character and length, and what follows.
fn fence(line: &str) -> Option<(char, usize, &str)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let c = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = trimmed.len() - trimmed.trim_start_matches(c).len();
    (n >= 3).then(|| (c, n, &trimmed[n..]))
}

/// The text with fenced code blocks blanked out (their lines kept empty).
fn without_fences(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut open: Option<(char, usize)> = None;
    for line in body.split_inclusive('\n') {
        let end = line.trim_end_matches(['\r', '\n']);
        let eol = &line[end.len()..];
        match (open, fence(end)) {
            (None, Some((c, n, _))) => open = Some((c, n)),
            (Some((c, n)), Some((fc, fn_, rest)))
                if fc == c && fn_ >= n && rest.trim().is_empty() =>
            {
                open = None
            }
            (None, None) => {
                out.push_str(line);
                continue;
            }
            _ => {}
        }
        out.push_str(eol);
    }
    out
}

/// The text with inline code spans blanked out: a run of backticks up to the
/// next run of the same length (an unmatched run is literal text).
fn without_code_spans(text: &str) -> String {
    let bytes = text.as_bytes();
    let run = |at: usize| bytes[at..].iter().take_while(|b| **b == b'`').count();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let n = run(i);
        let mut j = i + n;
        let mut close = None;
        while j < bytes.len() {
            if bytes[j] == b'`' {
                let m = run(j);
                if m == n {
                    close = Some(j);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(j) => {
                out.push_str(&text[copied..i]);
                out.extend(
                    text[i..j + n]
                        .chars()
                        .map(|c| if c == '\n' { '\n' } else { ' ' }),
                );
                i = j + n;
                copied = i;
            }
            None => i += n,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Whether a target starts with a URI scheme of two or more characters.
fn has_scheme(target: &str) -> bool {
    let Some(colon) = target.find(':') else {
        return false;
    };
    let scheme = &target[..colon];
    scheme.len() >= 2
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// The target without a link title (`b.md "t"`, `b.md 't'`, `b.md (t`: the
/// link pattern stops at the first `)`).
fn without_title(target: &str) -> &str {
    let Some(space) = target.find(char::is_whitespace) else {
        return target;
    };
    let rest = target[space..].trim_start();
    if rest.starts_with(['"', '\'', '(']) {
        &target[..space]
    } else {
        target
    }
}

/// The link targets of a body, in order.
pub(crate) fn markdown_links(body: &str) -> Vec<String> {
    let text = without_code_spans(&without_fences(body));
    LINK.captures_iter(&text)
        .filter_map(|c| c.get(2))
        .map(|m| without_title(m.as_str()))
        .filter(|t| !has_scheme(t))
        .map(|t| t.split('#').next().unwrap_or(t))
        .map(|t| t.split('?').next().unwrap_or(t))
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// The URL-decoded reading of a path (`my%20note.md` is `my note.md`), when
/// it differs and decodes to UTF-8.
pub(crate) fn decoded(target: &str) -> Option<String> {
    if !target.contains('%') {
        return None;
    }
    let bytes = target.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push(hi * 16 + lo);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok().filter(|d| d != target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_rules_kept() {
        assert_eq!(
            markdown_links(
                "[a](x.md#h) [b](https://e.com) [c](y.md?q) [d](#top) [e](z.md) ![i](p.png)"
            ),
            ["x.md", "y.md", "z.md", "p.png"]
        );
    }

    #[test]
    fn code_is_not_links() {
        let body = "[a](a.md)\n```\n[b](b.md)\n```\n~~~~md\n[c](c.md)\n```\n[d](d.md)\n~~~~\n    ```\n[e](e.md)\n`[f](f.md)` ``[g `x`](g.md)`` `unclosed [h](h.md)\n";
        assert_eq!(markdown_links(body), ["a.md", "e.md", "h.md"]);
    }

    #[test]
    fn schemes_titles_and_drives() {
        assert_eq!(
            markdown_links(
                "[m](mailto:a@b) [f](ftp://x) [t](tel:1) [u](file:///x) [w](C:/x.md) [q](b.md \"T\") [s](b.md 'T') [p](b.md (T)) [sp](my note.md)"
            ),
            ["C:/x.md", "b.md", "b.md", "b.md", "my note.md"]
        );
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(decoded("my%20note.md").as_deref(), Some("my note.md"));
        assert_eq!(decoded("caf%C3%A9.md").as_deref(), Some("café.md"));
        assert_eq!(decoded("plain.md"), None);
        assert_eq!(decoded("100%.md"), None);
        assert_eq!(decoded("bad%FF.md"), None);
    }
}
