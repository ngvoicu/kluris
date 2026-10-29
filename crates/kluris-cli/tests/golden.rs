//! The golden-test helpers of `common::golden`: each output normalization,
//! and the tree and git dumps.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{PINNED_ENV, decode, git_dump, normalize, normalize_json, tree_dump};

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

/// The resolved root as Python's `Path.resolve()` writes it (no `\\?\`).
fn resolved(root: &Path) -> String {
    let r = fs::canonicalize(root).unwrap().display().to_string();
    r.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(r)
}

#[test]
fn every_form_of_the_root_becomes_root() {
    let (_tmp, root) = root();
    let raw = root.display().to_string();
    let resolved = resolved(&root);
    let forms = [
        ("raw", raw.clone()),
        ("resolved", resolved.clone()),
        ("raw, / separators", posix(&raw)),
        ("resolved, / separators", posix(&resolved)),
        ("raw, JSON-escaped", json_escaped(&raw)),
        ("resolved, JSON-escaped", json_escaped(&resolved)),
        (
            "raw, / separators, JSON-escaped",
            json_escaped(&posix(&raw)),
        ),
        (
            "resolved, / separators, JSON-escaped",
            json_escaped(&posix(&resolved)),
        ),
    ];
    for (name, form) in forms {
        let text = format!("at {form}/brain/a.md: ok\n");
        assert_eq!(
            normalize(&text, &root),
            "at <ROOT>/brain/a.md: ok\n",
            "{name}: {form}"
        );
    }
}

#[test]
fn text_without_the_root_is_unchanged() {
    let (_tmp, root) = root();
    let text = "kluris 3.1.1 is installed\nupdated: '2020-01-02'\n";
    assert_eq!(normalize(text, &root), text);
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
        assert_eq!(normalize(text, &root), expected, "{text:?}");
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
        assert_eq!(normalize(&text, &root), expected, "{text:?}");
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
        assert_eq!(normalize(&text, &root), text);
    }
}

#[test]
fn crlf_reads_as_lf_and_a_lone_cr_stays() {
    let (_tmp, root) = root();
    assert_eq!(normalize("a\r\nb\rc\r\n", &root), "a\nb\rc\n");
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
        tree_dump(dir),
        "== a-b/x.md\nx\n\
         == a/x.md\ny\n\
         == backslash-r.txt\na\\\\rb\n\
         == bom.yml\n\\u{feff}key: v\n\
         == crlf.md\none\\r\ntwo\\r\n\
         == empty.txt\n\
         == escapes.txt\n\\\\x41 \\\\u{feff} C:\\\\Users\\\\\\\\\n\
         == latin1.txt\ncaf\\xe9 \\xff\\xfe end\n\
         == lone-cr.txt\na\\rb\n\
         == no-eol.txt\nlast line\n<no newline at end>\n\
         == notes/a.md\n---\ntitle: A\n---\nbody\n"
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
        tree_dump(dir),
        "== broken.md\n<symlink> gone.md\n\
         == link.md\n<symlink> maps/base.md\n\
         == linked-dir\n<symlink> maps\n\
         == maps/base.md\nbase\n"
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

#[test]
fn a_cr_and_the_text_backslash_r_dump_differently() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    fs::write(dir.join("f"), b"a\rb\n").unwrap();
    let cr = tree_dump(dir);
    fs::write(dir.join("f"), b"a\\rb\n").unwrap();
    let text = tree_dump(dir);
    assert_ne!(cr, text);
    assert_eq!(
        (cr.as_str(), text.as_str()),
        ("== f\na\\rb\n", "== f\na\\\\rb\n")
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
        normalize(&tree_dump(dir), root),
        "== config.yml\npath: <ROOT>\\\\brains\\\\b\n\
         == map.json\n{\"path\": \"<ROOT>\\\\\\\\brains\"}\n"
    );
}

#[test]
fn a_windows_root_with_a_non_ascii_name_inside_a_file_dumps_as_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path();
    let root = format!(r"{WINDOWS_ROOT}\golden ü root");
    fs::write(dir.join("config.yml"), format!("path: {root}\\b\n")).unwrap();
    assert_eq!(
        normalize(&tree_dump(dir), Path::new(&root)),
        "== config.yml\npath: <ROOT>\\\\b\n"
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
    insta::assert_snapshot!("tree_dump_format", tree_dump(dir));
}
