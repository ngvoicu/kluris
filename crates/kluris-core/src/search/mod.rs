//! `kluris search`: BM25F over heading chunks, computed in memory on every
//! call, nothing written (research-05 §6 rung 1; research-06 §6). How this
//! differs from v2 is PAR-020.
//!
//! 1. The items are v2's ([`items::collect`]): neurons, glossary
//!    entries (each its own result, `file: "glossary.md"`), `brain.md`. A
//!    neuron or `glossary.md` that does not read is left out and listed in
//!    [`SearchOutcome::skipped`] (PAR-024).
//! 2. Each item becomes search units: a neuron or `brain.md` one per heading
//!    chunk ([`chunk::chunks`]), with its title, tags and path repeated in
//!    every unit; a glossary entry one unit (term as title, definition as
//!    body). Text goes through the [`tokenize`] pipeline in the brain's
//!    language: `search: {language: en|ro}` in its `kluris.yml` when the
//!    caller passes it, else [`detect_language`].
//! 3. Query words are stemmed the same way; a word of 3+ characters that is
//!    not in the vocabulary matches the terms it is a prefix of
//!    ([`bm25::Index::prefix_terms`], best of them).
//! 4. [`bm25::Index`] scores the units; when the query has two or more
//!    words, a unit holding them in order in its title, heading or body gets
//!    the best unit's score on top (the phrase guard). An item scores its
//!    best unit.
//! 5. Every item v2's substring search returns for the same query
//!    ([`items::v2_match`]) is a candidate too, at [`FLOOR`] when BM25 gave
//!    it nothing, so nothing v2 found is lost. Deprecated neurons score
//!    ×0.5 and stay in the results.
//! 6. `--lobe`/`--tag` are v2's filters; results are ordered by score, then
//!    v2's substring score, `updated` (newest first), file and title, and cut
//!    to the limit (`limit <= 0`: none).

pub mod bm25;
pub mod chunk;
mod fxhash;
pub mod items;
pub mod snippet;
pub mod tokenize;

use std::cmp::Ordering;

use bm25::{Field, IdUnit, Index, QueryTerm};
use chunk::Chunk;
use items::{Item, ItemKind, V2Match};
use tokenize::Analyzer;

use crate::brain::deprecation::SkippedNeuron;
use crate::brain::store::BrainStore;

/// The score of an item only v2's substring match finds.
pub const FLOOR: f64 = 0.001;

/// How many `related` neurons a hit lists.
pub const RELATED_LIMIT: usize = 3;

/// The search language: which stemmer and stop words a brain gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Romanian,
}

impl Language {
    /// `en` or `ro`, in any case.
    pub fn from_code(code: &str) -> Option<Language> {
        match code.to_ascii_lowercase().as_str() {
            "en" => Some(Language::English),
            "ro" => Some(Language::Romanian),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Romanian => "ro",
        }
    }
}

/// Words only one of the languages uses often (folded).
const MARKERS_EN: &[&str] = &[
    "the", "and", "of", "to", "is", "that", "with", "for", "are", "this", "it", "be", "on", "as",
    "by", "from", "or", "was", "we", "what", "when", "which", "you", "your", "not", "have", "has",
];
const MARKERS_RO: &[&str] = &[
    "si", "sau", "de", "la", "pe", "cu", "este", "care", "din", "nu", "sa", "se", "ca", "mai",
    "pentru", "sunt", "unei", "unui", "lui", "ale", "dupa", "daca", "cand", "prin", "iar", "dar",
    "despre",
];

/// How much of each text language detection reads, and how many marker
/// words are enough.
const DETECT_BYTES: usize = 4096;
const DETECT_MARKERS: usize = 5000;

/// Romanian when Romanian marker words outnumber English ones in `texts`
/// (the first [`DETECT_BYTES`] of each, until [`DETECT_MARKERS`] markers are
/// counted), else English.
pub fn detect_language<'a, I: IntoIterator<Item = &'a str>>(texts: I) -> Language {
    let (mut en, mut ro) = (0usize, 0usize);
    for text in texts {
        if en + ro >= DETECT_MARKERS {
            break;
        }
        let mut end = text.len().min(DETECT_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        tokenize::for_each_word(&text[..end], |word| {
            if MARKERS_EN.contains(&word) {
                en += 1;
            } else if MARKERS_RO.contains(&word) {
                ro += 1;
            }
        });
    }
    if ro > en {
        Language::Romanian
    } else {
        Language::English
    }
}

/// Python's `int(text)` as Click's `--limit` reads it, clamped to `i64`.
pub fn parse_limit(text: &str) -> Option<i64> {
    let value = crate::yaml::py_int_from_str(text)?;
    Some(
        i64::try_from(&value).unwrap_or(if value.sign() == num_bigint::Sign::Minus {
            i64::MIN
        } else {
            i64::MAX
        }),
    )
}

/// What to search for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    pub query: String,
    pub lobe: Option<String>,
    pub tag: Option<String>,
    pub limit: i64,
    /// The brain's configured language; detected when `None`.
    pub language: Option<Language>,
}

