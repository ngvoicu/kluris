//! `kluris wake-up`'s payload: v2's `kluris_runtime.wake_up.build_payload`
//! (research-03 §1.6, §2.7). Expected values come from the v2.28.11 oracle
//! (`kluris wake-up --json` on copies of the fixture brains);
//! `parity/test_wake_up.py` re-checks them against the live oracle.

use std::fs;
use std::path::{Path, PathBuf};

use kluris_core::brain::deprecation::DeprecationIssue;
use kluris_core::brain::neuron_index::NeuronKind;
use kluris_core::brain::store::{BrainStore, LocalGitStore};
use kluris_core::pyrepr::{py_path_str, py_repr_str, py_str, py_windows_path_str};
use kluris_core::wake_up::{
    BRAIN_MD_MAX_BYTES, LobeSummary, RecentNeuron, is_iso_date_prefix, lobe_description,
    truncate_brain_md, wake_up,
};
use kluris_core::yaml;
use tempfile::TempDir;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/brains")
}

/// Copy a directory tree, iteratively.
fn copy_tree(src: &Path, dst: &Path) {
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        fs::create_dir_all(&to).unwrap();
        for entry in fs::read_dir(&from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                stack.push((entry.path(), target));
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

struct Brain {
    _tmp: TempDir,
    root: PathBuf,
}

fn brain(name: &str) -> Brain {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join(name);
    copy_tree(&fixtures().join(name), &root);
    copy_tree(&fixtures().join("_outside"), &tmp.path().join("outside"));
    Brain { _tmp: tmp, root }
}

fn lobe(name: &str, description: &str, neurons: usize, yaml_count: usize) -> LobeSummary {
    LobeSummary {
        name: name.into(),
        description: description.into(),
        neurons,
        yaml_count,
    }
}

fn recent(path: &str, updated: &str, kind: NeuronKind) -> RecentNeuron {
    RecentNeuron {
        path: path.into(),
        updated: updated.into(),
        kind,
    }
}

#[test]
fn wakeup_fixture_payload_matches_v2() {
    let b = brain("wakeup");
    let store = LocalGitStore::open(&b.root);
    let w = wake_up(&store, "wakeup", "/stored/path", "Fixture");
    assert_eq!(w.name, "wakeup");
    // Python's `str(Path(...))` of the stored path.
    let stored = if cfg!(windows) {
        "\\stored\\path"
    } else {
        "/stored/path"
    };
    assert_eq!(w.path, stored);
    assert_eq!(w.description, "Fixture");
    assert_eq!(
        w.brain_md,
        "# wakeup\n\nFixture brain for `kluris wake-up`.\n\n## Lobes\n\n- [alpha/](./alpha/map.md)\n- [beta/](./beta/map.md)"
    );
    // `sorted(Path)`: code points on POSIX, `str.lower()` on Windows.
    let mut lobes = vec![
        lobe("Zebra", "2026-05-05", 1, 0),
        lobe("alpha", "The alpha lobe.", 7, 2),
        lobe("beta", "The beta lobe, from the body.", 9, 0),
        lobe("desc-types", "Indented description line.", 0, 0),
        lobe("empty-lobe", "", 0, 0),
        lobe("legacy", "From the raw text.", 1, 0),
    ];
    if cfg!(windows) {
        let zebra = lobes.remove(0);
        lobes.push(zebra);
    }
    assert_eq!(w.lobes, lobes);
    assert_eq!(w.total_neurons, 18);
    assert_eq!(w.total_yaml_neurons, 2);
    assert_eq!(
        w.recent,
        [
            recent("beta/b2.md", "20260105", NeuronKind::Markdown),
            recent("Zebra/z.md", "2026-W10-3", NeuronKind::Markdown),
            recent("alpha/spec.yml", "2026-04-01", NeuronKind::Yaml),
            recent("root-note.md", "2026-03-01", NeuronKind::Markdown),
            recent("alpha/a3.md", "2026-02-01T10:00:00", NeuronKind::Markdown),
        ]
    );
    let glossary: Vec<(&str, &str)> = w
        .glossary
        .iter()
        .map(|e| (e.term.as_str(), e.definition.as_str()))
        .collect();
    assert_eq!(
        glossary,
        [
            ("Neuron", "one knowledge file"),
            ("Lobe", "a folder of neurons"),
            ("Brain", "the git repo"),
            ("Synapse", "a link between neurons"),
        ]
    );
    assert_eq!(
        w.deprecation,
        [
            DeprecationIssue::ReplacedByNotActive {
                file: "beta/old.md".into(),
                target: "older.md".into()
            },
            DeprecationIssue::DeprecatedWithoutReplacement {
                file: "beta/older.md".into()
            },
            DeprecationIssue::ActiveLinksToDeprecated {
                source: "beta/b7.md".into(),
                target: "beta/old.md".into()
            },
        ]
    );
    assert_eq!(w.parse_errors, 0);
}

#[test]
fn recent_ties_break_on_path_descending() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join("l")).unwrap();
    for (rel, updated) in [
        ("l/a.md", "2026-01-01"),
        ("l/c.md", "2026-01-01"),
        ("l/b.md", "2026-01-01"),
        ("z.md", "2026-01-01"),
        ("l/odd.md", "zzz"),
        ("l/odd2.md", "aaa"),
        ("l/later.md", "2026-01-01T00:00:01"),
    ] {
        fs::write(
            root.join(rel),
            format!("---\nupdated: \"{updated}\"\n---\n"),
        )
        .unwrap();
    }
    let w = wake_up(&LocalGitStore::open(root), "t", "t", "");
    let paths: Vec<&str> = w.recent.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["l/later.md", "z.md", "l/c.md", "l/b.md", "l/a.md"]);
    // Non-ISO values sort below every ISO one.
    fs::remove_file(root.join("l/later.md")).unwrap();
    for rel in ["l/a.md", "l/b.md", "l/c.md"] {
        fs::remove_file(root.join(rel)).unwrap();
    }
    let w = wake_up(&LocalGitStore::open(root), "t", "t", "");
    let paths: Vec<&str> = w.recent.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["z.md", "l/odd.md", "l/odd2.md"]);
}

