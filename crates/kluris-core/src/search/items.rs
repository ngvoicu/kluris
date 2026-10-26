//! What v2 searches, item for item (research-03 §2.6, v2's
//! `kluris_runtime/search.py`): every neuron that reads (one that does not is
//! skipped silently), each glossary entry, and `brain.md`; v2's literal
//! substring score; its `--lobe` and `--tag` filters.
//!
//! An item's `body` is v2's: for a markdown file the text after the
//! frontmatter with line breaks read as `\n` and Python's `strip()` applied
//! (python-frontmatter), for a YAML neuron the text after its `#---` block as
//! is.

use std::fs;
use std::path::Path;

use crate::brain::sandbox;
use crate::brain::store::BrainStore;
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
    /// The `/`-separated brain-relative path (`glossary.md` for every entry).
    pub file: String,
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
    /// glossary entry or when the file cannot be read again.
    pub body_line: Option<usize>,
}

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// What Python's text-mode read hands over: `\r\n` and `\r` become `\n`.
pub(crate) fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Python's `str.title()`: a cased character after a cased one is lowercased,
/// any other is titlecased.
pub fn py_title(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_cased = false;
    for c in text.chars() {
        if previous_cased {
            out.extend(c.to_lowercase());
        } else {
            push_titlecase(&mut out, c);
        }
        previous_cased = c.is_lowercase() || c.is_uppercase() || is_titlecase(c);
    }
    out
}

fn is_titlecase(c: char) -> bool {
    matches!(c, 'ǅ' | 'ǈ' | 'ǋ' | 'ǲ') || ('ᾈ'..='ᾏ').contains(&c) || ('ᾘ'..='ᾟ').contains(&c)
}

fn push_titlecase(out: &mut String, c: char) {
    let title = match c {
        'Ǆ' | 'ǅ' | 'ǆ' => Some('ǅ'),
        'Ǉ' | 'ǈ' | 'ǉ' => Some('ǈ'),
        'Ǌ' | 'ǋ' | 'ǌ' => Some('ǋ'),
        'Ǳ' | 'ǲ' | 'ǳ' => Some('ǲ'),
        _ => None,
    };
    if let Some(t) = title {
        out.push(t);
        return;
    }
    // A multi-character uppercase (`ß` -> `SS`) titlecases as its first
    // character followed by the rest lowercased (`Ss`).
    let mut upper = c.to_uppercase();
    if let Some(first) = upper.next() {
        out.push(first);
        for rest in upper {
            out.extend(rest.to_lowercase());
        }
    }
}

/// `Path(file).stem.replace("-", " ").title()`.
fn stem_title(file: &str) -> String {
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
fn normalized_str(value: &Value) -> Option<String> {
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
    status.to_lowercase() == "deprecated"
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

/// The 0-based line of `path` that `body` (a suffix of the file's text, BOM
/// dropped) starts on, plus the lines `skipped` (the stripped leading
/// whitespace) covers.
fn body_line(path: &Path, raw_body: &str, skipped: &str, lossy: bool) -> Option<usize> {
    let bytes = fs::read(path).ok()?;
    let text = if lossy {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        String::from_utf8(bytes).ok()?
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let prefix = text.strip_suffix(raw_body)?;
    Some(count_line_breaks(prefix) + count_line_breaks(skipped))
}

fn markdown_body(fm: &Frontmatter) -> (String, &str) {
    let lf = universal_newlines(&fm.body);
    let start = fm.body.len() - fm.body.trim_start_matches(is_py_space).len();
    (py_strip(&lf).to_string(), &fm.body[..start])
}

/// The searchable items of a brain, in v2's order: neurons (store order),
/// glossary entries, brain.md.
pub fn collect_items(store: &impl BrainStore) -> Vec<Item> {
    let mut items = Vec::new();
    for neuron in store.neurons() {
        let Ok(fm) = store.read(&neuron.rel) else {
            continue;
        };
        let yaml = neuron.kind == crate::brain::neuron_index::NeuronKind::Yaml;
        let (body, line) = if yaml {
            let line = body_line(&neuron.path, &fm.body, "", true);
            (fm.body.clone(), line)
        } else {
            let (body, skipped) = markdown_body(&fm);
            let line = body_line(&neuron.path, &fm.body, skipped, false);
            (body, line)
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
            file: neuron.rel.clone(),
            file_type: neuron.kind.as_str(),
            title,
            tags: tags(&fm.meta),
            deprecated: is_deprecated(&fm.meta),
            body,
            meta: fm.meta,
            body_line: line,
        });
    }

    if let Ok(entries) = store.glossary() {
        for entry in entries {
            items.push(Item {
                kind: ItemKind::Glossary,
                file: "glossary.md".into(),
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

    if let Some(read) = store.structure_file("brain.md") {
        let (body, meta, line) = match read {
            Ok(fm) => {
                let (body, skipped) = markdown_body(&fm);
                let line = body_line(&store.root().join("brain.md"), &fm.body, skipped, false);
                (body, fm.meta, line)
            }
            Err(_) => (String::new(), Map::new(), None),
        };
        items.push(Item {
            kind: ItemKind::BrainMd,
            file: "brain.md".into(),
            file_type: "markdown",
            title: markdown_title("brain.md", &body),
            tags: Vec::new(),
            body,
            deprecated: false,
            meta,
            body_line: line,
        });
    }
    items
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
    i64::try_from(haystack.to_lowercase().matches(needle).count()).unwrap_or(i64::MAX)
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
                .filter(|t| py_str(t).to_lowercase().contains(query_lower))
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

/// The neurons an item links to through `related:`, as brain-relative
/// paths: each string entry resolved against the item's directory (links
/// followed, as v2's deprecation check resolves them), kept when it lands on
/// a neuron of the store other than the item itself, at most `limit`, in
/// order.
pub fn related(store: &impl BrainStore, item: &Item, limit: usize) -> Vec<String> {
    let Some(Value::Seq(links)) = item.meta.get("related") else {
        return Vec::new();
    };
    let root = store.root();
    let Ok(resolved_root) = sandbox::resolve(root) else {
        return Vec::new();
    };
    let base = match item.file.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').fold(root.to_path_buf(), |p, d| p.join(d)),
        None => root.to_path_buf(),
    };
    let mut out: Vec<String> = Vec::new();
    for link in links.iter().filter_map(Value::as_str) {
        if out.len() >= limit {
            break;
        }
        let Ok(target) = sandbox::resolve(&base.join(link)) else {
            continue;
        };
        let Ok(rel) = target.strip_prefix(&resolved_root) else {
            continue;
        };
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let rel = rel.join("/");
        if rel != item.file && !out.contains(&rel) && store.neurons().iter().any(|n| n.rel == rel) {
            out.push(rel);
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
