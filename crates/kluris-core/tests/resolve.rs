//! The brain resolver: v2's `_resolve_brains`, `_check_brain_paths` and
//! `_pick_brain_interactively` (research-03 §1.0, `cli.py:268-395`), with
//! v2's exact texts. `parity/test_resolver.py` checks the same cases against
//! the v2.28.11 oracle through the binaries.

use std::fs;
use std::io::Cursor;
use std::path::Path;

use indexmap::IndexMap;
use kluris_core::config::{BrainEntry, GlobalConfig};
use kluris_core::resolve::{
    Choice, Picker, Request, Resolution, ResolveError, ResolvedBrain, check_brain_paths, resolve,
};
use tempfile::TempDir;

/// Brains `(name, dir)` under a temp root: a dir that is `None` is a path
/// that does not exist; `Some("<file>")` is a regular file.
struct Setup {
    _tmp: TempDir,
    config: GlobalConfig,
}

fn setup(brains: &[(&str, Option<&str>)]) -> Setup {
    let tmp = TempDir::new().unwrap();
    let mut map = IndexMap::new();
    for (name, kind) in brains {
        let path = tmp.path().join(name);
        match kind {
            Some("dir") => fs::create_dir(&path).unwrap(),
            Some("file") => fs::write(&path, "x").unwrap(),
            _ => {}
        }
        map.insert(
            (*name).to_string(),
            BrainEntry {
                path: path.to_string_lossy().into_owned(),
                description: String::new(),
            },
        );
    }
    Setup {
        _tmp: tmp,
        config: GlobalConfig { brains: map },
    }
}

fn dirs(names: &[&'static str]) -> Vec<(&'static str, Option<&'static str>)> {
    names.iter().map(|n| (*n, Some("dir"))).collect()
}

fn request(brain: Option<&str>, allow_all: bool, non_interactive: bool) -> Request<'_> {
    Request {
        brain,
        allow_all,
        non_interactive,
    }
}

fn names(brains: &[ResolvedBrain]) -> Vec<&str> {
    brains.iter().map(|b| b.name.as_str()).collect()
}

fn resolved(s: &Setup, req: Request) -> Vec<String> {
    match resolve(&s.config, &req) {
        Ok(Resolution::Brains(brains)) => names(&brains).iter().map(|n| n.to_string()).collect(),
        Ok(Resolution::Pick(_)) => panic!("expected brains, got the picker"),
        Err(e) => panic!("expected brains, got {e}"),
    }
}

fn error(s: &Setup, req: Request) -> String {
    match resolve(&s.config, &req) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error"),
    }
}

fn picker(s: &Setup, req: Request) -> Picker {
    match resolve(&s.config, &req) {
        Ok(Resolution::Pick(p)) => p,
        Ok(Resolution::Brains(b)) => panic!("expected the picker, got {:?}", names(&b)),
        Err(e) => panic!("expected the picker, got {e}"),
    }
}

const ALL_NOT_SUPPORTED: &str =
    "--brain all is only supported on dream, status, mri, and companion commands.";
const NO_BRAINS: &str = "No brains registered. Run 'kluris create <name>' to create one.";

// --- the cases of `_resolve_brains`, in v2's order ---------------------------

#[test]
fn brain_all_without_allow_all_is_refused_first() {
    // Before the zero-brains check, and with a brain named `all` too.
    for brains in [vec![], dirs(&["a", "b"]), dirs(&["all"])] {
        let s = setup(&brains);
        for non_interactive in [false, true] {
            assert_eq!(
                error(&s, request(Some("all"), false, non_interactive)),
                ALL_NOT_SUPPORTED
            );
        }
    }
}

#[test]
fn brain_all_with_zero_brains() {
    let s = setup(&[]);
    assert_eq!(error(&s, request(Some("all"), true, true)), NO_BRAINS);
}

#[test]
fn brain_all_is_every_brain_in_config_order() {
    let s = setup(&dirs(&["zeta", "alpha", "mid"]));
    assert_eq!(
        resolved(&s, request(Some("all"), true, false)),
        ["zeta", "alpha", "mid"]
    );
    assert_eq!(
        resolved(&s, request(Some("all"), true, true)),
        ["zeta", "alpha", "mid"]
    );
}

