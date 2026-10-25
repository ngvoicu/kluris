//! Reading markdown frontmatter and YAML neuron blocks. Expected metadata was
//! checked against v2.28.11 (`kluris_runtime.frontmatter.read_frontmatter`,
//! python-frontmatter 1.3.0, PyYAML 6.0.3) except where a comment says v3
//! differs on purpose.

use std::fs;
use std::path::{Path, PathBuf};

use kluris_core::frontmatter::{
    LineEnding, Problem, has_yaml_opt_in_block, is_yaml_neuron, parse, parse_yaml_neuron, read,
    read_neuron, read_yaml_neuron, split,
};
use kluris_core::yaml::{Date, Map, Value};
use num_bigint::BigInt;
use tempfile::TempDir;

fn str_(text: &str) -> Value {
    Value::Str(text.to_string())
}

fn meta(entries: &[(&str, Value)]) -> Map {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, bytes).unwrap();
    path
}

// --- split ------------------------------------------------------------------

#[test]
fn split_three_dash_delimiters() {
    assert_eq!(
        split("---\ntitle: x\n---\nbody\n"),
        (Some("title: x\n"), "body\n")
    );
}

#[test]
fn split_four_dash_and_trailing_whitespace_delimiters() {
    assert_eq!(
        split("----\ntitle: x\n----\nbody\n"),
        (Some("title: x\n"), "body\n")
    );
    assert_eq!(
        split("---  \ntitle: x\n---\t\nbody"),
        (Some("title: x\n"), "body")
    );
}

#[test]
fn split_needs_a_whole_delimiter_line_and_a_closing_one() {
    let text = "---x\ntitle: x\n---\nbody";
    assert_eq!(split(text), (None, text));
    let text = "---\ntitle: x\nbody";
    assert_eq!(split(text), (None, text));
    let text = "no frontmatter\n---\n";
    assert_eq!(split(text), (None, text));
}

#[test]
fn split_closes_at_the_first_delimiter_line() {
    assert_eq!(
        split("---\na: 1\n-----\nmore\n---\nx"),
        (Some("a: 1\n"), "more\n---\nx")
    );
}

#[test]
fn split_empty_frontmatter_and_closing_at_end_of_file() {
    assert_eq!(split("---\n---\nbody"), (Some(""), "body"));
    assert_eq!(split("---\na: 1\n---"), (Some("a: 1\n"), ""));
}

#[test]
fn split_skips_leading_whitespace_like_v2() {
    // python-frontmatter strips the text before looking for `---`.
    assert_eq!(
        split("\n\n  ---\ntitle: x\n---\nbody"),
        (Some("title: x\n"), "body")
    );
}

#[test]
fn split_skips_a_bom() {
    assert_eq!(
        split("\u{feff}---\ntitle: x\n---\nbody"),
        (Some("title: x\n"), "body")
    );
}

#[test]
fn split_crlf_keeps_the_body_bytes() {
    assert_eq!(
        split("---\r\ntitle: x\r\n---\r\n\r\n    code\r\n"),
        (Some("title: x\r\n"), "\r\n    code\r\n")
    );
}

#[test]
fn split_cr_only_line_breaks() {
    // v2 reads in text mode, where a lone CR is a line break.
    assert_eq!(split("---\ra: 1\r---\rbody"), (Some("a: 1\r"), "body"));
}

// --- parse (markdown) -------------------------------------------------------

#[test]
fn parse_reads_typed_metadata_and_byte_exact_body() {
    let fm = parse(
        "---\ncreated: 2026-01-02\nupdated: '2026-01-03'\ntags: [on, 2026]\nparent: ./map.md\n---\n    code first\nmore\n\n",
    )
    .unwrap();
    assert_eq!(
        fm.meta,
        meta(&[
            ("created", Value::Date(Date::new(2026, 1, 2).unwrap())),
            ("updated", str_("2026-01-03")),
            (
                "tags",
                Value::Seq(vec![Value::Bool(true), Value::Int(BigInt::from(2026))])
            ),
            ("parent", str_("./map.md")),
        ])
    );
    // v2 strips the body (losing the indentation); v3 keeps it byte-exact.
    assert_eq!(fm.body, "    code first\nmore\n\n");
    assert_eq!(fm.line_ending, LineEnding::Lf);
    assert!(!fm.had_bom);
}

#[test]
fn parse_keeps_leading_blank_lines_of_the_body() {
    let fm = parse("---\na: 1\n---\n\n\nbody\n").unwrap();
    assert_eq!(fm.body, "\n\nbody\n");
}

#[test]
fn parse_crlf() {
    let fm = parse("---\r\ntitle: x\r\nlist:\r\n- a\r\n---\r\n\r\nbody\r\n").unwrap();
    assert_eq!(
        fm.meta,
        meta(&[("title", str_("x")), ("list", Value::Seq(vec![str_("a")]))])
    );
    assert_eq!(fm.body, "\r\nbody\r\n");
    assert_eq!(fm.line_ending, LineEnding::Crlf);
}

