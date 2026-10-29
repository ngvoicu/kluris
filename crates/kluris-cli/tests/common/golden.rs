//! Golden-test helpers: the normalizations the parity harness applies
//! before it compares two runs, so a snapshot taken through them holds what
//! parity proved, and dumps of a folder and of a git repository that keep
//! every byte visible.
//!
//! Program output goes through [`decode`] (stdout, stderr) and [`normalize`]
//! or [`normalize_json`]. File bytes go through [`tree_dump`] first, never
//! raw into `normalize`: the dump writes a CR as the two characters `\r`, so
//! `normalize(&tree_dump(dir), root)` replaces the root and today's date
//! while a file's CRLF stays in the snapshot.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use kluris_core::json::{self, Json};
use regex::Regex;

/// The git date every sandboxed commit carries.
pub const PINNED_GIT_DATE: &str = "2026-01-01T12:00:00+00:00";

/// The environment every sandboxed run pins (the parity harness's), beside
/// the per-sandbox paths a test sets itself: `HOME`, `USERPROFILE`,
/// `KLURIS_CONFIG`, `GIT_CONFIG_GLOBAL` (an empty file) and the temp
/// folder (`TMPDIR`, `TEMP`, `TMP`). `TZ=UTC` makes Kluris's today the UTC
/// date that [`normalize`] reads as `<TODAY>`.
pub const PINNED_ENV: &[(&str, &str)] = &[
    ("KLURIS_NO_PROMPT", "1"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Kluris Parity"),
    ("GIT_AUTHOR_EMAIL", "parity@kluris.invalid"),
    ("GIT_COMMITTER_NAME", "Kluris Parity"),
    ("GIT_COMMITTER_EMAIL", "parity@kluris.invalid"),
    ("GIT_AUTHOR_DATE", PINNED_GIT_DATE),
    ("GIT_COMMITTER_DATE", PINNED_GIT_DATE),
    ("TZ", "UTC"),
    ("LC_ALL", "C.UTF-8"),
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("COLUMNS", "120"),
];

/// The variables a sandboxed run inherits (the harness clears the rest), so
/// the binary and git can start.
pub const PASSTHROUGH_ENV: &[&str] = &[
    "PATH",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "COMSPEC",
    "PATHEXT",
    "WINDIR",
];

const VERSION: &str = r"\d+\.\d+\.\d+(?:[-+.][0-9A-Za-z.+-]*)?";

static VERSION_RE: LazyLock<Regex> = LazyLock::new(|| re(VERSION));
static VERSION_OUTPUT_RE: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"\Akluris {VERSION}\n\z")));

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("{pattern}: {e}"))
}

/// stdout or stderr as text: strictly UTF-8 (anything else fails the test
/// and names the bytes, so two different invalid streams never compare
/// equal), with every CRLF read as LF and a lone CR kept.
pub fn decode(stream: &[u8], what: &str) -> String {
    match std::str::from_utf8(stream) {
        Ok(text) => text.replace("\r\n", "\n"),
        Err(e) => {
            let start = e.valid_up_to();
            let end = start + e.error_len().unwrap_or(stream.len() - start);
            panic!(
                "{what} is not valid UTF-8: invalid byte at {start}, bytes {:?} in {:?}",
                &stream[start..end],
                String::from_utf8_lossy(
                    &stream[start.saturating_sub(20)..(end + 20).min(stream.len())]
                ),
            )
        }
    }
}

/// Program output as a snapshot holds it: CRLF read as LF, every form of
/// the temp `root` as `<ROOT>`, a whole `--version` stdout as
/// `kluris <VERSION>`, and an `updated` value equal to today as `<TODAY>`.
pub fn normalize(text: &str, root: &Path) -> String {
    let mut text = text.replace("\r\n", "\n");
    for form in root_forms(root) {
        text = text.replace(&form, "<ROOT>");
    }
    if VERSION_OUTPUT_RE.is_match(&text) {
        text = "kluris <VERSION>\n".to_string();
    }
    mark_today(&text)
}

/// A `--json` stdout through [`normalize`], then the version at each of
/// `version_fields` (JSON pointers, `/releases/0/version`) as `<VERSION>`;
/// version text anywhere else may be user data and stays.
///
/// With version fields the text must be exactly Python's `json.dumps` of its
/// own parse (and its final newline, if any), as Kluris prints every
/// `--json`; the marked document is written back the same way. A missing
/// field, or one that does not hold one version, fails the test.
pub fn normalize_json(text: &str, root: &Path, version_fields: &[&str]) -> String {
    let text = normalize(text, root);
    if version_fields.is_empty() {
        return text;
    }
    let mut doc: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("output is not JSON ({e}): {text:?}"));
    let newline = if text.ends_with('\n') { "\n" } else { "" };
    let dumps = |doc: &serde_json::Value| json::to_string(&python_json(doc)) + newline;
    assert_eq!(
        text,
        dumps(&doc),
        "raw JSON {text:?} is not json.dumps of its parse"
    );
    for field in version_fields {
        let value = doc
            .pointer_mut(field)
            .unwrap_or_else(|| panic!("output has no version field at {field}: {text:?}"));
        let Some(s) = value.as_str() else {
            panic!("version field {field} is not a string: {value}");
        };
        let count = VERSION_RE.find_iter(s).count();
        let marked = VERSION_RE.replace(s, "<VERSION>").into_owned();
        assert!(
            count == 1 && (marked == "<VERSION>" || marked == "kluris <VERSION>"),
            "version field {field} does not hold a version: {s:?}"
        );
        *value = serde_json::Value::String(marked);
    }
    dumps(&doc)
}

