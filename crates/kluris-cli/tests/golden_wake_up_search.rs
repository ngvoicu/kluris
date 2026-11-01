//! Golden tests of `kluris wake-up` and `kluris search`.
//!
//! wake-up: each run's exit code, stdout and stderr, normalized by the
//! brain's own folder, so its `path` reads `<ROOT>` on every OS; every run
//! also checks that `path` is the brain's path as stored, native separators
//! and all.
//!
//! search: its ranking, scores and snippets are its own (BM25F over heading
//! chunks), so a snapshot holds what does not depend on them: the error
//! envelopes and exit codes, `total`, and the items found at a limit that
//! holds them all, sorted, each with its file type, `deprecated` flag and
//! matched fields. Every JSON output is checked whole against the `--json`
//! layout before it is read, its keys and their types are checked, and a
//! search that succeeds writes nothing to stderr. The bulk queries run
//! through the CLI against frozen rows (`tests/fixtures/search-rows.jsonl`)
//! from an independent literal-substring search: every item it found is in
//! the results while they are under the limit, with its file type,
//! `deprecated` flag and matched fields among the result's, and every
//! result is an item the filters keep.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use common::brains::Brains;
use common::search::{Found, found_list};
use common::{
    Form, Run, Sandbox, assert_unchanged, normalize, parse_json_line, run_end, section, tree_files,
    windows_variant, write, write_files,
};
use serde_json::Value;

/// A limit that holds every result.
const EVERYTHING: i64 = 1_000_000;

// --- output as the snapshots hold it ------------------------------------------

/// A run's stdout, the last line of its stderr (where an error message
/// ends) and its exit code, normalized by `root`.
fn render_error(run: &Run, root: &Path) -> String {
    let last = run
        .stderr
        .lines()
        .last()
        .map_or(String::new(), |l| format!("{l}\n"));
    format!(
        "{}{}{}",
        section(
            "stdout",
            &normalize(&run.stdout, root, Form::of_stream(&run.stdout))
        ),
        section("stderr, last line", &normalize(&last, root, Form::Text)),
        run_end(run.code),
    )
}

// --- wake-up --------------------------------------------------------------------

/// `wake-up --json` with `brain` registered; its output, normalized by the
/// brain's folder, after checking `path` is `brain` as stored.
fn wake_up_json(sb: &Sandbox, brain: &Path) -> String {
    let run = sb.kluris(&["wake-up", "--json"]);
    if run.code == 0 {
        let doc = parse_json_line(&run.stdout);
        assert_eq!(doc["path"], brain.display().to_string(), "{}", run.stdout);
    }
    run.render(brain)
}

/// `wake-up` (text) with `brain` registered; its output, normalized by the
/// brain's folder, after checking its `Path:` line is `brain` as stored.
fn wake_up_text(sb: &Sandbox, brain: &Path) -> String {
    let run = sb.kluris(&["wake-up"]);
    let line = format!("  Path: {}\n", brain.display());
    assert!(
        run.stdout.contains(&line),
        "{line:?} not in {:?}",
        run.stdout
    );
    run.render(brain)
}

/// The fixture brain `name`, registered under its name.
fn registered_fixture(sb: &Sandbox, name: &str, description: &str) -> Option<PathBuf> {
    let brain = sb.fixture_brain(name)?;
    sb.register(name, &brain, description);
    Some(brain)
}

/// The snapshots with a Windows variant: Windows lists the `wakeup` brain's
/// lobes without regard to case, so `Zebra` comes after `legacy` there.
const WINDOWS_VARIANTS: &[&str] = &[
    "wake_up_json_wakeup",
    "wake_up_text_wakeup",
    "wake_up_renamed",
];

/// `text` as the snapshot `name` (its Windows variant when
/// [`WINDOWS_VARIANTS`] names it).
fn snapshot(name: &str, text: &str) {
    let check = || insta::assert_snapshot!(name, text);
    if WINDOWS_VARIANTS.contains(&name) {
        windows_variant(check);
    } else {
        check();
    }
}

#[test]
fn wake_up_json_of_the_fixture_brains() {
    for name in ["wakeup", "full", "links", "malformed"] {
        let sb = Sandbox::new();
        let Some(brain) = registered_fixture(&sb, name, "Fixture brain") else {
            continue;
        };
        snapshot(&format!("wake_up_json_{name}"), &wake_up_json(&sb, &brain));
    }
}

#[test]
fn wake_up_text_of_the_fixture_brains() {
    for name in ["wakeup", "full"] {
        let sb = Sandbox::new();
        let brain = registered_fixture(&sb, name, "A described brain").unwrap();
        snapshot(&format!("wake_up_text_{name}"), &wake_up_text(&sb, &brain));
    }
}

#[test]
fn wake_up_of_an_empty_brain() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("empty");
    sb.register("empty", &brain, "Fixture brain");
    insta::assert_snapshot!("wake_up_json_empty", wake_up_json(&sb, &brain));
    sb.register("empty", &brain, "");
    insta::assert_snapshot!("wake_up_text_empty", wake_up_text(&sb, &brain));
}

