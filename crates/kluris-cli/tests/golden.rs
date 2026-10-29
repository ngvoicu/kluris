//! The golden-test helpers of `common::golden`: each output normalization,
//! and the tree and git dumps.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::shared::vendored_marker;
use common::{
    Entry, Form, NativePaths, PINNED_ENV, PINNED_GIT_DATE, Sandbox, Tree, changes_dump, decode,
    file_body, git_dump, git_state, native_path, native_paths, normalize, normalize_json,
    parse_json_line, slashed, tree_dump,
};

/// A temp root whose name has a space and a non-ASCII letter, so its
/// JSON-escaped form differs from its raw form.
fn root() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("golden ü root");
    fs::create_dir(&root).unwrap();
    (tmp, root)
}

/// `json.dumps(s)[1:-1]`: backslashes doubled, non-ASCII as `\uXXXX`.
fn json_escaped(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_ascii() => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out
}

fn posix(s: &str) -> String {
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.to_string()
    }
}

/// A dump's body as the file holds it.
fn as_is(text: &str) -> String {
    text.to_string()
}

/// The resolved root as Python's `Path.resolve()` writes it (no `\\?\`).
fn resolved(root: &Path) -> String {
    let r = fs::canonicalize(root).unwrap().display().to_string();
    r.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(r)
}