/// Every file under `dir`, `.git` excluded (see [`git_dump`]), sorted by its
/// `/`-joined relative path: a `== <path>` line, then its bytes. A CR is
/// written `\r`, U+FEFF (a BOM) `\u{feff}`, a byte that is not UTF-8
/// `\xNN`; a file that does not end in a newline gets a
/// `<no newline at end>` line. A symlink is `<symlink> <target>` and is not
/// followed. Folders show only through their files.
pub fn tree_dump(dir: &Path) -> String {
    let mut entries = Vec::new();
    walk(dir, "", &mut entries);
    entries.sort();
    entries
        .into_iter()
        .map(|(path, body)| format!("== {path}\n{body}"))
        .collect()
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let read = fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    for entry in read {
        let entry = entry.unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        let rel = format!("{prefix}{name}");
        let meta =
            fs::symlink_metadata(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if meta.is_symlink() {
            let target = fs::read_link(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let target = escape(target.as_os_str().as_encoded_bytes());
            out.push((rel, format!("<symlink> {target}\n")));
        } else if meta.is_dir() {
            walk(&path, &format!("{rel}/"), out);
        } else if meta.is_file() {
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut body = escape(&bytes);
            if !bytes.is_empty() && !bytes.ends_with(b"\n") {
                body.push_str("\n<no newline at end>\n");
            }
            out.push((rel, body));
        }
    }
}

/// The HEAD commit of `repo` (`git cat-file -p HEAD`) and the tree it
/// records (`git ls-tree -r HEAD`), each under a `== <command>` line, with
/// the bytes written as [`tree_dump`] writes them. Its commit and tree ids
/// hold only when the commits were made with [`PINNED_ENV`].
pub fn git_dump(repo: &Path) -> String {
    let mut out = String::new();
    for args in [["cat-file", "-p", "HEAD"], ["ls-tree", "-r", "HEAD"]] {
        let run = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", repo.join("kluris-golden-no-gitconfig"))
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap_or_else(|e| panic!("git {}: {e}", args.join(" ")));
        assert!(
            run.status.success(),
            "git {} in {}: {}",
            args.join(" "),
            repo.display(),
            String::from_utf8_lossy(&run.stderr)
        );
        out.push_str(&format!(
            "== git {}\n{}",
            args.join(" "),
            escape(&run.stdout)
        ));
    }
    out
}

/// The text forms of `root` a program can print, longest first so a
/// resolved form is not half-replaced by a shorter alias: as given and
/// resolved (macOS `/var` is `/private/var`), each with its native and `/`
/// separators, each also JSON-escaped.
fn root_forms(root: &Path) -> Vec<String> {
    let mut paths = vec![root.display().to_string()];
    if let Ok(resolved) = fs::canonicalize(root) {
        let resolved = resolved.display().to_string();
        // Windows' verbatim prefix, which Python's resolve() does not write.
        let resolved = match resolved.strip_prefix(r"\\?\") {
            Some(rest) if !rest.starts_with(r"UNC\") => rest.to_string(),
            _ => resolved,
        };
        paths.push(resolved);
    }
    let mut forms = BTreeSet::new();
    for path in paths {
        let posix = if cfg!(windows) {
            path.replace('\\', "/")
        } else {
            path.clone()
        };
        for form in [path, posix] {
            let quoted = json::to_string(&Json::Str(form.clone()));
            forms.insert(quoted[1..quoted.len() - 1].to_string());
            forms.insert(form);
        }
    }
    let mut forms: Vec<String> = forms.into_iter().filter(|f| !f.is_empty()).collect();
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms
}

/// `updated` set to today's UTC date, in the forms Kluris writes it: a YAML
/// line (plain or quoted, its CR as `tree_dump` writes it) and a JSON key.
fn mark_today(text: &str) -> String {
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string();
    // Each form keeps its own quoting; only the date becomes `<TODAY>`.
    let forms = [
        format!(r"(?m)^([ \t]*updated: '){today}('(?:\\r)?)$"),
        format!(r#"(?m)^([ \t]*updated: "){today}("(?:\\r)?)$"#),
        format!(r"(?m)^([ \t]*updated: ){today}((?:\\r)?)$"),
        format!(r#"("updated": "){today}(")"#),
    ];
    let mut text = text.to_string();
    for form in forms {
        text = re(&form).replace_all(&text, "${1}<TODAY>${2}").into_owned();
    }
    text
}

/// A parsed JSON document as `kluris_core::json` writes it (Python's
/// `json.dumps`).
fn python_json(value: &serde_json::Value) -> Json {
    use serde_json::Value;
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => Json::Int(i),
            (None, Some(u), _) => Json::BigInt(u.to_string()),
            (None, None, Some(f)) => Json::Float(f),
            _ => panic!("JSON number {n} has no Python form"),
        },
        Value::String(s) => Json::Str(s.clone()),
        Value::Array(items) => Json::Array(items.iter().map(python_json).collect()),
        Value::Object(map) => Json::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), python_json(v)))
                .collect(),
        ),
    }
}

/// Bytes as text that shows them: CR as `\r`, U+FEFF as `\u{feff}`, a byte
/// that is not UTF-8 as `\xNN`.
fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\r' => out.push_str("\\r"),
                '\u{feff}' => out.push_str("\\u{feff}"),
                c => out.push(c),
            }
        }
        for byte in chunk.invalid() {
            out.push_str(&format!("\\x{byte:02x}"));
        }
    }
    out
}