/// `updated` values YAML reads as every kind of scalar and collection, and
/// strings that only look like dates. `2026-02-30` is a timestamp no YAML
/// reader can construct: that neuron does not read.
const ODD_UPDATED: [&str; 30] = [
    "[2026-01-01, x]",
    "true",
    "1.5",
    "{a: 1, 'b''c': [null, 2026-01-01 10:00:00]}",
    "[2026-01-01 10:00:00+01:00, 2026-01-01 10:00:00-05:30, 2026-01-01 10:00:00Z]",
    r#""it's""#,
    r#"["it's", 'say "hi"', "tab\there", "\x7f\x85\xa0\u2028\U0001F600\u00e9"]"#,
    "1e3",
    "-0.0",
    ".inf",
    ".nan",
    "0x1F",
    "123456789012345678901234567890",
    "[]",
    "{}",
    "2026-01-01 10:00:05.000123",
    "2026-01-01 10:00:00+05:45",
    "''",
    "yesterday",
    "No",
    "-17",
    "2026-W10",
    "20260101xx",
    "2026-02-30",
    r#""2026-13-01""#,
    "[2026-01-01 10:00:00.000001]",
    r#""\u00e9t\u00e9 2026""#,
    r#""2026-01-0\u00e9""#,
    "!!str 2026-01-01",
    "2026-01-01t10:00:00z",
];

#[test]
fn wake_up_reads_odd_updated_values() {
    // Five neurons per brain, so every value that reads is in `recent`.
    for (batch, values) in ODD_UPDATED.chunks(5).enumerate() {
        let sb = Sandbox::new();
        let brain = sb.brain_dir("odd");
        for (i, value) in values.iter().enumerate() {
            write_files(
                &brain,
                &[(
                    &format!("lobe/n{i}.md"),
                    &format!("---\nupdated: {value}\n---\n# n{i}\n"),
                )],
            );
        }
        sb.register("odd", &brain, "Fixture brain");
        let out = wake_up_json(&sb, &brain);
        let doc = parse_json_line(&sb.kluris(&["wake-up", "--json"]).stdout);
        let readable = values.iter().filter(|v| **v != "2026-02-30").count();
        assert_eq!(doc["recent"].as_array().unwrap().len(), readable);
        insta::assert_snapshot!(format!("wake_up_odd_updated_{}", batch * 5), out);
    }
}

#[test]
fn wake_up_truncates_brain_md() {
    let bodies = [
        ("exact", "a".repeat(4000)),
        ("one_over", "a".repeat(4001)),
        ("cut_3_byte", "a".repeat(3998) + "日" + "tail"),
        ("cut_2_byte", "a".repeat(3999) + &"é".repeat(3)),
        ("many_lines", "line é日\u{1F600}\n".repeat(400)),
    ];
    for (id, body) in bodies {
        let sb = Sandbox::new();
        let brain = sb.brain_dir("big");
        write_files(
            &brain,
            &[(
                "brain.md",
                &format!("---\nupdated: 2026-01-01\n---\n\n{body}\n"),
            )],
        );
        sb.register("big", &brain, "Fixture brain");
        insta::assert_snapshot!(format!("wake_up_brain_md_{id}"), wake_up_json(&sb, &brain));
    }
}

#[test]
fn wake_up_reads_a_crlf_brain() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("crlf");
    let files = [
        (
            "brain.md",
            "---\nupdated: 2026-01-01\n---\n\n# crlf\n\nbody line\n\n",
        ),
        (
            "glossary.md",
            "# Glossary\n\n**A** - first\n| B | second |\n",
        ),
        ("lobe/map.md", "# Lobe\n\nup [x](x)\n\n  The lobe.  \n"),
        ("lobe/n.md", "---\nupdated: 2026-02-02\n---\n# n\n"),
        ("lobe/y.yml", "#---\n# updated: 2026-02-03\n#---\nk: v\n"),
        ("lobe/cr.md", "---\rupdated: 2026-02-04\r---\r# cr\r"),
    ];
    for (rel, text) in files {
        write(&brain.join(rel), text.replace('\n', "\r\n").as_bytes());
    }
    sb.register("crlf", &brain, "Fixture brain");
    insta::assert_snapshot!("wake_up_crlf", wake_up_json(&sb, &brain));
}

#[test]
fn wake_up_prints_the_stored_path_in_its_plain_form() {
    // The stored path ends in a separator, `.` or a doubled separator.
    let sep = std::path::MAIN_SEPARATOR_STR;
    let suffixes = [
        ("trailing", sep.to_string()),
        ("dot", format!("{sep}.{sep}")),
        ("double", format!("{sep}{sep}")),
    ];
    for (id, suffix) in suffixes {
        let sb = Sandbox::new();
        let brain = sb.brain_dir("p");
        let stored = format!("{}{suffix}", brain.display());
        sb.write_config(&[("p", Path::new(&stored), "")]);
        insta::assert_snapshot!(
            format!("wake_up_stored_path_{id}_json"),
            wake_up_json(&sb, &brain)
        );
        insta::assert_snapshot!(
            format!("wake_up_stored_path_{id}_text"),
            wake_up_text(&sb, &brain)
        );
    }
}