impl SearchRequest {
    /// A request with v2's defaults: no filters, limit 10.
    pub fn new(query: impl Into<String>) -> SearchRequest {
        SearchRequest {
            query: query.into(),
            lobe: None,
            tag: None,
            limit: 10,
            language: None,
        }
    }
}

/// One result.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub file: String,
    pub file_type: &'static str,
    pub title: String,
    /// Of `title`, `tag`, `path`, `heading`, `body`, in that order.
    pub matched_fields: Vec<&'static str>,
    pub snippet: String,
    /// `round(score × 100)`, at least 1.
    pub score: i64,
    pub deprecated: bool,
    /// The best chunk's heading.
    pub heading: Option<String>,
    /// The 1-based line of the file the best chunk starts on.
    pub line: Option<usize>,
    /// The query words found in the item, in query order.
    pub matched_terms: Vec<String>,
    /// The share of the query words found in the item, rounded to 2 places.
    pub coverage: f64,
    pub related: Vec<String>,
    pub updated: Option<String>,
    /// `replaced_by` of a deprecated neuron.
    pub replaced_by: Option<String>,
}

/// The results and the language they were searched in.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOutcome {
    pub hits: Vec<Hit>,
    pub language: Language,
    /// The files left out because they cannot be read, in brain order.
    pub skipped: Vec<SkippedNeuron>,
}

/// One unit's place: its item and chunk, and its text for snippets.
struct UnitInfo {
    item: usize,
    chunk: Chunk,
    /// Word ids of the title, heading and body, for the phrase guard.
    sequences: [Vec<u32>; 3],
}

/// An item's text for chunking and snippets: v2's body with `\n` breaks.
fn item_text(item: &Item) -> String {
    items::universal_newlines(&item.body)
}

/// The path field: the file without its extension.
fn path_text(file: &str) -> &str {
    match file.rfind('.') {
        Some(dot) if !file[dot..].contains('/') => &file[..dot],
        _ => file,
    }
}

/// The byte range of the line a markdown title comes from (the first line
/// starting with `# `), so the title counts once, in its own field.
fn title_line(item: &Item, text: &str) -> Option<(usize, usize)> {
    if item.kind == ItemKind::Glossary || item.file_type == "yaml" {
        return None;
    }
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("# ") {
            return Some((offset, offset + line.len()));
        }
        offset += line.len();
    }
    None
}

