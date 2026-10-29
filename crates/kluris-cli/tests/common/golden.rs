//! Golden-test helpers. A snapshot must hold the same text on every machine
//! and every day, yet show every byte Kluris chose: the normalizations
//! replace only what the run's place and time decide (the temp root, the
//! version of the binary, today's date, the OS's line endings in its
//! streams) with markers, and the dumps of a folder and of a git repository
//! write every byte so that a changed line ending, BOM or encoding shows.
//!
//! Program output goes through [`decode`] (stdout, stderr) and [`normalize`]
//! or [`normalize_json`]. File bytes go through a [`Dump`] ([`tree_dump`],
//! [`files_dump`]), never raw into `normalize`: the dump escapes them (a CR
//! as the two characters `\r`) before it puts each body through the
//! normalization it is given, so `tree_dump(dir, |t| normalize(t, root))`
//! replaces the root and today's date while a file's CRLF stays in the
//! snapshot. Where a test asserts that nothing changed it compares the
//! [`Tree`]s themselves.

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
///   a form written with `\` that goes on with `/` (a path below the root
///   written half in the OS's form) fails the test;
/// - a whole `--version` stdout, `kluris <version>` and its newline, becomes
///   `kluris <VERSION>`; a version anywhere else may be user data and stays;
/// - an `updated` value equal to today's UTC date (the date Kluris writes
///   under `TZ=UTC`, see [`PINNED_ENV`]) becomes `<TODAY>`.
pub fn normalize(text: &str, root: &Path) -> String {
    let mut text = text.replace("\r\n", "\n");
    for path in root_paths(root).iter().filter(|path| path.contains('\\')) {
        for form in path_forms(path) {
            assert!(
                !text.contains(&format!("{form}/")),
                "{form} goes on with `/`, not as the OS writes a path ({path}), in {text:?}"
            );
        }
    }
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

/// One entry of a folder as [`tree_files`] reads it: a file's bytes, or a
/// symlink's target (not followed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    File(Vec<u8>),
    Symlink(Vec<u8>),
}

impl Entry {
    /// The file's bytes, or the symlink's target.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Entry::File(bytes) | Entry::Symlink(bytes) => bytes,
        }
    }

    /// The file's bytes; a symlink fails the test.
    pub fn file(&self) -> &[u8] {
        match self {
            Entry::File(bytes) => bytes,
            Entry::Symlink(target) => panic!(
                "a symlink to {} where a file was expected",
                String::from_utf8_lossy(target)
            ),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Entry::File(_) => "file",
            Entry::Symlink(_) => "symlink",
        }
    }
}

/// Every file and symlink under `dir`, by its `/`-joined relative path.
/// Folders show only through what they hold. `.git` is left out: its
/// objects and index change with every run, and [`git_dump`] shows what
/// they record.
pub type Tree = BTreeMap<String, Entry>;

/// The [`Tree`] of `dir`.
pub fn tree_files(dir: &Path) -> Tree {
    let mut files = BTreeMap::new();
    walk(dir, "", &mut files);
    files
}

/// Every entry of `dir` as [`Dump`] writes it, each body through `show`.
pub fn tree_dump(dir: &Path, show: impl Fn(&str) -> String) -> String {
    files_dump(&tree_files(dir), show)
}

/// Every entry of `tree` as [`Dump`] writes it, each body through `show`.
pub fn files_dump(tree: &Tree, show: impl Fn(&str) -> String) -> String {
    let mut dump = Dump::new(show);
    for (path, entry) in tree {
        dump.entry(path, entry);
    }
    dump.end()
}