#[test]
fn lobe_counts_walk_from_the_lobe() {
    // A lobe's own README.md, map.md and dot-dirs do not count; root
    // neurons are in `recent` but in no lobe; a top-level dot-dir or
    // `node_modules` is no lobe.
    let b = brain("full");
    let w = wake_up(&LocalGitStore::open(&b.root), "full", "p", "");
    assert_eq!(
        w.lobes,
        [
            lobe("knowledge", "", 29, 2),
            lobe("knowledge-extra", "", 1, 0),
            lobe("projects", "", 9, 5),
        ]
    );
    assert_eq!(w.total_neurons, 39);
    assert_eq!(w.total_yaml_neurons, 7);
    assert_eq!(w.deprecation.len(), 21);
    assert_eq!(w.glossary.len(), 21);
    let paths: Vec<&str> = w.recent.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "projects/api/openapi.yml",
            "projects/api/endpoints.md",
            "root-spec.yml",
            "root-note.md",
            "projects/alpha.md"
        ]
    );
}

#[test]
fn malformed_neurons_are_skipped_and_counted() {
    let b = brain("malformed");
    let store = LocalGitStore::open(&b.root);
    let w = wake_up(&store, "malformed", "p", "");
    let skipped: Vec<&str> = w.skipped.iter().map(|s| s.rel.as_str()).collect();
    assert_eq!(skipped, ["lobe/bad-utf8.md", "lobe/bad-yaml.md"]);
    assert_eq!(w.parse_errors, 2);
    // The glossary does not parse: no entries.
    assert!(w.glossary.is_empty());
    assert!(store.glossary().is_err());
}

#[test]
fn a_symlinked_lobe_counts_from_its_own_root() {
    #[cfg(unix)]
    {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("brain");
        let shared = tmp.path().join("shared");
        fs::create_dir_all(root.join("real")).unwrap();
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("s.md"), "---\nupdated: 2026-01-01\n---\n").unwrap();
        fs::write(root.join("real/r.md"), "---\nupdated: 2026-01-02\n---\n").unwrap();
        std::os::unix::fs::symlink(&shared, root.join("linked")).unwrap();
        // A file link inside `real` that points at another lobe escapes
        // `real` and is not counted there.
        fs::create_dir_all(root.join("other")).unwrap();
        fs::write(root.join("other/o.md"), "---\nupdated: 2026-01-03\n---\n").unwrap();
        std::os::unix::fs::symlink("../other/o.md", root.join("real/to-other.md")).unwrap();
        let w = wake_up(&LocalGitStore::open(&root), "b", "b", "");
        assert_eq!(
            w.lobes,
            [
                lobe("linked", "", 1, 0),
                lobe("other", "", 1, 0),
                lobe("real", "", 1, 0)
            ]
        );
        // `recent` walks the brain, which never enters the linked lobe, but
        // lists the in-brain file link.
        let paths: Vec<&str> = w.recent.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["real/to-other.md", "other/o.md", "real/r.md"]);
    }
}

