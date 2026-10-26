//! `kluris sync` (TEST-K3-45), with real git: a bare remote and two clones
//! in temp dirs, "ours" (the brain being synced) and "theirs" (a teammate).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::dream::{DreamOptions, dream};
use kluris_core::git::SystemGit;
use kluris_core::sync::commit::{CommitRequest, commit};
use kluris_core::sync::merge::merge_markdown;
use kluris_core::sync::{
    ConflictKind, PENDING, SyncError, SyncReport, SyncRequest, check_git, sync,
};
use kluris_core::yaml::Date;
use tempfile::TempDir;

const ISOLATED_VARS: [&str; 8] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
    "GIT_INDEX_FILE",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "KLURIS_AGENT",
];

fn today() -> Date {
    Date::new(2026, 1, 1).unwrap()
}

fn options() -> DreamOptions<'static> {
    DreamOptions {
        name: "demo",
        description: "A demo brain",
        today: today(),
    }
}

/// A neuron with the given frontmatter lines (between `created` and
/// `updated`, sorted as the writer sorts them) and body.
fn neuron(extra: &str, body: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\n{extra}parent: ./map.md\nupdated: '2026-01-01'\n---\n\n{body}"
    )
}

/// One world at a time: each runs several git processes per step, and the
/// Windows runner caps a run at 64 processes.
static SERIAL: Mutex<()> = Mutex::new(());

struct World {
    _serial: MutexGuard<'static, ()>,
    dir: TempDir,
    remote: PathBuf,
    ours: PathBuf,
    theirs: PathBuf,
}

impl World {
    fn new() -> World {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        let w = World {
            _serial: serial,
            remote: dir.path().join("remote.git"),
            ours: dir.path().join("ours"),
            theirs: dir.path().join("theirs"),
            dir,
        };
        w.run(
            w.dir.path(),
            &["init", "-q", "--bare", "-b", "main", "remote.git"],
        );
        fs::create_dir_all(&w.ours).unwrap();
        w.run(&w.ours, &["init", "-q", "-b", "main"]);
        w.configure(&w.ours, "Ana Tester", "ana@example.com");
        w.write(&w.ours, ".gitignore", "kluris.yml\n");
        w.write(
            &w.ours,
            "kluris.yml",
            "name: demo\ndescription: A demo brain\n",
        );
        w.write(
            &w.ours,
            "glossary.md",
            "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n",
        );
        w.write(
            &w.ours,
            "notes/first.md",
            &neuron("", "# First\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n"),
        );
        w.write(&w.ours, "notes/keep.md", &neuron("", "# Keep\n"));
        w.write(&w.ours, "ideas/idea.md", &neuron("", "# Idea\n"));
        let mut store = LocalGitStore::open(&w.ours);
        dream(&mut store, &w.git(), &options());
        w.run(&w.ours, &["add", "-A"]);
        w.run(&w.ours, &["commit", "-q", "-m", "init"]);
        let url = w.remote.to_string_lossy().into_owned();
        w.run(&w.ours, &["remote", "add", "origin", &url]);
        w.run(&w.ours, &["push", "-q", "-u", "origin", "main"]);
        w.clone_as(&w.theirs, "Bob Teammate", "bob@example.com");
        w
    }

    fn clone_as(&self, at: &Path, name: &str, email: &str) {
        let url = self.remote.to_string_lossy().into_owned();
        let to = at.to_string_lossy().into_owned();
        self.run(self.dir.path(), &["clone", "-q", &url, &to]);
        self.configure(at, name, email);
        self.write(at, "kluris.yml", "name: demo\ndescription: A demo brain\n");
    }

    fn configure(&self, at: &Path, name: &str, email: &str) {
        self.run(at, &["config", "user.name", name]);
        self.run(at, &["config", "user.email", email]);
        self.run(at, &["config", "core.autocrlf", "false"]);
    }

