//! What v2 searches, item for item (research-03 §2.6, v2's
//! `kluris_runtime/search.py`): every neuron that reads, each glossary entry,
//! and `brain.md`; v2's literal substring score; its `--lobe` and `--tag`
//! filters. A neuron (or `glossary.md`) that does not read is left out, as in
//! v2, and reported ([`collect`], PAR-024).
//!
//! Everything is read through the [`BrainStore`]: a neuron by its own path
//! ([`BrainStore::read_neuron`]), the body's first line from the read.
//!
//! An item's `body` is v2's: for a markdown file the text after the
//! frontmatter with line breaks read as `\n` and Python's `strip()` applied
//! (python-frontmatter), for a YAML neuron the text after its `#---` block as
//! is.

use crate::brain::deprecation::SkippedNeuron;
use crate::brain::store::{BrainStore, ReadError};
use crate::frontmatter::{Frontmatter, py_splitlines};
use crate::pyrepr::py_str;
use crate::yaml::{Map, Value, date_iso, datetime_iso, is_py_space};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Neuron,
    Glossary,
    BrainMd,
}

/// One searchable item.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    /// The brain-relative path as v2 prints it: `/`-separated, a `\` in a
    /// file name shown as `/` too (v2's `_rel`); `glossary.md` for every
    /// entry.
    pub file: String,
    /// The walked path ([`crate::brain::neuron_index::Neuron::rel`]): the
    /// file's identity.
    pub source: String,
    /// `"markdown"` or `"yaml"`.
    pub file_type: &'static str,
    pub title: String,
    /// The frontmatter `tags` when it is a list, else none.
    pub tags: Vec<Value>,
    /// v2's body (the definition for a glossary entry).
    pub body: String,
    pub deprecated: bool,
    /// The frontmatter (empty for glossary entries).
    pub meta: Map,
    /// The 0-based line of the file the body starts on; `None` for a
    /// glossary entry or a `brain.md` that does not read.
    pub body_line: Option<usize>,
}

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// What Python's text-mode read hands over: `\r\n` and `\r` become `\n`.
pub(crate) fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Python's `str.title()` ([`crate::pycase::title`]).
pub use crate::pycase::title as py_title;

/// `Path(file).stem.replace("-", " ").title()`.
pub(crate) fn stem_title(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file);
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => &name[..dot],
        _ => name,
    };
    py_title(&stem.replace('-', " "))
}

/// v2's `extract_neuron_title`: the first line starting with `# `, after it
/// and stripped; else the file stem title-cased.
pub fn markdown_title(file: &str, body: &str) -> String {
    for line in py_splitlines(body) {
        let line = line.trim_end_matches(['\r', '\n']);
        // `str.splitlines` also breaks at characters `trim_end` left alone.
        let line = line.trim_end_matches(|c| {
            matches!(
                c,
                '\u{b}'
                    | '\u{c}'
                    | '\u{1c}'
                    | '\u{1d}'
                    | '\u{1e}'
                    | '\u{85}'
                    | '\u{2028}'
                    | '\u{2029}'
            )
        });
        if let Some(rest) = line.strip_prefix("# ") {
            return py_strip(rest).to_string();
        }
    }
    stem_title(file)
}

/// A top-level frontmatter value as v2 sees it after `_normalize_metadata`:
/// a date or datetime is its ISO text.
pub(crate) fn normalized_str(value: &Value) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.clone()),
        Value::Date(d) => Some(date_iso(d)),
        Value::DateTime(dt) => Some(datetime_iso(dt, 'T')),
        _ => None,
    }
}

fn is_deprecated(meta: &Map) -> bool {
    let status = match meta.get("status") {
        None => return false,
        Some(value) => normalized_str(value).unwrap_or_else(|| py_str(value)),
    };
    crate::pycase::lower(&status) == "deprecated"
}

