//! The search tokenizer (research-05 §3 "Tokenizer").
//!
//! - Words are Unicode word segments (UAX #29), each split again at every
//!   character that is neither alphanumeric nor a combining mark, so
//!   `KLURIS_NO_PROMPT`, `kluris.yml` and `/api/v1/imports/users` become
//!   separate words. Apostrophes inside a word are dropped (`don't` is
//!   `dont`).
//! - A word's surface form is [`fold`]ed: lowercased, NFKD-decomposed, and
//!   stripped of combining marks, so `pretul` and `prețul` are one word.
//! - Its term is the Snowball stem of the surface form in the brain's
//!   [`Language`]. The stemmer sees the folded word: Snowball's Romanian
//!   rules key on `ă â î ş ţ`, so stemming the unfolded word would split a
//!   word typed without diacritics from the same word written with them.
//! - Stop words are flagged, never removed from documents; [`query_terms`]
//!   drops them from a query unless that leaves nothing (`"how do I"` still
//!   searches). English stop words apply to every brain, Romanian ones to
//!   Romanian brains only (`ca` is a word in "CA bundle").

use super::fxhash::FxMap;

use rust_stemmers::{Algorithm, Stemmer};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;
use unicode_segmentation::UnicodeSegmentation;

use super::Language;

/// English stop words (folded): the list research-05 measured with.
pub const STOP_EN: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "has",
    "have", "how", "i", "if", "in", "into", "is", "it", "its", "may", "me", "must", "my", "no",
    "not", "of", "on", "or", "our", "should", "so", "that", "the", "their", "them", "then",
    "there", "these", "this", "to", "was", "we", "what", "when", "where", "which", "who", "why",
    "will", "with", "you", "your", "us", "were", "been",
];

/// Romanian stop words (folded).
pub const STOP_RO: &[&str] = &[
    "si", "sau", "la", "de", "pe", "in", "cu", "ce", "cum", "un", "o", "unei", "unui", "este", "e",
    "sa", "se", "din", "care", "nu", "mai", "al", "ale", "ai", "lui", "le", "ne", "ca", "pentru",
    "sunt", "fi", "dar", "iar", "daca", "cand", "unde", "cat", "prin", "dupa", "despre", "spre",
    "intre", "ei", "lor",
];

/// One word of a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The folded surface form.
    pub word: String,
    /// The stem of [`Token::word`]: what the index matches on.
    pub term: String,
    /// Byte offsets of the word in the original text.
    pub start: usize,
    pub end: usize,
    /// Whether the word is a stop word in this language.
    pub stop: bool,
}

/// Lowercase, NFKD, without combining marks.
pub fn fold(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    text.to_lowercase()
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .collect()
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02BC}')
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || is_combining_mark(c) || is_apostrophe(c)
}

/// Whether a folded word is a stop word in `language`.
pub fn is_stop_word(word: &str, language: Language) -> bool {
    STOP_EN.contains(&word) || (language == Language::Romanian && STOP_RO.contains(&word))
}

/// Lowercase and fold `piece` into `out` (cleared first), without
/// apostrophes.
fn fold_into(piece: &str, out: &mut String) {
    out.clear();
    if piece.is_ascii() {
        out.extend(
            piece
                .bytes()
                .filter(|b| *b != b'\'')
                .map(|b| char::from(b.to_ascii_lowercase())),
        );
        return;
    }
    out.extend(
        piece
            .to_lowercase()
            .nfkd()
            .filter(|c| !is_combining_mark(*c) && !is_apostrophe(*c)),
    );
}

/// Strings numbered in first-seen order.
#[derive(Debug, Default, Clone)]
pub struct Interner {
    ids: FxMap<String, u32>,
    names: Vec<String>,
}

impl Interner {
    pub fn id(&mut self, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = u32::try_from(self.names.len()).unwrap_or(u32::MAX);
        self.ids.insert(name.to_string(), id);
        self.names.push(name.to_string());
        id
    }

    pub fn get(&self, name: &str) -> Option<u32> {
        self.ids.get(name).copied()
    }

    pub fn name(&self, id: u32) -> &str {
        self.names.get(id as usize).map_or("", String::as_str)
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }
}

/// What the analyzer knows about one folded word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    /// The folded word's id ([`Analyzer::words`]).
    pub word: u32,
    /// Its stem's id ([`Analyzer::terms`]).
    pub term: u32,
    pub stop: bool,
}

/// Tokenizes texts in one language, numbering words and stems.
pub struct Analyzer {
    language: Language,
    stemmer: Stemmer,
    cache: FxMap<String, Entry>,
    words: Interner,
    terms: Interner,
    buffer: String,
}