#[test]
fn unknown_brain_name() {
    for brains in [vec![], dirs(&["a"]), dirs(&["a", "b"])] {
        let s = setup(&brains);
        for allow_all in [false, true] {
            assert_eq!(
                error(&s, request(Some("nope"), allow_all, true)),
                "No brain named 'nope' is registered. Run 'kluris list' to see available brains."
            );
        }
    }
    // Names are matched exactly.
    let s = setup(&dirs(&["Demo"]));
    assert_eq!(
        error(&s, request(Some("demo"), false, true)),
        "No brain named 'demo' is registered. Run 'kluris list' to see available brains."
    );
}

#[test]
fn named_brain_is_that_brain() {
    let s = setup(&dirs(&["a", "b", "c"]));
    for allow_all in [false, true] {
        for non_interactive in [false, true] {
            assert_eq!(
                resolved(&s, request(Some("b"), allow_all, non_interactive)),
                ["b"]
            );
        }
    }
}

#[test]
fn an_empty_brain_name_is_no_name() {
    // Click's `if brain_name:` treats `--brain ""` as absent.
    let s = setup(&dirs(&["only"]));
    assert_eq!(resolved(&s, request(Some(""), false, true)), ["only"]);
    let s = setup(&[]);
    assert_eq!(error(&s, request(Some(""), true, true)), NO_BRAINS);
    let s = setup(&dirs(&["a", "b"]));
    assert_eq!(
        error(&s, request(Some(""), false, true)),
        "Multiple brains registered. Pass --brain NAME. Available: a, b"
    );
}

#[test]
fn zero_brains() {
    let s = setup(&[]);
    for allow_all in [false, true] {
        for non_interactive in [false, true] {
            assert_eq!(
                error(&s, request(None, allow_all, non_interactive)),
                NO_BRAINS
            );
        }
    }
}

#[test]
fn exactly_one_brain_resolves_without_a_picker() {
    let s = setup(&dirs(&["solo"]));
    for allow_all in [false, true] {
        for non_interactive in [false, true] {
            assert_eq!(
                resolved(&s, request(None, allow_all, non_interactive)),
                ["solo"]
            );
        }
    }
}

#[test]
fn several_brains_non_interactive_list_them() {
    let s = setup(&dirs(&["b", "a", "c"]));
    assert_eq!(
        error(&s, request(None, false, true)),
        "Multiple brains registered. Pass --brain NAME. Available: b, a, c"
    );
    assert_eq!(
        error(&s, request(None, true, true)),
        "Multiple brains registered. Pass --brain NAME or 'all'. Available: b, a, c"
    );
}

#[test]
fn several_brains_interactive_get_the_picker() {
    let s = setup(&dirs(&["a", "b"]));
    let p = picker(&s, request(None, false, false));
    assert_eq!(p.option_count(), 2);
    let p = picker(&s, request(None, true, false));
    assert_eq!(p.option_count(), 3);
}

// --- `_check_brain_paths` ------------------------------------------------------

fn missing_message(rows: &[(&str, String)]) -> String {
    let lines: Vec<String> = rows.iter().map(|(n, p)| format!("  - {n}: {p}")).collect();
    format!(
        "The following registered brain(s) have missing or invalid paths:\n{}\nRun `kluris remove <name>` or fix the path in the global config.",
        lines.join("\n")
    )
}

fn path_of(s: &Setup, name: &str) -> String {
    s.config.brains[name].path.clone()
}

#[test]
fn a_missing_or_file_path_is_refused_after_resolving() {
    let s = setup(&[("gone", None), ("file", Some("file")), ("ok", Some("dir"))]);
    for name in ["gone", "file"] {
        assert_eq!(
            error(&s, request(Some(name), false, true)),
            missing_message(&[(name, path_of(&s, name))])
        );
    }
    // `all` lists every bad brain, in config order.
    assert_eq!(
        error(&s, request(Some("all"), true, true)),
        missing_message(&[("gone", path_of(&s, "gone")), ("file", path_of(&s, "file"))])
    );
    // The only brain.
    let solo = setup(&[("solo", None)]);
    assert_eq!(
        error(&solo, request(None, false, true)),
        missing_message(&[("solo", path_of(&solo, "solo"))])
    );
    // A non-interactive multi-brain error comes before any path check.
    assert_eq!(
        error(&s, request(None, false, true)),
        "Multiple brains registered. Pass --brain NAME. Available: gone, file, ok"
    );
}