    /// The git Kluris runs, with no user or system config.
    fn git(&self) -> SystemGit {
        let mut git = SystemGit::new()
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path());
        for var in ISOLATED_VARS {
            git = git.env_remove(var);
        }
        git
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path());
        for var in ISOLATED_VARS {
            cmd.env_remove(var);
        }
        cmd
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> String {
        let out = self.command(cwd, args).output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn path(at: &Path, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(at.to_path_buf(), |p, part| p.join(part))
    }

    fn write(&self, at: &Path, rel: &str, text: &str) {
        let path = World::path(at, rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(&self, at: &Path, rel: &str) -> String {
        fs::read_to_string(World::path(at, rel)).unwrap()
    }

    /// Commit `rels` in a clone as `kluris commit` does (maps regenerated).
    fn commit(&self, at: &Path, rels: &[&str]) {
        let paths: Vec<String> = rels.iter().map(|r| r.to_string()).collect();
        let report = commit(
            at,
            &self.git(),
            &CommitRequest {
                paths: &paths,
                message: None,
                agent: None,
                no_verify: false,
                maps: options(),
            },
        )
        .unwrap();
        assert!(report.commit.is_some(), "nothing committed for {rels:?}");
    }

    /// The teammate writes, commits and pushes.
    fn theirs_push(&self, files: &[(&str, &str)], deleted: &[&str]) {
        for (rel, text) in files {
            self.write(&self.theirs, rel, text);
        }
        for rel in deleted {
            let path = World::path(&self.theirs, rel);
            if path.is_dir() {
                fs::remove_dir_all(path).unwrap();
            } else {
                fs::remove_file(path).unwrap();
            }
        }
        let mut rels: Vec<&str> = files.iter().map(|(rel, _)| *rel).collect();
        rels.extend_from_slice(deleted);
        self.commit(&self.theirs, &rels);
        self.run(&self.theirs, &["push", "-q"]);
    }

    /// We write and commit (not pushed).
    fn ours_commit(&self, files: &[(&str, &str)], deleted: &[&str]) {
        for (rel, text) in files {
            self.write(&self.ours, rel, text);
        }
        for rel in deleted {
            fs::remove_file(World::path(&self.ours, rel)).unwrap();
        }
        let mut rels: Vec<&str> = files.iter().map(|(rel, _)| *rel).collect();
        rels.extend_from_slice(deleted);
        self.commit(&self.ours, &rels);
    }

    fn request(&self) -> SyncRequest<'static> {
        SyncRequest {
            no_verify: false,
            maps: options(),
            network_time_limit: Duration::from_secs(60),
        }
    }

    fn sync(&self) -> Result<SyncReport, SyncError> {
        sync(&self.ours, &self.git(), &self.request())
    }

    fn head(&self, at: &Path) -> String {
        self.run(at, &["rev-parse", "HEAD"]).trim().to_string()
    }

    fn remote_head(&self) -> String {
        self.run(&self.remote, &["rev-parse", "main"])
            .trim()
            .to_string()
    }

    fn show(&self, at: &Path, rev: &str, rel: &str) -> String {
        self.run(at, &["show", &format!("{rev}:{rel}")])
    }

    fn status(&self, at: &Path) -> String {
        self.run(at, &["status", "--porcelain", "--untracked-files=all"])
    }

    /// Every file in a clone (not `.git`), with its bytes.
    fn files(&self, at: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut pending = vec![at.to_path_buf()];
        while let Some(dir) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if entry.file_name() == ".git" {
                    continue;
                }
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let rel = path
                        .strip_prefix(at)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    /// HEAD, the index, the status and every file of "ours", and the remote.
    fn snapshot(&self) -> (String, String, String, BTreeMap<String, Vec<u8>>, String) {
        (
            self.head(&self.ours),
            self.run(&self.ours, &["ls-files", "--stage"]),
            self.status(&self.ours),
            self.files(&self.ours),
            self.remote_head(),
        )
    }

    fn parents(&self, at: &Path, rev: &str) -> usize {
        self.run(at, &["rev-list", "--parents", "-n", "1", rev])
            .split_whitespace()
            .count()
            - 1
    }

    fn hook(&self, at: &Path, name: &str, body: &str) {
        let dir = at.join(".git").join("hooks");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

fn no_markers(text: &str) {
    for marker in ["<<<<<<<", ">>>>>>>", "|||||||"] {
        assert!(
            !text.contains(marker),
            "conflict marker {marker} in:\n{text}"
        );
    }
    assert!(
        !text.lines().any(|l| l == "======="),
        "conflict marker ======= in:\n{text}"
    );
}

/// No file of a clone holds a conflict marker.
fn no_markers_anywhere(w: &World, at: &Path) {
    for (rel, bytes) in w.files(at) {
        if let Ok(text) = String::from_utf8(bytes) {
            assert!(
                !text.contains("<<<<<<<") && !text.contains(">>>>>>>"),
                "conflict marker in {rel}:\n{text}"
            );
        }
    }
}

// --- fetch, fast-forward, push, offline ------------------------------------------------

#[test]
fn nothing_to_do() {
    let w = World::new();
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && !r.pushed && !r.offline, "{r:?}");
    assert!(r.auto_merged.is_empty() && r.conflicts.is_empty());
    assert!(r.skipped_dirty.is_empty() && r.blocked.is_empty());
    assert_eq!(w.snapshot(), before);
}

#[test]
fn behind_only_fast_forwards() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.fetched && r.merged && !r.pushed, "{r:?}");
    assert_eq!(w.head(&w.ours), w.remote_head());
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "ideas/map.md").contains("new.md"));
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn ahead_only_pushes() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), w.head(&w.ours));
}

