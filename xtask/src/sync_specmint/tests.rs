//! v2.28.11's `tests/test_sync_specmint.py`, ported, plus the Python texts
//! the port must reproduce (generated with v2's script under the oracle's
//! Python 3.11) and a real `--check` against the Specmint repos beside the
//! workspace.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::*;

const SKILL: &str = "---
name: specmint-core
---

# Spec Mint Core

See `references/spec-format.md`.

## Cross-Tool Compatibility

- **Claude Code**: Skill via `npx skills add`

## Behavioral Notes

Be nice.
";

const HTML_STEP: &str = "4. Copy assets. AI tools
   should know the skill install location; for skill installs it
   is typically the installed skill's assets/ directory (e.g.
   `~/.claude/skills/specmint-core-html/assets/`). These
";

/// v2 `_compat_section("specmint-core")`.
const CORE_COMPAT: &str = "\n## Kluris Companion Compatibility\n\nKluris embeds this playbook as the `specmint-core` companion. Read and\nfollow it directly from `~/.kluris/companions/specmint-core/SKILL.md`.\nPaths in this file that start with `references/` resolve against\n`~/.kluris/companions/specmint-core/`. `.specs/...` paths are project\npaths. Do not ask the user to install specmint separately. Specs remain\nplain markdown/YAML files that humans can edit and Git can diff.\n";

/// v2 `_compat_section("specmint-core-html")`.
const CORE_HTML_COMPAT: &str = "\n## Kluris Companion Compatibility\n\nKluris embeds this playbook as the `specmint-core-html` companion. Read and\nfollow it directly from `~/.kluris/companions/specmint-core-html/SKILL.md`.\nPaths in this file that start with `references/` or `assets/` resolve\nagainst `~/.kluris/companions/specmint-core-html/`. `.specs/...` paths, and\nthe relative `../assets/` links inside a `SPEC.html`, are project paths. Do\nnot ask the user to install specmint separately. `SPEC.html` stays plain\nHTML with a JSON metadata blob: open it in any browser, edit it in any text\neditor, and keep it diff-friendly with the conventions in\n`references/spec-format.md`.\n";

fn source(tmp: &Path, skill: &str, files: &[&str]) -> PathBuf {
    let src = tmp.join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("SKILL.md"), skill).unwrap();
    for rel in files {
        let path = src.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }
    src
}

fn tree_of(pairs: &[(&str, &[u8])]) -> Tree {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_vec()))
        .collect()
}

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

// --- the sanitizer ------------------------------------------------------------

#[test]
fn compat_sections_are_v2s_text() {
    assert_eq!(compat_section("specmint-core"), CORE_COMPAT);
    assert_eq!(compat_section("specmint-core-html"), CORE_HTML_COMPAT);
    for name in NAMES {
        for line in compat_section(name).lines() {
            assert!(line.chars().count() <= 75, "{name}: {line}");
        }
    }
}

#[test]
fn sanitize_replaces_compat_section() {
    let out = sanitize_for_kluris("specmint-core", SKILL).unwrap();

    assert!(!out.contains("npx skills"));
    assert!(!out.contains("## Cross-Tool Compatibility"));
    assert!(out.contains("~/.kluris/companions/specmint-core/SKILL.md"));
    assert!(out.ends_with("## Behavioral Notes\n\nBe nice.\n"));
    let expected = format!(
        "---\nname: specmint-core\n---\n\n# Spec Mint Core\n\nSee `references/spec-format.md`.\n{CORE_COMPAT}\n## Behavioral Notes\n\nBe nice.\n"
    );
    assert_eq!(out, expected);
}

#[test]
fn sanitize_fails_loudly_when_heading_is_renamed() {
    let renamed = SKILL.replace("## Cross-Tool Compatibility", "## Tool Compatibility");
    let err = sanitize_for_kluris("specmint-core", &renamed).unwrap_err();
    assert_eq!(
        err,
        "specmint-core: sanitizer rule 'Cross-Tool Compatibility section' matched 0 times \
         (expected 1). Specmint's SKILL.md changed; review the rule."
    );
}

#[test]
fn sanitize_fails_when_the_end_heading_is_missing() {
    let renamed = SKILL.replace("## Behavioral Notes", "## Notes");
    let err = sanitize_for_kluris("specmint-core", &renamed).unwrap_err();
    assert!(err.contains("matched 0 times"), "{err}");
}

#[test]
fn sanitize_fails_when_the_section_appears_twice() {
    let twice = format!("{SKILL}{}", &SKILL[SKILL.find("\n## Cross").unwrap()..]);
    let err = sanitize_for_kluris("specmint-core", &twice).unwrap_err();
    assert!(err.contains("matched 2 times"), "{err}");
}