fn contains_sequence(haystack: &[u32], needle: &[u32]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Search the brain in `store`.
pub fn search(store: &impl BrainStore, request: &SearchRequest) -> SearchOutcome {
    let (all, skipped) = items::collect(store);
    let language = request.language.unwrap_or_else(|| {
        detect_language(
            all.iter()
                .filter(|i| i.kind == ItemKind::Neuron)
                .map(|i| i.body.as_str()),
        )
    });
    let mut outcome = SearchOutcome {
        hits: Vec::new(),
        language,
        skipped,
    };
    if request.limit <= 0 || request.query.is_empty() {
        return outcome;
    }
    let mut analyzer = Analyzer::new(language);

    // Units: term ids per field, word ids for the phrase guard.
    let texts: Vec<String> = all.iter().map(item_text).collect();
    let mut units: Vec<IdUnit> = Vec::new();
    let mut infos: Vec<UnitInfo> = Vec::new();
    let scan = |analyzer: &mut Analyzer, text: &str, words: &mut Vec<u32>, terms: &mut Vec<u32>| {
        analyzer.scan(text, |_, _, e| {
            words.push(e.word);
            terms.push(e.term);
        });
    };
    for (index, item) in all.iter().enumerate() {
        let text = &texts[index];
        let (mut title_ids, mut title_terms) = (Vec::new(), Vec::new());
        scan(&mut analyzer, &item.title, &mut title_ids, &mut title_terms);
        let (mut unused, mut tag_terms) = (Vec::new(), Vec::new());
        for tag in &item.tags {
            scan(
                &mut analyzer,
                &crate::pyrepr::py_str(tag),
                &mut unused,
                &mut tag_terms,
            );
        }
        let mut path_terms = Vec::new();
        if item.kind != ItemKind::Glossary {
            scan(
                &mut analyzer,
                path_text(&item.file),
                &mut unused,
                &mut path_terms,
            );
        }
        let pieces = if item.kind == ItemKind::Glossary {
            vec![Chunk {
                heading: None,
                line: 0,
                start: 0,
                end: text.len(),
            }]
        } else {
            chunk::chunks(text)
        };
        let skip = title_line(item, text);
        for piece in pieces {
            let (mut heading_ids, mut heading_terms) = (Vec::new(), Vec::new());
            if let Some(h) = piece.heading.as_deref() {
                scan(&mut analyzer, h, &mut heading_ids, &mut heading_terms);
            }
            let (mut body_ids, mut body_terms) = (Vec::new(), Vec::new());
            analyzer.scan(&text[piece.start..piece.end], |start, _, e| {
                if !skip.is_some_and(|(from, to)| (from..to).contains(&(piece.start + start))) {
                    body_ids.push(e.word);
                    body_terms.push(e.term);
                }
            });
            units.push([
                title_terms.clone(),
                tag_terms.clone(),
                path_terms.clone(),
                heading_terms,
                body_terms,
            ]);
            infos.push(UnitInfo {
                item: index,
                chunk: piece,
                sequences: [title_ids.clone(), heading_ids, body_ids],
            });
        }
    }
    let index = Index::from_ids(&units, analyzer.terms().names());
    drop(units);

    // The query.
    let query_tokens = analyzer.query_terms(&request.query);
    let query: Vec<QueryTerm> = query_tokens
        .iter()
        .map(|t| {
            if !index.has_term(&t.term) && t.word.chars().count() >= 3 {
                let expanded = index.prefix_terms(&t.term);
                if !expanded.is_empty() {
                    return QueryTerm::any(expanded);
                }
            }
            QueryTerm::exact(&t.term)
        })
        .collect();
    let scored = index.score_detailed(&query);
    let mut scores = scored.scores.clone();

    // The phrase guard: the query's words (stop words too) in order.
    let mut phrase: Vec<u32> = Vec::new();
    analyzer.scan(&request.query, |_, _, e| phrase.push(e.word));
    if phrase.len() >= 2 {
        let top = scores.iter().copied().fold(0.0, f64::max);
        for (u, info) in infos.iter().enumerate() {
            if scores[u] > 0.0 && info.sequences.iter().any(|s| contains_sequence(s, &phrase)) {
                scores[u] += top;
            }
        }
    }

    // Items: the best unit, plus v2's substring candidates.
    let query_lower = crate::pycase::lower(&request.query);
    let mut best: Vec<Option<(usize, f64)>> = vec![None; all.len()];
    let mut item_terms: Vec<Vec<bool>> = vec![Vec::new(); all.len()];
    for (u, info) in infos.iter().enumerate() {
        for &qi in &scored.terms[u] {
            let found = &mut item_terms[info.item];
            if found.is_empty() {
                found.resize(query.len(), false);
            }
            if let Some(slot) = found.get_mut(qi as usize) {
                *slot = true;
            }
        }
        let entry = &mut best[info.item];
        if entry.is_none_or(|(_, s)| scores[u] > s) {
            *entry = Some((u, scores[u]));
        }
    }
    struct Candidate {
        item: usize,
        unit: Option<usize>,
        score: f64,
        v2: Option<V2Match>,
    }
    let mut candidates: Vec<Candidate> = Vec::new();
    for (i, item) in all.iter().enumerate() {
        if !items::passes_filters(item, request.lobe.as_deref(), request.tag.as_deref()) {
            continue;
        }
        let v2 = items::v2_match(item, &query_lower);
        let (unit, bm25) = match best[i] {
            Some((u, s)) if s > 0.0 => (Some(u), s),
            Some((u, _)) => (Some(u), 0.0),
            None => (None, 0.0),
        };
        if bm25 <= 0.0 && v2.is_none() {
            continue;
        }
        let mut score = if bm25 > 0.0 { bm25 } else { FLOOR };
        if item.deprecated {
            score *= 0.5;
        }
        let unit = if bm25 > 0.0 {
            unit
        } else {
            substring_unit(&infos, i, &texts[i], &query_lower).or(unit)
        };
        candidates.push(Candidate {
            item: i,
            unit,
            score,
            v2,
        });
    }
    let updated: Vec<Option<String>> = all.iter().map(items::updated).collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                let v2 = |c: &Candidate| c.v2.as_ref().map_or(0, |m| m.score);
                v2(b).cmp(&v2(a))
            })
            .then_with(|| updated[b.item].cmp(&updated[a.item]))
            .then_with(|| all[a.item].file.cmp(&all[b.item].file))
            .then_with(|| all[a.item].title.cmp(&all[b.item].title))
            .then_with(|| a.item.cmp(&b.item))
    });
    let limit = usize::try_from(request.limit).unwrap_or(usize::MAX);
    candidates.truncate(limit);

    // Hits.
    for c in candidates {
        let item = &all[c.item];
        let text = &texts[c.item];
        let info = c.unit.map(|u| &infos[u]);
        let field_mask = c.unit.map_or(0, |u| scored.fields[u]);
        let mut matched_fields: Vec<&'static str> = Vec::new();
        for field in Field::ALL {
            let name = field.name();
            let by_term = field_mask & (1 << field as usize) != 0;
            let by_v2 = c.v2.as_ref().is_some_and(|m| m.fields.contains(&name));
            if by_term || by_v2 {
                matched_fields.push(name);
            }
        }
        let found = &item_terms[c.item];
        let matched_terms: Vec<String> = query_tokens
            .iter()
            .enumerate()
            .filter(|(i, _)| found.get(*i).copied().unwrap_or(false))
            .map(|(_, t)| t.word.clone())
            .collect();
        let coverage = if query_tokens.is_empty() {
            0.0
        } else {
            (matched_terms.len() as f64 / query_tokens.len() as f64 * 100.0).round() / 100.0
        };
        let snippet = match info {
            Some(info) => {
                let chunk_text = &text[info.chunk.start..info.chunk.end];
                let spans = match_spans(&mut analyzer, chunk_text, &query, &query_lower);
                snippet::snippet(chunk_text, &spans)
            }
            None => String::new(),
        };
        let (heading, line) = match info {
            Some(info) if item.kind != ItemKind::Glossary => (
                info.chunk.heading.clone(),
                item.body_line.map(|l| l + info.chunk.line + 1),
            ),
            _ => (None, None),
        };
        outcome.hits.push(Hit {
            file: item.file.clone(),
            file_type: item.file_type,
            title: item.title.clone(),
            matched_fields,
            snippet,
            score: ((c.score * 100.0).round_ties_even() as i64).max(1),
            deprecated: item.deprecated,
            heading,
            line,
            matched_terms,
            coverage,
            related: if item.kind == ItemKind::Neuron {
                items::related(store, item, RELATED_LIMIT)
            } else {
                Vec::new()
            },
            updated: updated[c.item].clone(),
            replaced_by: if item.deprecated {
                items::replaced_by(item)
            } else {
                None
            },
        });
    }
    outcome
}

