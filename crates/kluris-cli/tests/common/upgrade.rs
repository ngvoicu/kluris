//! The homes an earlier Kluris set up, frozen as plain files under
//! `tests/fixtures/upgrade-home/<world>/`, made live in a temp folder:
//!
//! - `home/` is copied to `<root>/work/home`, with `@WORK@` (the folder
//!   the home was set up in) replaced by `<root>/work`: native in
//!   `.kluris/config.yml`, which holds the brain paths as the OS writes
//!   them, and with `/` separators everywhere else (the skills and the
//!   Windsurf workflows write paths in POSIX form);
//! - each brain is rebuilt in `<root>/work/brains/<name>`: `commit-<n>/` holds
//!   the files commit n adds or changes, committed in order with the pinned
//!   identity and date of the sandbox, then `worktree/` holds the files
//!   on disk that differ from the last commit (untracked, ignored or changed).
//!
//! A world is a [`Sandbox`]: its commands run with only the pinned
//! environment and the world's own paths, and their output and the files
//! they change are written as snapshot text: [`World::render`] and
//! [`World::changes_since`].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use regex::Regex;

use super::golden::{PINNED_GIT_DATE, decode, file_body, normalize, tree_files};
use super::sandbox::{Run, Sandbox, copy_tree, fixture, write};

/// A brain of a frozen world: its commit messages, oldest first (none: a
/// brain without git), and the commit id its last commit must get.
pub struct Brain {
    pub name: &'static str,
    pub commits: &'static [&'static str],
    pub head: Option<&'static str>,
}

const PLACEHOLDER: &str = "@WORK@";

static GIT_VERSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"git version [^"\n]*"#).expect("git version pattern"));

/// A [`Sandbox`] holding a frozen world.
pub struct World {
    sb: Sandbox,
}

impl Deref for World {
    type Target = Sandbox;

    fn deref(&self) -> &Sandbox {
        &self.sb
    }
}

/// The `kluris` under test.
pub fn kluris_bin() -> PathBuf {
    PathBuf::from(assert_cmd::cargo::cargo_bin!("kluris"))
}

impl World {
    /// An empty world: a home and nothing in it.
    pub fn empty() -> World {
        World { sb: Sandbox::new() }
    }