/// The root as given and resolved, each as the OS writes it and with `/`
/// separators: the spellings a path can have before a form escapes it.
fn root_spellings(root: &Path) -> [(&'static str, String); 4] {
    let raw = root.display().to_string();
    let resolved = resolved(root);
    [
        ("raw", raw.clone()),
        ("resolved", resolved.clone()),
        ("raw, / separators", posix(&raw)),
        ("resolved, / separators", posix(&resolved)),
    ]
}

#[test]
fn every_spelling_of_the_root_becomes_root_in_each_form() {
    let (_tmp, root) = root();
    for (name, spelling) in root_spellings(&root) {
        let text = format!("at {spelling}: ok\n");
        assert_eq!(
            normalize(&text, &root, Form::Text),
            "at <ROOT>: ok\n",
            "text, {name}"
        );
        let doc = format!("{{\"at\": {}}}\n", json_line(&spelling).trim_end());
        assert_eq!(
            normalize(&doc, &root, Form::Json),
            "{\"at\": \"<ROOT>\"}\n",
            "JSON, {name}"
        );
        let tmp = tempfile::TempDir::new().unwrap();
        common::write(
            &tmp.path().join("plain.yml"),
            format!("at: {spelling}\n").as_bytes(),
        );
        common::write(
            &tmp.path().join("quoted.yml"),
            format!("at: {doc}").as_bytes(),
        );
        common::write(&tmp.path().join("notes.md"), text.as_bytes());
        assert_eq!(
            tree_dump(tmp.path(), |t| normalize(t, &root, Form::Dumped)),
            "== notes.md (file, 14 bytes)\nat <ROOT>: ok\n\
             == plain.yml (file, 11 bytes)\nat: <ROOT>\n\
             == quoted.yml (file, 21 bytes)\nat: {\"at\": \"<ROOT>\"}\n\
             <end of dump>\n",
            "dump, {name}"
        );
    }
}

/// The root spelled in another form's escaping is not the root in this
/// form: JSON's `\uXXXX` in text stays as it is (a root with a `\`, which
/// the OS writes on Windows, is refused instead).
#[test]
fn a_root_in_another_forms_escaping_stays() {
    let (_tmp, root) = root();
    let escaped = json_escaped(&root.display().to_string());
    assert_ne!(escaped, root.display().to_string());
    let text = format!("at {escaped}: ok\n");
    assert_eq!(normalize(&text, &root, Form::Text), text);
}

#[test]
fn text_without_the_root_is_unchanged() {
    let (_tmp, root) = root();
    let text = "kluris 3.1.1 is installed\nupdated: '2020-01-02'\n";
    assert_eq!(normalize(text, &root, Form::Text), text);
}

#[test]
fn a_whole_version_stdout_becomes_version() {
    let (_tmp, root) = root();
    let cases = [
        ("kluris 3.1.1\n", "kluris <VERSION>\n"),
        ("kluris 3.2.0-rc.1\n", "kluris <VERSION>\n"),
        ("kluris 3.2.0+build.7\n", "kluris <VERSION>\n"),
        // Anything but the whole stdout is user data and stays.
        ("kluris 3.1.1", "kluris 3.1.1"),
        ("kluris 3.1.1\nmore\n", "kluris 3.1.1\nmore\n"),
        ("Kluris 3.1.1 is installed\n", "Kluris 3.1.1 is installed\n"),
        ("kluris 3.1\n", "kluris 3.1\n"),
    ];
    for (text, expected) in cases {
        assert_eq!(normalize(text, &root, Form::Text), expected, "{text:?}");
    }
}

fn today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

#[test]
fn an_updated_value_of_today_becomes_today() {
    let (_tmp, root) = root();
    let d = today();
    let cases = [
        (format!("updated: '{d}'\n"), "updated: '<TODAY>'\n"),
        (format!("updated: {d}\n"), "updated: <TODAY>\n"),
        (format!("updated: \"{d}\"\n"), "updated: \"<TODAY>\"\n"),
        // A CRLF file's line as tree_dump writes it.
        (format!("updated: '{d}'\\r\n"), "updated: '<TODAY>'\\r\n"),
        (
            format!("{{\"updated\": \"{d}\", \"n\": 1}}\n"),
            "{\"updated\": \"<TODAY>\", \"n\": 1}\n",
        ),
    ];
    for (text, expected) in cases {
        let form = Form::of_stream(&text);
        assert_eq!(normalize(&text, &root, form), expected, "{text:?}");
    }
}

#[test]
fn only_an_updated_value_of_today_is_today() {
    let (_tmp, root) = root();
    let d = today();
    for text in [
        "updated: '2020-01-02'\n".to_string(),
        format!("created: '{d}'\n"),
        format!("{{\"created\": \"{d}\"}}\n"),
        format!("last_updated: '{d}'\n"),
        format!("updated: '{d}T10:00:00'\n"),
    ] {
        assert_eq!(normalize(&text, &root, Form::of_stream(&text)), text);
    }
}

#[test]
fn crlf_reads_as_lf_and_a_lone_cr_stays() {
    let (_tmp, root) = root();
    assert_eq!(normalize("a\r\nb\rc\r\n", &root, Form::Text), "a\nb\rc\n");
    assert_eq!(decode(b"a\r\nb\rc\r\n", "stdout"), "a\nb\rc\n");
}

#[test]
#[should_panic(expected = "stdout is not valid UTF-8: invalid byte at 2")]
fn a_stream_that_is_not_utf8_fails() {
    decode(b"ok\xffno", "stdout");
}

#[test]
fn version_fields_named_by_a_test_become_version() {
    let (_tmp, root) = root();
    let text = format!(
        "{{\"ok\": true, \"version\": \"3.1.1\", \"latest\": \"kluris 3.2.0-rc.1\", \"path\": \"{}\", \"note\": \"3.1.1\"}}\n",
        json_escaped(&root.display().to_string())
    );
    assert_eq!(
        normalize_json(&text, &root, &["/version", "/latest"]),
        "{\"ok\": true, \"version\": \"<VERSION>\", \"latest\": \"kluris <VERSION>\", \"path\": \"<ROOT>\", \"note\": \"3.1.1\"}\n"
    );
}

#[test]
fn version_fields_inside_lists_and_objects() {
    let (_tmp, root) = root();
    let text = "{\"releases\": [{\"v\": \"3.0.0\"}, {\"v\": \"3.1.1\"}], \"x\": 1.0}";
    assert_eq!(
        normalize_json(text, &root, &["/releases/1/v"]),
        "{\"releases\": [{\"v\": \"3.0.0\"}, {\"v\": \"<VERSION>\"}], \"x\": 1.0}"
    );
}

#[test]
fn json_without_version_fields_is_normalized_as_text() {
    let (_tmp, root) = root();
    let text = "{\"version\": \"3.1.1\", \"updated\": \"2020-01-02\"}\r\n";
    assert_eq!(
        normalize_json(text, &root, &[]),
        "{\"version\": \"3.1.1\", \"updated\": \"2020-01-02\"}\n"
    );
}

#[test]
#[should_panic(expected = "no version field at /missing")]
fn a_missing_version_field_fails() {
    let (_tmp, root) = root();
    normalize_json("{\"version\": \"3.1.1\"}\n", &root, &["/missing"]);
}

#[test]
#[should_panic(expected = "version field /version does not hold a version")]
fn a_version_field_without_a_version_fails() {
    let (_tmp, root) = root();
    normalize_json("{\"version\": \"latest\"}\n", &root, &["/version"]);
}

#[test]
#[should_panic(expected = "version field /version is not a string")]
fn a_version_field_that_is_not_a_string_fails() {
    let (_tmp, root) = root();
    normalize_json("{\"version\": 3}\n", &root, &["/version"]);
}

#[test]
#[should_panic(expected = "is not json.dumps of its parse")]
fn json_not_written_as_python_writes_it_fails() {
    let (_tmp, root) = root();
    normalize_json("{\"version\":\"3.1.1\"}\n", &root, &["/version"]);
}

#[test]
fn a_json_line_in_the_json_layout_parses() {
    let text = "{\"t\": \"\\u00e9\\ud83d\\ude00\\n\\\"\", \"a\": [1, 2.5, 1e-05, null, {}], \"ok\": true}\n";
    let doc = parse_json_line(text);
    assert_eq!(doc["t"], "é\u{1F600}\n\"");
    assert_eq!(doc["a"][2], 1e-05);
}

/// Outputs that hold a JSON document, each laid out otherwise than the
/// `--json` layout.
const NOT_THE_JSON_LAYOUT: [&str; 13] = [
    "{\"ok\":true,\"n\":1}\n",
    "{\"ok\": true}",
    "{\"ok\": true}\n\n",
    " {\"ok\": true}\n",
    "{\n  \"ok\": true\n}\n",
    "{\"t\": \"\u{e9}\"}\n",
    "{\"t\": \"\\u00E9\"}\n",
    "{\"t\": \"a\\/b\"}\n",
    "{\"t\": \"\\u0041\"}\n",
    "{\"n\": 1e2}\n",
    "{\"n\": 1.50}\n",
    "{\"a\": 1, \"a\": 2}\n",
    "[1,2]\n",
];

#[test]
fn a_json_line_in_another_layout_fails() {
    let passed: Vec<&str> = NOT_THE_JSON_LAYOUT
        .into_iter()
        .filter(|text| std::panic::catch_unwind(|| parse_json_line(text)).is_ok())
        .collect();
    assert!(
        passed.is_empty(),
        "passed as the --json layout: {passed:#?}"
    );
}

#[test]
fn tree_dump_lists_every_file_by_path_with_its_bytes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    let files: &[(&str, &[u8])] = &[
        ("notes/a.md", b"---\ntitle: A\n---\nbody\n"),
        ("crlf.md", b"one\r\ntwo\r\n"),
        ("bom.yml", b"\xef\xbb\xbfkey: v\n"),
        ("latin1.txt", b"caf\xe9 \xff\xfe end\n"),
        ("lone-cr.txt", b"a\rb\n"),
        ("backslash-r.txt", b"a\\rb\n"),
        ("escapes.txt", b"\\x41 \\u{feff} C:\\Users\\\\\n"),
        ("no-eol.txt", b"last line"),
        ("empty.txt", b""),
        ("a-b/x.md", b"x\n"),
        ("a/x.md", b"y\n"),
        (".git/HEAD", b"ref: refs/heads/main\n"),
        ("sub/.git/config", b"[core]\n"),
    ];
    for (path, bytes) in files {
        let p = dir.join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }
    assert_eq!(
        tree_dump(dir, as_is),
        "== a-b/x.md (file, 2 bytes)\nx\n\
         == a/x.md (file, 2 bytes)\ny\n\
         == backslash-r.txt (file, 5 bytes)\na\\\\rb\n\
         == bom.yml (file, 10 bytes)\n\\u{feff}key: v\n\
         == crlf.md (file, 10 bytes)\none\\r\ntwo\\r\n\
         == empty.txt (file, 0 bytes)\n\
         == escapes.txt (file, 25 bytes)\n\\\\x41 \\\\u{feff} C:\\\\Users\\\\\\\\\n\
         == latin1.txt (file, 12 bytes)\ncaf\\xe9 \\xff\\xfe end\n\
         == lone-cr.txt (file, 4 bytes)\na\\rb\n\
         == no-eol.txt (file, 9 bytes)\nlast line\n<no newline at end>\n\
         == notes/a.md (file, 22 bytes)\n---\ntitle: A\n---\nbody\n\
         <end of dump>\n"
    );
}