#[test]
fn wake_up_takes_name_and_description_from_the_config() {
    let sb = Sandbox::new();
    let brain = sb.fixture_brain("wakeup").unwrap();
    sb.register("renamed", &brain, "déjà vu — ok");
    snapshot("wake_up_renamed", &wake_up_json(&sb, &brain));
}

#[test]
fn wake_up_reads_frontmatter_behind_a_bom() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("bom");
    let bom = "\u{feff}";
    write_files(
        &brain,
        &[
            (
                "brain.md",
                &format!("{bom}---\nupdated: 2026-01-01\n---\n\n# bom\n"),
            ),
            (
                "lobe/map.md",
                &format!("{bom}---\ndescription: From frontmatter\n---\n# Lobe\n\nbody line\n"),
            ),
            (
                "lobe/n.md",
                &format!("{bom}---\nupdated: 2026-03-03\n---\n# n\n"),
            ),
            ("lobe/plain.md", "---\nupdated: 2026-01-01\n---\n# plain\n"),
        ],
    );
    sb.register("bom", &brain, "Fixture brain");
    insta::assert_snapshot!("wake_up_bom", wake_up_json(&sb, &brain));
}

#[test]
fn wake_up_gives_a_map_md_that_is_not_utf8_no_description() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("bad-map");
    write(
        &brain.join("lobe/map.md"),
        b"# Lobe\n\n\xff\xfe description\n",
    );
    write_files(&brain, &[("lobe/n.md", "---\nupdated: 2026-01-01\n---\n")]);
    sb.register("bad-map", &brain, "Fixture brain");
    insta::assert_snapshot!("wake_up_map_md_not_utf8", wake_up_json(&sb, &brain));
}

#[test]
fn wake_up_counts_frontmatter_it_refuses() {
    // A merge key, `!!binary` and `!!set` are refused; a non-string key does
    // not read.
    let sb = Sandbox::new();
    let brain = sb.brain_dir("refused");
    write_files(
        &brain,
        &[
            (
                "lobe/merge.md",
                "---\nbase: &b {a: 1}\nm:\n  <<: *b\nupdated: 2026-01-02\n---\n",
            ),
            (
                "lobe/binary.md",
                "---\nb: !!binary aGk=\nupdated: 2026-01-03\n---\n",
            ),
            (
                "lobe/set.md",
                "---\ns: !!set {a: null}\nupdated: 2026-01-04\n---\n",
            ),
            ("lobe/int-key.md", "---\n1: x\nupdated: 2026-01-05\n---\n"),
            ("lobe/plain.md", "---\nupdated: 2026-01-01\n---\n"),
        ],
    );
    sb.register("refused", &brain, "Fixture brain");
    insta::assert_snapshot!("wake_up_refused_frontmatter", wake_up_json(&sb, &brain));
}

#[test]
fn wake_up_writes_nothing() {
    let sb = Sandbox::new();
    let brain = registered_fixture(&sb, "wakeup", "Fixture brain").unwrap();
    let before = tree_files(&sb.work);
    let run = sb.kluris(&["wake-up", "--json"]);
    assert_eq!(run.code, 0, "{}", run.render(&sb.root));
    assert!(!run.stdout.is_empty());
    assert_unchanged(&before, &tree_files(&sb.work), "wake-up");
    assert!(brain.is_dir());
}

#[test]
fn wake_up_of_a_team_brain_with_nothing_new() {
    for (id, text) in [("json", false), ("text", true)] {
        let sb = Sandbox::new();
        let brain = sb.team_brain(false);
        let (files, refs) = (tree_files(&sb.work), sb.team_refs());
        let out = if text {
            wake_up_text(&sb, &brain)
        } else {
            wake_up_json(&sb, &brain)
        };
        assert_unchanged(&files, &tree_files(&sb.work), "the read");
        assert_eq!(sb.team_refs(), refs);
        insta::assert_snapshot!(format!("wake_up_team_nothing_new_{id}"), out);
    }
}

#[test]
fn wake_up_pulls_a_teammates_push() {
    // The push lands before the read: the brain is read with the
    // teammate's `k/zebra.md`, and the `sync` object (or the stderr line)
    // says so.
    for (id, text) in [("json", false), ("text", true)] {
        let sb = Sandbox::new();
        let brain = sb.team_brain(true);
        assert!(!sb.has_remote_main());
        let out = if text {
            wake_up_text(&sb, &brain)
        } else {
            wake_up_json(&sb, &brain)
        };
        assert!(brain.join("k/zebra.md").is_file(), "not pulled");
        insta::assert_snapshot!(format!("wake_up_team_pulled_{id}"), out);
    }
}

// --- search: the CLI ---------------------------------------------------------------

/// The keys of every result, in order, before those a deprecated hit adds.
const RESULT_KEYS: [&str; 13] = [
    "file",
    "file_type",
    "title",
    "matched_fields",
    "snippet",
    "score",
    "deprecated",
    "heading",
    "line",
    "matched_terms",
    "coverage",
    "related",
    "updated",
];