#[test]
fn no_brain_md_or_glossary() {
    let tmp = TempDir::new().unwrap();
    let w = wake_up(&LocalGitStore::open(tmp.path()), "", "/x/name/", "");
    assert_eq!(w.brain_md, "");
    assert!(w.glossary.is_empty());
    assert!(w.lobes.is_empty());
    assert!(w.recent.is_empty());
    // No registered name: the path's last part.
    assert_eq!(w.name, "name");
}

#[test]
fn brain_md_is_stripped_with_universal_newlines() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join("brain.md"),
        "---\r\nupdated: 2026-01-01\r\n---\r\n\r\n  # T\r\n\rbody\u{2028}\r\n\u{a0}\n",
    )
    .unwrap();
    let w = wake_up(&LocalGitStore::open(tmp.path()), "t", "t", "");
    assert_eq!(w.brain_md, "# T\n\nbody");
    // brain.md frontmatter that does not parse: empty.
    fs::write(tmp.path().join("brain.md"), "---\na: [\n---\nbody\n").unwrap();
    let w = wake_up(&LocalGitStore::open(tmp.path()), "t", "t", "");
    assert_eq!(w.brain_md, "");
}

#[test]
fn brain_md_truncation() {
    assert_eq!(BRAIN_MD_MAX_BYTES, 4000);
    let exact = "a".repeat(4000);
    assert_eq!(truncate_brain_md(&exact), exact);
    let over = "a".repeat(4001);
    assert_eq!(
        truncate_brain_md(&over),
        format!("{}\n\n[... truncated]", "a".repeat(4000))
    );
    // A character cut at byte 4000 is dropped.
    let text = format!("{}\u{65e5}tail", "a".repeat(3998));
    assert_eq!(
        truncate_brain_md(&text),
        format!("{}\n\n[... truncated]", "a".repeat(3998))
    );
}

#[test]
fn lobe_description_rules() {
    let md = |text: &str| -> String {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("l")).unwrap();
        fs::write(tmp.path().join("l/map.md"), text).unwrap();
        lobe_description(&LocalGitStore::open(tmp.path()), "l")
    };
    assert_eq!(md("---\ndescription: ' x '\n---\n# T\nbody\n"), "x");
    assert_eq!(md("---\ndescription: '  '\n---\n# T\nbody\n"), "body");
    assert_eq!(md("---\ndescription: 2026-01-02\n---\n"), "2026-01-02");
    assert_eq!(
        md("---\ndescription: 2026-01-02 10:00:00\n---\n"),
        "2026-01-02T10:00:00"
    );
    assert_eq!(md("---\ndescription: [a]\n---\n# T\nbody\n"), "body");
    assert_eq!(md("no title\n# T\n"), "");
    assert_eq!(
        md("before\n# T\n\nup x\nsideways y\n## H\n- [a](a)\n- b\n"),
        "- b"
    );
    assert_eq!(md("#T\n# T\n#\nx\n"), "#");
    assert_eq!(md("  # T  \n\u{2028}  line\u{85}more\n"), "line");
    // The raw text when the frontmatter does not parse.
    assert_eq!(md("---\nx: [\n---\n# T\nraw\n"), "raw");
    assert_eq!(md("---\n# T\nx: [\n---\nnext\n"), "x: [");
    assert_eq!(md("# T\n\u{0}"), "\u{0}");
    // Not UTF-8 (v2 raises): empty.
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join("l")).unwrap();
    fs::write(tmp.path().join("l/map.md"), b"# T\n\xff\n").unwrap();
    assert_eq!(lobe_description(&LocalGitStore::open(tmp.path()), "l"), "");
    // No map.md.
    assert_eq!(
        lobe_description(&LocalGitStore::open(tmp.path()), "nope"),
        ""
    );
}