#[cfg(unix)]
#[test]
fn tree_dump_writes_a_symlink_as_its_target() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    fs::create_dir(dir.join("maps")).unwrap();
    fs::write(dir.join("maps/base.md"), "base\n").unwrap();
    std::os::unix::fs::symlink("maps/base.md", dir.join("link.md")).unwrap();
    std::os::unix::fs::symlink("maps", dir.join("linked-dir")).unwrap();
    std::os::unix::fs::symlink("gone.md", dir.join("broken.md")).unwrap();
    assert_eq!(
        tree_dump(dir, as_is),
        "== broken.md (symlink, 7 bytes)\ngone.md\n\
         == link.md (symlink, 12 bytes)\nmaps/base.md\n\
         == linked-dir (symlink, 4 bytes)\nmaps\n\
         == maps/base.md (file, 5 bytes)\nbase\n\
         <end of dump>\n"
    );
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .envs(PINNED_ENV.iter().copied())
        .env("GIT_CONFIG_GLOBAL", repo.join("gitconfig-empty"))
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn git_dump_prints_the_head_commit_and_its_tree() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = tmp.path();
    git(repo, &["init", "-q"]);
    fs::create_dir(repo.join("notes")).unwrap();
    fs::write(repo.join("notes/a.md"), "a\n").unwrap();
    fs::write(repo.join("kluris.yml"), "name: b\n").unwrap();
    git(repo, &["add", "notes/a.md", "kluris.yml"]);
    git(repo, &["commit", "-q", "-m", "brain: first"]);
    let tree = git(repo, &["rev-parse", "HEAD^{tree}"]);
    let a = git(repo, &["rev-parse", "HEAD:notes/a.md"]);
    let yml = git(repo, &["rev-parse", "HEAD:kluris.yml"]);
    let who = "Kluris Parity <parity@kluris.invalid> 1767268800 +0000";
    assert_eq!(
        git_dump(repo),
        format!(
            "== git cat-file -p HEAD\n\
             tree {tree}\n\
             author {who}\n\
             committer {who}\n\
             \n\
             brain: first\n\
             == git ls-tree -r HEAD\n\
             100644 blob {yml}\tkluris.yml\n\
             100644 blob {a}\tnotes/a.md\n",
            tree = tree.trim_end(),
            yml = yml.trim_end(),
            a = a.trim_end(),
        )
    );
}