/// The fields a result can match, in their order.
const FIELDS: [&str; 5] = ["title", "tag", "path", "heading", "body"];

/// A sandbox with the search-eval brain `acme-sme` registered alone.
fn demo() -> (Sandbox, PathBuf) {
    let sb = Sandbox::new();
    let brain = sb.eval_brain("acme-sme");
    sb.register("acme-sme", &brain, "");
    (sb, brain)
}

/// The keys and types of a `search --json` document; `total` the number of
/// results, at most `limit`; `skipped` only when files were left out.
fn check_shape(doc: &Value, brain: &str, query: &str, limit: i64) {
    let map = doc.as_object().unwrap();
    let mut keys = vec!["ok", "brain", "query", "total", "results", "engine"];
    if let Some(skipped) = doc.get("skipped") {
        keys.push("skipped");
        let skipped = skipped.as_array().unwrap();
        assert!(
            !skipped.is_empty(),
            "skipped is present only when non-empty"
        );
        for s in skipped {
            let s = s.as_object().unwrap();
            assert_eq!(s.keys().collect::<Vec<_>>(), ["file", "error"]);
            assert!(s.values().all(Value::is_string), "{s:?}");
        }
    }
    if doc.get("sync").is_some() {
        // What the read's sync did, after the search's own keys.
        keys.push("sync");
    }
    assert_eq!(map.keys().collect::<Vec<_>>(), keys);
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["brain"], brain);
    assert_eq!(doc["query"], query);
    assert_eq!(doc["engine"], "bm25");
    let results = doc["results"].as_array().unwrap();
    assert_eq!(doc["total"].as_u64(), Some(results.len() as u64));
    assert!(results.len() as i64 <= limit.max(0));
    for r in results {
        let keys: Vec<&String> = r.as_object().unwrap().keys().collect();
        let mut want: Vec<&str> = RESULT_KEYS.to_vec();
        if r["deprecated"] == true {
            want.push("replaced_by");
        }
        assert_eq!(keys, want, "{r}");
        let file = r["file"].as_str().unwrap();
        assert!(!file.contains('\\'), "{file}");
        assert!(["markdown", "yaml"].contains(&r["file_type"].as_str().unwrap()));
        assert!(r["title"].is_string() && r["snippet"].is_string(), "{r}");
        assert!(r["score"].as_i64().is_some_and(|s| s >= 1), "{r}");
        assert!(r["deprecated"].is_boolean());
        let fields: Vec<&str> = r["matched_fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        let ordered: Vec<&str> = FIELDS.into_iter().filter(|f| fields.contains(f)).collect();
        assert!(!fields.is_empty() && fields == ordered, "{fields:?}");
        assert!(r["heading"].is_null() || r["heading"].is_string());
        assert!(r["line"].is_null() || r["line"].as_u64().is_some_and(|l| l >= 1));
        assert!(
            r["matched_terms"]
                .as_array()
                .unwrap()
                .iter()
                .all(Value::is_string)
        );
        assert!(r["coverage"].is_f64(), "{r}");
        assert!((0.0..=1.0).contains(&r["coverage"].as_f64().unwrap()));
        let related = r["related"].as_array().unwrap();
        assert!(related.len() <= 3 && related.iter().all(Value::is_string));
        assert!(r["updated"].is_null() || r["updated"].is_string());
        if r["deprecated"] == true {
            assert!(r["replaced_by"].is_null() || r["replaced_by"].is_string());
        }
    }
}

impl Found {
    /// A frozen row's `[file, title, file_type, deprecated, matched_fields]`;
    /// an item the filters keep has no matched fields.
    fn of_row(row: &Value) -> Found {
        let s = |i: usize| row[i].as_str().unwrap().to_string();
        Found {
            file: s(0),
            title: s(1),
            file_type: s(2),
            deprecated: row[3].as_bool().unwrap(),
            matched_fields: row
                .get(4)
                .map_or(&[][..], |f| f.as_array().unwrap())
                .iter()
                .map(|f| f.as_str().unwrap().to_string())
                .collect(),
        }
    }
}

/// `search --brain <brain> --json --limit <limit> [--lobe] [--tag] -- <q>`:
/// exit 0, nothing on stderr (files it skips are listed in the document),
/// stdout in the `--json` layout and shape-checked; its document.
fn search_json(sb: &Sandbox, brain: &str, q: &Query) -> Value {
    let limit = q.limit.to_string();
    let mut args = vec!["search", "--brain", brain, "--json", "--limit", &limit];
    if let Some(lobe) = &q.lobe {
        args.extend(["--lobe", lobe]);
    }
    if let Some(tag) = &q.tag {
        args.extend(["--tag", tag]);
    }
    args.extend(["--", &q.q]);
    let run = sb.kluris(&args);
    assert_eq!(run.code, 0, "{}", run.render(&sb.root));
    assert_eq!(run.stderr, "", "{}", run.render(&sb.root));
    let doc = parse_json_line(&run.stdout);
    check_shape(&doc, brain, &q.q, q.limit);
    doc
}

