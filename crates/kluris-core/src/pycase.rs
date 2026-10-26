//! Python 3.11's `str.title()` and `str.lower()`, as CPython computes them
//! (`Objects/unicodeobject.c`: `do_title`, `do_lower`, `lower_ucs4`,
//! `handle_capital_sigma`) over its Unicode 14.0 database, which
//! `pycase_tables.rs` holds (generated from the oracle by
//! `parity/tools/gen_pycase.py`).
//!
//! - `title`: a character after a cased one is lowercased, any other is
//!   titlecased with its full titlecase mapping (not its uppercase: `ǆ` is
//!   `ǅ`, `ᾀ` is `ᾈ`, Georgian Mkhedruli stays as it is).
//! - Lowercasing maps `Σ` to `ς` in the Final_Sigma context: after a cased
//!   character (case-ignorable ones skipped) and not before one.

#[path = "pycase_tables.rs"]
mod tables;

fn find_map(table: &'static [(u32, &'static str)], c: char) -> Option<&'static str> {
    table
        .binary_search_by_key(&u32::from(c), |(cp, _)| *cp)
        .ok()
        .map(|i| table[i].1)
}

fn in_ranges(table: &[(u32, u32)], c: char) -> bool {
    let cp = u32::from(c);
    let i = table.partition_point(|(_, end)| *end < cp);
    table.get(i).is_some_and(|(start, _)| *start <= cp)
}

/// Unicode `Cased`.
pub fn is_cased(c: char) -> bool {
    if c.is_ascii() {
        return c.is_ascii_alphabetic();
    }
    in_ranges(tables::CASED, c)
}

/// Unicode `Case_Ignorable`.
pub fn is_case_ignorable(c: char) -> bool {
    in_ranges(tables::CASE_IGNORABLE, c)
}

fn push_lower(out: &mut String, c: char) {
    match find_map(tables::LOWER, c) {
        Some(mapped) => out.push_str(mapped),
        None => out.push(c),
    }
}

fn push_title(out: &mut String, c: char) {
    match find_map(tables::TITLE, c) {
        Some(mapped) => out.push_str(mapped),
        None => out.push(c),
    }
}

/// `handle_capital_sigma`: whether the `Σ` at `i` is word-final.
fn final_sigma(chars: &[char], i: usize) -> bool {
    let before = chars[..i].iter().rev().find(|c| !is_case_ignorable(**c));
    if !before.is_some_and(|c| is_cased(*c)) {
        return false;
    }
    let after = chars[i + 1..].iter().find(|c| !is_case_ignorable(**c));
    !after.is_some_and(|c| is_cased(*c))
}

/// `lower_ucs4`: the lowercase of the character at `i`.
fn push_lower_at(out: &mut String, chars: &[char], i: usize) {
    if chars[i] == 'Σ' {
        out.push(if final_sigma(chars, i) { 'ς' } else { 'σ' });
    } else {
        push_lower(out, chars[i]);
    }
}

/// Python's `str.title()`.
pub fn title(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut previous_cased = false;
    for (i, &c) in chars.iter().enumerate() {
        if previous_cased {
            push_lower_at(&mut out, &chars, i);
        } else {
            push_title(&mut out, c);
        }
        previous_cased = is_cased(c);
    }
    out
}

/// Python's `str.lower()`.
pub fn lower(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for i in 0..chars.len() {
        push_lower_at(&mut out, &chars, i);
    }
    out
}
