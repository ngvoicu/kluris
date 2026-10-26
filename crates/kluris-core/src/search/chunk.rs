//! Heading chunks (research-05 §3, the biggest lexical gain): a body is cut
//! at its `##` and `###` headings (not inside a code fence), and a section
//! longer than [`WINDOW_WORDS`] words is cut again into windows of about that
//! many words, always at a line break, so every chunk starts on a real line
//! of the file. The prototype cut windows mid-line; line windows keep `line`
//! exact at the cost of windows a few words longer.

/// The words a window holds before the next line starts a new one.
pub const WINDOW_WORDS: usize = 220;

/// One chunk of a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The `##`/`###` heading the chunk is under; `None` before the first.
    pub heading: Option<String>,
    /// The 0-based line of the body the chunk starts on: its heading's line
    /// for a section's first window.
    pub line: usize,
    /// The byte range of the chunk's text in the body (the heading line is
    /// not in it).
    pub start: usize,
    pub end: usize,
}

/// The text of a `##`/`###` heading line, if it is one.
fn heading(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("###")
        .or_else(|| line.strip_prefix("##"))?;
    if !rest.starts_with([' ', '\t']) {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end();
    (!text.is_empty()).then(|| text.to_string())
}

/// The fence a line opens or closes (` ``` ` or `~~~`, up to 3 spaces in).
fn fence(line: &str) -> Option<char> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    ['`', '~']
        .into_iter()
        .find(|&c| trimmed.chars().take(3).filter(|&x| x == c).count() == 3)
}

struct Open {
    heading: Option<String>,
    line: usize,
    start: usize,
    words: usize,
}

/// The chunks of a body (`\n` line breaks). A body without words is one
/// chunk; otherwise chunks with neither words nor a heading are left out.
pub fn chunks(body: &str) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut open = Open {
        heading: None,
        line: 0,
        start: 0,
        words: 0,
    };
    let mut in_fence: Option<char> = None;
    let mut offset = 0;
    let close = |open: &Open, end: usize, out: &mut Vec<Chunk>| {
        if open.words > 0 || open.heading.is_some() {
            out.push(Chunk {
                heading: open.heading.clone(),
                line: open.line,
                start: open.start,
                end,
            });
        }
    };
    for (index, raw) in body.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let line_start = offset;
        offset += raw.len();
        if let Some(c) = fence(line) {
            in_fence = match in_fence {
                Some(open_char) if open_char == c => None,
                None => Some(c),
                other => other,
            };
        } else if in_fence.is_none()
            && let Some(text) = heading(line)
        {
            close(&open, line_start, &mut out);
            open = Open {
                heading: Some(text),
                line: index,
                start: offset,
                words: 0,
            };
            continue;
        }
        let words = line.split_whitespace().count();
        if open.words > 0 && open.words + words > WINDOW_WORDS {
            close(&open, line_start, &mut out);
            open = Open {
                heading: open.heading.clone(),
                line: index,
                start: line_start,
                words: 0,
            };
        }
        open.words += words;
    }
    close(&open, body.len(), &mut out);
    if out.is_empty() {
        out.push(Chunk {
            heading: None,
            line: 0,
            start: 0,
            end: body.len(),
        });
    }
    out
}