#[test]
fn parse_bom_is_detected_and_frontmatter_still_read() {
    // v2 misses frontmatter behind a BOM; v3 reads it.
    let fm = parse("\u{feff}---\ntitle: x\n---\nbody").unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("x"))]));
    assert_eq!(fm.body, "body");
    assert!(fm.had_bom);
}

#[test]
fn parse_empty_frontmatter() {
    let fm = parse("---\n---\nbody\n").unwrap();
    assert!(fm.meta.is_empty());
    assert_eq!(fm.body, "body\n");
}

#[test]
fn parse_without_frontmatter_returns_the_whole_text() {
    let text = "# Title\n\nNo frontmatter here.\n";
    let fm = parse(text).unwrap();
    assert!(fm.meta.is_empty());
    assert_eq!(fm.body, text);
    let fm = parse("\u{feff}# Title\n").unwrap();
    assert_eq!(fm.body, "# Title\n");
    assert!(fm.had_bom);
}

#[test]
fn parse_json_and_toml_frontmatter_are_not_frontmatter() {
    // v2 reads `{` ... `}` as JSON frontmatter (python-frontmatter); v3 does
    // not (parity/allowlist.md). TOML was never read (no `toml` package).
    for text in ["{\n\"a\": 1\n}\nbody\n", "+++\na = 1\n+++\nbody\n"] {
        let fm = parse(text).unwrap();
        assert!(fm.meta.is_empty(), "{text:?}");
        assert_eq!(fm.body, text);
    }
}

#[test]
fn parse_non_mapping_yaml_is_empty_meta_like_v2() {
    let fm = parse("---\n- a\n- b\n---\nbody").unwrap();
    assert!(fm.meta.is_empty());
    assert_eq!(fm.body, "body");
}

#[test]
fn parse_invalid_yaml_is_an_error() {
    let e = parse("---\ntitle: Foo: bar\n---\nbody\n").unwrap_err();
    assert_eq!(e.line, 1, "{e}");
    assert!(parse("---\n1: x\n---\nbody\n").is_err(), "non-string key");
    assert!(parse("---\na: !!binary aGk=\n---\n").is_err(), "binary");
}

// --- read (markdown files) --------------------------------------------------

#[test]
fn read_markdown_file() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "n.md", b"---\r\ntitle: x\r\n---\r\nbody\r\n");
    let fm = read(&path).unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("x"))]));
    assert_eq!(fm.body, "body\r\n");
    assert_eq!(fm.line_ending, LineEnding::Crlf);
}

#[test]
fn read_bom_file() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "n.md", b"\xef\xbb\xbf---\ntitle: x\n---\nbody\n");
    let fm = read(&path).unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("x"))]));
    assert_eq!(fm.body, "body\n");
    assert!(fm.had_bom);
}

#[test]
fn read_invalid_yaml_names_the_file() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "bad.md", b"---\ntitle: Foo: bar\n---\nbody\n");
    let e = read(&path).unwrap_err();
    assert_eq!(e.path, path);
    assert!(
        matches!(e.problem, Problem::Yaml(ref y) if y.line == 1),
        "{e}"
    );
    assert!(e.to_string().contains("bad.md"), "{e}");
}

#[test]
fn read_errors_for_missing_and_non_utf8_files() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.md");
    assert!(matches!(
        read(&missing).unwrap_err().problem,
        Problem::Io(_)
    ));
    let path = write(&dir, "latin1.md", b"---\ntitle: caf\xe9\n---\n");
    assert_eq!(read(&path).unwrap_err().problem, Problem::NotUtf8);
}

// --- YAML neurons ------------------------------------------------------------

#[test]
fn yaml_neuron_block_is_read_and_the_rest_kept() {
    let fm = parse_yaml_neuron("#---\n# title: Spec\n# tags: [a, b]\n#---\nkey: value\n").unwrap();
    assert_eq!(
        fm.meta,
        meta(&[
            ("title", str_("Spec")),
            ("tags", Value::Seq(vec![str_("a"), str_("b")])),
        ])
    );
    assert_eq!(fm.body, "key: value\n");
}

#[test]
fn yaml_neuron_prefix_hash_without_space_and_leading_blank_lines() {
    let fm = parse_yaml_neuron("\n\n#---\n#title: x\n#---\nrest").unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("x"))]));
    assert_eq!(fm.body, "rest");
}

#[test]
fn yaml_neuron_keeps_yaml_types() {
    let fm = parse_yaml_neuron("#---  \n# a: 2026-01-02\n# b: yes\n#---\t\nrest").unwrap();
    assert_eq!(
        fm.meta,
        meta(&[
            ("a", Value::Date(Date::new(2026, 1, 2).unwrap())),
            ("b", Value::Bool(true)),
        ])
    );
    assert_eq!(fm.body, "rest");
}