// --- Python semantics ------------------------------------------------------------

#[test]
fn iso_date_prefix_is_python_3_11_date_fromisoformat_of_the_first_10_chars() {
    let valid = [
        "2026-01-01",
        "20260101xx",
        "2026W011xx",
        "2026-W01-1",
        "2026W01",
        "2026-W01",
        "20260101",
        "2026-W53-1",
        "2020-W53-7",
        "2026-01-01T10:00:00",
        "2026-01-01 anything",
        "0001-01-01",
        "9999-12-31",
        "2024-02-29",
        // Only the first 10 characters are read.
        "2026-01-01T",
    ];
    let invalid = [
        "2026-01",
        "2026-02-30",
        "2026-W54-1",
        "2026-W01-8",
        "0000-01-01",
        "2026\u{2212}01-01",
        "\u{662}\u{660}\u{662}\u{666}-01-01",
        "2026-1-01x",
        "2026-01-0x",
        "2026-W00-1",
        "2026W0110",
        "2026-W011x",
        "2026-W01x1",
        "2025-W53-1",
        "2023-02-29",
        "2026-13-01",
        "",
        "yesterday",
        "True",
        "1.5",
        "2026-01-0\u{e9}",
        "202601\u{e9}",
        "[datetime.date(2026, 1, 1)]",
    ];
    for text in valid {
        assert!(is_iso_date_prefix(text), "{text:?} should be ISO");
    }
    for text in invalid {
        assert!(!is_iso_date_prefix(text), "{text:?} should not be ISO");
    }
}

fn str_of(yaml_value: &str) -> String {
    let doc = yaml::load(&format!("v: {yaml_value}")).unwrap();
    let map = doc.into_map().unwrap();
    py_str(&map["v"])
}

#[test]
fn py_str_matches_python_str_after_normalization() {
    let cases: &[(&str, &str)] = &[
        ("[2026-01-01, x]", "[datetime.date(2026, 1, 1), 'x']"),
        ("true", "True"),
        ("1.5", "1.5"),
        (
            "{a: 1, 'b''c': [null, 2026-01-01 10:00:00]}",
            "{'a': 1, \"b'c\": [None, datetime.datetime(2026, 1, 1, 10, 0)]}",
        ),
        (
            "[2026-01-01 10:00:00+01:00, 2026-01-01 10:00:00-05:30, 2026-01-01 10:00:00Z, 2026-01-01 10:00:00.5]",
            "[datetime.datetime(2026, 1, 1, 10, 0, tzinfo=datetime.timezone(datetime.timedelta(seconds=3600))), datetime.datetime(2026, 1, 1, 10, 0, tzinfo=datetime.timezone(datetime.timedelta(days=-1, seconds=66600))), datetime.datetime(2026, 1, 1, 10, 0, tzinfo=datetime.timezone.utc), datetime.datetime(2026, 1, 1, 10, 0, 0, 500000)]",
        ),
        ("\"it's\"", "it's"),
        (
            "[\"it's\", 'say \"hi\"', \"both ' \\\"\", \"tab\\there\", \"\\x7f\\x85\\xa0\\u2028\\U0001F600\\u00e9\\u0378\"]",
            "[\"it's\", 'say \"hi\"', 'both \\' \"', 'tab\\there', '\\x7f\\x85\\xa0\\u2028\u{1F600}\u{e9}\\u0378']",
        ),
        ("1e3", "1e3"),
        ("-0.0", "-0.0"),
        (".inf", "inf"),
        ("-.inf", "-inf"),
        (".nan", "nan"),
        ("0x1F", "31"),
        ("1_000", "1000"),
        (
            "123456789012345678901234567890",
            "123456789012345678901234567890",
        ),
        ("[]", "[]"),
        ("{}", "{}"),
        ("[[]]", "[[]]"),
        ("2026-01-01 10:00:05.000123", "2026-01-01T10:00:05.000123"),
        ("2026-01-01 10:00:00.000100", "2026-01-01T10:00:00.000100"),
        ("1e300", "1e300"),
        ("1.0e-5", "1e-05"),
        ("{x: {y: true}}", "{'x': {'y': True}}"),
        ("[yes, No, off]", "[True, False, False]"),
        ("2026-01-01T10:00:00+00:30", "2026-01-01T10:00:00+00:30"),
        ("''", ""),
        ("2026-01-01", "2026-01-01"),
        (
            "[2026-01-01 10:00:05]",
            "[datetime.datetime(2026, 1, 1, 10, 0, 5)]",
        ),
        (
            "[2026-01-01 10:00:00.000001]",
            "[datetime.datetime(2026, 1, 1, 10, 0, 0, 1)]",
        ),
    ];
    for (yaml_value, expected) in cases {
        assert_eq!(&str_of(yaml_value), expected, "value {yaml_value:?}");
    }
}

