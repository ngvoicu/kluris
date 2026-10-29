//! What a golden test pins of a `search --json` output. The order of the
//! results, their scores, snippets and the other fields the search engine
//! ranks with are its own, so a snapshot holds the envelope, the exit code,
//! `total` and the results sorted by file and title, each with its file
//! type, `deprecated` flag and matched fields.

use serde_json::Value;

use super::golden::parse_json_line;
use super::sandbox::Run;

/// A result as a slice shows it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Found {
    pub file: String,
    pub title: String,
    pub file_type: String,
    pub deprecated: bool,
    pub matched_fields: Vec<String>,
}

impl Found {
    /// A result of a `search --json` document.
    pub fn of_json(r: &Value) -> Found {
        let s = |key: &str| {
            r[key]
                .as_str()
                .unwrap_or_else(|| panic!("{key} is not a string: {r}"))
                .to_string()
        };
        Found {
            file: s("file"),
            title: s("title"),
            file_type: s("file_type"),
            deprecated: r["deprecated"]
                .as_bool()
                .unwrap_or_else(|| panic!("deprecated is not a boolean: {r}")),
            matched_fields: r["matched_fields"]
                .as_array()
                .unwrap_or_else(|| panic!("matched_fields is not a list: {r}"))
                .iter()
                .map(|f| f.as_str().expect("a matched field").to_string())
                .collect(),
        }
    }

    /// The results of a `search --json` document, in its order.
    pub fn all_of(doc: &Value) -> Vec<Found> {
        doc["results"]
            .as_array()
            .unwrap_or_else(|| panic!("results is not a list: {doc}"))
            .iter()
            .map(Found::of_json)
            .collect()
    }
}

/// `total`, then the results sorted by file and title, one per line:
/// `FILE | "TITLE" | FILE_TYPE | deprecated BOOL | FIELDS`, the title
/// written as a JSON string.
pub fn found_list(doc: &Value) -> String {
    let mut found = Found::all_of(doc);
    found.sort();
    let mut out = format!("total: {}\n", doc["total"]);
    for f in found {
        out.push_str(&format!(
            "{} | {} | {} | deprecated {} | {}\n",
            f.file,
            Value::String(f.title),
            f.file_type,
            f.deprecated,
            f.matched_fields.join(" ")
        ));
    }
    out
}

/// A successful `search --json` run, checked whole first: exit 0, nothing
/// on stderr, stdout one line in the `--json` layout ([`parse_json_line`])
/// with the envelope's keys in their order and `total` the number of
/// results. Its document, and the slice a snapshot holds: `ok`, `brain`,
/// `query`, `engine` and the exit code, then [`found_list`].
pub fn search_slice(run: &Run) -> (Value, String) {
    assert_eq!(run.code, 0, "search failed: {run:?}");
    assert_eq!(run.stderr, "", "search wrote to stderr: {run:?}");
    let doc = parse_json_line(&run.stdout);
    let keys: Vec<&String> = doc
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {doc}"))
        .keys()
        .collect();
    assert_eq!(keys, ["ok", "brain", "query", "total", "results", "engine"]);
    let results = doc["results"].as_array().expect("results").len();
    assert_eq!(doc["total"].as_u64(), Some(results as u64), "{doc}");
    let slice = format!(
        "ok: {}\nbrain: {}\nquery: {}\nengine: {}\nexit: {}\n{}",
        doc["ok"],
        doc["brain"],
        doc["query"],
        doc["engine"],
        run.code,
        found_list(&doc)
    );
    (doc, slice)
}