#[test]
fn check_brain_paths_keeps_the_stored_path_text() {
    let tmp = TempDir::new().unwrap();
    let raw = format!("{}/./nope/", tmp.path().display());
    let brains = [ResolvedBrain {
        name: "x".into(),
        entry: BrainEntry {
            path: raw.clone(),
            description: String::new(),
        },
    }];
    let err = check_brain_paths(&brains).unwrap_err();
    assert_eq!(err.to_string(), missing_message(&[("x", raw)]));
    // A path that Python's `Path()` normalizes to an existing dir is fine.
    let ok = [ResolvedBrain {
        name: "y".into(),
        entry: BrainEntry {
            path: format!(
                "{}{}.{}",
                tmp.path().display(),
                std::path::MAIN_SEPARATOR,
                std::path::MAIN_SEPARATOR
            ),
            description: String::new(),
        },
    }];
    assert_eq!(check_brain_paths(&ok), Ok(()));
}

#[test]
fn an_empty_stored_path_is_the_current_directory() {
    // Python's `Path("")` is `.`, which exists.
    let brains = [ResolvedBrain {
        name: "dot".into(),
        entry: BrainEntry {
            path: String::new(),
            description: String::new(),
        },
    }];
    assert_eq!(check_brain_paths(&brains), Ok(()));
}

// --- the picker ----------------------------------------------------------------

#[test]
fn picker_menu_text() {
    let s = setup(&[("foo", Some("dir")), ("bar", None), ("baz", Some("file"))]);
    let menu = |allow_all| picker(&s, request(None, allow_all, false)).menu();
    let foo = path_of(&s, "foo");
    let bar = path_of(&s, "bar");
    let baz = path_of(&s, "baz");
    // `(missing)` follows `exists()`: a file is not missing here.
    assert_eq!(
        menu(false),
        format!(
            "Multiple brains registered. Pick one:\n  [1] foo  ({foo})\n  [2] bar  ({bar})  (missing)\n  [3] baz  ({baz})\n"
        )
    );
    assert_eq!(
        menu(true),
        format!(
            "Multiple brains registered. Pick one:\n  [1] foo  ({foo})\n  [2] bar  ({bar})  (missing)\n  [3] baz  ({baz})\n  [4] all\n"
        )
    );
}