#[test]
fn no_upstream_is_offline_and_keeps_local_commits() {
    let w = World::new();
    w.run(&w.ours, &["branch", "--unset-upstream"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let head = w.head(&w.ours);
    let remote = w.remote_head();
    let r = w.sync().unwrap();
    assert!(r.offline && !r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(w.head(&w.ours), head);
    assert_eq!(w.remote_head(), remote);
}

#[test]
fn an_unreachable_remote_is_offline_with_nothing_changed() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let gone = w.dir.path().join("gone.git").to_string_lossy().into_owned();
    w.run(&w.ours, &["remote", "set-url", "origin", &gone]);
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert!(r.offline && !r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(w.snapshot(), before);
}

// --- auto-merged -------------------------------------------------------------------------

#[test]
fn different_files_merge_into_a_sync_commit_and_push() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let ours_before = w.head(&w.ours);
    let r = w.sync().unwrap();
    assert!(r.fetched && r.merged && r.pushed, "{r:?}");
    assert!(
        r.auto_merged.is_empty(),
        "different files are not listed: {r:?}"
    );
    let head = w.head(&w.ours);
    assert_eq!(w.parents(&w.ours, &head), 2);
    assert_eq!(w.run(&w.ours, &["rev-parse", "HEAD^1"]).trim(), ours_before);
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%B"]),
        "kluris: sync\n\nKluris-Sync: auto\n\n"
    );
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%an <%ae>"]).trim(),
        "Ana Tester <ana@example.com>"
    );
    assert_eq!(w.remote_head(), head);
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
    assert!(w.read(&w.ours, "notes/second.md").contains("# Second"));
    assert_eq!(w.status(&w.ours), "");
    no_markers_anywhere(&w, &w.ours);
}

