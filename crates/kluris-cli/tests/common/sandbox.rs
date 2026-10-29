//! A sandboxed `kluris` for golden tests: a temp root that holds the run's
//! HOME, config, brains and temp folder, an environment built from nothing
//! but [`PINNED_ENV`] and [`PASSTHROUGH_ENV`], and each run's exit code and
//! streams rendered as one snapshot text.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kluris_core::json;

use super::golden::{PASSTHROUGH_ENV, PINNED_ENV, decode, normalize, python_json};

/// The length of every sandbox root, as Kluris prints it. A table whose
/// column widths follow a path's length (`list`) is then laid out the same
/// on every machine once the root reads `<ROOT>`.
pub const ROOT_LEN: usize = 120;

/// One test's sandbox. Everything a run touches lives under `work/`: HOME
/// (`work/home`), the config (`work/home/.kluris/config.yml`) and the
/// brains a test builds; the root also holds an empty global git config and
/// the temp folder.
pub struct Sandbox {
    _tmp: tempfile::TempDir,
    /// The resolved root, padded to [`ROOT_LEN`].
    pub root: PathBuf,
    pub work: PathBuf,
    pub home: PathBuf,
    pub config: PathBuf,
}

impl Sandbox {
    pub fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = resolved(tmp.path());
        let used = base.display().to_string().chars().count() + 1;
        assert!(
            used < ROOT_LEN,
            "the temp folder {} is longer than a sandbox root may be ({ROOT_LEN})",
            base.display()
        );
        let root = base.join("0".repeat(ROOT_LEN - used));
        let work = root.join("work");
        let home = work.join("home");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(root.join("tmp")).unwrap();
        fs::write(root.join("gitconfig-empty"), b"").unwrap();
        let config = home.join(".kluris").join("config.yml");
        Sandbox {
            _tmp: tmp,
            root,
            work,
            home,
            config,
        }
    }

    /// `cmd` with the sandbox's environment and nothing else.
    pub fn env<'a>(&self, cmd: &'a mut Command) -> &'a mut Command {
        cmd.env_clear();
        for key in PASSTHROUGH_ENV {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        let tmp = self.root.join("tmp");
        cmd.envs(PINNED_ENV.iter().copied())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("KLURIS_CONFIG", &self.config)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig-empty"))
            .env("TMPDIR", &tmp)
            .env("TEMP", &tmp)
            .env("TMP", &tmp)
    }

    /// `kluris args` in `work/`, its stdin an empty pipe.
    pub fn kluris(&self, args: &[&str]) -> Run {
        Run::of(self.command(args))
    }

    /// `kluris args` in `work/` with the sandbox's environment and its stdin
    /// an empty pipe, for a test that changes either before it runs it with
    /// [`Run::of`].
    pub fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
        self.env(&mut cmd)
            .args(args)
            .current_dir(&self.work)
            .stdin(Stdio::piped());
        cmd
    }

    /// `git args` in `cwd` with the sandbox's identity, its commits dated
    /// `date`; a failure fails the test. Returns stdout.
    pub fn git(&self, cwd: &Path, args: &[&str], date: &str) -> String {
        let mut cmd = Command::new("git");
        let out = self
            .env(&mut cmd)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// A new repository in `repo` on the branch `main`, line endings as
    /// written.
    pub fn init(&self, repo: &Path) {
        let date = super::PINNED_GIT_DATE;
        self.git(repo, &["init", "-q"], date);
        self.git(repo, &["config", "core.autocrlf", "false"], date);
        self.git(repo, &["checkout", "-q", "-b", "main"], date);
    }

    /// Commit everything in `repo` at `date`.
    pub fn commit(&self, repo: &Path, message: &str, date: &str) {
        self.git(repo, &["add", "-A"], date);
        self.git(
            repo,
            &["commit", "-q", "--allow-empty", "-m", message],
            date,
        );
    }

    /// `git status --porcelain` of `repo`, paths unquoted.
    pub fn porcelain(&self, repo: &Path) -> String {
        self.git(
            repo,
            &["-c", "core.quotePath=false", "status", "--porcelain"],
            super::PINNED_GIT_DATE,
        )
    }

    /// `config.yml` with `(name, path, description)` brains in order, each
    /// value a YAML double-quoted scalar; no brains is `brains: {}`.
    pub fn write_config(&self, brains: &[(&str, &Path, &str)]) {
        let mut lines = vec![if brains.is_empty() {
            "brains: {}".to_string()
        } else {
            "brains:".to_string()
        }];
        for (name, path, description) in brains {
            lines.push(format!("  {}:", quoted(name)));
            lines.push(format!("    path: {}", quoted(&path.display().to_string())));
            lines.push(format!("    description: {}", quoted(description)));
        }
        fs::create_dir_all(self.config.parent().unwrap()).unwrap();
        fs::write(&self.config, lines.join("\n") + "\n").unwrap();
    }
}

/// Writes each `(relative path, text)` under `root`, creating folders.
pub fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (rel, text) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