#[test]
fn sanitize_rewrites_html_asset_location() {
    let skill = SKILL.replace("Be nice.\n", HTML_STEP);
    let out = sanitize_for_kluris("specmint-core-html", &skill).unwrap();

    assert!(!out.contains("~/.claude/skills"));
    assert!(out.ends_with(
        "4. Copy assets. Under Kluris that directory is\n   `~/.kluris/companions/specmint-core-html/assets/`. These\n"
    ));
}

#[test]
fn sanitize_rewrites_the_for_skill_installs_form() {
    let step = "2. For skill installs it is typically the installed skill's assets/ directory (e.g. `~/.claude/skills/specmint-tdd-html/assets/`).\n";
    let skill = SKILL.replace("Be nice.\n", step);
    let out = sanitize_for_kluris("specmint-tdd-html", &skill).unwrap();
    assert!(out.ends_with(
        "2. Under Kluris that directory is\n   `~/.kluris/companions/specmint-tdd-html/assets/`.\n"
    ));
}

#[test]
fn sanitize_fails_when_html_asset_sentence_is_missing() {
    let err = sanitize_for_kluris("specmint-core-html", SKILL).unwrap_err();
    assert!(err.contains("asset location"), "{err}");
    assert!(err.contains("matched 0 times"), "{err}");
}

#[test]
fn non_html_companions_skip_the_asset_rule() {
    let skill = SKILL.replace("Be nice.\n", HTML_STEP);
    let out = sanitize_for_kluris("specmint-tdd", &skill).unwrap();
    assert!(out.contains("~/.claude/skills/specmint-core-html/assets/"));
}

// --- path mentions --------------------------------------------------------------

#[test]
fn path_mentions_match_v2s_regex() {
    // `_PATH_MENTION.findall(text)` under Python 3.11.
    let cases: [(&str, &[&str]); 13] = [
        ("see references/a.md.", &["references/a.md."]),
        ("x references/a.md", &["references/a.md"]),
        ("./references/a.md", &[]),
        ("xreferences/a.md", &[]),
        ("foo*assets/x.js", &["assets/x.js"]),
        ("references/assets/x", &["references/assets"]),
        ("`references/*.md`", &["references/*.md"]),
        (".specs/assets/x", &[]),
        ("-assets/y", &[]),
        ("(assets/a-b_c.css)", &["assets/a-b_c.css"]),
        ("éreferences/a", &[]),
        ("references/", &[]),
        ("assets/a/b", &["assets/a"]),
    ];
    for (text, expected) in cases {
        assert_eq!(path_mentions(text), expected, "{text:?}");
    }
}

#[test]
fn missing_mentions_checks_files_and_globs() {
    let tree = tree_of(&[
        ("SKILL.md", b"see references/a.md. and `references/*.md` and assets/*.js"),
        ("references/a.md", b"see references/b.md"),
        ("assets/x.css", b"references/nope.md is not markdown"),
    ]);
    assert_eq!(
        missing_mentions(&tree).unwrap(),
        ["SKILL.md: assets/*.js", "references/a.md: references/b.md"]
    );
}

// --- building a companion ---------------------------------------------------------

#[test]
fn build_copies_allowlist_only() {
    let tmp = TempDir::new().unwrap();
    let src = source(
        tmp.path(),
        SKILL,
        &[
            "references/spec-format.md",
            "README.md",
            "evals/evals.json",
            ".idea/x.xml",
            "references/.DS_Store",
            "references/__pycache__/x.pyc",
        ],
    );
    let mut out = Vec::new();
    let tree = build("specmint-core", &src, &mut out).unwrap();

    assert_eq!(
        tree.keys().collect::<Vec<_>>(),
        ["SKILL.md", "references/spec-format.md"]
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "specmint-core: not shipped: .idea, README.md, evals\n"
    );
}

#[test]
fn build_fails_without_skill_md() {
    let tmp = TempDir::new().unwrap();
    let err = build("specmint-core", tmp.path(), &mut Vec::new()).unwrap_err();
    assert!(err.starts_with("specmint-core: missing SKILL.md at "), "{err}");
}

#[test]
fn build_fails_when_a_mentioned_file_is_not_shipped() {
    let tmp = TempDir::new().unwrap();
    let src = source(tmp.path(), SKILL, &[]);
    let err = build("specmint-core", &src, &mut Vec::new()).unwrap_err();
    assert_eq!(
        err,
        "specmint-core: mentioned files are not shipped: SKILL.md: references/spec-format.md"
    );
}

#[test]
fn build_accepts_plugin_text_the_sanitizer_removes() {
    let tmp = TempDir::new().unwrap();
    let src = source(tmp.path(), SKILL, &["references/spec-format.md"]);
    let tree = build("specmint-core", &src, &mut Vec::new()).unwrap();
    let skill = String::from_utf8(tree["SKILL.md"].clone()).unwrap();
    assert!(!skill.contains("npx skills"));
}