fn tags(meta: &Map) -> Vec<Value> {
    match meta.get("tags") {
        Some(Value::Seq(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn count_line_breaks(text: &str) -> usize {
    universal_newlines(text).matches('\n').count()
}

/// v2's markdown body and the 0-based file line it starts on (the stripped
/// leading whitespace moves it down).
fn markdown_body(fm: &Frontmatter) -> (String, usize) {
    let lf = universal_newlines(&fm.body);
    let start = fm.body.len() - fm.body.trim_start_matches(is_py_space).len();
    (
        py_strip(&lf).to_string(),
        fm.body_line + count_line_breaks(&fm.body[..start]),
    )
}

/// How v2 prints a walked path (`str(relative_to(...)).replace("\\", "/")`).
fn rendered(rel: &str) -> String {
    rel.replace('\\', "/")
}

/// The searchable items of a brain, in v2's order: neurons (store order),
/// glossary entries, brain.md.
pub fn collect_items(store: &impl BrainStore) -> Vec<Item> {
    collect(store).0
}

/// The searchable items, and the files left out because they do not read:
/// neurons (by v2's printed path) and `glossary.md`, in that order.
pub fn collect(store: &impl BrainStore) -> (Vec<Item>, Vec<SkippedNeuron>) {
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for neuron in store.neurons() {
        let fm = match store.read_neuron(neuron) {
            Ok(fm) => fm,
            Err(error) => {
                skipped.push(SkippedNeuron {
                    rel: rendered(&neuron.rel),
                    error,
                });
                continue;
            }
        };
        let yaml = neuron.kind == crate::brain::neuron_index::NeuronKind::Yaml;
        let (body, line) = if yaml {
            (fm.body.clone(), fm.body_line)
        } else {
            markdown_body(&fm)
        };
        let title = if yaml {
            fm.meta
                .get("title")
                .and_then(normalized_str)
                .map(|t| py_strip(&t).to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| stem_title(&neuron.rel))
        } else {
            markdown_title(&neuron.rel, &body)
        };
        items.push(Item {
            kind: ItemKind::Neuron,
            file: rendered(&neuron.rel),
            source: neuron.rel.clone(),
            file_type: neuron.kind.as_str(),
            title,
            tags: tags(&fm.meta),
            deprecated: is_deprecated(&fm.meta),
            body,
            meta: fm.meta,
            body_line: Some(line),
        });
    }

    match store.glossary() {
        Ok(entries) => {
            for entry in entries {
                items.push(Item {
                    kind: ItemKind::Glossary,
                    file: "glossary.md".into(),
                    source: "glossary.md".into(),
                    file_type: "markdown",
                    title: entry.term,
                    tags: Vec::new(),
                    body: entry.definition,
                    deprecated: false,
                    meta: Map::new(),
                    body_line: None,
                });
            }
        }
        Err(error) => skipped.push(SkippedNeuron {
            rel: "glossary.md".into(),
            error: ReadError::File(error),
        }),
    }

    if let Some(read) = store.structure_file("brain.md") {
        let (body, meta, line) = match read {
            Ok(fm) => {
                let (body, line) = markdown_body(&fm);
                (body, fm.meta, Some(line))
            }
            Err(_) => (String::new(), Map::new(), None),
        };
        items.push(Item {
            kind: ItemKind::BrainMd,
            file: "brain.md".into(),
            source: "brain.md".into(),
            file_type: "markdown",
            title: markdown_title("brain.md", &body),
            tags: Vec::new(),
            body,
            deprecated: false,
            meta,
            body_line: line,
        });
    }
    (items, skipped)
}

/// v2's substring match of one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct V2Match {
    /// `10·title + 5·tags + 3·path + body` occurrence counts.
    pub score: i64,
    /// The fields that matched, in v2's order.
    pub fields: Vec<&'static str>,
}

fn count(haystack: &str, needle: &str) -> i64 {
    i64::try_from(crate::pycase::lower(haystack).matches(needle).count()).unwrap_or(i64::MAX)
}

/// v2's `score_hit` and `matched_fields` for a lowercased query; `None` when
/// the score is 0.
pub fn v2_match(item: &Item, query_lower: &str) -> Option<V2Match> {
    if query_lower.is_empty() {
        return None;
    }
    let counts = [
        ("title", count(&item.title, query_lower), 10),
        (
            "tag",
            item.tags
                .iter()
                .filter(|t| crate::pycase::lower(&py_str(t)).contains(query_lower))
                .count() as i64,
            5,
        ),
        ("path", count(&item.file, query_lower), 3),
        ("body", count(&item.body, query_lower), 1),
    ];
    let score = counts.iter().fold(0i64, |acc, (_, n, w)| {
        acc.saturating_add(n.saturating_mul(*w))
    });
    (score > 0).then(|| V2Match {
        score,
        fields: counts
            .iter()
            .filter(|(_, n, _)| *n > 0)
            .map(|(f, _, _)| *f)
            .collect(),
    })
}

/// v2's `_passes_filters`: `file` under `lobe.rstrip('/') + '/'`, and `tag`
/// one of the item's tags (a string tag equal to it).
pub fn passes_filters(item: &Item, lobe: Option<&str>, tag: Option<&str>) -> bool {
    if let Some(lobe) = lobe {
        let prefix = format!("{}/", lobe.trim_end_matches('/'));
        if !item.file.starts_with(&prefix) {
            return false;
        }
    }
    if let Some(tag) = tag
        && !item
            .tags
            .iter()
            .any(|t| matches!(t, Value::Str(s) if s == tag))
    {
        return false;
    }
    true
}

/// The neurons an item links to through `related:`, as v2 prints paths:
/// each string entry resolved by the store ([`BrainStore::resolve_link`]:
/// on disk, symlinks followed, inside the brain only), kept when it lands on
/// a neuron of the store other than the item itself, at most `limit`, in
/// order.
pub fn related(store: &impl BrainStore, item: &Item, limit: usize) -> Vec<String> {
    let Some(Value::Seq(links)) = item.meta.get("related") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for link in links.iter().filter_map(Value::as_str) {
        if out.len() >= limit {
            break;
        }
        let Some(rel) = store.resolve_link(&item.source, link) else {
            continue;
        };
        let shown = rendered(&rel);
        if rel != item.source
            && !out.contains(&shown)
            && store.neurons().iter().any(|n| n.rel == rel)
        {
            out.push(shown);
        }
    }
    out
}

/// `str(updated)` when the frontmatter has a non-null `updated`.
pub fn updated(item: &Item) -> Option<String> {
    match item.meta.get("updated") {
        None | Some(Value::Null) => None,
        Some(value) => Some(py_str(value)),
    }
}

/// A deprecated neuron's `replaced_by`, as written (a date as its ISO text).
pub fn replaced_by(item: &Item) -> Option<String> {
    item.meta
        .get("replaced_by")
        .and_then(normalized_str)
        .filter(|s| !s.is_empty())
}