#[test]
fn picker_parses_input_like_click_int_range() {
    let s = setup(&dirs(&["a", "b"]));
    let p = picker(&s, request(None, true, false));
    let cases: &[(&str, Choice)] = &[
        ("", Choice::Empty),
        ("1", Choice::Picked(1)),
        ("3", Choice::Picked(3)),
        (" 2 ", Choice::Picked(2)),
        ("\t2\x0b", Choice::Picked(2)),
        ("+2", Choice::Picked(2)),
        ("002", Choice::Picked(2)),
        ("\u{ff11}", Choice::Picked(1)),
        ("\u{0662}", Choice::Picked(2)),
        ("\u{3000}1\u{a0}", Choice::Picked(1)),
        (
            "0",
            Choice::Invalid("0 is not in the range 1<=x<=3.".into()),
        ),
        (
            "-1",
            Choice::Invalid("-1 is not in the range 1<=x<=3.".into()),
        ),
        (
            "-0",
            Choice::Invalid("0 is not in the range 1<=x<=3.".into()),
        ),
        (
            "4",
            Choice::Invalid("4 is not in the range 1<=x<=3.".into()),
        ),
        (
            "1_0",
            Choice::Invalid("10 is not in the range 1<=x<=3.".into()),
        ),
        (
            "99999999999999999999999",
            Choice::Invalid("99999999999999999999999 is not in the range 1<=x<=3.".into()),
        ),
        (
            "1__0",
            Choice::Invalid("'1__0' is not a valid integer range.".into()),
        ),
        (
            "_1",
            Choice::Invalid("'_1' is not a valid integer range.".into()),
        ),
        (
            "1_",
            Choice::Invalid("'1_' is not a valid integer range.".into()),
        ),
        (
            "abc",
            Choice::Invalid("'abc' is not a valid integer range.".into()),
        ),
        (
            " ",
            Choice::Invalid("' ' is not a valid integer range.".into()),
        ),
        (
            "1.0",
            Choice::Invalid("'1.0' is not a valid integer range.".into()),
        ),
        (
            "0x1",
            Choice::Invalid("'0x1' is not a valid integer range.".into()),
        ),
        (
            "+ 1",
            Choice::Invalid("'+ 1' is not a valid integer range.".into()),
        ),
        (
            " 'x",
            Choice::Invalid("\" 'x\" is not a valid integer range.".into()),
        ),
        (
            "a'\"b",
            Choice::Invalid("'a\\'\"b' is not a valid integer range.".into()),
        ),
        (
            "\\",
            Choice::Invalid("'\\\\' is not a valid integer range.".into()),
        ),
        (
            "\x1c1",
            Choice::Invalid("'\\x1c1' is not a valid integer range.".into()),
        ),
        (
            "\u{e9}t\u{e9}",
            Choice::Invalid("'\u{e9}t\u{e9}' is not a valid integer range.".into()),
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(&p.parse(input), expected, "input {input:?}");
    }
}

#[test]
fn picker_choose_returns_the_brain_or_all() {
    let s = setup(&dirs(&["a", "b"]));
    let p = picker(&s, request(None, true, false));
    assert_eq!(names(&p.clone().choose(2).unwrap()), ["b"]);
    assert_eq!(names(&p.choose(3).unwrap()), ["a", "b"]);
    let p = picker(&s, request(None, false, false));
    assert_eq!(names(&p.choose(1).unwrap()), ["a"]);
}

#[test]
fn picker_choice_is_path_checked() {
    let s = setup(&[("ok", Some("dir")), ("gone", None)]);
    let p = picker(&s, request(None, true, false));
    assert_eq!(names(&p.clone().choose(1).unwrap()), ["ok"]);
    assert_eq!(
        p.clone().choose(2).unwrap_err().to_string(),
        missing_message(&[("gone", path_of(&s, "gone"))])
    );
    assert_eq!(
        p.choose(3).unwrap_err().to_string(),
        missing_message(&[("gone", path_of(&s, "gone"))])
    );
}

fn run_picker(p: Picker, input: &str) -> (Result<Vec<String>, ResolveError>, String) {
    let mut out = Vec::new();
    let result = p
        .run(&mut Cursor::new(input.as_bytes().to_vec()), &mut out)
        .map(|b| names(&b).iter().map(|n| n.to_string()).collect());
    (result, String::from_utf8(out).unwrap())
}

#[test]
fn picker_loop_reprompts_like_click() {
    let s = setup(&dirs(&["a", "b"]));
    let menu = picker(&s, request(None, true, false)).menu();
    let (result, out) = run_picker(
        picker(&s, request(None, true, false)),
        "abc\n\n 'x\n0\n9\n2\n",
    );
    assert_eq!(result.unwrap(), ["b"]);
    assert_eq!(
        out,
        format!(
            "{menu}Choice: Error: 'abc' is not a valid integer range.\nChoice: Choice: Error: \" 'x\" is not a valid integer range.\nChoice: Error: 0 is not in the range 1<=x<=3.\nChoice: Error: 9 is not in the range 1<=x<=3.\nChoice: "
        )
    );
    // CRLF line endings, and a last line without a newline.
    let (result, _) = run_picker(picker(&s, request(None, true, false)), "x\r\n3");
    assert_eq!(result.unwrap(), ["a", "b"]);
}

#[test]
fn picker_end_of_input_aborts() {
    let s = setup(&dirs(&["a", "b"]));
    let menu = picker(&s, request(None, false, false)).menu();
    for input in ["", "\n", "nope\n"] {
        let (result, out) = run_picker(picker(&s, request(None, false, false)), input);
        assert_eq!(
            result.unwrap_err(),
            ResolveError::Aborted,
            "input {input:?}"
        );
        assert!(out.starts_with(&menu), "{out:?}");
        assert!(out.ends_with("Choice: "), "{out:?}");
    }
    assert_eq!(ResolveError::Aborted.to_string(), "Aborted!");
}

#[test]
fn resolver_paths_keep_the_stored_text() {
    // The resolved entry is the config entry, unchanged.
    let s = setup(&dirs(&["a"]));
    let Ok(Resolution::Brains(brains)) = resolve(&s.config, &request(None, false, true)) else {
        panic!("expected brains");
    };
    assert_eq!(brains[0].entry, s.config.brains["a"]);
    assert!(Path::new(&brains[0].entry.path).is_dir());
}