/// A repository in `sb` whose `a.md` holds `work` and whose HEAD holds
/// `committed` for it, its status clean: git is told the file did not
/// change (`update-index --assume-unchanged`).
fn repo_committing(sb: &Sandbox, committed: &str, work: &str) -> PathBuf {
    let repo = sb.work.join("repo");
    fs::create_dir(&repo).unwrap();
    sb.init(&repo);
    fs::write(repo.join("a.md"), committed).unwrap();
    sb.commit(&repo, "initial", PINNED_GIT_DATE);
    fs::write(repo.join("a.md"), work).unwrap();
    sb.git(
        &repo,
        &["update-index", "--assume-unchanged", "a.md"],
        PINNED_GIT_DATE,
    );
    assert_eq!(sb.porcelain(&repo), "");
    repo
}

#[test]
fn the_git_state_shows_committed_bytes_the_work_tree_does_not_hold() {
    let (plain_sb, filtered_sb) = (Sandbox::new(), Sandbox::new());
    let plain = repo_committing(&plain_sb, "body\n", "body\n");
    let filtered = repo_committing(&filtered_sb, "BODY\n", "body\n");
    assert_eq!(
        tree_dump(&plain, str::to_string),
        tree_dump(&filtered, str::to_string)
    );

    let plain = git_state(&plain, str::to_string);
    let filtered = git_state(&filtered, str::to_string);
    assert_ne!(plain, filtered);
    let blobs = "== git cat-file blob <each blob of HEAD>\n== a.md (file, 5 bytes)\n";
    assert!(
        plain.ends_with(&format!("{blobs}<the work tree's bytes>\n<end of dump>\n")),
        "{plain}"
    );
    assert!(
        filtered.ends_with(&format!("{blobs}BODY\n<end of dump>\n")),
        "{filtered}"
    );
}

#[test]
fn a_cr_and_the_text_backslash_r_dump_differently() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("f"), b"a\rb\n").unwrap();
    let cr = tree_dump(dir, as_is);
    fs::write(dir.join("f"), b"a\\rb\n").unwrap();
    let text = tree_dump(dir, as_is);
    assert_ne!(cr, text);
    assert_eq!(
        (cr.as_str(), text.as_str()),
        (
            "== f (file, 4 bytes)\na\\rb\n<end of dump>\n",
            "== f (file, 5 bytes)\na\\\\rb\n<end of dump>\n"
        )
    );
}

/// A Windows root, which need not exist here: only its text matters.
const WINDOWS_ROOT: &str = r"C:\Users\runneradmin\AppData\Local\Temp\.tmpGx7Q2a";

#[test]
fn a_windows_root_inside_a_file_dumps_as_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    let root = Path::new(WINDOWS_ROOT);
    let escaped = WINDOWS_ROOT.replace('\\', "\\\\");
    let files = [
        ("config.yml", format!("path: {WINDOWS_ROOT}\\brains\\b\n")),
        (
            "map.json",
            format!("{{\"path\": \"{escaped}\\\\brains\"}}\n"),
        ),
    ];
    for (name, text) in &files {
        fs::write(dir.join(name), text).unwrap();
    }
    assert_eq!(
        tree_dump(dir, |text| normalize(text, root, Form::Dumped)),
        "== config.yml (file, 22 bytes)\npath: <ROOT>\\\\brains\\\\b\n\
         == map.json (file, 27 bytes)\n{\"path\": \"<ROOT>\\\\\\\\brains\"}\n\
         <end of dump>\n"
    );
}

#[test]
fn a_windows_root_with_a_non_ascii_name_inside_a_file_dumps_as_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    let root = format!(r"{WINDOWS_ROOT}\golden ü root");
    fs::write(dir.join("config.yml"), format!("path: {root}\\b\n")).unwrap();
    assert_eq!(
        tree_dump(dir, |text| normalize(text, Path::new(&root), Form::Dumped)),
        "== config.yml (file, 15 bytes)\npath: <ROOT>\\\\b\n<end of dump>\n"
    );
}

#[test]
fn a_tree_dump_snapshot_shows_every_escape() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("crlf.md"), b"one\r\ntwo\r\n").unwrap();
    fs::write(dir.join("bom.md"), b"\xef\xbb\xbfhi\n").unwrap();
    fs::write(dir.join("latin1.md"), b"caf\xe9\n").unwrap();
    fs::write(dir.join("backslashes.md"), b"C:\\brain \\r \\x41\n").unwrap();
    fs::write(dir.join("no-eol.md"), b"end").unwrap();
    insta::assert_snapshot!("tree_dump_format", tree_dump(dir, as_is));
}

/// A path the OS writes with `\\`, found written with `/`, fails, raw or
/// JSON-escaped; in its own form it is shown as given.
#[test]
fn native_path_refuses_a_path_written_with_slashes() {
    let native = Path::new(r"C:\Users\me\work\brains\calm\kluris.yml");
    let shown = "<ROOT>/work/brains/calm/kluris.yml";
    assert_eq!(
        native_path(r"at C:\Users\me\work\brains\calm\kluris.yml", native, shown),
        format!("at {shown}")
    );
    for text in [
        "at C:/Users/me/work/brains/calm/kluris.yml",
        r#"{"error": "C:/Users/me/work/brains/calm/kluris.yml"}"#,
    ] {
        let refused = std::panic::catch_unwind(|| native_path(text, native, shown));
        assert!(refused.is_err(), "{text} was not refused");
    }
}

