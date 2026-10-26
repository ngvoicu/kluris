//! `kluris search`: v2's JSON keys and types, the added keys, errors and the
//! text output's wording (research-03 §1.5; research-05 §6).

use std::path::Path;

use assert_cmd::Command;
use assert_fs::TempDir;
use assert_fs::prelude::*;
use serde_json::Value;

fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
    cmd
}

fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// A home with one registered brain `demo`.
fn setup() -> TempDir {
    let home = TempDir::new().unwrap();
    let brain = home.child("demo");
    brain
        .child("brain.md")
        .write_str("---\nupdated: 2026-01-01\n---\n# demo\n\n## Lobes\n\n- [k](k/map.md)\n")
        .unwrap();
    brain
        .child("glossary.md")
        .write_str("# Glossary\n\n**JWT** - JSON Web Token\n")
        .unwrap();
    brain.child("k/map.md").write_str("# k\n").unwrap();
    brain
        .child("k/jwt.md")
        .write_str(
            "---\nparent: map.md\nupdated: 2026-02-03\ntags: [auth]\nrelated: [old.md]\n---\n# JWT tokens\n\n## Signing\n\nWe sign every JWT with a rotating key.\n",
        )
        .unwrap();
    brain
        .child("k/old.md")
        .write_str(
            "---\nstatus: deprecated\nreplaced_by: jwt.md\nupdated: 2025-01-01\n---\n# Old JWT notes\n\nJWT used to be signed by hand.\n",
        )
        .unwrap();
    home.child(".kluris/config.yml")
        .write_str(&format!(
            "brains:\n  demo:\n    path: {}\n    description: ''\n",
            yaml_path(brain.path())
        ))
        .unwrap();
    home
}

fn json(home: &TempDir, args: &[&str]) -> (String, Value) {
    let out = kluris(home.path()).args(args).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let value = serde_json::from_str(&text).unwrap();
    (text, value)
}

fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn json_keeps_v2s_keys_in_order_and_adds_the_new_ones() {
    let home = setup();
    let (text, doc) = json(&home, &["search", "jwt", "--json"]);
    assert_eq!(
        keys(&doc),
        ["ok", "brain", "query", "total", "results", "engine"]
    );
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["brain"], "demo");
    assert_eq!(doc["query"], "jwt");
    assert_eq!(doc["engine"], "bm25");
    let results = doc["results"].as_array().unwrap();
    assert_eq!(doc["total"].as_u64().unwrap() as usize, results.len());
    assert!(results.len() >= 3, "{text}");
    for r in results {
        let k = keys(r);
        assert_eq!(
            k[..7],
            [
                "file",
                "file_type",
                "title",
                "matched_fields",
                "snippet",
                "score",
                "deprecated"
            ],
            "{r}"
        );
        let deprecated = r["deprecated"].as_bool().unwrap();
        let mut added = vec![
            "heading",
            "line",
            "matched_terms",
            "coverage",
            "related",
            "updated",
        ];
        if deprecated {
            added.push("replaced_by");
        }
        assert_eq!(k[7..], added[..], "{r}");
        assert!(r["file"].is_string() && r["title"].is_string() && r["snippet"].is_string());
        assert!(["markdown", "yaml"].contains(&r["file_type"].as_str().unwrap()));
        assert!(r["score"].is_i64() && r["score"].as_i64().unwrap() >= 1);
        for f in r["matched_fields"].as_array().unwrap() {
            assert!(["title", "tag", "path", "heading", "body"].contains(&f.as_str().unwrap()));
        }
        assert!(r["heading"].is_string() || r["heading"].is_null());
        assert!(r["line"].is_u64() || r["line"].is_null());
        assert!(r["matched_terms"].is_array() && r["related"].is_array());
        assert!(r["coverage"].is_f64(), "coverage is always a float: {r}");
        assert!(r["updated"].is_string() || r["updated"].is_null());
    }
    assert!(text.contains("\"coverage\": 1.0"), "{text}");
    // Python's json.dumps spacing.
    assert!(
        text.starts_with("{\"ok\": true, \"brain\": \"demo\", \"query\": \"jwt\", \"total\": ")
    );
    assert!(text.ends_with(", \"engine\": \"bm25\"}\n"));
}

#[test]
fn hits_carry_heading_line_related_updated_and_replaced_by() {
    let home = setup();
    let (_, doc) = json(&home, &["search", "rotating key", "--json"]);
    let top = &doc["results"][0];
    assert_eq!(top["file"], "k/jwt.md");
    assert_eq!(top["heading"], "Signing");
    assert_eq!(top["line"], 9);
    assert_eq!(top["related"], serde_json::json!(["k/old.md"]));
    assert_eq!(top["updated"], "2026-02-03");
    let (_, doc) = json(&home, &["search", "signed by hand", "--json"]);
    let old = &doc["results"][0];
    assert_eq!(
        (old["file"].as_str(), old["deprecated"].as_bool()),
        (Some("k/old.md"), Some(true))
    );
    assert_eq!(old["replaced_by"], "jwt.md");
}