/// Entries of a folder as text in which no two different contents read
/// the same, nor any content as a record of the dump's own:
/// - each entry starts with a header line `== PATH (KIND, N bytes)`, KIND
///   `file` or `symlink`, N the length of the bytes its body shows, so a
///   header is read from its end and no path can imitate one;
/// - a file's body is its bytes as [`escape`] writes them, put through the
///   dump's `show` (the root, a path the OS writes in its own form, today's
///   date as markers, so N counts a marker as written), with a `<` or `=`
///   that starts a line written `\<` or `\=`: a body line never starts
///   like a header or a marker line. Bytes that do not end in a newline
///   are followed by a `<no newline at end>` line;
/// - a symlink's body is one line, its target written the same way with a
///   newline as `\n`;
/// - an entry shown by a [`Reference`] keeps its header, N as the
///   reference's own text shows the bytes, and has its one marker line
///   (`<vendored/...>`, `<snapshot ...>`) for a body; a removed entry is
///   `== PATH (removed)`;
/// - the dump ends with an `<end of dump>` line, so the last body's
///   trailing blank lines are inside the snapshot, which insta compares
///   with its trailing whitespace trimmed.
pub struct Dump<F> {
    show: F,
    out: String,
}

impl<F: Fn(&str) -> String> Dump<F> {
    /// An empty dump whose bodies go through `show`.
    pub fn new(show: F) -> Self {
        Dump {
            show,
            out: String::new(),
        }
    }

    /// `entry` at `path`, its body written in full.
    pub fn entry(&mut self, path: &str, entry: &Entry) {
        let body = self.body(entry);
        self.header(path, entry, body.len);
        self.out.push_str(&body.text);
    }

    /// `entry` at `path`, shown by `reference` in place of its body.
    pub fn reference(&mut self, path: &str, entry: &Entry, reference: &Reference) {
        let marker = &reference.marker;
        assert!(
            marker.starts_with('<') && marker.ends_with('>') && !marker.contains('\n'),
            "a reference is one `<...>` line: {marker:?}"
        );
        self.header(path, entry, reference.len);
        self.out.push_str(&format!("{marker}\n"));
    }

    /// The entry at `path`, gone.
    pub fn removed(&mut self, path: &str) {
        self.out
            .push_str(&format!("== {} (removed)\n", escape_line(path.as_bytes())));
    }

    /// The dump, closed by its `<end of dump>` line.
    pub fn end(mut self) -> String {
        self.out.push_str("<end of dump>\n");
        self.out
    }

    fn header(&mut self, path: &str, entry: &Entry, len: usize) {
        self.out.push_str(&format!(
            "== {} ({}, {})\n",
            escape_line(path.as_bytes()),
            entry.kind(),
            byte_count(len)
        ));
    }

    fn body(&self, entry: &Entry) -> Body {
        match entry {
            Entry::File(bytes) => file_body(bytes, &self.show),
            Entry::Symlink(target) => {
                let text = guard_line_starts(&(self.show)(&escape_line(target)));
                let len = shown_len(&text);
                Body {
                    text: format!("{text}\n"),
                    len,
                }
            }
        }
    }
}

/// A file's body as a [`Dump`] writes it, and the length of the bytes it
/// shows.
#[derive(Debug, PartialEq, Eq)]
pub struct Body {
    pub text: String,
    pub len: usize,
}

/// A one-line `<...>` marker a [`Dump`] writes in place of an entry's body,
/// and the length of the bytes it stands for, as the marker's own text
/// shows them (a shared snapshot's body, a vendored file's bytes).
pub struct Reference {
    pub marker: String,
    pub len: usize,
}

/// `N bytes` (`1 byte`).
pub fn byte_count(len: usize) -> String {
    if len == 1 {
        "1 byte".to_string()
    } else {
        format!("{len} bytes")
    }
}

/// A file's bytes as a [`Dump`] writes its body, through `show`.
pub fn file_body(bytes: &[u8], show: impl Fn(&str) -> String) -> Body {
    let mut text = guard_line_starts(&show(&escape(bytes)));
    let len = shown_len(&text);
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str("\n<no newline at end>\n");
    }
    Body { text, len }
}

/// `text` with a `<` or `=` that starts a line written `\<` or `\=`.
fn guard_line_starts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if line.starts_with(['<', '=']) {
            out.push('\\');
        }
        out.push_str(line);
    }
    out
}