    /// The frozen world `name` with its `brains`, made live.
    pub fn frozen(name: &str, brains: &[Brain]) -> World {
        let world = World::empty();
        let src = fixture("upgrade-home").join(name);
        let home = src.join("home");
        for rel in tree_files(&home).into_keys() {
            let bytes = fs::read(home.join(&rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
            write(&world.home.join(&rel), &world.place(&rel, bytes));
        }
        for brain in brains {
            world.rebuild(&src.join("brains").join(brain.name), brain);
        }
        world
    }

    /// The frozen bytes of the home file `rel` with `@WORK@` replaced.
    fn place(&self, rel: &str, bytes: Vec<u8>) -> Vec<u8> {
        let Ok(text) = String::from_utf8(bytes.clone()) else {
            return bytes;
        };
        if !text.contains(PLACEHOLDER) {
            return bytes;
        }
        let text = if rel == ".kluris/config.yml" {
            let native = Regex::new(r"@WORK@((?:/[^/\s]+)*)").expect("pattern");
            native
                .replace_all(&text, |c: &regex::Captures| {
                    let mut path = self.work.clone();
                    path.extend(c[1].split('/').filter(|p| !p.is_empty()));
                    path.display().to_string()
                })
                .into_owned()
        } else {
            text.replace(
                PLACEHOLDER,
                &self.work.display().to_string().replace('\\', "/"),
            )
        };
        text.into_bytes()
    }

    fn rebuild(&self, src: &Path, brain: &Brain) {
        let dest = self.brain(brain.name);
        fs::create_dir_all(&dest).expect("brain");
        let copy = |step: &str| {
            let from = src.join(step);
            assert!(from.is_dir(), "{} has no {step}/", src.display());
            copy_tree(&from, &dest);
        };
        let git = |args: &[&str]| self.git(&dest, args, PINNED_GIT_DATE);
        if !brain.commits.is_empty() {
            git(&["init", "-q", "-b", "main"]);
        }
        for (n, message) in brain.commits.iter().enumerate() {
            copy(&format!("commit-{}", n + 1));
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", message]);
        }
        if src.join("worktree").is_dir() {
            copy("worktree");
        }
        if let Some(head) = brain.head {
            assert_eq!(
                git(&["rev-parse", "HEAD"]).trim_end(),
                head,
                "{}: the rebuilt history is not the one the earlier Kluris made",
                brain.name
            );
        }
    }

    pub fn brain(&self, name: &str) -> PathBuf {
        self.work.join("brains").join(name)
    }

    /// PATH with the folder of the `kluris` under test first, then the
    /// test's own PATH (for git).
    pub fn default_path(&self) -> OsString {
        let own = std::env::var_os("PATH").unwrap_or_default();
        let bin = kluris_bin();
        let dirs = std::iter::once(bin.parent().expect("bin dir").to_path_buf())
            .chain(std::env::split_paths(&own));
        std::env::join_paths(dirs).expect("PATH")
    }

    /// `kluris <args>` in `work/`, with PATH [`World::default_path`] and the
    /// `extra` variables.
    pub fn kluris(&self, args: &[&str], extra: &[(&str, &Path)]) -> Run {
        self.kluris_on(self.default_path(), args, extra)
    }

    /// `kluris <args>` in `work/`, with `path` as the whole PATH.
    pub fn kluris_on(&self, path: OsString, args: &[&str], extra: &[(&str, &Path)]) -> Run {
        let mut cmd = Command::new(kluris_bin());
        let out = self
            .env(&mut cmd)
            .env("PATH", path)
            .envs(extra.iter().map(|(k, v)| (*k, v.as_os_str())))
            .args(args)
            .current_dir(&self.work)
            .stdin(Stdio::null())
            .output()
            .expect("kluris");
        let what = format!("kluris {}", args.join(" "));
        Run {
            code: out.status.code().unwrap_or(-1),
            stdout: decode(&out.stdout, &format!("stdout of `{what}`")),
            stderr: decode(&out.stderr, &format!("stderr of `{what}`")),
        }
    }

    /// A run as [`Run::render`] writes it, after what the machine decides
    /// beyond the temp root: the path of the `kluris` under test (as printed
    /// and JSON-escaped) becomes `<KLURIS>`, and git's version
    /// `git version <GIT>`.
    pub fn render(&self, run: &Run) -> String {
        let bin = kluris_bin().display().to_string();
        let mark = |text: &str| {
            let text = text
                .replace(&bin.replace('\\', "\\\\"), "<KLURIS>")
                .replace(&bin, "<KLURIS>");
            GIT_VERSION
                .replace_all(&text, "git version <GIT>")
                .into_owned()
        };
        Run {
            code: run.code,
            stdout: mark(&run.stdout),
            stderr: mark(&run.stderr),
        }
        .render(&self.root)
    }

    /// Every file under `work/` as [`tree_files`] reads it.
    pub fn files(&self) -> BTreeMap<String, Vec<u8>> {
        tree_files(&self.work)
    }

    /// What changed under `work/` since `before` ([`World::files`]), as
    /// [`changes`] writes it, through [`normalize`].
    pub fn changes_since(&self, before: &BTreeMap<String, Vec<u8>>) -> String {
        normalize(&changes(before, &self.files()), &self.root)
    }
}

/// What changed from `before` to `after` (both [`tree_files`]), by path:
/// `== + <path>` (added) and `== ~ <path>` (changed) with the new bytes as
/// [`file_body`] writes them, or `<same as <path>>` when an earlier entry of
/// this listing has the same bytes; `== - <path>` (removed).
/// `<nothing changed>` when nothing did.
pub fn changes(before: &BTreeMap<String, Vec<u8>>, after: &BTreeMap<String, Vec<u8>>) -> String {
    let mut out = String::new();
    let mut shown: BTreeMap<&[u8], &str> = BTreeMap::new();
    let paths: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for path in paths {
        match (before.get(path), after.get(path)) {
            (Some(_), None) => out.push_str(&format!("== - {path}\n")),
            (old, Some(new)) if old != Some(new) => {
                let mark = if old.is_some() { '~' } else { '+' };
                out.push_str(&format!("== {mark} {path}\n"));
                match shown.get(new.as_slice()) {
                    Some(first) => out.push_str(&format!("<same as {first}>\n")),
                    None => {
                        out.push_str(&file_body(new));
                        shown.insert(new, path);
                    }
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        out.push_str("<nothing changed>\n");
    }
    out
}