#[test]
fn search_errors() {
    let errors: [(&str, &[&str]); 8] = [
        ("empty_json", &["search", "", "--json"]),
        ("empty_text", &["search", ""]),
        (
            "limit_abc_json",
            &["search", "--limit", "abc", "q", "--json"],
        ),
        ("all_json", &["search", "q", "--brain", "all", "--json"]),
        ("all_text", &["search", "q", "--brain", "all"]),
        (
            "unknown_json",
            &["search", "q", "--brain", "nope", "--json"],
        ),
        ("unknown_text", &["search", "q", "--brain", "nope"]),
        ("no_query_json", &["search", "--json"]),
    ];
    let (sb, _) = demo();
    for (id, args) in errors {
        insta::assert_snapshot!(
            format!("search_error_{id}"),
            render_error(&sb.kluris(args), &sb.root)
        );
    }
}

#[test]
fn search_limit_that_is_not_an_integer_is_a_usage_error() {
    let (sb, _) = demo();
    let run = sb.kluris(&["search", "--limit", "abc", "q"]);
    let first = run.stderr.lines().next().unwrap_or("");
    assert!(
        first.starts_with("Usage: kluris search"),
        "{}",
        run.render(&sb.root)
    );
    insta::assert_snapshot!("search_limit_abc_text", render_error(&run, &sb.root));
}

#[test]
fn search_limits() {
    // Read as an integer is read: spaces around it, a sign, `_` between
    // digits. A limit of 0 or less gives no results.
    let limits = [
        ("0", "0", 0),
        ("minus_1", "-1", -1),
        ("minus_100", "-100", -100),
        ("space_3", " 3", 3),
        ("plus_2", "+2", 2),
        ("underscore_1_0", "1_0", 10),
    ];
    let (sb, _) = demo();
    for (id, text, n) in limits {
        let run = sb.kluris(&["search", "backup", "--limit", text, "--json"]);
        assert_eq!(run.code, 0, "{}", run.render(&sb.root));
        let doc = parse_json_line(&run.stdout);
        check_shape(&doc, "acme-sme", "backup", n);
        if n <= 0 {
            assert_eq!(doc["results"], serde_json::json!([]));
        }
        insta::assert_snapshot!(
            format!("search_limit_{id}"),
            format!("limit {text:?}: total {}\n", doc["total"])
        );
    }
}

#[test]
fn search_with_two_brains_needs_brain() {
    let sb = Sandbox::new();
    let a = sb.eval_brain("acme-sme");
    let b = sb.eval_brain("clinica-sme");
    sb.write_config(&[("acme-sme", &a, ""), ("clinica-sme", &b, "")]);
    for (id, args) in [
        ("json", &["search", "q", "--json"][..]),
        ("text", &["search", "q"][..]),
    ] {
        insta::assert_snapshot!(
            format!("search_two_brains_{id}"),
            render_error(&sb.kluris(args), &sb.root)
        );
    }
}

#[test]
fn search_with_no_brains_registered() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let run = sb.kluris(&["search", "q", "--json"]);
    insta::assert_snapshot!(
        "search_no_brains",
        format!(
            "{}{}",
            section(
                "stdout",
                &normalize(&run.stdout, &sb.root, Form::of_stream(&run.stdout))
            ),
            run_end(run.code)
        )
    );
}

#[test]
fn search_text_output() {
    let (sb, _) = demo();
    insta::assert_snapshot!(
        "search_text_no_results",
        sb.kluris(&["search", "zebra-not-there"]).render(&sb.root)
    );
    let run = sb.kluris(&["search", "backup"]);
    assert_eq!(run.code, 0, "{}", run.render(&sb.root));
    assert!(
        run.stdout.contains("operations/backups.md"),
        "{}",
        run.render(&sb.root)
    );
    let head = run.stdout.trim().lines().next().unwrap_or("");
    insta::assert_snapshot!("search_text_backup_head", format!("{head}\n"));
}

#[test]
fn search_writes_nothing() {
    let (sb, _) = demo();
    let before = tree_files(&sb.work);
    let run = sb.kluris(&["search", "backup", "--json"]);
    assert_eq!(run.code, 0, "{}", run.render(&sb.root));
    assert_unchanged(&before, &tree_files(&sb.work), "search");
}

#[test]
fn search_json_on_the_eval_brain() {
    let queries = [
        ("backup", "backup"),
        ("release_notes", "release notes"),
        ("jwt", "jwt"),
        ("glossary", "glossary"),
        ("dlq", "DLQ"),
        (
            "named_parameter_jdbc_template",
            "NamedParameterJdbcTemplate",
        ),
        ("bangs", "!!!!"),
    ];
    let (sb, _) = demo();
    for (id, q) in queries {
        let query = Query::all(q);
        let doc = search_json(&sb, "acme-sme", &query);
        let failures = disagreements(FROZEN.query("eval-cli", "acme-sme", &query), &doc);
        assert!(failures.is_empty(), "{}", failures.join("\n"));
        insta::assert_snapshot!(format!("search_eval_{id}"), found_list(&doc));
    }
}

