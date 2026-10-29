//! The frozen format corpora in `tests/fixtures/corpus/`: generated once from
//! PyYAML 6.0.3 and Python, each file opening with a header that records how.
//!
//! A JSON-lines corpus opens with one JSON object (its header, `rows` the
//! number of rows after it); a TSV corpus with `# key: value` lines (`# rows:
//! N` among them).

use std::fs;
use std::path::PathBuf;

use kluris_core::yaml::{self, Value, tagged_json};
use serde_json::Value as Json;

pub fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus")
        .join(name)
}

fn read(name: &str) -> String {
    let path = path(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn header_rows(name: &str, rows: Option<u64>, found: usize) {
    let rows = rows.unwrap_or_else(|| panic!("{name}: the header has no row count"));
    assert_eq!(found as u64, rows, "{name}: the header counts {rows} rows");
}

/// The header and the rows of a JSON-lines corpus.
pub fn jsonl(name: &str) -> (Json, Vec<Json>) {
    let text = read(name);
    let mut lines = text.lines().enumerate();
    let header: Json = match lines.next() {
        Some((_, line)) => serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("{name}: the header is not JSON: {e}")),
        None => panic!("{name} is empty"),
    };
    let rows: Vec<Json> = lines
        .map(|(n, line)| {
            serde_json::from_str(line).unwrap_or_else(|e| panic!("{name}:{}: not JSON: {e}", n + 1))
        })
        .collect();
    header_rows(name, header["rows"].as_u64(), rows.len());
    (header, rows)
}

/// The header lines (without `# `) and the tab-separated rows of a TSV corpus.
pub fn tsv(name: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let text = read(name);
    let (header, rows): (Vec<&str>, Vec<&str>) = text.lines().partition(|l| l.starts_with('#'));
    let header: Vec<String> = header
        .iter()
        .map(|l| l.trim_start_matches('#').trim().to_string())
        .collect();
    let count = header
        .iter()
        .find_map(|l| l.strip_prefix("rows: "))
        .and_then(|n| n.parse().ok());
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    header_rows(name, count, rows.len());
    (header, rows)
}

pub fn value(json: &Json) -> Value {
    tagged_json::decode(json).unwrap_or_else(|e| panic!("bad tagged JSON {json}: {e}"))
}

/// What reading `text` must give, from a corpus outcome: `{"value": ...}` or
/// `{"error": ...}` (error messages are not compared). An outcome marked
/// `deliberate` (an allowlisted difference from the oracle) keeps the
/// oracle's reading and holds Kluris's own under `kluris`, which is the one
/// checked.
pub fn check_load(text: &str, outcome: &Json) -> Result<(), String> {
    let expected = if outcome.get("deliberate").is_some() {
        &outcome["kluris"]
    } else {
        outcome
    };
    let got = yaml::load(text);
    match (expected.get("value"), got) {
        (Some(want), Ok(got)) if value(want) == got => Ok(()),
        (Some(want), Ok(got)) => Err(format!("reads {}, not {want}", tagged_json::encode(&got))),
        (Some(want), Err(e)) => Err(format!("fails ({e}), not {want}")),
        (None, Err(_)) => Ok(()),
        (None, Ok(got)) => Err(format!(
            "reads {} where the corpus fails ({})",
            tagged_json::encode(&got),
            expected["error"]
        )),
    }
}

/// Row failures, reported together at the end.
#[derive(Default)]
pub struct Failures(Vec<String>);

impl Failures {
    pub fn add(&mut self, row: usize, what: impl std::fmt::Display) {
        self.0.push(format!("row {row}: {what}"));
    }

    pub fn assert_none(self, rows: usize) {
        assert!(
            self.0.is_empty(),
            "{} of {rows} rows fail:\n{}",
            self.0.len(),
            self.0
                .iter()
                .take(40)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// A text shortened for a failure message.
pub fn short(text: &str) -> String {
    let mut out: String = text.chars().take(160).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    format!("{out:?}")
}