#[test]
fn build_fails_when_plugin_text_survives_sanitizing() {
    let tmp = TempDir::new().unwrap();
    let skill = SKILL.replace("Be nice.", "Run `npx skills add ngvoicu/specmint-core`.");
    let src = source(tmp.path(), &skill, &["references/spec-format.md"]);
    let err = build("specmint-core", &src, &mut Vec::new()).unwrap_err();
    assert_eq!(
        err,
        "specmint-core: sanitized SKILL.md still mentions forbidden paths: npx skills"
    );
}

#[test]
fn build_refuses_links() {
    let tmp = TempDir::new().unwrap();
    let src = source(tmp.path(), SKILL, &["references/spec-format.md"]);
    let outside = tmp.path().join("outside.md");
    fs::write(&outside, "x").unwrap();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, src.join("references/leak.md")).is_ok();
    #[cfg(windows)]
    let linked =
        std::os::windows::fs::symlink_file(&outside, src.join("references/leak.md")).is_ok();
    if !linked {
        eprintln!("symlinks unavailable; skipped");
        return;
    }
    let err = build("specmint-core", &src, &mut Vec::new()).unwrap_err();
    assert!(err.starts_with("specmint-core: refusing symlink "), "{err}");
}

#[test]
fn build_fails_on_skill_md_that_is_not_utf8() {
    let tmp = TempDir::new().unwrap();
    let src = source(tmp.path(), SKILL, &["references/spec-format.md"]);
    fs::write(src.join("SKILL.md"), b"\xff\xfe").unwrap();
    assert!(build("specmint-core", &src, &mut Vec::new()).is_err());
}

// --- the report ---------------------------------------------------------------------

#[test]
fn report_shows_per_file_changes() {
    let mut out = Vec::new();
    let changed = report(
        "specmint-core",
        &tree_of(&[("SKILL.md", b"a\n"), ("old.md", b"x\n")]),
        &tree_of(&[("SKILL.md", b"b\n"), ("references/new.md", b"y\n")]),
        false,
        &mut out,
    );
    let out = String::from_utf8(out).unwrap();
    assert!(changed);
    assert!(out.contains("changed  SKILL.md (+1 -1 lines"), "{out}");
    assert!(out.contains("added    references/new.md"), "{out}");
    assert!(out.contains("removed  old.md"), "{out}");

    let mut out = Vec::new();
    let same = tree_of(&[("a", b"1")]);
    assert!(!report("specmint-core", &same, &same, false, &mut out));
    assert_eq!(out, b"  in sync\n");
}

#[test]
fn report_with_full_diff_is_v2s_text() {
    let mut out = Vec::new();
    report(
        "specmint-core",
        &tree_of(&[("SKILL.md", b"a\nb\nc\nd\ne\nf\ng\nh\n"), ("old.md", b"x\n")]),
        &tree_of(&[
            ("SKILL.md", b"a\nB\nc\nd\ne\nf\ng\nh\ni"),
            ("references/new.md", b"y\nz\n"),
        ]),
        true,
        &mut out,
    );
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "  changed  SKILL.md (+2 -1 lines; 16 -> 17 bytes)\n--- old/specmint-core/SKILL.md\n+++ new/specmint-core/SKILL.md\n@@ -1,8 +1,9 @@\n a\n-b\n+B\n c\n d\n e\n f\n g\n h\n+i  removed  old.md (2 bytes, 1 lines)\n  added    references/new.md (4 bytes, 2 lines)\n"
    );
}

#[test]
fn line_counts_follow_python_splitlines() {
    assert_eq!(line_count(b""), 0);
    assert_eq!(line_count(b"a"), 1);
    assert_eq!(line_count(b"a\n"), 1);
    assert_eq!(line_count(b"a\r\nb\rc\x0bd\x0ce\x1cf\x1dg\x1eh"), 8);
    assert_eq!(line_count("a\u{85}b\u{2028}c\u{2029}d".as_bytes()), 4);
    assert_eq!(line_count(b"a\n\n"), 2);
}

// --- the whole task -------------------------------------------------------------------

fn four_sources(tmp: &Path) -> Vec<String> {
    let html_skill = SKILL.replace("Be nice.\n", HTML_STEP);
    let mut args = Vec::new();
    for (name, flag) in NAMES.iter().zip(["--core", "--tdd", "--core-html", "--tdd-html"]) {
        let src = tmp.join("sources").join(name);
        fs::create_dir_all(src.join("references")).unwrap();
        let is_html = name.ends_with("-html");
        let skill = if is_html {
            html_skill.replace("specmint-core-html", name)
        } else {
            SKILL.to_string()
        };
        fs::write(src.join("SKILL.md"), skill).unwrap();
        fs::write(src.join("references/spec-format.md"), "format\n").unwrap();
        fs::write(src.join("README.md"), "not shipped\n").unwrap();
        if is_html {
            fs::create_dir(src.join("assets")).unwrap();
            fs::write(src.join("assets/spec-runtime.js"), "// js\n").unwrap();
        }
        args.push(flag.to_string());
        args.push(src.to_string_lossy().into_owned());
    }
    args
}