#[test]
fn search_filters() {
    let filters: [(Option<&str>, Option<&str>); 8] = [
        (Some("operations"), None),
        (Some("operations/"), None),
        (Some("knowledge"), Some("auth")),
        (None, Some("postgres")),
        (None, Some("Postgres")),
        (Some("nope"), None),
        (Some("architecture/overview.md"), None),
        (Some(""), None),
    ];
    let queries = [
        ("backup", "backup"),
        ("the", "the"),
        ("postgres", "postgres"),
        ("release", "release"),
        ("a", "a"),
        ("bangs", "!!!!"),
        ("auth", "auth"),
    ];
    let (sb, _) = demo();
    for (id, q) in queries {
        let mut out = String::new();
        for (lobe, tag) in filters {
            let query = Query {
                lobe: lobe.map(String::from),
                tag: tag.map(String::from),
                ..Query::all(q)
            };
            let doc = search_json(&sb, "acme-sme", &query);
            let failures = disagreements(FROZEN.query("filters", "acme-sme", &query), &doc);
            assert!(failures.is_empty(), "{}", failures.join("\n"));
            let lobe = lobe.map(|l| format!(" --lobe {l:?}")).unwrap_or_default();
            let tag = tag.map(|t| format!(" --tag {t:?}")).unwrap_or_default();
            out.push_str(&section(&format!("search{lobe}{tag}"), &found_list(&doc)));
        }
        insta::assert_snapshot!(format!("search_filters_{id}"), out);
    }
}

#[test]
fn search_snippets_are_whole_utf8() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("utf8");
    let filler = "ș日\u{1F600}é ".repeat(120);
    write_files(
        &brain,
        &[(
            "k/a.md",
            &format!("---\nupdated: 2026-01-01\n---\n# A\n\n{filler}zebra{filler}\n"),
        )],
    );
    write(
        &brain.join("k/crlf.md"),
        b"---\r\nupdated: 2026-01-01\r\n---\r\n# C\r\n\r\nline one\r\nzebra two\r\n",
    );
    sb.register("utf8", &brain, "");
    let doc = search_json(&sb, "utf8", &Query::default_limit("zebra"));
    for r in doc["results"].as_array().unwrap() {
        let snippet = r["snippet"].as_str().unwrap();
        assert!(snippet.contains("zebra") && !snippet.contains('\r'), "{r}");
        assert!(!snippet.contains('\u{fffd}'), "{r}");
    }
    insta::assert_snapshot!("search_utf8", found_list(&doc));
}

#[test]
fn search_deprecated_hit_keeps_the_flag_and_names_its_replacement() {
    let (sb, _) = demo();
    let doc = search_json(&sb, "acme-sme", &Query::all("session"));
    let old: Vec<&Value> = doc["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["file"] == "knowledge/deprecated-sessions.md")
        .collect();
    assert_eq!(old.len(), 1, "{doc}");
    assert_eq!(old[0]["deprecated"], true);
    assert_eq!(old[0]["replaced_by"], "../architecture/auth-tokens.md");
}

#[test]
fn search_skips_and_reports_a_malformed_neuron() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("bad");
    write_files(
        &brain,
        &[
            ("k/bad.md", "---\nrelated: [\n---\n# Bad\n\nzebra\n"),
            (
                "k/good.md",
                "---\nupdated: 2026-01-01\n---\n# Good\n\nzebra\n",
            ),
        ],
    );
    sb.register("bad", &brain, "");
    let doc = search_json(&sb, "bad", &Query::default_limit("zebra"));
    let skipped = serde_json::to_string_pretty(&doc["skipped"]).unwrap();
    insta::assert_snapshot!(
        "search_malformed_json",
        format!(
            "{}{}",
            found_list(&doc),
            section("skipped", &(skipped + "\n"))
        )
    );
    let run = sb.kluris(&["search", "zebra"]);
    assert_eq!(run.code, 0, "{}", run.render(&sb.root));
    assert!(
        run.stdout.contains("result(s) for 'zebra' in bad"),
        "{}",
        run.render(&sb.root)
    );
    insta::assert_snapshot!(
        "search_malformed_text_stderr",
        normalize(&run.stderr, &sb.root, Form::Text)
    );
}

/// File names a neuron without a title is titled after: cased by Unicode's
/// title-casing rules, `-` read as a space.
const TITLE_NAMES: [&str; 19] = [
    "ΟΣ",
    "თბილისი",
    "ᾈ",
    "ŉ-x",
    "ǆemal",
    "straße",
    "ßa",
    "o'neil",
    "2nd-try",
    "ΣΑΣ-ΣΑΣ",
    "ﬁle",
    "ꭰꭱ",
    "ΑΣ.Β-c",
    "ALL-CAPS",
    "İstanbul",
    "ǰx",
    "ʰa",
    "aΣ",
    "𐐨𐐩",
];