#[test]
fn yaml_neuron_crlf_and_bom() {
    let fm = parse_yaml_neuron("#---\r\n# title: x\r\n#---\r\nk: v\r\n").unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("x"))]));
    assert_eq!(fm.body, "k: v\r\n");
    assert_eq!(fm.line_ending, LineEnding::Crlf);
    let fm = parse_yaml_neuron("\u{feff}#---\n# title: x\n#---\nk: v\n").unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("x"))]));
    assert!(fm.had_bom);
    assert_eq!(fm.body, "k: v\n");
}

#[test]
fn yaml_neuron_empty_block_and_non_mapping_block() {
    let fm = parse_yaml_neuron("#---\n#---\nrest").unwrap();
    assert!(fm.meta.is_empty());
    assert_eq!(fm.body, "rest");
    let fm = parse_yaml_neuron("#---\n# - a\n#---\nrest").unwrap();
    assert!(fm.meta.is_empty());
    assert_eq!(fm.body, "rest");
}

#[test]
fn yaml_without_a_complete_block_has_no_metadata() {
    for text in [
        "#---\n# title: x\nkey: v\n",       // no closing line
        "#---\n  # title: x\n#---\nrest\n", // indented line: the gate passes, the reader refuses
        "  #---\n# a: 1\n#---\nrest",       // indented opener
        "key: v\n",
    ] {
        let fm = parse_yaml_neuron(text).unwrap();
        assert!(fm.meta.is_empty(), "{text:?}");
        assert_eq!(fm.body, text);
    }
}

#[test]
fn yaml_neuron_invalid_yaml_is_an_error() {
    // v2 silently reads this as no metadata; v3 reports it.
    let e = parse_yaml_neuron("#---\n# title: Foo: bar\n#---\nk: v\n").unwrap_err();
    assert_eq!(e.line, 1, "{e}");
}

#[test]
fn yaml_neuron_files_and_the_opt_in_gate() {
    let dir = TempDir::new().unwrap();
    let spec = write(
        &dir,
        "spec.yml",
        b"#---\n# title: Spec\n#---\nopenapi: 3.1.0\n",
    );
    assert!(has_yaml_opt_in_block(&spec));
    assert!(is_yaml_neuron(&spec));
    let fm = read_yaml_neuron(&spec).unwrap();
    assert_eq!(fm.meta, meta(&[("title", str_("Spec"))]));
    assert_eq!(fm.body, "openapi: 3.1.0\n");
    assert_eq!(read_neuron(&spec).unwrap(), fm);

    let yaml = write(&dir, "other.yaml", b"#---\n# a: 1\n#---\n");
    assert!(is_yaml_neuron(&yaml));

    let open = write(&dir, "open.yml", b"#---\n# title: x\nkey: v\n");
    assert!(!has_yaml_opt_in_block(&open));
    assert!(!is_yaml_neuron(&open));

    let indented = write(&dir, "indented.yml", b"#---\n  # title: x\n#---\n");
    assert!(has_yaml_opt_in_block(&indented));

    let config = write(&dir, "kluris.yml", b"#---\n# title: x\n#---\nname: b\n");
    assert!(has_yaml_opt_in_block(&config));
    assert!(!is_yaml_neuron(&config), "kluris.yml is never a neuron");

    let md = write(&dir, "n.md", b"#---\n# a: 1\n#---\n");
    assert!(!is_yaml_neuron(&md));
    assert!(!is_yaml_neuron(Path::new("does/not/exist.yml")));
}

#[test]
fn opt_in_gate_reads_only_the_first_4096_bytes() {
    let dir = TempDir::new().unwrap();
    let mut long = b"#---\n".to_vec();
    while long.len() < 4100 {
        long.extend_from_slice(b"# filler: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n");
    }
    long.extend_from_slice(b"#---\nrest\n");
    let path = write(&dir, "long.yml", &long);
    assert!(!has_yaml_opt_in_block(&path));
    assert!(!is_yaml_neuron(&path));
}

#[test]
fn read_neuron_dispatches_on_the_suffix() {
    let dir = TempDir::new().unwrap();
    let upper = write(&dir, "Spec.YML", b"#---\n# a: 1\n#---\nrest");
    let fm = read_neuron(&upper).unwrap();
    assert_eq!(fm.meta, meta(&[("a", Value::Int(BigInt::from(1)))]));
    let md = write(&dir, "n.md", b"---\na: 1\n---\nbody");
    assert_eq!(read_neuron(&md).unwrap().body, "body");
}

#[test]
fn read_yaml_neuron_invalid_utf8_is_replaced_like_v2() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "x.yml", b"#---\n# a: 1\n#---\nk: caf\xe9\n");
    let fm = read_yaml_neuron(&path).unwrap();
    assert_eq!(fm.body, "k: caf\u{fffd}\n");
}