/// For an item only v2's substring match finds: the unit whose chunk holds
/// the first case-insensitive occurrence of the query in the body.
fn substring_unit(infos: &[UnitInfo], item: usize, text: &str, query_lower: &str) -> Option<usize> {
    let (at, _) = find_lowercase(text, query_lower)?;
    infos
        .iter()
        .enumerate()
        .find(|(_, info)| {
            info.item == item
                && info.chunk.start <= at
                && at < info.chunk.end.max(info.chunk.start + 1)
        })
        .map(|(u, _)| u)
}

/// The byte range in `text` of the first occurrence of `query_lower` in
/// `text` lowercased.
fn find_lowercase(text: &str, query_lower: &str) -> Option<(usize, usize)> {
    if query_lower.is_empty() {
        return None;
    }
    let mut lowered = String::with_capacity(text.len());
    let mut origin: Vec<usize> = Vec::with_capacity(text.len() + 1);
    for (i, c) in text.char_indices() {
        for l in c.to_lowercase() {
            let before = lowered.len();
            lowered.push(l);
            origin.extend(std::iter::repeat_n(i, lowered.len() - before));
        }
    }
    origin.push(text.len());
    let at = lowered.find(query_lower)?;
    let end = origin
        .get(at + query_lower.len())
        .copied()
        .unwrap_or(text.len());
    let end = if end == origin[at] {
        text[origin[at]..]
            .chars()
            .next()
            .map_or(text.len(), |c| origin[at] + c.len_utf8())
    } else {
        end
    };
    Some((origin[at], end))
}

/// The byte ranges of the words of `text` that match a query term, or else
/// the first substring occurrence of the whole query.
fn match_spans(
    analyzer: &mut Analyzer,
    text: &str,
    query: &[QueryTerm],
    query_lower: &str,
) -> Vec<(usize, usize)> {
    let wanted: Vec<u32> = query
        .iter()
        .flat_map(|q| q.terms.iter())
        .filter_map(|t| analyzer.terms().get(t))
        .collect();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    analyzer.scan(text, |start, end, e| {
        if wanted.contains(&e.term) {
            spans.push((start, end));
        }
    });
    if !spans.is_empty() {
        return spans;
    }
    find_lowercase(text, query_lower).into_iter().collect()
}