#[test]
fn search_titles_neurons_after_their_file_names() {
    let sb = Sandbox::new();
    let brain = sb.brain_dir("titles");
    for name in TITLE_NAMES {
        write_files(
            &brain,
            &[
                (
                    &format!("md/{name}.md"),
                    "---\nupdated: 2026-01-01\n---\nno heading\n",
                ),
                (
                    &format!("yml/{name}.yml"),
                    "#---\n# updated: 2026-01-01\n#---\nk: v\n",
                ),
            ],
        );
    }
    sb.register("titles", &brain, "");
    let doc = search_json(&sb, "titles", &Query::all("/"));
    let titles: BTreeMap<String, String> = doc["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let s = |k: &str| r[k].as_str().unwrap().to_string();
            (s("file"), s("title"))
        })
        .collect();
    assert_eq!(titles.len(), 2 * TITLE_NAMES.len());
    assert_eq!(titles["md/ΟΣ.md"], "Ος");
    assert_eq!(titles["md/თბილისი.md"], "თბილისი");
    assert_eq!(titles["md/ᾈ.md"], "ᾈ");
    let out: String = titles.iter().map(|(f, t)| format!("{f} | {t}\n")).collect();
    insta::assert_snapshot!("search_file_name_titles", out);
}

#[cfg(unix)]
#[test]
fn search_reads_a_file_name_with_a_backslash() {
    // On POSIX `x\y.md` is one file, shown with `/` for the backslash.
    let sb = Sandbox::new();
    let brain = sb.brain_dir("bs");
    write_files(
        &brain,
        &[(
            "k/x\\y.md",
            "---\nupdated: 2026-01-01\n---\n# Odd\n\nzebra\n",
        )],
    );
    sb.register("bs", &brain, "");
    assert_agrees_with_frozen(&sb, "backslash", "bs");
    let doc = search_json(&sb, "bs", &Query::default_limit("zebra"));
    insta::assert_snapshot!("search_backslash_file_name", found_list(&doc));
}

#[test]
fn search_of_a_team_brain_with_nothing_new() {
    let sb = Sandbox::new();
    sb.team_brain(false);
    let (files, refs) = (tree_files(&sb.work), sb.team_refs());
    let run = sb.kluris(&["search", "jwt", "--json"]);
    assert_eq!(run.code, 0, "{}", run.render(&sb.root));
    assert_eq!(run.stderr, "");
    let doc = parse_json_line(&run.stdout);
    check_shape(&doc, "team", "jwt", 10);
    assert_unchanged(&files, &tree_files(&sb.work), "the read");
    assert_eq!(sb.team_refs(), refs);
    insta::assert_snapshot!("search_team_nothing_new_jwt", found_list(&doc));
}

#[test]
fn search_pulls_a_teammates_push() {
    // The push lands before the search: `zebras` finds the teammate's
    // `k/zebra.md`.
    for query in ["zebras", "jwt"] {
        let sb = Sandbox::new();
        let brain = sb.team_brain(true);
        let run = sb.kluris(&["search", query, "--json"]);
        assert_eq!(run.code, 0, "{}", run.render(&sb.root));
        assert_eq!(run.stderr, "");
        let doc = parse_json_line(&run.stdout);
        check_shape(&doc, "team", query, 10);
        assert_eq!(doc["sync"]["pulled"], 1, "{doc}");
        assert!(brain.join("k/zebra.md").is_file(), "not pulled");
        insta::assert_snapshot!(format!("search_team_pulled_{query}"), found_list(&doc));
    }
}

// --- search: the frozen rows -------------------------------------------------------

/// A search: the query, its filters and its limit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Query {
    q: String,
    lobe: Option<String>,
    tag: Option<String>,
    limit: i64,
}

impl Query {
    /// No filters, a limit that holds every result.
    fn all(q: &str) -> Query {
        Query {
            q: q.to_string(),
            lobe: None,
            tag: None,
            limit: EVERYTHING,
        }
    }

    /// No filters, the default limit of 10.
    fn default_limit(q: &str) -> Query {
        Query {
            limit: 10,
            ..Query::all(q)
        }
    }
}

/// A frozen query: its set, the brain it searches, and what the frozen
/// search found, sorted, with the items the query's filters keep.
struct FrozenQuery {
    set: String,
    brain: String,
    query: Query,
    found: Vec<Found>,
    kept: Vec<Found>,
}

/// The frozen rows of `tests/fixtures/search-rows.jsonl`: a provenance
/// line, then a row per query and a row per brain and filter pair (the
/// items those filters keep, without matched fields).
struct Frozen {
    queries: Vec<FrozenQuery>,
}

static FROZEN: LazyLock<Frozen> = LazyLock::new(Frozen::read);