#[test]
fn py_repr_str_quotes_and_escapes_like_python() {
    let cases: &[(&str, &str)] = &[
        ("abc", "'abc'"),
        ("it's", "\"it's\""),
        ("a'\"b", "'a\\'\"b'"),
        ("\\", "'\\\\'"),
        ("\t\n\r", "'\\t\\n\\r'"),
        ("\u{0}\u{1f}\u{7f}", "'\\x00\\x1f\\x7f'"),
        ("\u{a0}\u{ad}", "'\\xa0\\xad'"),
        ("\u{2028}\u{feff}", "'\\u2028\\ufeff'"),
        ("\u{e000}", "'\\ue000'"),
        ("\u{1f600}", "'\u{1f600}'"),
        ("\u{e0001}", "'\\U000e0001'"),
        ("\u{e01ef}", "'\u{e01ef}'"),
        ("é日本", "'é日本'"),
        (" ", "' '"),
        ("\u{3000}", "'\\u3000'"),
    ];
    for (text, expected) in cases {
        assert_eq!(&py_repr_str(text), expected, "text {text:?}");
    }
}

#[test]
fn py_path_str_is_str_of_python_path() {
    if cfg!(windows) {
        let cases: &[(&str, &str)] = &[
            ("C:\\a\\b", "C:\\a\\b"),
            ("C:/a/b/", "C:\\a\\b"),
            ("C:\\a\\.\\b\\\\c", "C:\\a\\b\\c"),
            ("C:\\a\\..\\b", "C:\\a\\..\\b"),
            ("", "."),
            (".", "."),
            ("a/./b/", "a\\b"),
            ("C:\\", "C:\\"),
            ("\\\\server\\share\\x\\", "\\\\server\\share\\x"),
        ];
        for (raw, expected) in cases {
            assert_eq!(&py_path_str(raw), expected, "raw {raw:?}");
        }
    } else {
        let cases: &[(&str, &str)] = &[
            ("/a/b", "/a/b"),
            ("/a/b/", "/a/b"),
            ("/a//b/./c/", "/a/b/c"),
            ("/a/../b", "/a/../b"),
            ("//a", "//a"),
            ("///a", "/a"),
            ("", "."),
            (".", "."),
            ("./a", "a"),
            ("a/.", "a"),
            ("/", "/"),
            ("C:\\a\\b", "C:\\a\\b"),
        ];
        for (raw, expected) in cases {
            assert_eq!(&py_path_str(raw), expected, "raw {raw:?}");
        }
    }
}

#[test]
fn py_windows_path_str_is_str_of_pure_windows_path() {
    // Rows from Python 3.11's `str(PureWindowsPath(raw))`.
    let cases: &[(&str, &str)] = &[
        ("C:\\a\\b", "C:\\a\\b"),
        ("C:/a/b/", "C:\\a\\b"),
        ("C:\\a\\.\\b\\\\c", "C:\\a\\b\\c"),
        ("C:\\a\\..\\b", "C:\\a\\..\\b"),
        ("", "."),
        (".", "."),
        ("a/./b/", "a\\b"),
        ("C:\\", "C:\\"),
        ("\\\\server\\share\\x\\", "\\\\server\\share\\x"),
        ("\\\\server\\share", "\\\\server\\share\\"),
        ("//server/share/", "\\\\server\\share\\"),
        ("\\\\server", "\\server"),
        ("\\\\server\\\\x", "\\server\\x"),
        ("C:", "C:"),
        ("C:a/b", "C:a\\b"),
        ("\\\\\\a", "\\a"),
        ("/x/y/", "\\x\\y"),
    ];
    for (raw, expected) in cases {
        assert_eq!(&py_windows_path_str(raw), expected, "raw {raw:?}");
    }
}