#[test]
fn glossary_entries_keep_v2s_shape() {
    let home = setup();
    let (_, doc) = json(&home, &["search", "JWT", "--json", "--limit", "20"]);
    let glossary: Vec<&Value> = doc["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["file"] == "glossary.md")
        .collect();
    assert_eq!(glossary.len(), 1);
    assert_eq!(glossary[0]["title"], "JWT");
    assert_eq!(glossary[0]["file_type"], "markdown");
    assert_eq!(glossary[0]["deprecated"], false);
    assert_eq!(glossary[0]["line"], Value::Null);
}

#[test]
fn limit_zero_or_negative_returns_no_results() {
    let home = setup();
    for limit in ["0", "-1"] {
        let (text, _) = json(&home, &["search", "jwt", "--limit", limit, "--json"]);
        assert_eq!(
            text,
            "{\"ok\": true, \"brain\": \"demo\", \"query\": \"jwt\", \"total\": 0, \"results\": [], \"engine\": \"bm25\"}\n"
        );
    }
}

#[test]
fn an_empty_query_is_refused_before_any_brain_is_resolved() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .args(["search", "", "--json"])
        .assert()
        .code(1)
        .stdout("{\"ok\": false, \"error\": \"Query cannot be empty.\"}\n");
    kluris(home.path())
        .args(["search", ""])
        .assert()
        .code(1)
        .stderr("Error: Query cannot be empty.\n");
}

#[test]
fn a_limit_that_is_not_an_integer_is_clicks_error() {
    let home = setup();
    kluris(home.path())
        .args(["search", "--limit", "abc", "q", "--json"])
        .assert()
        .code(1)
        .stdout("{\"ok\": false, \"error\": \"Invalid value for '--limit': 'abc' is not a valid integer.\"}\n");
    let out = kluris(home.path())
        .args(["search", "--limit", "abc", "q"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("Usage: kluris search"), "{stderr}");
    assert!(
        stderr.ends_with("Error: Invalid value for '--limit': 'abc' is not a valid integer.\n"),
        "{stderr}"
    );
}

#[test]
fn brain_all_is_refused() {
    let home = setup();
    kluris(home.path())
        .args(["search", "jwt", "--brain", "all", "--json"])
        .assert()
        .code(1)
        .stdout("{\"ok\": false, \"error\": \"--brain all is only supported on dream, status, mri, and companion commands.\"}\n");
}

#[test]
fn text_output_uses_v2s_wording() {
    let home = setup();
    let out = kluris(home.path())
        .args(["search", "zebra"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "No results for 'zebra' in demo.\n"
    );
    let out = kluris(home.path())
        .args(["search", "rotating key"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("result(s) for 'rotating key' in demo"),
        "{text}"
    );
    assert!(text.contains("        k/jwt.md\n"), "{text}");
}

#[test]
fn a_two_line_snippet_indents_its_continuation() {
    let home = setup();
    home.child("demo/k/two.md")
        .write_str(
            "---\nparent: map.md\nupdated: 2026-01-02\n---\n# Two lines\n\nfirst line about needles\nsecond line about needles\n",
        )
        .unwrap();
    let (_, doc) = json(&home, &["search", "needles", "--json"]);
    let hit = doc["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["file"] == "k/two.md")
        .unwrap();
    let snippet = hit["snippet"].as_str().unwrap();
    assert!(
        snippet.contains("first line about needles\nsecond line about needles"),
        "the JSON snippet keeps the raw lines: {snippet:?}"
    );
    let out = kluris(home.path())
        .args(["search", "needles"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    for line in snippet.trim_end_matches('\n').split('\n') {
        if line.is_empty() {
            continue;
        }
        assert!(
            text.contains(&format!("        {line}\n")),
            "snippet line {line:?} lost its indent:\n{text}"
        );
    }
}

#[test]
fn skipped_files_are_reported_in_json_and_on_stderr() {
    // T-79 finding 2: a malformed neuron is skipped and reported.
    let home = setup();
    home.child("demo/k/bad.md")
        .write_str("---\nrelated: [\n---\n# Bad\n\njwt\n")
        .unwrap();
    let (text, doc) = json(&home, &["search", "jwt", "--json"]);
    assert_eq!(
        keys(&doc),
        [
            "ok", "brain", "query", "total", "results", "engine", "skipped"
        ]
    );
    let skipped = doc["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{text}");
    assert_eq!(keys(&skipped[0]), ["file", "error"]);
    assert_eq!(skipped[0]["file"], "k/bad.md");
    let error = skipped[0]["error"].as_str().unwrap();
    assert!(error.starts_with("invalid frontmatter: "), "{error}");
    assert!(
        !error.contains(&home.path().display().to_string()),
        "no absolute path: {error}"
    );
    assert!(
        doc["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["file"] != "k/bad.md")
    );

    let out = kluris(home.path())
        .args(["search", "jwt"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("Skipped k/bad.md: invalid frontmatter: "),
        "{stderr}"
    );
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("result(s) for 'jwt' in demo")
    );
}