impl Analyzer {
    pub fn new(language: Language) -> Analyzer {
        let algorithm = match language {
            Language::English => Algorithm::English,
            Language::Romanian => Algorithm::Romanian,
        };
        Analyzer {
            language,
            stemmer: Stemmer::create(algorithm),
            cache: FxMap::default(),
            words: Interner::default(),
            terms: Interner::default(),
            buffer: String::new(),
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// The folded words seen so far.
    pub fn words(&self) -> &Interner {
        &self.words
    }

    /// The stems seen so far.
    pub fn terms(&self) -> &Interner {
        &self.terms
    }

    /// Call `f(start, end, entry)` for every word of `text`.
    pub fn scan(&mut self, text: &str, mut f: impl FnMut(usize, usize, Entry)) {
        let mut buffer = std::mem::take(&mut self.buffer);
        pieces(text, |start, end| {
            fold_into(&text[start..end], &mut buffer);
            if let Some(entry) = self.entry(&buffer) {
                f(start, end, entry);
            }
        });
        self.buffer = buffer;
    }

    fn entry(&mut self, word: &str) -> Option<Entry> {
        if let Some(entry) = self.cache.get(word) {
            return Some(*entry);
        }
        if !word.chars().any(char::is_alphanumeric) {
            return None;
        }
        let stem = self.stemmer.stem(word);
        let entry = Entry {
            word: self.words.id(word),
            term: self.terms.id(&stem),
            stop: is_stop_word(word, self.language),
        };
        self.cache.insert(word.to_string(), entry);
        Some(entry)
    }

    /// Every word of `text`, stop words flagged.
    pub fn tokens(&mut self, text: &str) -> Vec<Token> {
        let mut found = Vec::new();
        self.scan(text, |start, end, entry| found.push((start, end, entry)));
        found
            .into_iter()
            .map(|(start, end, e)| Token {
                word: self.words.name(e.word).to_string(),
                term: self.terms.name(e.term).to_string(),
                start,
                end,
                stop: e.stop,
            })
            .collect()
    }

    /// The words of a query: stop words dropped unless only stop words are
    /// left, and each term once (first occurrence).
    pub fn query_terms(&mut self, query: &str) -> Vec<Token> {
        let all = self.tokens(query);
        let content: Vec<Token> = all.iter().filter(|t| !t.stop).cloned().collect();
        let mut picked = if content.is_empty() { all } else { content };
        let mut seen = std::collections::HashSet::new();
        picked.retain(|t| seen.insert(t.term.clone()));
        picked
    }
}

/// The byte ranges of the word pieces of `text`: Unicode word segments split
/// at every character that is not alphanumeric, a combining mark or an
/// apostrophe.
///
/// Word segments never span ASCII whitespace, so the text is cut there
/// first and a run that is all ASCII takes [`ascii_pieces`], which gives the
/// same pieces without the segmenter.
fn pieces(text: &str, mut f: impl FnMut(usize, usize)) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // ASCII whitespace bytes are always character boundaries.
        let run = &text[start..i];
        if run.is_ascii() {
            ascii_pieces(run.as_bytes(), start, &mut f);
        } else {
            unicode_pieces(run, start, &mut f);
        }
    }
}

/// UAX #29 plus the split at non-word characters, for ASCII: runs of
/// letters and digits, an apostrophe joining two letters or two digits.
fn ascii_pieces(run: &[u8], offset: usize, f: &mut impl FnMut(usize, usize)) {
    let mut start: Option<usize> = None;
    for i in 0..=run.len() {
        let joins = |i: usize| {
            run.get(i) == Some(&b'\'')
                && i > 0
                && match (run.get(i - 1), run.get(i + 1)) {
                    (Some(a), Some(b)) => {
                        (a.is_ascii_alphabetic() && b.is_ascii_alphabetic())
                            || (a.is_ascii_digit() && b.is_ascii_digit())
                    }
                    _ => false,
                }
        };
        let word =
            run.get(i).is_some_and(u8::is_ascii_alphanumeric) || (start.is_some() && joins(i));
        if word {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            f(offset + s, offset + i);
        }
    }
}

fn unicode_pieces(run: &str, offset: usize, f: &mut impl FnMut(usize, usize)) {
    for (at, segment) in run.unicode_word_indices() {
        let mut piece_start: Option<usize> = None;
        for (i, c) in segment.char_indices().chain([(segment.len(), ' ')]) {
            if i < segment.len() && is_word_char(c) {
                piece_start.get_or_insert(i);
                continue;
            }
            if let Some(start) = piece_start.take() {
                f(offset + at + start, offset + at + i);
            }
        }
    }
}

/// Call `f` with each folded word of `text`, unstemmed (language detection).
pub fn for_each_word(text: &str, mut f: impl FnMut(&str)) {
    let mut buffer = String::new();
    pieces(text, |start, end| {
        fold_into(&text[start..end], &mut buffer);
        if buffer.chars().any(char::is_alphanumeric) {
            f(&buffer);
        }
    });
}

/// [`Analyzer::tokens`] with a fresh analyzer.
pub fn tokens(text: &str, language: Language) -> Vec<Token> {
    Analyzer::new(language).tokens(text)
}

/// [`Analyzer::query_terms`] with a fresh analyzer.
pub fn query_terms(query: &str, language: Language) -> Vec<Token> {
    Analyzer::new(language).query_terms(query)
}