/// A path inside another (a brain and its `kluris.yml`) is shown whole,
/// in JSON and in text, in whichever order the paths are given.
#[test]
fn native_paths_shows_a_path_inside_another() {
    let brain = Path::new(r"C:\Users\me\work\brains\a");
    let yml = Path::new(r"C:\Users\me\work\brains\a\kluris.yml");
    let texts = [
        (
            r#"{"path": "C:\\Users\\me\\work\\brains\\a", "yml": "C:\\Users\\me\\work\\brains\\a\\kluris.yml"}"#,
            r#"{"path": "C:/Users/me/work/brains/a", "yml": "C:/Users/me/work/brains/a/kluris.yml"}"#,
        ),
        (
            r"C:\Users\me\work\brains\a: no C:\Users\me\work\brains\a\kluris.yml",
            r"C:/Users/me/work/brains/a: no C:/Users/me/work/brains/a/kluris.yml",
        ),
    ];
    for paths in [[brain, yml], [yml, brain]] {
        let shown_paths = paths.map(slashed);
        let pairs: Vec<(&Path, &str)> = paths
            .iter()
            .zip(&shown_paths)
            .map(|(path, shown)| (*path, shown.as_str()))
            .collect();
        for (text, shown) in texts {
            assert_eq!(native_paths(text, &pairs), shown);
        }
    }
}

/// The gits a sandbox runs, a push to a bare remote and a commit among
/// them, start no automatic maintenance or gc: a detached one would still
/// be writing the repository (its `objects/maintenance.lock`) while a test
/// dumps it.
#[test]
fn sandboxed_git_starts_no_automatic_maintenance() {
    let sb = Sandbox::new();
    let remote = sb.work.join("remote.git");
    let clone = sb.work.join("clone");
    let s = |p: &Path| p.display().to_string();
    let git = |cwd: &Path, args: &[&str]| sb.git(cwd, args, PINNED_GIT_DATE);
    git(
        &sb.work,
        &["init", "-q", "--bare", "-b", "main", &s(&remote)],
    );
    git(&sb.work, &["clone", "-q", &s(&remote), &s(&clone)]);
    fs::write(clone.join("a.md"), "a\n").unwrap();
    let trace = sb.root.join("trace2.json");
    for args in [
        &["add", "-A"][..],
        &["commit", "-q", "-m", "a"],
        &["push", "-q", "-u", "origin", "main"],
    ] {
        let mut cmd = Command::new("git");
        let out = sb
            .env(&mut cmd)
            .env("GIT_TRACE2_EVENT", &trace)
            .args(args)
            .current_dir(&clone)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }
    let started: Vec<String> = fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|e| e["event"] == "child_start" || e["event"] == "start")
        .map(|e| e["argv"].to_string())
        .filter(|argv| argv.contains("\"maintenance\"") || argv.contains("\"gc\""))
        .collect();
    assert!(
        started.is_empty(),
        "automatic maintenance started: {started:?}"
    );
}

// --- every byte of a file shows --------------------------------------------

/// insta compares a stored snapshot with its trailing whitespace trimmed, so
/// two texts a snapshot cannot tell apart are equal once trimmed.
fn snapshots_differ(a: &str, b: &str) -> bool {
    a.trim_end() != b.trim_end()
}

/// The dump of a folder holding `files`.
fn dump_of(files: &[(&str, &[u8])]) -> String {
    let tmp = tempfile::TempDir::new().unwrap();
    for (rel, bytes) in files {
        common::write(&tmp.path().join(rel), bytes);
    }
    tree_dump(tmp.path(), as_is)
}

