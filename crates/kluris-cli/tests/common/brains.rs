//! Brains for a [`Sandbox`] to read: the fixture brains copied in, and a
//! brain cloned from a local bare remote a teammate pushes to.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::golden::PINNED_GIT_DATE;
use super::sandbox::{Sandbox, copy_tree, write_files};

pub trait Brains {
    /// `brain` registered alone as `name` with `description`.
    fn register(&self, name: &str, brain: &Path, description: &str);

    /// `<work>/brains/<name>`, created.
    fn brain_dir(&self, name: &str) -> PathBuf;

    /// The fixture brain `name` (`kluris-core`'s `tests/fixtures/brains/`)
    /// copied to `<work>/brains/<name>`, with `_outside/` beside it as
    /// `outside/`, an `outside-the-brain.md` beside that, and for `links`
    /// the symlinks `links.symlinks` lists. `None` when this machine cannot
    /// create symlinks.
    fn fixture_brain(&self, name: &str) -> Option<PathBuf>;

    /// The search-eval brain `name` (`xtask/fixtures/search-eval/brains/`)
    /// copied to `<work>/brains/<name>`.
    fn eval_brain(&self, name: &str) -> PathBuf;

    /// A bare remote `<work>/remote.git`, a teammate's clone of it, and the
    /// clone registered as the brain `team`, whose branch tracks the
    /// remote's `main`. With `teammate_pushed` the teammate then pushes
    /// `k/zebra.md` and a tag `v1`, which wait upstream.
    fn team_brain(&self, teammate_pushed: bool) -> PathBuf;

    /// Whether the team brain has the remote's `main` among its objects.
    fn has_remote_main(&self) -> bool;

    /// The team brain's refs: `git for-each-ref`, then the bytes of
    /// `packed-refs` and `FETCH_HEAD` (`None` when absent).
    fn team_refs(&self) -> (String, Option<Vec<u8>>, Option<Vec<u8>>);
}

impl Brains for Sandbox {
    fn register(&self, name: &str, brain: &Path, description: &str) {
        self.write_config(&[(name, brain, description)]);
    }

    fn brain_dir(&self, name: &str) -> PathBuf {
        let brain = self.work.join("brains").join(name);
        fs::create_dir_all(&brain).unwrap();
        brain
    }

    fn fixture_brain(&self, name: &str) -> Option<PathBuf> {
        let base = self.work.join("brains");
        let brain = base.join(name);
        copy_tree(&core_fixtures().join(name), &brain);
        copy_tree(&core_fixtures().join("_outside"), &base.join("outside"));
        fs::write(base.join("outside-the-brain.md"), "# outside\n").unwrap();
        if name == "links" {
            let list = fs::read_to_string(core_fixtures().join("links.symlinks")).unwrap();
            for line in list.lines().filter(|l| !l.starts_with('#')) {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let [kind, link, target] = fields[..] else {
                    continue;
                };
                let target = target.replace('/', std::path::MAIN_SEPARATOR_STR);
                if let Err(e) = symlink(&target, &brain.join(link), kind == "dir") {
                    eprintln!("cannot create symlinks on this machine: {e}");
                    return None;
                }
            }
        }
        Some(brain)
    }

    fn eval_brain(&self, name: &str) -> PathBuf {
        let brain = self.work.join("brains").join(name);
        copy_tree(&eval_fixtures().join("brains").join(name), &brain);
        brain
    }

    fn team_brain(&self, teammate_pushed: bool) -> PathBuf {
        let remote = self.work.join("remote.git");
        let teammate = self.work.join("teammate");
        let brain = self.work.join("brains").join("team");
        let git = |cwd: &Path, args: &[&str]| self.git(cwd, args, PINNED_GIT_DATE);
        let s = |p: &Path| p.display().to_string();
        git(
            &self.work,
            &["init", "-q", "--bare", "-b", "main", &s(&remote)],
        );
        git(&self.work, &["clone", "-q", &s(&remote), &s(&teammate)]);
        write_files(
            &teammate,
            &[
                ("brain.md", "# team\n\n## Lobes\n\n- [k](k/map.md)\n"),
                ("glossary.md", "# Glossary\n\n**JWT** - JSON Web Token\n"),
                ("k/map.md", "# k\n"),
                (
                    "k/jwt.md",
                    "---\nparent: map.md\nupdated: 2026-02-03\n---\n# JWT tokens\n\nWe sign every JWT.\n",
                ),
            ],
        );
        git(&teammate, &["add", "-A"]);
        git(&teammate, &["commit", "-q", "-m", "seed"]);
        git(&teammate, &["push", "-q", "-u", "origin", "main"]);
        fs::create_dir_all(brain.parent().unwrap()).unwrap();
        git(&self.work, &["clone", "-q", &s(&remote), &s(&brain)]);
        if teammate_pushed {
            write_files(
                &teammate,
                &[(
                    "k/zebra.md",
                    "---\nparent: map.md\nupdated: 2026-03-01\n---\n# Zebra\n\nAll about zebras.\n",
                )],
            );
            git(&teammate, &["add", "-A"]);
            git(&teammate, &["commit", "-q", "-m", "zebra"]);
            git(&teammate, &["tag", "v1"]);
            git(&teammate, &["push", "-q", "origin", "main", "v1"]);
        }
        self.register("team", &brain, "");
        brain
    }

    fn has_remote_main(&self) -> bool {
        let remote = self.work.join("remote.git");
        let main = self.git(&remote, &["rev-parse", "main"], PINNED_GIT_DATE);
        let mut cmd = Command::new("git");
        self.env(&mut cmd)
            .current_dir(self.work.join("brains").join("team"))
            .args(["cat-file", "-e", &format!("{}^{{commit}}", main.trim())])
            .output()
            .unwrap()
            .status
            .success()
    }

    fn team_refs(&self) -> (String, Option<Vec<u8>>, Option<Vec<u8>>) {
        let brain = self.work.join("brains").join("team");
        let read = |name: &str| fs::read(brain.join(".git").join(name)).ok();
        (
            self.git(&brain, &["for-each-ref"], PINNED_GIT_DATE),
            read("packed-refs"),
            read("FETCH_HEAD"),
        )
    }
}

/// `kluris-core`'s fixture brains.
pub fn core_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../kluris-core/tests/fixtures/brains")
}

/// The search-eval fixture: its brains and `queries.jsonl`.
pub fn eval_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/fixtures/search-eval")
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path, _dir: bool) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &str, link: &Path, dir: bool) -> std::io::Result<()> {
    if dir {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}