#[test]
fn generated_maps_are_rebuilt_from_the_merged_tree() {
    let w = World::new();
    // Both sides add a neuron to the same lobe: both regenerate notes/map.md.
    w.theirs_push(&[("notes/theirs.md", &neuron("", "# Theirs\n"))], &[]);
    w.ours_commit(&[("notes/ours.md", &neuron("", "# Ours\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    let map = w.show(&w.ours, "HEAD", "notes/map.md");
    no_markers(&map);
    assert!(map.contains("- [ours.md](./ours.md) — Ours"), "{map}");
    assert!(map.contains("- [theirs.md](./theirs.md) — Theirs"), "{map}");
    assert_eq!(w.read(&w.ours, "notes/map.md"), map);
    assert!(
        !r.auto_merged.contains(&"notes/map.md".to_string()),
        "{r:?}"
    );
    assert_eq!(w.status(&w.ours), "");
}

#[test]
fn a_new_lobe_on_each_side_rebuilds_brain_md() {
    let w = World::new();
    w.theirs_push(&[("theirs-lobe/t.md", &neuron("", "# T\n"))], &[]);
    w.ours_commit(&[("ours-lobe/o.md", &neuron("", "# O\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.conflicts.is_empty(), "{r:?}");
    let brain = w.show(&w.ours, "HEAD", "brain.md");
    no_markers(&brain);
    assert!(
        brain.contains("[ours-lobe/]") && brain.contains("[theirs-lobe/]"),
        "{brain}"
    );
}

#[test]
fn non_overlapping_body_edits_merge_and_are_listed() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert_eq!(r.auto_merged, ["notes/first.md"]);
    let text = w.read(&w.ours, "notes/first.md");
    no_markers(&text);
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

#[test]
fn dates_take_the_later_updated_and_the_earlier_created() {
    let w = World::new();
    let theirs = "---\ncreated: '2025-12-01'\nparent: ./map.md\nupdated: '2026-03-01'\n---\n\n# First\n\nOne, theirs.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n";
    let ours = "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-01'\n---\n\n# First\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive, ours.\n";
    w.theirs_push(&[("notes/first.md", theirs)], &[]);
    w.ours_commit(&[("notes/first.md", ours)], &[]);
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty(), "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(text.contains("created: '2025-12-01'\n"), "{text}");
    assert!(text.contains("updated: '2026-03-01'\n"), "{text}");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

#[test]
fn related_and_tags_are_the_union_ours_first() {
    let w = World::new();
    let with = |tags: &str, related: &str| {
        format!(
            "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n{related}tags:\n{tags}updated: '2026-01-01'\n---\n\n# Keep\n"
        )
    };
    let base = with("- api\n", "- ../ideas/idea.md\n");
    w.ours_commit(&[("notes/keep.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[(
            "notes/keep.md",
            &with(
                "- api\n- zeta\n- auth\n",
                "- ../ideas/idea.md\n- first.md\n",
            ),
        )],
        &[],
    );
    w.ours_commit(
        &[(
            "notes/keep.md",
            &with("- api\n- beta\n", "- ../ideas/idea.md\n"),
        )],
        &[],
    );
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty(), "{r:?}");
    assert_eq!(r.auto_merged, ["notes/keep.md"]);
    let text = w.read(&w.ours, "notes/keep.md");
    assert!(
        text.contains("tags:\n- api\n- beta\n- zeta\n- auth\n"),
        "{text}"
    );
    assert!(
        text.contains("related:\n- ../ideas/idea.md\n- first.md\n"),
        "{text}"
    );
}

#[test]
fn crlf_neurons_merge_and_keep_their_line_endings() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md").replace('\n', "\r\n");
    w.ours_commit(&[("notes/first.md", &base)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty() && r.merged, "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text:?}"
    );
    assert_eq!(
        text.matches('\n').count(),
        text.matches("\r\n").count(),
        "{text:?}"
    );
}

#[test]
fn non_ascii_names_sync() {
    let w = World::new();
    w.theirs_push(&[("été/naïve.md", &neuron("", "# Naïve\n"))], &[]);
    w.ours_commit(&[("notes/café ünï.md", &neuron("", "# Café\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    assert!(w.read(&w.ours, "été/naïve.md").contains("# Naïve"));
    assert!(w.read(&w.ours, "brain.md").contains("été"));
    assert_eq!(w.status(&w.ours), "");
}

// --- sent to a person --------------------------------------------------------------------

/// Sync ends in exactly one conflict of `kind` at `path`, and nothing
/// changed: HEAD, index, status, files and the remote are as they were.
fn assert_conflict(w: &World, path: &str, kind: ConflictKind) {
    let before = w.snapshot();
    let ours = w.head(&w.ours);
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(r.conflicts.len(), 1, "{r:?}");
    let c = &r.conflicts[0];
    assert_eq!((c.path.as_str(), c.kind), (path, kind), "{r:?}");
    assert_eq!(w.snapshot(), before, "a conflict changed something");
    no_markers_anywhere(w, &w.ours);

    let pending = w.ours.join(".git").join(PENDING);
    let doc: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&pending).unwrap()).unwrap();
    let keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["base", "ours", "theirs", "conflicts", "created"]);
    assert_eq!(doc["ours"], ours.as_str());
    assert_eq!(doc["theirs"], w.remote_head().as_str());
    assert_eq!(doc["conflicts"][0]["path"], path);
    assert_eq!(doc["conflicts"][0]["kind"], kind.as_str());
    let entry_keys: Vec<&str> = doc["conflicts"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(entry_keys, ["path", "kind", "base", "ours", "theirs"]);

    // An open conflict refuses the next sync, pointing at `kluris resolve`.
    match w.sync() {
        Err(e @ SyncError::OpenConflict(_)) => {
            assert!(e.to_string().contains("kluris resolve"), "{e}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
}

#[test]
fn overlapping_body_edits_are_a_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("Three.", "Three, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
    assert_conflict(&w, "notes/first.md", ConflictKind::Content);
}

#[test]
fn delete_vs_edit_is_a_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(&[], &["notes/first.md"]);
    w.ours_commit(
        &[("notes/first.md", &base.replace("Three.", "Three, ours."))],
        &[],
    );
    assert_conflict(&w, "notes/first.md", ConflictKind::DeleteEdit);
    let r_conflict = fs::read_to_string(w.ours.join(".git").join(PENDING)).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&r_conflict).unwrap();
    assert!(doc["conflicts"][0]["theirs"].is_null(), "{doc}");
    assert!(doc["conflicts"][0]["base"].is_string() && doc["conflicts"][0]["ours"].is_string());
}

#[test]
fn different_parents_are_a_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/keep.md");
    w.theirs_push(
        &[(
            "notes/keep.md",
            &base.replace("parent: ./map.md", "parent: ../ideas/map.md"),
        )],
        &[],
    );
    w.ours_commit(
        &[(
            "notes/keep.md",
            &base.replace("parent: ./map.md", "parent: ../brain.md"),
        )],
        &[],
    );
    assert_conflict(&w, "notes/keep.md", ConflictKind::Parent);
}

#[test]
fn a_non_kluris_key_changed_differently_is_a_conflict() {
    let w = World::new();
    w.ours_commit(
        &[("notes/keep.md", &neuron("owner: team\n", "# Keep\n"))],
        &[],
    );
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    w.theirs_push(
        &[("notes/keep.md", &neuron("owner: bob\n", "# Keep\n"))],
        &[],
    );
    w.ours_commit(
        &[("notes/keep.md", &neuron("owner: ana\n", "# Keep\n"))],
        &[],
    );
    assert_conflict(&w, "notes/keep.md", ConflictKind::Frontmatter);
}

#[test]
fn the_same_change_on_both_sides_is_no_conflict() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    let same = base.replace("Three.", "Three, both.");
    w.theirs_push(&[("notes/first.md", &same)], &[]);
    w.ours_commit(&[("notes/first.md", &same)], &[]);
    let r = w.sync().unwrap();
    assert!(r.conflicts.is_empty() && r.merged, "{r:?}");
    assert!(w.read(&w.ours, "notes/first.md").contains("Three, both."));
}

// --- dirty files ---------------------------------------------------------------------------

#[test]
fn incoming_changes_to_a_dirty_file_stop_the_sync() {
    let w = World::new();
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.write(
        &w.ours,
        "notes/first.md",
        &base.replace("Five.", "Five, mine (uncommitted)."),
    );
    w.write(
        &w.ours,
        "glossary.md",
        "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n- mine\n",
    );
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert!(r.fetched && !r.merged && !r.pushed, "{r:?}");
    assert_eq!(r.blocked, ["notes/first.md"]);
    assert_eq!(r.skipped_dirty, ["glossary.md", "notes/first.md"]);
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_untracked_file_in_the_way_stops_the_sync() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.write(&w.ours, "ideas/new.md", "# my own draft\n");
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert_eq!(r.blocked, ["ideas/new.md"]);
    assert!(!r.merged);
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_ignored_file_in_the_way_stops_the_sync() {
    let w = World::new();
    w.theirs_push(&[(".gitignore", "kluris.yml\nnotes/local.md\n")], &[]);
    w.run(&w.ours, &["pull", "-q"]);
    // The teammate stops ignoring it and commits theirs; ours is ignored here.
    w.write(&w.ours, "notes/local.md", "# my local note\n");
    let gi = "kluris.yml\n";
    w.write(&w.theirs, ".gitignore", gi);
    w.write(&w.theirs, "notes/local.md", &neuron("", "# Local\n"));
    w.run(&w.theirs, &["add", "-A"]);
    w.run(&w.theirs, &["commit", "-q", "-m", "unignore"]);
    w.run(&w.theirs, &["push", "-q"]);
    let before = w.snapshot();
    let r = w.sync().unwrap();
    assert_eq!(r.blocked, ["notes/local.md"], "{r:?}");
    assert_eq!(w.snapshot(), before);
}

#[test]
fn dirty_files_the_changes_do_not_touch_stay_and_are_listed() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let mine = "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n- mine\n";
    w.write(&w.ours, "glossary.md", mine);
    w.write(&w.ours, "notes/draft.md", "# draft\n");
    let r = w.sync().unwrap();
    assert!(r.merged && r.blocked.is_empty(), "{r:?}");
    assert_eq!(r.skipped_dirty, ["glossary.md", "notes/draft.md"]);
    assert_eq!(w.read(&w.ours, "glossary.md"), mine);
    assert_eq!(w.read(&w.ours, "notes/draft.md"), "# draft\n");
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
}

// --- the push race, hooks ----------------------------------------------------------------

#[test]
fn a_push_rejected_because_the_remote_moved_is_merged_again_and_retried() {
    let w = World::new();
    let third = w.dir.path().join("third");
    w.clone_as(&third, "Cy Third", "cy@example.com");
    w.write(&third, "ideas/race.md", &neuron("", "# Race\n"));
    w.commit(&third, &["ideas/race.md"]);
    // Our pre-push hook makes the third clone push first, once.
    let marker = w.dir.path().join("raced");
    let body = format!(
        "if [ ! -e '{m}' ]; then touch '{m}'; cd '{t}' && unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE && git push -q >&2; fi\nexit 0",
        m = marker.display().to_string().replace('\\', "/"),
        t = third.display().to_string().replace('\\', "/"),
    );
    w.hook(&w.ours, "pre-push", &body);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync().unwrap();
    assert!(marker.exists(), "the race did not happen");
    assert!(r.pushed && r.merged, "{r:?}");
    let remote = w.remote_head();
    assert_eq!(remote, w.head(&w.ours));
    assert!(w.read(&w.ours, "ideas/race.md").contains("# Race"));
    assert!(
        w.show(&w.ours, &remote, "notes/second.md")
            .contains("# Second")
    );
}

#[test]
fn a_failing_pre_merge_commit_hook_stops_the_merge() {
    let w = World::new();
    w.hook(&w.ours, "pre-merge-commit", "echo 'not now' >&2\nexit 1");
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.snapshot();
    match w.sync() {
        Err(SyncError::Hook { name, code }) => assert_eq!((name, code), ("pre-merge-commit", 1)),
        other => panic!("{other:?}"),
    }
    assert_eq!(w.snapshot(), before);
    // --no-verify skips it; post-merge runs once the work tree is updated.
    let ran = w.dir.path().join("post-merge-ran");
    w.hook(
        &w.ours,
        "post-merge",
        &format!("touch '{}'", ran.display().to_string().replace('\\', "/")),
    );
    let r = sync(
        &w.ours,
        &w.git(),
        &SyncRequest {
            no_verify: true,
            ..w.request()
        },
    )
    .unwrap();
    assert!(r.merged && r.pushed, "{r:?}");
    assert!(ran.exists(), "post-merge did not run");
}

// --- refusals ------------------------------------------------------------------------------

#[test]
fn a_brain_nested_in_a_larger_repo_is_refused() {
    let w = World::new();
    fs::create_dir_all(w.ours.join("sub")).unwrap();
    match sync(&w.ours.join("sub"), &w.git(), &w.request()) {
        Err(SyncError::Nested { .. }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_brain_without_git_is_refused() {
    let w = World::new();
    let plain = w.dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert!(matches!(
        sync(&plain, &w.git(), &w.request()),
        Err(SyncError::NotARepo)
    ));
}

#[test]
fn a_merge_in_progress_is_refused() {
    let w = World::new();
    w.run(&w.ours, &["checkout", "-q", "-b", "side"]);
    w.ours_commit(&[("notes/side.md", &neuron("", "# Side\n"))], &[]);
    w.run(&w.ours, &["checkout", "-q", "main"]);
    w.run(
        &w.ours,
        &[
            "merge",
            "-q",
            "--no-ff",
            "--no-commit",
            "-s",
            "ours",
            "side",
        ],
    );
    let before = w.snapshot();
    assert!(matches!(w.sync(), Err(SyncError::InProgress(_))));
    assert_eq!(w.snapshot(), before);
}

#[cfg(unix)]
#[test]
fn an_older_git_is_a_clean_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new().unwrap();
    let fake = dir.path().join("git");
    fs::write(&fake, "#!/bin/sh\necho 'git version 2.37.9'\n").unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    match check_git(&SystemGit::new().program(&fake)) {
        Err(e @ SyncError::GitTooOld { .. }) => assert!(e.to_string().contains("2.38"), "{e}"),
        other => panic!("{other:?}"),
    }
}

// --- criss-cross ---------------------------------------------------------------------------

/// With several merge bases (a criss-cross history), the conflict rules take
/// the first one `git merge-base --all` lists as each file's base.
#[test]
fn a_criss_cross_history_merges_with_the_first_merge_base() {
    let w = World::new();
    // P on ours, Q on theirs; each merges the other: two merge bases.
    w.ours_commit(&[("notes/p.md", &neuron("", "# P\n"))], &[]);
    let p = w.head(&w.ours);
    w.theirs_push(&[("ideas/q.md", &neuron("", "# Q\n"))], &[]);
    let q = w.remote_head();
    w.run(&w.ours, &["fetch", "-q"]);
    w.run(&w.theirs, &["fetch", "-q", "origin"]);
    let ours_path = w.ours.to_string_lossy().into_owned();
    w.run(&w.theirs, &["fetch", "-q", &ours_path, "main:refs/ours"]);
    w.run(&w.ours, &["merge", "-q", "--no-edit", &q]);
    w.run(&w.theirs, &["merge", "-q", "--no-edit", "refs/ours"]);
    let base = w.read(&w.ours, "notes/first.md");
    w.theirs_push(
        &[("notes/first.md", &base.replace("One.", "One, theirs."))],
        &[],
    );
    w.ours_commit(
        &[("notes/first.md", &base.replace("Five.", "Five, ours."))],
        &[],
    );
    let bases = w.run(&w.ours, &["merge-base", "--all", "HEAD", "origin/main"]);
    w.run(&w.ours, &["fetch", "-q"]);
    let bases_now = w.run(&w.ours, &["merge-base", "--all", "HEAD", "origin/main"]);
    assert_eq!(bases_now.lines().count(), 2, "{bases} / {bases_now}");
    assert!(bases_now.contains(&p) || bases_now.contains(&q));
    let r = w.sync().unwrap();
    assert!(r.merged && r.pushed && r.conflicts.is_empty(), "{r:?}");
    let text = w.read(&w.ours, "notes/first.md");
    assert!(
        text.contains("One, theirs.") && text.contains("Five, ours."),
        "{text}"
    );
}

// --- the merge never writes markers, never loses a change ----------------------------------

/// A small deterministic generator (xorshift64*).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn file(created: &str, updated: &str, tags: &[String], body: &[String]) -> String {
    let mut text = format!("---\ncreated: '{created}'\nparent: ./map.md\n");
    if !tags.is_empty() {
        text.push_str("tags:\n");
        for t in tags {
            text.push_str(&format!("- {t}\n"));
        }
    }
    text.push_str(&format!("updated: '{updated}'\n---\n\n"));
    text.push_str(&body.join("\n"));
    text.push('\n');
    text
}

#[test]
fn auto_merges_never_write_markers_and_keep_every_change() {
    let w = World::new();
    let git = w.git();
    let cases: usize = std::env::var("KLURIS_SYNC_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..cases {
        let n = 8 + rng.below(20);
        let base_body: Vec<String> = (0..n).map(|i| format!("line {i}")).collect();
        let base_tags: Vec<String> = (0..rng.below(3)).map(|i| format!("t{i}")).collect();
        // Ours edits in the first half, theirs in the second, at least two
        // unchanged lines apart: never overlapping.
        let i = rng.below(n / 2 - 1);
        let j = n / 2 + 1 + rng.below(n - n / 2 - 1);
        let mut ours_body = base_body.clone();
        ours_body[i] = format!("ours {case}");
        if rng.below(2) == 0 {
            ours_body.insert(i + 1, format!("ours added {case}"));
        }
        let mut theirs_body = base_body.clone();
        theirs_body[j] = format!("theirs {case}");
        let mut ours_tags = base_tags.clone();
        ours_tags.push(format!("o{}", rng.below(5)));
        let mut theirs_tags = base_tags.clone();
        theirs_tags.push(format!("h{}", rng.below(5)));
        let day = |r: &mut Rng| format!("2026-0{}-1{}", 1 + r.below(9), r.below(10));
        let (oc, ou, tc, tu) = (day(&mut rng), day(&mut rng), day(&mut rng), day(&mut rng));
        let base = file("2026-01-01", "2026-01-01", &base_tags, &base_body);
        let ours = file(&oc, &ou, &ours_tags, &ours_body);
        let theirs = file(&tc, &tu, &theirs_tags, &theirs_body);
        let merged = merge_markdown(
            Some(base.as_bytes()),
            ours.as_bytes(),
            theirs.as_bytes(),
            &git,
        )
        .unwrap_or_else(|kind| panic!("case {case}: {kind:?}\n{ours}\n{theirs}"));
        let text = String::from_utf8(merged.bytes).unwrap();
        no_markers(&text);
        for line in ours_body
            .iter()
            .chain(&theirs_body)
            .filter(|l| !l.starts_with("line "))
        {
            assert!(
                text.contains(line.as_str()),
                "case {case} lost {line:?}:\n{text}"
            );
        }
        for tag in ours_tags.iter().chain(&theirs_tags) {
            assert!(
                text.contains(&format!("- {tag}\n")),
                "case {case} lost tag {tag}:\n{text}"
            );
        }
        let (created, updated) = (oc.clone().min(tc.clone()), ou.clone().max(tu.clone()));
        assert!(
            text.contains(&format!("created: '{created}'\n")),
            "case {case}:\n{text}"
        );
        assert!(
            text.contains(&format!("updated: '{updated}'\n")),
            "case {case}:\n{text}"
        );
    }
}