#[test]
fn a_missing_final_newline_differs_from_a_marker_line() {
    assert_ne!(
        file_body(b"body", as_is).text,
        file_body(b"body\n<no newline at end>\n", as_is).text
    );
    let a = dump_of(&[("a.md", b"body")]);
    let b = dump_of(&[("a.md", b"body\n<no newline at end>\n")]);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

#[test]
fn a_body_holding_a_record_boundary_differs_from_two_files() {
    let a = dump_of(&[("a.md", b"x\n== b.md\ny\n")]);
    let b = dump_of(&[("a.md", b"x\n"), ("b.md", b"y\n")]);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

#[test]
fn trailing_blank_lines_of_the_last_file_show() {
    let a = dump_of(&[("a.md", b"a\n"), ("z.md", b"body\n")]);
    let b = dump_of(&[("a.md", b"a\n"), ("z.md", b"body\n\n\n")]);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

#[cfg(unix)]
#[test]
fn a_body_holding_a_symlink_marker_differs_from_a_symlink() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::os::unix::fs::symlink("maps/base.md", tmp.path().join("link.md")).unwrap();
    let a = tree_dump(tmp.path(), as_is);
    let b = dump_of(&[("link.md", b"<symlink> maps/base.md\n")]);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

/// `changes_dump` from nothing, or from `before`, to `after`, a companion
/// file holding its vendored bytes shown by reference.
fn changes_of(before: &[(&str, &[u8])], after: &[(&str, &[u8])]) -> String {
    let map = |files: &[(&str, &[u8])]| -> Tree {
        files
            .iter()
            .map(|(rel, bytes)| (rel.to_string(), Entry::File(bytes.to_vec())))
            .collect()
    };
    changes_dump(&map(before), &map(after), as_is, vendored_marker)
}

#[test]
fn a_body_holding_a_removal_record_differs_from_a_removal() {
    let before: &[(&str, &[u8])] = &[("a.md", b"x\n")];
    let removed = changes_of(before, &[]);
    let rewritten = changes_of(before, &[("a.md", b"<removed>\n")]);
    assert!(snapshots_differ(&removed, &rewritten), "{removed:?}");
}

#[test]
fn a_body_holding_a_vendored_marker_differs_from_the_vendored_file() {
    let rel = "specmint-core/SKILL.md";
    let path = format!("home/.kluris/companions/{rel}");
    let vendored = fs::read(common::shared::vendored().join(rel)).unwrap();
    let marker = format!("<vendored/{rel}>\n");
    let installed = changes_of(&[], &[(&path, &vendored)]);
    let imitated = changes_of(&[], &[(&path, marker.as_bytes())]);
    assert!(snapshots_differ(&installed, &imitated), "{installed:?}");
}

// --- every printed path is checked whole -----------------------------------

const MIXED_ROOT: &str = r"C:\Temp\run";
const MIXED_BRAIN: &str = r"C:\Temp\run\work\brains\alpha";

/// Whether [`native_path`] refuses `text` for the brain [`MIXED_BRAIN`].
fn refused(text: &str) -> bool {
    std::panic::catch_unwind(|| {
        native_path(text, Path::new(MIXED_BRAIN), "<ROOT>/work/brains/alpha")
    })
    .is_err()
}

#[test]
fn a_path_written_partly_with_slashes_is_refused() {
    for text in [
        r"at C:\Temp\run/work/brains/alpha",
        r"at C:\Temp\run\work/brains/alpha",
        r"at C:\Temp\run\work\brains/alpha",
        r"at C:/Temp/run\work\brains\alpha",
    ] {
        assert!(refused(text), "{text} was not refused");
    }
}

#[test]
fn a_json_escaped_path_written_partly_with_slashes_is_refused() {
    for text in [
        r#"{"path": "C:\\Temp\\run/work/brains/alpha"}"#,
        r#"{"path": "C:\\Temp\\run\\work\\brains/alpha"}"#,
    ] {
        assert!(refused(text), "{text} was not refused");
    }
}

/// `bytes`, a file's, dumped with the brain [`MIXED_BRAIN`] checked and
/// shown in each body ([`Form::Dumped`]); `None` when it is refused.
fn dumped_shown(name: &str, bytes: &[u8]) -> Option<String> {
    let tmp = tempfile::TempDir::new().unwrap();
    common::write(&tmp.path().join(name), bytes);
    let native = NativePaths::new(&[(Path::new(MIXED_BRAIN), "<ROOT>/work/brains/alpha")]);
    std::panic::catch_unwind(|| tree_dump(tmp.path(), |text| native.show(text, Form::Dumped))).ok()
}

#[test]
fn a_dumped_path_written_partly_with_slashes_is_refused() {
    let files: [(&str, &[u8]); 2] = [
        ("config.yml", b"path: C:\\Temp\\run/work/brains/alpha\n"),
        (
            "map.json",
            b"{\"path\": \"C:\\\\Temp\\\\run/work/brains/alpha\"}\n",
        ),
    ];
    for (name, bytes) in files {
        let dump = dumped_shown(name, bytes);
        assert!(dump.is_none(), "{name} was not refused: {dump:?}");
    }
}

#[test]
fn a_path_continued_with_slashes_is_refused() {
    for text in [
        r"at C:\Temp\run\work\brains\alpha/notes/a.md",
        r#"{"path": "C:\\Temp\\run\\work\\brains\\alpha/notes/a.md"}"#,
    ] {
        assert!(refused(text), "{text} was not refused");
    }
}

#[test]
fn a_root_continued_with_slashes_is_refused() {
    for text in [
        r"at C:\Temp\run/work/home/.kluris/config.yml",
        r#"{"path": "C:\\Temp\\run/work/home"}"#,
    ] {
        let form = Form::of_stream(text);
        let refused =
            std::panic::catch_unwind(|| normalize(text, Path::new(MIXED_ROOT), form)).is_err();
        assert!(refused, "{text} was not refused");
    }
}

/// `path` as a JSON line holds it: one string, quoted and escaped.
fn json_line(path: &str) -> String {
    kluris_core::json::to_line(&kluris_core::json::Json::Str(path.to_string()))
}

/// A run whose stdout is `stdout` and stderr `stderr`, its brain path
/// [`MIXED_BRAIN`] checked and shown; `None` when it is refused.
fn run_shown(stdout: &str, stderr: &str) -> Option<common::Run> {
    let run = common::Run {
        code: 0,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    };
    std::panic::catch_unwind(|| run.native_paths(&[Path::new(MIXED_BRAIN)])).ok()
}

/// A JSON path field whose parsed value doubles every backslash is not the
/// brain's path, though its JSON text is the brain's path as a dump writes
/// a JSON file.
#[test]
fn a_json_path_with_doubled_backslashes_is_refused() {
    let good = format!("{{\"path\": {}}}\n", json_line(MIXED_BRAIN).trim_end());
    let doubled = MIXED_BRAIN.replace('\\', r"\\");
    let wrong = format!("{{\"path\": {}}}\n", json_line(&doubled).trim_end());
    assert_ne!(parse_json_line(&good), parse_json_line(&wrong));
    let shown = run_shown(&good, "").expect("the brain's own path is refused");
    assert_eq!(
        shown.stdout,
        "{\"path\": \"C:/Temp/run/work/brains/alpha\"}\n"
    );
    assert!(run_shown(&wrong, "").is_none(), "{wrong} was not refused");
}

/// A path printed as text with every backslash doubled is not the brain's
/// path, though it is the brain's path as JSON or a dump writes it.
#[test]
fn a_text_path_with_doubled_backslashes_is_refused() {
    let good = format!("Error: {MIXED_BRAIN} is not a brain\n");
    let wrong = good.replace('\\', r"\\");
    let shown = run_shown("", &good).expect("the brain's own path is refused");
    assert_eq!(
        shown.stderr,
        "Error: C:/Temp/run/work/brains/alpha is not a brain\n"
    );
    assert!(run_shown("", &wrong).is_none(), "{wrong} was not refused");
    assert!(
        run_shown(&wrong, "").is_none(),
        "{wrong} on stdout was not refused"
    );
}

/// A YAML file in a dump: the brain's path as a plain scalar or a quoted
/// one is shown, a plain scalar with every backslash doubled (whose dumped
/// text is the quoted scalar's) is refused.
#[test]
fn a_dumped_yaml_path_with_doubled_backslashes_is_refused() {
    let dump = |yaml: &str| dumped_shown("config.yml", yaml.as_bytes());
    let plain = format!("path: {MIXED_BRAIN}\n");
    let quoted = format!("path: {}", json_line(MIXED_BRAIN));
    let doubled = format!("path: {}\n", MIXED_BRAIN.replace('\\', r"\\"));
    for yaml in [&plain, &quoted] {
        let shown = dump(yaml).unwrap_or_else(|| panic!("{yaml} was refused"));
        assert!(shown.contains("<ROOT>/work/brains/alpha"), "{shown}");
    }
    assert!(dump(&doubled).is_none(), "{doubled} was not refused");
}

/// A JSON file in a dump: its path field is checked on its parsed value,
/// so one with every backslash doubled is refused.
#[test]
fn a_dumped_json_path_with_doubled_backslashes_is_refused() {
    let good = format!("{{\"path\": {}}}\n", json_line(MIXED_BRAIN).trim_end());
    let doubled = MIXED_BRAIN.replace('\\', r"\\");
    let wrong = format!("{{\"path\": {}}}\n", json_line(&doubled).trim_end());
    let shown = dumped_shown("map.json", good.as_bytes()).expect("the brain's own path is refused");
    assert!(
        shown.contains("{\"path\": \"<ROOT>/work/brains/alpha\"}"),
        "{shown}"
    );
    assert!(
        dumped_shown("map.json", wrong.as_bytes()).is_none(),
        "{wrong} was not refused"
    );
}

/// A YAML file's text that spells a path outside its values (a comment)
/// is refused: only the values are checked one by one.
#[test]
fn a_dumped_path_outside_the_parsed_values_is_refused() {
    let doubled = MIXED_BRAIN.replace('\\', r"\\");
    let yaml = format!("# was {doubled}\npath: {MIXED_BRAIN}\n");
    assert!(
        dumped_shown("config.yml", yaml.as_bytes()).is_none(),
        "{yaml} was not refused"
    );
}

/// A run with `stdout` and `stderr` rendered under the root [`MIXED_ROOT`];
/// `None` when it is refused.
fn rendered_under_mixed_root(stdout: &str, stderr: &str) -> Option<String> {
    let run = common::Run {
        code: 0,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    };
    std::panic::catch_unwind(|| run.render(Path::new(MIXED_ROOT))).ok()
}

/// A path under the root that no test lists, printed as text with every
/// backslash doubled, is not under the root as the OS writes it.
#[test]
fn a_text_root_with_doubled_backslashes_is_refused() {
    let good = format!("wrote {MIXED_ROOT}\\work\\notes\\a.md\n");
    let wrong = good.replace('\\', r"\\");
    let shown = rendered_under_mixed_root("", &good).expect("the root is refused");
    // A run's section writes each backslash doubled.
    assert!(
        shown.contains(r"wrote <ROOT>\\work\\notes\\a.md"),
        "{shown}"
    );
    let wrong_shown = rendered_under_mixed_root("", &wrong);
    assert!(
        wrong_shown.is_none(),
        "{wrong} was not refused: {wrong_shown:?}"
    );
}

/// The same path in a JSON value whose parsed text doubles every
/// backslash.
#[test]
fn a_json_root_with_doubled_backslashes_is_refused() {
    let path = format!("{MIXED_ROOT}\\work\\notes\\a.md");
    let good = format!("{{\"path\": {}}}\n", json_line(&path).trim_end());
    let doubled = path.replace('\\', r"\\");
    let wrong = format!("{{\"path\": {}}}\n", json_line(&doubled).trim_end());
    let shown = rendered_under_mixed_root(&good, "").expect("the root is refused");
    assert!(
        shown.contains(r#"{"path": "<ROOT>\\\\work\\\\notes\\\\a.md"}"#),
        "{shown}"
    );
    let wrong_shown = rendered_under_mixed_root(&wrong, "");
    assert!(
        wrong_shown.is_none(),
        "{wrong} was not refused: {wrong_shown:?}"
    );
}

// --- the run and the version fields ----------------------------------------

/// `git raw` in a sandbox, its alias `raw` a shell command that writes
/// `{"a": "<the byte FF>"}` with `redirect` (`>&1` stdout, `>&2` stderr).
fn raw_byte_run(redirect: &str) -> common::Run {
    let sb = Sandbox::new();
    let mut cmd = Command::new("git");
    sb.env(&mut cmd)
        .arg("-c")
        .arg(format!(
            "alias.raw=!printf '{{\"a\": \"\\377\"}}\\n' {redirect}"
        ))
        .arg("raw")
        .current_dir(&sb.work);
    common::Run::of(cmd)
}

#[test]
#[should_panic(expected = "is not valid UTF-8: invalid byte at 7, bytes [255]")]
fn a_run_whose_stdout_is_not_utf8_fails() {
    let run = raw_byte_run(">&1");
    assert!(run.stdout.is_empty(), "stdout was read: {:?}", run.stdout);
}

#[test]
#[should_panic(expected = "is not valid UTF-8: invalid byte at 7, bytes [255]")]
fn a_run_whose_stderr_is_not_utf8_fails() {
    let run = raw_byte_run(">&2");
    assert!(run.stderr.is_empty(), "stderr was read: {:?}", run.stderr);
}

/// With the version field marked, everything else in the document still
/// tells two outputs apart: one that does not hold where the other does is
/// equally a difference.
#[test]
fn a_marked_version_field_hides_nothing_else() {
    let (_tmp, root) = root();
    let cases = [
        (
            "final newline",
            "{\"version\": \"2.28.11\", \"n\": 1}\n",
            "{\"version\": \"3.0.0\", \"n\": 1}",
        ),
        (
            "key order",
            "{\"version\": \"2.28.11\", \"a\": 1, \"b\": 2}\n",
            "{\"version\": \"3.0.0\", \"b\": 2, \"a\": 1}\n",
        ),
        (
            "true and 1",
            "{\"version\": \"2.28.11\", \"n\": true}\n",
            "{\"version\": \"3.0.0\", \"n\": 1}\n",
        ),
        (
            "1 and 1.0",
            "{\"version\": \"2.28.11\", \"n\": 1}\n",
            "{\"version\": \"3.0.0\", \"n\": 1.0}\n",
        ),
        (
            "-0.0 and 0.0",
            "{\"version\": \"2.28.11\", \"n\": -0.0}\n",
            "{\"version\": \"3.0.0\", \"n\": 0.0}\n",
        ),
    ];
    for (what, a, b) in cases {
        let marked = |text: &str| {
            std::panic::catch_unwind(|| normalize_json(text, &root, &["/version"])).ok()
        };
        let (a, b) = (marked(a), marked(b));
        assert!(a.is_some(), "{what}: the first document does not hold");
        assert_ne!(a, b, "{what}: the marked documents are the same");
    }
}

// --- every byte of a run shows ---------------------------------------------

/// A run that ended with `code`, its streams `stdout` and `stderr`, as its
/// snapshot holds it.
fn rendered(stdout: &str, stderr: &str, code: i32) -> String {
    let (_tmp, root) = root();
    common::Run {
        code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
    .render(&root)
}

#[test]
fn a_stdout_holding_a_stderr_header_differs_from_a_stderr() {
    let a = rendered("x\n== stderr\ny\n", "", 0);
    let b = rendered("x\n", "y\n== stderr\n", 0);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

#[test]
fn a_stream_holding_an_exit_line_differs_from_the_exit() {
    let a = rendered("a\n== stderr\n== exit 0\n", "b\n", 1);
    let b = rendered("a\n", "== exit 0\n== stderr\nb\n", 1);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

#[test]
fn trailing_blank_lines_of_stderr_show() {
    let a = rendered("out\n", "err\n", 2);
    let b = rendered("out\n", "err\n\n\n", 2);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}

#[test]
fn a_stream_without_its_final_newline_differs_from_a_marker_line() {
    let a = rendered("", "err", 1);
    let b = rendered("", "err\n<no newline at end>\n", 1);
    assert!(snapshots_differ(&a, &b), "{a:?}");
}
