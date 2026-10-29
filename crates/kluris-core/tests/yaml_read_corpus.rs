//! The YAML reader against the frozen reader corpus
//! (`fixtures/corpus/yaml_load.jsonl`): YAML texts Hypothesis built from
//! fragments, each row holding what PyYAML 6.0.3's `SafeLoader` (merge keys
//! refused) reads from it, a value of the value model or an error. Error
//! messages are not compared.

mod common;

use common::corpus::{self, Failures, check_load, short};

#[test]
fn every_text_reads_as_safe_loader_reads_it() {
    let (_, rows) = corpus::jsonl("yaml_load.jsonl");
    let mut failures = Failures::default();
    for (n, row) in rows.iter().enumerate() {
        let text = row["text"]
            .as_str()
            .unwrap_or_else(|| panic!("row {}: no text", n + 1));
        if let Err(e) = check_load(text, row) {
            failures.add(n + 1, format!("{}: {e}", short(text)));
        }
    }
    failures.assert_none(rows.len());
}