fn sorted_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn main_writes_then_second_run_is_a_noop() {
    let tmp = TempDir::new().unwrap();
    let mut args = four_sources(tmp.path());
    let dest = tmp.path().join("vendored");
    fs::create_dir(&dest).unwrap();
    args.extend(["--dest".to_string(), dest.to_string_lossy().into_owned()]);
    let with = |extra: &[&str]| {
        let mut a = args.clone();
        a.extend(extra.iter().map(|s| s.to_string()));
        a
    };

    let mut out = Vec::new();
    assert_eq!(run(&with(&["--check"]), &workspace(), &mut out), Ok(1));
    assert!(sorted_names(&dest).is_empty(), "--check writes nothing");

    let mut out = Vec::new();
    assert_eq!(run(&with(&[]), &workspace(), &mut out), Ok(0));
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("synced specmint-tdd-html -> "), "{out}");
    let mut expected: Vec<String> = NAMES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(sorted_names(&dest), expected);
    let html = dest.join("specmint-tdd-html");
    assert_eq!(
        read_tree(&html).keys().collect::<Vec<_>>(),
        ["SKILL.md", "assets/spec-runtime.js", "references/spec-format.md"]
    );
    assert!(
        fs::read_to_string(html.join("SKILL.md"))
            .unwrap()
            .contains("~/.kluris/companions/specmint-tdd-html/assets/")
    );

    let mut out = Vec::new();
    assert_eq!(run(&with(&["--check"]), &workspace(), &mut out), Ok(0));
    assert_eq!(String::from_utf8(out).unwrap().matches("in sync").count(), 4);
    let mut out = Vec::new();
    assert_eq!(run(&with(&[]), &workspace(), &mut out), Ok(0));
    assert!(!String::from_utf8(out).unwrap().contains("synced"));
}

#[test]
fn nothing_is_written_when_one_companion_fails() {
    let tmp = TempDir::new().unwrap();
    let mut args = four_sources(tmp.path());
    // The last companion's heading was renamed upstream.
    let last = PathBuf::from(args.last().unwrap()).join("SKILL.md");
    let text = fs::read_to_string(&last).unwrap();
    fs::write(&last, text.replace("## Cross-Tool", "## Tool")).unwrap();
    let dest = tmp.path().join("vendored");
    fs::create_dir(&dest).unwrap();
    args.extend(["--dest".to_string(), dest.to_string_lossy().into_owned()]);

    let err = run(&args, &workspace(), &mut Vec::new()).unwrap_err();
    assert!(err.starts_with("specmint-tdd-html: sanitizer rule"), "{err}");
    assert!(sorted_names(&dest).is_empty());
}

#[test]
fn unknown_arguments_are_usage_errors() {
    assert!(run(&["--bogus".to_string()], &workspace(), &mut Vec::new()).is_err());
    assert!(run(&["--core".to_string()], &workspace(), &mut Vec::new()).is_err());
}

#[test]
fn vendored_companions_pass_the_sync_checks() {
    for name in NAMES {
        let tree = read_tree(&workspace().join("vendored").join(name));
        let skill = String::from_utf8(tree["SKILL.md"].clone()).unwrap();
        assert!(forbidden_hits(&skill).is_empty(), "{name}");
        assert!(missing_mentions(&tree).unwrap().is_empty(), "{name}");
        assert!(skill.contains(&compat_section(name)), "{name}");
        assert!(skill.contains(&format!("~/.kluris/companions/{name}/SKILL.md")));
        assert!(!skill.contains("## Cross-Tool Compatibility"));
        if name.ends_with("-html") {
            assert!(skill.contains(&format!("~/.kluris/companions/{name}/assets/")));
        }
    }
}

/// `cargo xtask sync-specmint --check` against today's Specmint (v3.0.1)
/// beside the workspace: the vendored copy is in sync. Skipped (with a
/// message) where the Specmint repos are not checked out.
#[test]
fn real_check_against_specmint_is_in_sync() {
    let specmint = workspace().parent().unwrap().join("specmint");
    if !NAMES
        .iter()
        .all(|n| specmint.join(n).join("SKILL.md").is_file())
    {
        eprintln!(
            "skipped: no Specmint checkout at {}",
            specmint.display()
        );
        return;
    }
    let mut out = Vec::new();
    let code = run(&["--check".to_string()], &workspace(), &mut out).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.matches("  in sync\n").count(), 4, "{out}");
}
