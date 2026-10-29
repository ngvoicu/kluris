//! The YAML writer against the frozen writer corpus
//! (`fixtures/corpus/yaml_emit.jsonl`): mappings drawn by Hypothesis, each
//! row holding what PyYAML 6.0.3 writes for it in every profile and what the
//! profile's loader reads back from that text (python-frontmatter's
//! `CSafeLoader` for markdown, `SafeLoader` for YAML neurons and configs).

mod common;

use common::corpus::{self, Failures, check_load, short};
use kluris_core::yaml::{Profile, emit};

const CORPUS: &str = "yaml_emit.jsonl";
const PROFILES: [(&str, Profile); 3] = [
    ("md", Profile::Md),
    ("yml", Profile::Yml),
    ("config", Profile::Config),
];

#[test]
fn every_value_is_written_as_pyyaml_writes_it() {
    let (_, rows) = corpus::jsonl(CORPUS);
    let mut failures = Failures::default();
    for (n, row) in rows.iter().enumerate() {
        let Ok(map) = corpus::value(&row["value"]).into_map() else {
            panic!("row {}: the value is not a mapping", n + 1);
        };
        for (name, profile) in PROFILES {
            let want = row[name]
                .as_str()
                .unwrap_or_else(|| panic!("row {}: no {name}", n + 1));
            let got = emit(&map, profile);
            if got != want {
                failures.add(
                    n + 1,
                    format!(
                        "[{name}] PyYAML writes {}\n  Rust writes   {}",
                        short(want),
                        short(&got)
                    ),
                );
            }
        }
    }
    failures.assert_none(rows.len());
}

#[test]
fn every_written_form_reads_back_as_its_loader_reads_it() {
    let (_, rows) = corpus::jsonl(CORPUS);
    let mut failures = Failures::default();
    for (n, row) in rows.iter().enumerate() {
        for (name, _) in PROFILES {
            let text = row[name]
                .as_str()
                .unwrap_or_else(|| panic!("row {}: no {name}", n + 1));
            if let Err(e) = check_load(text, &row["reread"][name]) {
                failures.add(n + 1, format!("[{name}] {}: {e}", short(text)));
            }
        }
    }
    failures.assert_none(rows.len());
}