/// The number of bytes `text`, a body as a [`Dump`] writes it, shows: each
/// escape counts as the bytes it stands for.
fn shown_len(text: &str) -> usize {
    let mut len = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            len += c.len_utf8();
            continue;
        }
        len += match chars.next() {
            Some('\\' | 'r' | 'n' | '<' | '=') => 1,
            Some('x') => {
                chars.nth(1);
                1
            }
            Some('u') => {
                let rest: String = chars.by_ref().take(6).collect();
                assert_eq!(rest, "{feff}", "an unknown escape in {text:?}");
                3
            }
            other => panic!("an unknown escape \\{other:?} in {text:?}"),
        };
    }
    len
}

fn walk(dir: &Path, prefix: &str, out: &mut Tree) {
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
            let target = target.as_os_str().as_encoded_bytes().to_vec();
            out.insert(rel, Entry::Symlink(target));
        } else if meta.is_dir() {
            walk(&path, &format!("{rel}/"), out);
        } else if meta.is_file() {
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            out.insert(rel, Entry::File(bytes));
        }
    }
}

/// The HEAD commit of `repo` (`git cat-file -p HEAD`) and the tree it
/// records (`git ls-tree -r HEAD`), each under a `== <command>` line, with
/// the bytes written as [`escape`] writes them. Its commit and tree ids
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

/// `root` as given and resolved (macOS `/var` is `/private/var`), as the
/// OS writes each.
fn root_paths(root: &Path) -> Vec<String> {
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
    paths
}