/// Writes `bytes` to `path`, creating its folders.
pub fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Runs `check` with its snapshots named for the paths this OS prints: a
/// configured path is printed as written, with native separators, so on
/// Windows (`\`) such a snapshot is the `windows` variant of the one macOS
/// and Linux share.
pub fn printed_paths(check: impl FnOnce()) {
    #[allow(unused_mut)]
    let mut settings = insta::Settings::clone_current();
    #[cfg(windows)]
    settings.set_snapshot_suffix("windows");
    settings.bind(check);
}

/// `tests/fixtures/<name>` of this crate.
pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Copies the folder `from` to `to`, byte for byte, creating folders.
pub fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|e| panic!("{}: {e}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|e| panic!("{}: {e}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest)
                .unwrap_or_else(|e| panic!("{}: {e}", entry.path().display()));
        }
    }
}

/// How a run ended: its exit code and its decoded streams.
#[derive(Debug)]
pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    /// Runs `cmd` (a piped stdin is closed at once, an empty pipe) and
    /// collects how it ended.
    pub fn of(mut cmd: Command) -> Self {
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy()).collect();
        let what = format!("kluris {}", args.join(" "));
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        if let Some(mut stdin) = child.stdin.take() {
            stdin.flush().unwrap();
        }
        let out = child.wait_with_output().unwrap();
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: decode(&out.stdout, &format!("stdout of `{what}`")),
            stderr: decode(&out.stderr, &format!("stderr of `{what}`")),
        }
    }

    /// The run as a snapshot holds it: each stream [`normalize`]d under a
    /// `== stdout` / `== stderr` line (a stream that does not end in a
    /// newline is followed by `<no newline at end>`), then `== exit N`, so
    /// every stream's last byte shows.
    pub fn render(&self, root: &Path) -> String {
        let mut out = String::new();
        for (name, text) in [("stdout", &self.stdout), ("stderr", &self.stderr)] {
            let text = normalize(text, root);
            out.push_str(&format!("== {name}\n{text}"));
            if !text.is_empty() && !text.ends_with('\n') {
                out.push_str("\n<no newline at end>\n");
            }
        }
        out.push_str(&format!("== exit {}\n", self.code));
        out
    }
}

/// `args` as a shell would take them back: a word with nothing a shell
/// reads as is, anything else (an empty word too) in single quotes.
pub fn command_line(args: &[&str]) -> String {
    let words: Vec<String> = args
        .iter()
        .map(|a| {
            let plain = !a.is_empty()
                && a.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_=./,:".contains(c));
            if plain {
                a.to_string()
            } else {
                format!("'{}'", a.replace('\'', r"'\''"))
            }
        })
        .collect();
    std::iter::once("kluris".to_string())
        .chain(words)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A YAML double-quoted scalar, written as JSON writes a string.
fn quoted(s: &str) -> String {
    kluris_core::json::to_string(&kluris_core::json::Json::Str(s.to_string()))
}

/// `path` resolved, without Windows' verbatim prefix (which programs do not
/// print).
fn resolved(path: &Path) -> PathBuf {
    let path = fs::canonicalize(path).unwrap();
    let text = path.display().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Write each `(relative path, bytes)` under `dir`, making its folders.
pub fn write_tree<P: AsRef<str>, B: AsRef<[u8]>>(
    dir: &Path,
    files: impl IntoIterator<Item = (P, B)>,
) {
    for (rel, bytes) in files {
        let path = dir.join(rel.as_ref());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
}

/// The paths whose bytes differ between two [`super::tree_files`], each marked
/// `+` (new), `-` (gone) or `~` (changed); `(none)` when none do.
pub fn changed_files(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> String {
    let mut lines = String::new();
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for path in paths {
        let mark = match (before.get(path), after.get(path)) {
            (None, Some(_)) => '+',
            (Some(_), None) => '-',
            (Some(a), Some(b)) if a != b => '~',
            _ => continue,
        };
        lines.push_str(&format!("{mark} {path}\n"));
    }
    if lines.is_empty() {
        lines.push_str("(none)\n");
    }
    lines
}

/// A `--json` stdout as a snapshot shows it: checked to be exactly one line
/// of `json.dumps` layout (`kluris_core::json::to_line` of its parse), then
/// written with an indent of 2 so that each value has its own line. The
/// root and today are normalized first.
pub fn json_dump(stdout: &str, root: &Path) -> String {
    let text = normalize(stdout, root);
    let doc: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {text:?}"));
    let value = python_json(&doc);
    assert_eq!(
        text,
        json::to_line(&value),
        "stdout is not one line of json.dumps of its parse"
    );
    json::to_string_indented(&value, 2) + "\n"
}

/// A stream as the snapshot of a run shows it: as it is, with a
/// `<no newline at end>` line when it does not end in one.
pub fn stream(text: &str) -> String {
    if text.is_empty() || text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n<no newline at end>\n")
    }
}
