//! Golden-test helpers. A snapshot must hold the same text on every machine
//! and every day, yet show every byte Kluris chose: the normalizations
//! replace only what the run's place and time decide (the temp root, the
//! version of the binary, today's date, the OS's line endings in its
//! streams) with markers, and the dumps of a folder and of a git repository
//! write every byte so that a changed line ending, BOM or encoding shows.
//!
//! Program output goes through [`decode`] (stdout, stderr) and [`normalize`]
//! or [`normalize_json`]. File bytes go through [`tree_dump`] first, never
//! raw into `normalize`: the dump writes a CR as the two characters `\r`, so
//! `normalize(&tree_dump(dir), root)` replaces the root and today's date
//! while a file's CRLF stays in the snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::LazyLock;

use kluris_core::json::{self, Json};
use regex::Regex;

/// The git date every sandboxed commit carries.
pub const PINNED_GIT_DATE: &str = "2026-01-01T12:00:00+00:00";

/// The environment every sandboxed run pins, so that commits, dates, colour
/// and line wrapping are the same everywhere, and no git starts automatic
/// maintenance or gc (`maintenance.auto=false`, `gc.auto=0`): a commit or
/// a push to a bare remote would otherwise leave a detached one writing the
/// repository while a test dumps it. A test sets the per-sandbox paths
/// beside them itself: `HOME`, `USERPROFILE`, `KLURIS_CONFIG`,
/// `GIT_CONFIG_GLOBAL` (the sandbox's git config) and the temp folder (`TMPDIR`,
/// `TEMP`, `TMP`). `TZ=UTC` makes Kluris's today the UTC
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
    ("GIT_CONFIG_COUNT", "2"),
    ("GIT_CONFIG_KEY_0", "maintenance.auto"),
    ("GIT_CONFIG_VALUE_0", "false"),
    ("GIT_CONFIG_KEY_1", "gc.auto"),
    ("GIT_CONFIG_VALUE_1", "0"),
    ("TZ", "UTC"),
    ("LC_ALL", "C.UTF-8"),
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("COLUMNS", "120"),
];

/// The variables a sandboxed run inherits from the test's own environment
/// (it starts from an empty one), so the binary and git can start.
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

/// Program output as a snapshot holds it:
/// - CRLF reads as LF (Windows' streams), a lone CR stays;
/// - every form of the temp `root` (see [`root_forms`]) becomes `<ROOT>`;
/// - a whole `--version` stdout, `kluris <version>` and its newline, becomes
///   `kluris <VERSION>`; a version anywhere else may be user data and stays;
/// - an `updated` value equal to today's UTC date (the date Kluris writes
///   under `TZ=UTC`, see [`PINNED_ENV`]) becomes `<TODAY>`.
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
/// A version field holds one version and nothing else, or `kluris` and one
/// version; a missing field or any other value fails the test. With version
/// fields the text must be laid out exactly as Kluris writes every `--json`
/// (`kluris_core::json`, Python's `json.dumps` layout: one line, `, ` and
/// `: ` separators, keys in their order, non-ASCII as `\uXXXX`, its final
/// newline if any), so writing the marked document back the same way
/// changes nothing but the marked values.
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

/// Every file under `dir`, sorted by its `/`-joined relative path: a
/// `== <path>` line, then its bytes as [`file_body`] writes them. A symlink
/// is `<symlink> <target>` and is not followed. Folders show only through
/// their files. `.git` is left out: its objects and index change with every
/// run, and [`git_dump`] shows what they record.
pub fn tree_dump(dir: &Path) -> String {
    tree_files(dir)
        .into_iter()
        .map(|(path, bytes)| format!("== {path}\n{}", file_body(&bytes)))
        .collect()
}

/// Every file under `dir` as [`tree_dump`] lists it, by its `/`-joined
/// relative path: its bytes, or `<symlink> <target>` and a newline for a
/// symlink.
pub fn tree_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    walk(dir, "", &mut files);
    files
}

/// A file's bytes as [`tree_dump`] writes them: as [`escape`] writes them (a
/// CR, a BOM and a byte that is not UTF-8 visible; a backslash doubled),
/// then a `<no newline at end>` line when they do not end in a newline.
pub fn file_body(bytes: &[u8]) -> String {
    let mut body = escape(bytes);
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        body.push_str("\n<no newline at end>\n");
    }
    body
}

fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
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
            let mut link = b"<symlink> ".to_vec();
            link.extend_from_slice(target.as_os_str().as_encoded_bytes());
            link.push(b'\n');
            out.insert(rel, link);
        } else if meta.is_dir() {
            walk(&path, &format!("{rel}/"), out);
        } else if meta.is_file() {
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            out.insert(rel, bytes);
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

/// How the repository at `repo` is set up and what its HEAD holds, each
/// under a `== <command>` line: the HEAD commit (`git cat-file -p HEAD`)
/// without its `tree` line, the mode, type and path of every file it holds
/// (`git ls-tree -r HEAD` without the object ids), its branch (`git
/// symbolic-ref HEAD`), its remotes (`git remote -v`) and its work tree's
/// state (`git status --porcelain --ignored`). The ids are left out because
/// a brain's files hold today's date, so their ids change every day; the
/// committed bytes are the work tree's whenever the status lists no change
/// to them, and a tree dump shows those. A command that fails is `<exit N>`
/// (git words its errors by version, so its stderr is left out); a folder
/// without `.git` is `<no repository>`.
pub fn git_state(repo: &Path) -> String {
    if !repo.join(".git").exists() {
        return "<no repository>\n".to_string();
    }
    let commands: [&[&str]; 5] = [
        &["cat-file", "-p", "HEAD"],
        &["ls-tree", "-r", "HEAD"],
        &["symbolic-ref", "HEAD"],
        &["remote", "-v"],
        &["status", "--porcelain", "--ignored"],
    ];
    let mut out = String::new();
    for args in commands {
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
        out.push_str(&format!("== git {}\n", args.join(" ")));
        let text = escape(&run.stdout);
        match (run.status.code(), args[0]) {
            (Some(0), "cat-file") => text
                .lines()
                .filter(|line| !line.starts_with("tree "))
                .for_each(|line| out.push_str(&format!("{line}\n"))),
            // `<mode> <type> <id>\t<path>`
            (Some(0), "ls-tree") => text.lines().for_each(|line| {
                let (meta, path) = line.split_once('\t').unwrap_or((line, ""));
                let fields: Vec<&str> = meta.split(' ').collect();
                out.push_str(&format!("{} {}\t{path}\n", fields[0], fields[1]));
            }),
            (Some(0), _) => out.push_str(&text),
            (code, _) => out.push_str(&format!("<exit {}>\n", code.unwrap_or(-1))),
        }
    }
    out
}

/// The text forms of `root` a program can print or a file can hold, longest
/// first so a resolved form is not half-replaced by a shorter alias: as
/// given and resolved (macOS `/var` is `/private/var`), each with its native
/// and, on Windows, `/` separators; each of those also JSON-escaped; and
/// each of all these as [`tree_dump`] writes it, backslashes doubled.
fn root_forms(root: &Path) -> Vec<String> {
    let mut paths = vec![root.display().to_string()];
    if let Ok(resolved) = fs::canonicalize(root) {
        let resolved = resolved.display().to_string();
        // Windows' verbatim prefix, which programs do not print.
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
            let escaped = quoted[1..quoted.len() - 1].to_string();
            for f in [form, escaped] {
                forms.insert(f.replace('\\', "\\\\"));
                forms.insert(f);
            }
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

/// A parsed JSON document as `kluris_core::json` holds it, to be written in
/// Kluris's `--json` layout.
pub fn python_json(value: &serde_json::Value) -> Json {
    use serde_json::Value;
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => Json::Int(i),
            (None, Some(u), _) => Json::BigInt(u.to_string()),
            (None, None, Some(f)) => Json::Float(f),
            _ => panic!("JSON number {n} is neither an integer nor a float"),
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

/// Bytes as text that shows them: a CR as `\r`, U+FEFF (a BOM) as
/// `\u{feff}`, a byte that is not UTF-8 as `\xNN`, and a backslash as `\\`,
/// so each escape can only mean what it stands for.
fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\\' => out.push_str("\\\\"),
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

/// A path written in the OS's own form (`\` on Windows) shown as `shown`,
/// in each form text can hold it: raw, JSON-escaped, and each of those as
/// [`tree_dump`] writes it (backslashes doubled). The snapshot then holds
/// on every OS; the path written with `/` where the OS writes `\` fails
/// the test, and a path in any other form still shows.
pub fn native_path(text: &str, native: &Path, shown: &str) -> String {
    let escaped = |s: &str| {
        let quoted = json::to_string(&json::Json::Str(s.to_string()));
        quoted[1..quoted.len() - 1].to_string()
    };
    let dumped = |s: &str| s.replace('\\', "\\\\");
    let raw = native.display().to_string();
    let slashed = raw.replace('\\', "/");
    if slashed != raw {
        for form in [escaped(&slashed), slashed.clone()] {
            assert!(
                !text.contains(&form),
                "{form} is written with `/`, not as the OS writes it ({raw}), in {text:?}"
            );
        }
    }
    let mut forms = [
        (dumped(&escaped(&raw)), dumped(&escaped(shown))),
        (escaped(&raw), escaped(shown)),
        (dumped(&raw), dumped(shown)),
        (raw.clone(), shown.to_string()),
    ];
    forms.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    forms
        .iter()
        .fold(text.to_string(), |text, (from, to)| text.replace(from, to))
}
