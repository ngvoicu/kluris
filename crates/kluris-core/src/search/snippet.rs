//! Result snippets: about [`WIDTH`] characters of a chunk, centred on the
//! window that holds the most matched words, `...` where the chunk goes on.
//! Every cut falls on a character boundary.

/// The snippet width in characters (v2's).
pub const WIDTH: usize = 200;

/// The snippet of `text` around the matched words at byte ranges `spans`
/// (sorted, inside `text`); the opening of the text when nothing matched.
pub fn snippet(text: &str, spans: &[(usize, usize)]) -> String {
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain([text.len()])
        .collect();
    let char_at = |byte: usize| bounds.partition_point(|&b| b < byte);
    let chars = bounds.len() - 1;
    let (from, to) = match best_window(spans, &char_at) {
        None => (0, chars.min(WIDTH)),
        Some((first, last)) => {
            let used = last - first;
            let spare = WIDTH.saturating_sub(used);
            let mut from = first.saturating_sub(spare / 2);
            let to = (from + WIDTH.max(used)).min(chars);
            from = from.min(to.saturating_sub(WIDTH.max(used)));
            (from, to)
        }
    };
    let slice = &text[bounds[from]..bounds[to]];
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    if !text[..bounds[from]].trim().is_empty() {
        out.push_str("...");
    }
    out.push_str(trimmed);
    if !text[bounds[to]..].trim().is_empty() {
        out.push_str("...");
    }
    out
}

/// The character range from the first to the last matched word of the
/// [`WIDTH`]-character window, anchored at a matched word, that holds the
/// most matched words (the earliest on a tie).
fn best_window(
    spans: &[(usize, usize)],
    char_at: &dyn Fn(usize) -> usize,
) -> Option<(usize, usize)> {
    let spans: Vec<(usize, usize)> = spans
        .iter()
        .map(|&(s, e)| (char_at(s), char_at(e)))
        .collect();
    let mut best: Option<(usize, (usize, usize))> = None;
    for (i, &(start, end)) in spans.iter().enumerate() {
        let mut last = end;
        let mut count = 0;
        for &(_, e) in &spans[i..] {
            if e > start + WIDTH {
                break;
            }
            count += 1;
            last = last.max(e);
        }
        if best.is_none_or(|(n, _)| count > n) {
            best = Some((count, (start, last)));
        }
    }
    best.map(|(_, range)| range)
}