/// The text forms of `root` a program can print or a file can hold, longest
/// first so a resolved form is not half-replaced by a shorter alias: as
/// given and resolved (macOS `/var` is `/private/var`), each with its native
/// and, on Windows, `/` separators; each of those also JSON-escaped; and
/// each of all these as a [`Dump`] writes it, backslashes doubled.
fn root_forms(root: &Path) -> Vec<String> {
    let mut forms = BTreeSet::new();
    for path in root_paths(root) {
        let posix = if cfg!(windows) {
            path.replace('\\', "/")
        } else {
            path.clone()
        };
        for form in [path, posix] {
            let escaped = json_escaped(&form);
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
/// line (plain or quoted, its CR as a [`Dump`] writes it) and a JSON key.
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

/// Bytes as [`escape`] writes them, with a newline as `\n`: one line.
fn escape_line(bytes: &[u8]) -> String {
    escape(bytes).replace('\n', "\\n")
}

/// A path written in the OS's own form (`\` on Windows) shown as `shown`,
/// in each form text can hold it: raw, JSON-escaped, and each of those as
/// a dump writes it (backslashes doubled). The snapshot then holds on every
/// OS, and the path written any other way than the OS writes it fails the
/// test (see [`NativePaths`]).
pub fn native_path(text: &str, native: &Path, shown: &str) -> String {
    native_paths(text, &[(native, shown)])
}

/// [`native_path`] for several paths at once (see [`NativePaths`]).
pub fn native_paths(text: &str, paths: &[(&Path, &str)]) -> String {
    NativePaths::new(paths).show(text)
}

/// Paths written in the OS's own form, each shown as given. Every path is
/// checked on the text as it came, whole: where the OS writes `\`, each
/// spelling of the path's parts joined by `\`, `\\` or `/` must be one of
/// the path's own forms, and none of those forms may go on with `/` (a
/// path below it written half in the OS's form). Then each form is
/// replaced, longest first, so a path inside another (a brain and its
/// `kluris.yml`) is neither half-replaced nor taken for one written with
/// `/`.
pub struct NativePaths {
    replacements: Vec<(String, String)>,
    checks: Vec<(String, Vec<String>, Vec<Regex>)>,
}

impl NativePaths {
    pub fn new(paths: &[(&Path, &str)]) -> Self {
        let mut replacements = Vec::new();
        let mut checks = Vec::new();
        for (native, shown) in paths {
            let raw = native.display().to_string();
            replacements.extend(path_forms(&raw).into_iter().zip(path_forms(shown)));
            if raw.contains('\\') {
                let parts: Vec<&str> = raw.split(['\\', '/']).collect();
                let spellings: BTreeSet<String> = [
                    |p: &str| p.to_string(),
                    |p: &str| json_escaped(p),
                    |p: &str| json_escaped(p).replace('\\', "\\\\"),
                ]
                .iter()
                .map(|form| {
                    parts
                        .iter()
                        .map(|part| regex::escape(&form(part)))
                        .collect::<Vec<_>>()
                        .join(r"(?:\\+|/)")
                })
                .collect();
                let forms = path_forms(&raw).into_iter().collect();
                checks.push((raw, forms, spellings.iter().map(|p| re(p)).collect()));
            }
        }
        replacements.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        NativePaths {
            replacements,
            checks,
        }
    }

    /// `text` with every path checked, then shown as given.
    pub fn show(&self, text: &str) -> String {
        for (raw, forms, spellings) in &self.checks {
            for found in spellings.iter().flat_map(|s| s.find_iter(text)) {
                assert!(
                    forms.iter().any(|form| form == found.as_str()),
                    "{} is {raw} written with other separators than the OS writes, in {text:?}",
                    found.as_str()
                );
            }
            for form in forms {
                assert!(
                    !text.contains(&format!("{form}/")),
                    "{form} goes on with `/`, not as the OS writes a path ({raw}), in {text:?}"
                );
            }
        }
        self.replacements
            .iter()
            .fold(text.to_string(), |text, (from, to)| text.replace(from, to))
    }
}

/// Fails the test unless each place `rendered` (`printed` as a snapshot
/// holds it) shows `shown`, raw or JSON-escaped, stands for the path
/// `native` that `printed` holds in one of its own forms: as many as
/// `printed` holds. A `shown` that a normalization made out of the path
/// written another way (the root written as the OS writes it, the rest with
/// `/`) is one too many.
pub fn assert_shown_from_native(printed: &str, rendered: &str, native: &Path, shown: &str) {
    let count = |text: &str, forms: &[String]| -> usize {
        let forms: BTreeSet<&String> = forms.iter().collect();
        forms
            .iter()
            .map(|form| text.matches(form.as_str()).count())
            .sum()
    };
    let raw = native.display().to_string();
    let forms = path_forms(&raw);
    let natives = count(printed, &forms[1..]);
    let shown_forms = [json_escaped(shown), shown.to_string()];
    let shown_count = count(rendered, &shown_forms);
    assert_eq!(
        shown_count, natives,
        "{shown} shows {shown_count} times, but {raw} is printed {natives} times in its own form, in {printed:?}"
    );
}

/// The forms text can hold the path `raw` in: JSON-escaped and as a dump
/// writes it (backslashes doubled), each of the two, and raw; longest first.
fn path_forms(raw: &str) -> Vec<String> {
    let dumped = |s: &str| s.replace('\\', "\\\\");
    vec![
        dumped(&json_escaped(raw)),
        json_escaped(raw),
        dumped(raw),
        raw.to_string(),
    ]
}

/// `s` as a JSON string holds it, without its quotes.
fn json_escaped(s: &str) -> String {
    let quoted = json::to_string(&Json::Str(s.to_string()));
    quoted[1..quoted.len() - 1].to_string()
}

/// Runs `check` with its snapshots named `<name>@windows` on Windows: for
/// an output that differs there for a reason of the OS's own, which the
/// caller names (a folder listed without regard to case, a symlink the test
/// cannot make there, the OS's own error text). A path the OS writes in its
/// own form is no such reason: [`native_paths`] shows it as on every OS.
pub fn windows_variant(check: impl FnOnce()) {
    if cfg!(windows) {
        insta::with_settings!({ snapshot_suffix => "windows" }, { check() });
    } else {
        check();
    }
}

/// `path` with `/` between its parts, as a snapshot shows a path the OS
/// writes in its own form.
pub fn slashed(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