impl Frozen {
    fn read() -> Frozen {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/search-rows.jsonl");
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut rows = text
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap());
        let header = rows.next().unwrap();
        assert!(
            header["fixture"].is_string(),
            "no provenance line: {header}"
        );
        let (filter_rows, query_rows): (Vec<Value>, Vec<Value>) =
            rows.partition(|row| row.get("items").is_some());
        let s = |row: &Value, key: &str| row[key].as_str().map(String::from);
        let filters = |row: &Value| (s(row, "brain").unwrap(), s(row, "lobe"), s(row, "tag"));
        let found = |row: &Value, key: &str| -> Vec<Found> {
            row[key]
                .as_array()
                .unwrap()
                .iter()
                .map(Found::of_row)
                .collect()
        };
        let kept: BTreeMap<_, _> = filter_rows
            .iter()
            .map(|row| (filters(row), found(row, "items")))
            .collect();
        let queries = query_rows
            .iter()
            .map(|row| {
                let (brain, lobe, tag) = filters(row);
                let kept = &kept[&(brain.clone(), lobe.clone(), tag.clone())];
                FrozenQuery {
                    set: s(row, "set").unwrap(),
                    query: Query {
                        q: s(row, "q").unwrap(),
                        lobe,
                        tag,
                        limit: row["limit"].as_i64().unwrap(),
                    },
                    found: found(row, "results"),
                    kept: kept.clone(),
                    brain,
                }
            })
            .collect();
        Frozen { queries }
    }

    /// The queries of `set` on `brain`, at least one.
    fn set(&self, set: &str, brain: &str) -> Vec<&FrozenQuery> {
        let rows: Vec<&FrozenQuery> = self
            .queries
            .iter()
            .filter(|row| row.set == set && row.brain == brain)
            .collect();
        assert!(!rows.is_empty(), "no frozen {set} queries for {brain}");
        rows
    }

    /// The frozen row of `query` in `set` on `brain`.
    fn query(&self, set: &str, brain: &str, query: &Query) -> &FrozenQuery {
        self.set(set, brain)
            .into_iter()
            .find(|row| row.query == *query)
            .unwrap_or_else(|| panic!("no frozen {set} row for {brain} {query:?}"))
    }
}

/// Where the results of `doc` disagree with `row`: a result that is not an
/// item the filters keep, or has another file type or `deprecated` flag
/// than that item; a frozen result that is missing while the results are
/// under the limit (results are matched by file and title, each once:
/// glossary entries share `glossary.md`); a frozen result whose file type
/// or `deprecated` flag differs, or whose matched fields are not all among
/// the result's.
fn disagreements(row: &FrozenQuery, doc: &Value) -> Vec<String> {
    let found = Found::all_of(doc);
    let label = format!("{} {:?}", row.brain, row.query);
    let same = |a: &Found, b: &Found| a.file == b.file && a.title == b.title;
    let flags = |f: &Found| (f.file_type.clone(), f.deprecated);
    let mut failures = Vec::new();
    for f in &found {
        match row.kept.iter().find(|k| same(k, f)) {
            None => failures.push(format!("{label}: {f:?} is not an item the filters keep")),
            Some(k) if flags(k) != flags(f) => failures.push(format!(
                "{label}: {f:?} has another type or flag than its item {k:?}"
            )),
            Some(_) => {}
        }
    }
    let mut unclaimed: Vec<&Found> = found.iter().collect();
    for want in &row.found {
        let Some(at) = unclaimed.iter().position(|f| same(f, want)) else {
            if (found.len() as i64) < row.query.limit {
                failures.push(format!("{label}: misses {want:?}"));
            }
            continue;
        };
        let f = unclaimed.remove(at);
        if flags(f) != flags(want) {
            failures.push(format!("{label}: {f:?} differs from {want:?}"));
        }
        if !want
            .matched_fields
            .iter()
            .all(|x| f.matched_fields.contains(x))
        {
            failures.push(format!(
                "{label}: {} fields {:?} lack {:?}",
                f.file, f.matched_fields, want.matched_fields
            ));
        }
    }
    failures
}

/// Every frozen query of `set` on `brain` through `kluris search --json`
/// (exit, stderr, layout and shape checked by [`search_json`]), its
/// results against the frozen row.
fn assert_agrees_with_frozen(sb: &Sandbox, set: &str, brain: &str) {
    let failures: Vec<String> = FROZEN
        .set(set, brain)
        .into_iter()
        .flat_map(|row| disagreements(row, &search_json(sb, brain, &row.query)))
        .collect();
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures[..failures.len().min(40)].join("\n")
    );
}

#[test]
fn search_of_the_fixture_brains_agrees_with_the_frozen_rows() {
    for name in ["wakeup", "full", "malformed"] {
        let sb = Sandbox::new();
        let brain = sb.fixture_brain(name).unwrap();
        sb.register(name, &brain, "");
        assert_agrees_with_frozen(&sb, "fixture", name);
    }
}

#[test]
fn search_of_the_eval_brains_agrees_with_the_frozen_rows() {
    for name in ["acme-sme", "clinica-sme"] {
        let sb = Sandbox::new();
        let brain = sb.eval_brain(name);
        sb.register(name, &brain, "");
        assert_agrees_with_frozen(&sb, "eval", name);
    }
}

#[test]
fn search_with_another_language_agrees_with_the_frozen_rows() {
    // `search: {language: en}` changes the stemming of a Romanian brain,
    // never what the substring match finds.
    let sb = Sandbox::new();
    let brain = sb.eval_brain("clinica-sme");
    write_files(
        &brain,
        &[(
            "kluris.yml",
            "name: clinica-sme\ndescription: ''\ncompanions: []\nsearch:\n  language: en\n",
        )],
    );
    sb.register("clinica-sme", &brain, "");
    assert_agrees_with_frozen(&sb, "language", "clinica-sme");
}
