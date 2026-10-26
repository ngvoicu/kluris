//! `kluris dream` in the core (`kluris_core::dream`): v2's `_sync_brain_state`
//! and validators (research-03 §2.5) with the decided differences (quoted
//! dates, the file's own line endings, no map.md in pruned directories, paths
//! `/`-joined). Expected bytes follow v2.28.11's writers; `parity/test_dream.py`
//! checks the same rules against the live oracle.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kluris_core::brain::deprecation::DeprecationIssue;
use kluris_core::brain::store::LocalGitStore;
use kluris_core::dream::{
    BrokenSynapse, DreamOptions, DreamReport, FieldProblem, Fixes, FrontmatterIssue, dream,
};
use kluris_core::git::{Commit, FileDates, Git, GitError, Repo, SystemGit};
use kluris_core::yaml::Date;
use tempfile::TempDir;

const TODAY: (u32, u32, u32) = (2026, 10, 26);

fn today() -> Date {
    Date::new(TODAY.0, TODAY.1, TODAY.2).unwrap()
}

/// A git whose history is given: `repo` finds the brain at the work tree
/// root and `file_dates` returns the dates; nothing else is called.
struct FakeGit {
    dates: Option<FileDates>,
}

impl FakeGit {
    fn none() -> FakeGit {
        FakeGit { dates: None }
    }

    /// `(path, latest, created)` author dates.
    fn with(entries: &[(&str, &str, &str)]) -> FakeGit {
        let mut dates = FileDates::default();
        for (path, latest, created) in entries {
            dates.latest.insert(path.to_string(), latest.to_string());
            dates.created.insert(path.to_string(), created.to_string());
        }
        FakeGit { dates: Some(dates) }
    }
}

impl Git for FakeGit {
    fn repo(&self, dir: &Path) -> Option<Repo> {
        self.dates.as_ref().map(|_| Repo {
            dir: dir.to_path_buf(),
            prefix: String::new(),
        })
    }
    fn init(&self, _: &Path) -> Result<(), GitError> {
        unreachable!("dream never runs git init")
    }
    fn checkout_new_branch(&self, _: &Path, _: &str) -> Result<(), GitError> {
        unreachable!()
    }
    fn remote_add(&self, _: &Path, _: &str, _: &str) -> Result<(), GitError> {
        unreachable!()
    }
    fn add(&self, _: &Path, _: &str) -> Result<(), GitError> {
        unreachable!()
    }
    fn commit(&self, _: &Path, _: &str) -> Result<(), GitError> {
        unreachable!()
    }
    fn log(&self, _: &Repo, _: usize) -> Result<Vec<Commit>, GitError> {
        unreachable!()
    }
    fn status_short(&self, _: &Repo) -> Result<String, GitError> {
        unreachable!()
    }
    fn file_dates(&self, _: &Repo) -> FileDates {
        self.dates.clone().unwrap_or_default()
    }
    fn version(&self) -> Result<String, GitError> {
        unreachable!()
    }
}

struct Brain {
    _dir: TempDir,
    root: PathBuf,
}

impl Brain {
    fn new(files: &[(&str, &str)]) -> Brain {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("brain");
        fs::create_dir_all(&root).unwrap();
        let brain = Brain { _dir: dir, root };
        for (rel, text) in files {
            brain.write(rel, text.as_bytes());
        }
        brain
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn mkdir(&self, rel: &str) {
        fs::create_dir_all(self.path(rel)).unwrap();
    }

    fn path(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.root.clone(), |p, part| p.join(part))
    }

    fn text(&self, rel: &str) -> String {
        fs::read_to_string(self.path(rel)).unwrap()
    }

    fn exists(&self, rel: &str) -> bool {
        self.path(rel).exists()
    }

    /// Every file under the root, by `/`-separated path.
    fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut pending = vec![(self.root.clone(), String::new())];
        while let Some((dir, rel)) = pending.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                let child = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}/{name}")
                };
                if entry.file_type().unwrap().is_dir() {
                    if name != ".git" {
                        pending.push((entry.path(), child));
                    }
                } else {
                    out.insert(child, fs::read(entry.path()).unwrap());
                }
            }
        }
        out
    }

    fn dream(&self, git: &impl Git) -> DreamReport {
        self.dream_on(git, today())
    }

    fn dream_on(&self, git: &impl Git, today: Date) -> DreamReport {
        let mut store = LocalGitStore::open(&self.root);
        dream(
            &mut store,
            git,
            &DreamOptions {
                name: "demo",
                description: "A demo brain",
                today,
            },
        )
    }
}

fn neuron(title: &str, extra: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\n{extra}parent: ./map.md\nupdated: '2026-01-01'\n---\n\n# {title}\n"
    )
}

const GLOSSARY: &str = "# Glossary\n";

// --- step 1: dates from git -------------------------------------------------------

#[test]
fn date_sync_takes_updated_and_created_from_git() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/a.md", &neuron("A", "")),
        (
            "lobe/same.md",
            "---\ncreated: 2026-01-01\nparent: ./map.md\nupdated: 2026-02-03\n---\n\n# Same\n",
        ),
        (
            "lobe/c.md",
            "---\nparent: ./map.md\nupdated: '2020-01-01'\n---\n\n# C\n",
        ),
        (
            "lobe/n.yml",
            "#---\n# parent: ./map.md\n# updated: '2020-01-01'\n#---\nkey: v\n",
        ),
        ("lobe/uncommitted.md", &neuron("U", "")),
    ]);
    let git = FakeGit::with(&[
        (
            "lobe/a.md",
            "2026-02-03T09:30:00+05:00",
            "2026-01-01T12:00:00+00:00",
        ),
        (
            "lobe/same.md",
            "2026-02-03T09:30:00+05:00",
            "2026-01-01T12:00:00+00:00",
        ),
        // Author dates are cut to their first 10 characters, in the author's
        // own zone: 23:30 in UTC-5 is still the 31st.
        (
            "lobe/c.md",
            "2026-02-03T23:59:59-08:00",
            "2025-12-31T23:30:00-05:00",
        ),
        (
            "lobe/n.yml",
            "2026-02-03T00:00:00+14:00",
            "2025-01-01T00:00:00+00:00",
        ),
        (
            "glossary.md",
            "2026-02-03T00:00:00+00:00",
            "2026-02-03T00:00:00+00:00",
        ),
    ]);
    let before = b.snapshot();
    let report = b.dream(&git);

    assert_eq!(
        b.text("lobe/a.md"),
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-03'\n---\n\n# A\n"
    );
    // `updated` already equal (as a date) and `created` present: not written.
    assert_eq!(
        b.text("lobe/same.md").as_bytes(),
        &before["lobe/same.md"][..]
    );
    assert_eq!(
        b.text("lobe/c.md"),
        "---\ncreated: '2025-12-31'\nparent: ./map.md\nupdated: '2026-02-03'\n---\n\n# C\n"
    );
    // YAML neurons get `updated` only.
    assert_eq!(
        b.text("lobe/n.yml"),
        "#---\n# parent: ./map.md\n# updated: '2026-02-03'\n#---\nkey: v\n"
    );
    assert_eq!(
        b.text("lobe/uncommitted.md").as_bytes(),
        &before["lobe/uncommitted.md"][..]
    );
    assert_eq!(b.text("glossary.md"), GLOSSARY);
    assert_eq!(report.fixes.dates_updated, 3);
}

#[test]
fn a_brain_without_git_gets_no_dates() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/a.md", "---\nparent: ./map.md\n---\n\n# A\n"),
    ]);
    let report = b.dream(&FakeGit::none());
    assert_eq!(report.fixes.dates_updated, 0);
    assert_eq!(b.text("lobe/a.md"), "---\nparent: ./map.md\n---\n\n# A\n");
    assert_eq!(
        report.frontmatter_issues,
        [
            FrontmatterIssue {
                file: "lobe/a.md".into(),
                field: "created",
                problem: FieldProblem::Missing
            },
            FrontmatterIssue {
                file: "lobe/a.md".into(),
                field: "updated",
                problem: FieldProblem::Missing
            },
        ]
    );
}

#[test]
fn date_sync_uses_brain_relative_paths_in_a_nested_repo_with_non_ascii_names() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::write(root.join("gitconfig-empty"), "").unwrap();
    let env = [
        ("GIT_CONFIG_GLOBAL", root.join("gitconfig-empty")),
        ("GIT_CONFIG_NOSYSTEM", "1".into()),
        ("GIT_CEILING_DIRECTORIES", root.to_path_buf()),
        ("GIT_AUTHOR_DATE", "2026-03-04T08:00:00+00:00".into()),
        ("GIT_COMMITTER_DATE", "2026-03-04T08:00:00+00:00".into()),
        ("GIT_AUTHOR_NAME", "t".into()),
        ("GIT_AUTHOR_EMAIL", "t@t.invalid".into()),
        ("GIT_COMMITTER_NAME", "t".into()),
        ("GIT_COMMITTER_EMAIL", "t@t.invalid".into()),
    ];
    let outer = root.join("outer");
    let brain = outer.join("docs").join("brain");
    fs::create_dir_all(brain.join("lobe")).unwrap();
    fs::write(brain.join("glossary.md"), GLOSSARY).unwrap();
    fs::write(brain.join("lobe").join("café.md"), neuron("Café", "")).unwrap();
    let run = |args: &[&str]| {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(&outer);
        for (k, v) in &env {
            cmd.env(k, v);
        }
        assert!(cmd.status().unwrap().success(), "git {args:?}");
    };
    run(&["init", "-q"]);
    run(&["add", "-A"]);
    run(&["commit", "-q", "-m", "init"]);
    let mut git = SystemGit::new();
    for (k, v) in &env {
        git = git.env(k, v);
    }
    let mut store = LocalGitStore::open(&brain);
    let report = dream(
        &mut store,
        &git,
        &DreamOptions {
            name: "demo",
            description: "",
            today: today(),
        },
    );
    assert_eq!(report.fixes.dates_updated, 1);
    assert!(
        fs::read_to_string(brain.join("lobe").join("café.md"))
            .unwrap()
            .contains("updated: '2026-03-04'")
    );
}

// --- steps 2-3: parents and reverse links --------------------------------------------

#[test]
fn missing_parents_are_inferred_below_the_root_with_quoted_dates() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        (
            "root.md",
            "---\ncreated: 2026-01-01\nupdated: 2026-01-01\n---\n\n# Root\n",
        ),
        (
            "lobe/x.md",
            "---\ncreated: 2026-01-01\nupdated: 2026-01-01\nwhen: 2026-01-02T10:00:00Z\nnested: [2026-01-03]\nbase: &a [p, q]\ncopy: *a\n---\n\n# X\n",
        ),
        ("lobe/y.yml", "#---\n# updated: '2026-01-01'\n#---\nk: 1\n"),
    ]);
    let report = b.dream(&FakeGit::none());
    assert_eq!(report.fixes.parents_inferred, 2);
    // Top-level dates and datetimes quoted (the converged form), nested
    // ones typed, anchors as plain copies.
    assert_eq!(
        b.text("lobe/x.md"),
        "---\nbase:\n- p\n- q\ncopy:\n- p\n- q\ncreated: '2026-01-01'\nnested:\n- 2026-01-03\nparent: ./map.md\nupdated: '2026-01-01'\nwhen: '2026-01-02T10:00:00+00:00'\n---\n\n# X\n"
    );
    assert_eq!(
        b.text("lobe/y.yml"),
        "#---\n# parent: ./map.md\n# updated: '2026-01-01'\n#---\nk: 1\n"
    );
    assert!(!b.text("root.md").contains("parent"));
}

#[test]
fn reverse_links_are_added_in_path_order() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/s1.md", &neuron("S1", "related:\n- ../other/t.md\n")),
        ("lobe/s2.md", &neuron("S2", "related:\n- ../other/t.md\n")),
        ("lobe/s3.md", &neuron("S3", "related:\n- ./s1.md\n")),
        ("lobe/p.md", &neuron("P", "related:\n- ./q.md\n")),
        ("lobe/q.md", &neuron("Q", "related:\n- p.md\n")),
        ("other/t.md", &neuron("T", "related: not a list\n")),
    ]);
    let report = b.dream(&FakeGit::none());
    assert_eq!(report.fixes.reverse_synapses_added, 3);
    assert_eq!(
        b.text("other/t.md"),
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n- ../lobe/s1.md\n- ../lobe/s2.md\nupdated: '2026-01-01'\n---\n\n# T\n"
    );
    assert_eq!(
        b.text("lobe/s1.md"),
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n- ../other/t.md\n- s3.md\nupdated: '2026-01-01'\n---\n\n# S1\n"
    );
    assert_eq!(b.text("lobe/q.md"), neuron("Q", "related:\n- p.md\n"));
    assert_eq!(report.one_way.len(), 0);
}

#[test]
fn a_file_without_frontmatter_gets_dates_and_a_parent() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/plain.md", "# Plain\n\nText.\n"),
    ]);
    let git = FakeGit::with(&[(
        "lobe/plain.md",
        "2026-02-03T09:30:00+00:00",
        "2026-01-05T09:30:00+00:00",
    )]);
    let report = b.dream(&git);
    assert_eq!(
        b.text("lobe/plain.md"),
        "---\ncreated: '2026-01-05'\nparent: ./map.md\nupdated: '2026-02-03'\n---\n\n# Plain\n\nText.\n"
    );
    assert_eq!(
        report.fixes,
        Fixes {
            dates_updated: 1,
            parents_inferred: 1,
            reverse_synapses_added: 0,
            orphan_references_added: 1,
            total: 3,
        }
    );
}

// --- steps 4-6: maps -------------------------------------------------------------------

fn maps_brain() -> Brain {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("alpha-lobe/one.md", &neuron("One Title", "")),
        (
            "alpha-lobe/two-words.md",
            "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\nno heading\n",
        ),
        (
            "alpha-lobe/spec.yml",
            "#---\n# parent: ./map.md\n# title: Spec T\n# updated: '2026-01-01'\n#---\nk: 1\n",
        ),
        ("alpha-lobe/raw.yml", "k: 1\n"),
        ("alpha-lobe/Upper.MD", "# Up\n"),
        ("alpha-lobe/README.md", "# Readme\n"),
        (
            "alpha-lobe/map.md",
            "# Alpha Lobe\n\nAlpha described here.\n\nup [brain.md](../brain.md)\n",
        ),
        ("alpha-lobe/sub/deep.md", &neuron("Deep", "")),
        (
            "beta/map.md",
            "---\ndescription: Beta things\n---\n\n# Beta\n\nold body\n",
        ),
        ("alpha-lobe/.hidden/map.md", "# Hidden\n"),
        (".archive/map.md", "# Archive\n"),
        (".archive/old.md", "# Old\n"),
        ("node_modules/pkg/index.md", "# pkg\n"),
        ("alpha-lobe/__pycache__/x.md", "# x\n"),
    ]);
    b.mkdir(".github/workflows");
    b
}

const SUB_MAP: &str = "---\nauto_generated: true\nparent: ../map.md\nsiblings: []\nupdated: '2026-10-26'\n---\n\n# Sub\n\nup [alpha-lobe](../map.md)\n\n## Contents\n\n- [deep.md](./deep.md) — Deep\n";

/// `sorted(iterdir())` compares names by code point, and on Windows by
/// `str.lower()`, so `Upper.MD` moves there.
const ALPHA_MAP: &str = if cfg!(windows) {
    "---\nauto_generated: true\ndescription: Alpha described here.\nparent: ../brain.md\nsiblings:\n- ../beta/map.md\nupdated: '2026-10-26'\n---\n\n# Alpha Lobe\n\nAlpha described here.\n\nup [brain.md](../brain.md)\nsideways [beta](../beta/map.md)\n\n## Contents\n\n- [sub/](./sub/map.md)\n- [one.md](./one.md) — One Title\n- [spec.yml](./spec.yml) — Spec T\n- [two-words.md](./two-words.md) — Two Words\n- [Upper.MD](./Upper.MD) — Up\n"
} else {
    "---\nauto_generated: true\ndescription: Alpha described here.\nparent: ../brain.md\nsiblings:\n- ../beta/map.md\nupdated: '2026-10-26'\n---\n\n# Alpha Lobe\n\nAlpha described here.\n\nup [brain.md](../brain.md)\nsideways [beta](../beta/map.md)\n\n## Contents\n\n- [sub/](./sub/map.md)\n- [Upper.MD](./Upper.MD) — Up\n- [one.md](./one.md) — One Title\n- [spec.yml](./spec.yml) — Spec T\n- [two-words.md](./two-words.md) — Two Words\n"
};

const BETA_MAP: &str = "---\nauto_generated: true\ndescription: Beta things\nparent: ../brain.md\nsiblings:\n- ../alpha-lobe/map.md\nupdated: '2026-10-26'\n---\n\n# Beta\n\nBeta things\n\nup [brain.md](../brain.md)\nsideways [alpha-lobe](../alpha-lobe/map.md)\n\n## Contents\n\n(empty)\n";

const BRAIN_MD: &str = "---\nauto_generated: true\nupdated: '2026-10-26'\n---\n\n# demo\n\nA demo brain\n\n## Lobes\n\n- [alpha-lobe/](./alpha-lobe/map.md) — Alpha described here.\n- [beta/](./beta/map.md) — Beta things\n\n## Reference\n\n- [glossary.md](./glossary.md) — Domain-specific terms, acronyms, and conventions\n";

#[test]
fn maps_and_brain_md_are_generated_like_v2() {
    let b = maps_brain();
    let report = b.dream(&FakeGit::none());
    assert_eq!(b.text("alpha-lobe/sub/map.md"), SUB_MAP);
    assert_eq!(b.text("alpha-lobe/map.md"), ALPHA_MAP);
    assert_eq!(b.text("beta/map.md"), BETA_MAP);
    assert_eq!(b.text("brain.md"), BRAIN_MD);
    assert_eq!(
        report.maps_regenerated,
        ["alpha-lobe/sub", "alpha-lobe", "beta"]
    );
    assert_eq!(report.lobes, ["alpha-lobe", "beta"]);
}

#[test]
fn pruned_directories_get_no_map_and_are_never_listed() {
    let b = maps_brain();
    b.dream(&FakeGit::none());
    for rel in [
        ".github/map.md",
        ".github/workflows/map.md",
        "node_modules/map.md",
        "node_modules/pkg/map.md",
        "alpha-lobe/__pycache__/map.md",
    ] {
        assert!(!b.exists(rel), "{rel} written");
    }
    assert_eq!(b.text(".archive/map.md"), "# Archive\n");
    assert_eq!(b.text("alpha-lobe/.hidden/map.md"), "# Hidden\n");
    assert!(!b.text("alpha-lobe/map.md").contains("hidden"));
    assert!(!b.text("beta/map.md").contains("archive"));
}

#[test]
fn a_map_is_written_only_when_its_body_changed() {
    let b = maps_brain();
    b.dream(&FakeGit::none());
    let first = b.snapshot();
    // Stale metadata, same body: kept as it is.
    let stale = b.text("beta/map.md").replace(
        "updated: '2026-10-26'",
        "updated: '2020-01-01'\nextra: kept",
    );
    b.write("beta/map.md", stale.as_bytes());
    let report = b.dream_on(&FakeGit::none(), Date::new(2026, 10, 27).unwrap());
    assert_eq!(b.text("beta/map.md"), stale);
    let mut after = b.snapshot();
    after.insert("beta/map.md".into(), first["beta/map.md"].clone());
    assert_eq!(after, first, "a second dream changed files");
    assert!(report.written.is_empty(), "{:?}", report.written);

    // A changed body is rewritten, with today's date.
    b.write("alpha-lobe/new.md", neuron("New", "").as_bytes());
    let report = b.dream_on(&FakeGit::none(), Date::new(2026, 10, 28).unwrap());
    assert!(
        b.text("alpha-lobe/map.md")
            .contains("- [new.md](./new.md) — New\n")
    );
    assert!(
        b.text("alpha-lobe/map.md")
            .contains("updated: '2026-10-28'")
    );
    assert_eq!(report.written, ["alpha-lobe/map.md"]);
}

#[test]
fn brain_md_with_no_lobes() {
    let b = Brain::new(&[("glossary.md", GLOSSARY)]);
    b.dream(&FakeGit::none());
    assert_eq!(
        b.text("brain.md"),
        "---\nauto_generated: true\nupdated: '2026-10-26'\n---\n\n# demo\n\nA demo brain\n\n## Lobes\n\n\n\n## Reference\n\n- [glossary.md](./glossary.md) — Domain-specific terms, acronyms, and conventions\n"
    );
}

// --- line endings ----------------------------------------------------------------

#[test]
fn files_keep_their_line_endings_and_new_files_are_lf() {
    let crlf = |text: &str| text.replace('\n', "\r\n");
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/a.md", &crlf(&neuron("A", ""))),
        (
            "lobe/n.yml",
            &crlf("#---\n# parent: ./map.md\n# updated: '2020-01-01'\n#---\nkey: v\n"),
        ),
        ("other/map.md", &crlf("# Other\n\nOld body.\n")),
        ("other/b.md", &neuron("B", "")),
    ]);
    let git = FakeGit::with(&[
        (
            "lobe/a.md",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
        (
            "lobe/n.yml",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
    ]);
    b.dream(&git);
    assert_eq!(
        b.text("lobe/a.md"),
        crlf("---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-03'\n---\n\n# A\n")
    );
    assert_eq!(
        b.text("lobe/n.yml"),
        crlf("#---\n# parent: ./map.md\n# updated: '2026-02-03'\n#---\nkey: v\n")
    );
    assert!(b.text("other/map.md").contains("\r\n## Contents\r\n"));
    assert!(!b.text("other/map.md").replace("\r\n", "").contains('\n'));
    assert!(!b.text("lobe/map.md").contains('\r'));
    assert!(!b.text("brain.md").contains('\r'));
}

// --- validators ---------------------------------------------------------------------

#[test]
fn validators_report_what_remains() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("README.md", "# Readme\n\n[broken](./nope.md)\n"),
        (
            "lobe/a.md",
            &neuron(
                "A",
                "related:\n- ./b.md\n- ./gone.md\n",
            )
            .replace(
                "# A\n",
                "# A\n\n[ok](./b.md) [missing](./nope.md#x) [web](https://e.com) [dir](../lobe) [out](../../outside.md) [q](./b.md?x=1)\n",
            ),
        ),
        ("lobe/b.md", &neuron("B", "related:\n- ./a.md\n")),
        ("lobe/d.md", &neuron("D", "related:\n- ../README.md\n")),
        (
            "lobe/e.md",
            "---\nparent: ./map.md\nrelated: y\nreplaced_by: [1]\ntags: x\n---\n\n# E\n",
        ),
        ("lobe/f.yml", "#---\n# parent: ./map.md\n#---\nk: 1\n"),
        ("lobe/old.md", &neuron("Old", "status: deprecated\n")),
        ("lobe/user.md", &neuron("User", "related:\n- ./old.md\n")),
        ("root.md", &neuron("Root", "")),
    ]);
    let report = b.dream(&FakeGit::none());
    let broken = |file: &str, target: &str| BrokenSynapse {
        file: file.into(),
        target: target.into(),
    };
    assert_eq!(
        report.broken,
        [
            broken("lobe/a.md", "./nope.md"),
            broken("lobe/a.md", "../../outside.md"),
            broken("lobe/a.md", "./gone.md"),
        ]
    );
    assert_eq!(report.one_way.len(), 1);
    assert_eq!(report.one_way[0].source, "lobe/d.md");
    assert_eq!(report.one_way[0].target, "README.md");
    assert_eq!(report.orphans, ["root.md"]);
    let issue = |file: &str, field: &'static str, problem| FrontmatterIssue {
        file: file.into(),
        field,
        problem,
    };
    assert_eq!(
        report.frontmatter_issues,
        [
            issue("lobe/e.md", "created", FieldProblem::Missing),
            issue("lobe/e.md", "updated", FieldProblem::Missing),
            issue("lobe/e.md", "related", FieldProblem::Type),
            issue("lobe/e.md", "tags", FieldProblem::Type),
            issue("lobe/e.md", "replaced_by", FieldProblem::Type),
            issue("lobe/f.yml", "updated", FieldProblem::Missing),
        ]
    );
    assert_eq!(
        report.deprecation,
        [
            DeprecationIssue::DeprecatedWithoutReplacement {
                file: "lobe/old.md".into()
            },
            DeprecationIssue::ActiveLinksToDeprecated {
                source: "lobe/user.md".into(),
                target: "lobe/old.md".into()
            },
        ]
    );
    assert!(!report.healthy());
}

#[test]
fn deprecation_warnings_alone_keep_the_brain_healthy() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/old.md", &neuron("Old", "status: deprecated\n")),
    ]);
    let report = b.dream(&FakeGit::none());
    assert_eq!(report.deprecation.len(), 1);
    assert!(report.healthy(), "{report:?}");
}

// --- one change set, files skipped -------------------------------------------------------

#[test]
fn every_write_is_listed_once_and_a_neuron_that_does_not_parse_is_skipped() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("lobe/bad.md", "---\ntitle: Foo: bar\n---\n\n# Bad\n"),
        (
            "lobe/good.md",
            "---\nupdated: '2020-01-01'\n---\n\n# Good\n",
        ),
    ]);
    let git = FakeGit::with(&[
        (
            "lobe/bad.md",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
        (
            "lobe/good.md",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
    ]);
    let before = b.snapshot();
    let report = b.dream(&git);
    assert_eq!(b.text("lobe/bad.md").as_bytes(), &before["lobe/bad.md"][..]);
    assert_eq!(
        b.text("lobe/good.md"),
        "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-02-03'\n---\n\n# Good\n"
    );
    assert!(
        report.skipped.iter().any(|s| s.rel == "lobe/bad.md"),
        "{:?}",
        report.skipped
    );
    assert_eq!(report.written, ["brain.md", "lobe/good.md", "lobe/map.md"]);
    let after = b.snapshot();
    let changed: Vec<&String> = after
        .keys()
        .filter(|rel| before.get(*rel) != after.get(*rel))
        .collect();
    assert_eq!(changed, ["brain.md", "lobe/good.md", "lobe/map.md"]);
}

// --- write safety (review T-103) ---------------------------------------------------

/// A YAML neuron with a pending date refresh and the byte `FF` in its body.
const NOT_UTF8_YAML: &[u8] =
    b"#---\n# parent: ./map.md\n# title: Spec\n# updated: '2026-01-01'\n#---\nopenapi: 3.0.0\ninfo: invalid-\xff-byte\n";

const PENDING_DATES: [(&str, &str, &str); 1] = [(
    "notes/spec.yml",
    "2026-02-03T09:30:00+00:00",
    "2026-01-01T00:00:00+00:00",
)];

#[test]
fn a_yaml_neuron_that_is_not_utf8_is_skipped_and_left_byte_identical() {
    let b = Brain::new(&[("glossary.md", GLOSSARY)]);
    b.write("notes/spec.yml", NOT_UTF8_YAML);
    let report = b.dream(&FakeGit::with(&PENDING_DATES));
    assert_eq!(fs::read(b.path("notes/spec.yml")).unwrap(), NOT_UTF8_YAML);
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.rel == "notes/spec.yml" && s.reason == "the file is not valid UTF-8"),
        "{:?}",
        report.skipped
    );
    assert!(!report.written.contains(&"notes/spec.yml".to_string()));
    assert_eq!(report.fixes.dates_updated, 0);
    assert!(!report.healthy());
}

/// A `map.md` whose frontmatter does not parse, with text a person wrote.
const MALFORMED_MAP: &str = "---\nrelated: [\n---\n\n# Notes\n\nKeep this.\n";

#[test]
fn an_unparseable_map_is_kept_and_reported() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("notes/map.md", MALFORMED_MAP),
        ("notes/a.md", &neuron("A", "")),
    ]);
    let report = b.dream(&FakeGit::none());
    assert_eq!(b.text("notes/map.md"), MALFORMED_MAP);
    assert!(
        report.skipped.iter().any(|s| s.rel == "notes/map.md"),
        "{:?}",
        report.skipped
    );
    assert!(!report.written.contains(&"notes/map.md".to_string()));
}

#[test]
fn an_unparseable_brain_md_is_kept_and_reported() {
    let bad = "---\nupdated: [\n---\n\n# demo\n\nMy own words.\n";
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("brain.md", bad),
        ("notes/a.md", &neuron("A", "")),
    ]);
    let report = b.dream(&FakeGit::none());
    assert_eq!(b.text("brain.md"), bad);
    assert!(
        report.skipped.iter().any(|s| s.rel == "brain.md"),
        "{:?}",
        report.skipped
    );
    assert!(!report.written.contains(&"brain.md".to_string()));
}

/// The invariant behind T-103 findings 3 and 4: over every scenario, a path
/// dream lists in `skipped` keeps its bytes (through a symlink too) and is
/// not among the written files.
#[test]
fn a_skipped_path_is_never_written() {
    let bad_md = "---\ntitle: Foo: bar\n---\n\n# Bad\n";
    let not_utf8_md: &[u8] = b"---\ncreated: '2026-01-01'\n---\n\n# Caf\xe9\n";
    let bad_block = "#---\n# title: Foo: bar\n#---\nk: 1\n";
    let scenarios: Vec<Vec<(&str, &[u8])>> = vec![
        vec![("notes/bad.md", bad_md.as_bytes())],
        vec![("notes/bad.md", not_utf8_md)],
        vec![("notes/bad.yml", bad_block.as_bytes())],
        vec![("notes/spec.yml", NOT_UTF8_YAML)],
        vec![("notes/map.md", MALFORMED_MAP.as_bytes())],
        vec![("brain.md", b"---\nupdated: [\n---\n\n# demo\n")],
        vec![("notes/map.md", b"---\nupdated: '2026-01-01'\n---\n\n# Caf\xe9\n")],
        vec![
            ("notes/bad.md", bad_md.as_bytes()),
            (
                "notes/links.md",
                b"---\ncreated: '2026-01-01'\nparent: ./map.md\nrelated:\n- ./bad.md\nupdated: '2026-01-01'\n---\n\n# Links\n",
            ),
        ],
        vec![
            ("notes/map.md", MALFORMED_MAP.as_bytes()),
            ("notes/deep/map.md", MALFORMED_MAP.as_bytes()),
            ("notes/deep/x.md", b"# no frontmatter\n"),
        ],
    ];
    let git = FakeGit::with(&[
        (
            "notes/bad.md",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
        (
            "notes/spec.yml",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
        (
            "notes/good.md",
            "2026-02-03T09:30:00+00:00",
            "2026-01-01T00:00:00+00:00",
        ),
    ]);
    for (i, files) in scenarios.iter().enumerate() {
        for linked in [false, true] {
            let b = Brain::new(&[
                ("glossary.md", GLOSSARY),
                (
                    "notes/good.md",
                    "---\nupdated: '2020-01-01'\n---\n\n# Good\n",
                ),
            ]);
            for (rel, bytes) in files {
                b.write(rel, bytes);
            }
            #[cfg(unix)]
            if linked {
                // The same bad file reached through a symlink as well.
                for (rel, _) in files {
                    if !rel.ends_with("map.md") && *rel != "brain.md" {
                        let name = rel.rsplit('/').next().unwrap();
                        std::os::unix::fs::symlink(name, b.path(&format!("{rel}.alias.md")))
                            .unwrap();
                    }
                }
            }
            if linked && cfg!(not(unix)) {
                continue;
            }
            let before = b.snapshot();
            let report = b.dream(&git);
            let after = b.snapshot();
            assert!(!report.skipped.is_empty(), "scenario {i}: nothing skipped");
            for skipped in &report.skipped {
                assert_eq!(
                    before.get(&skipped.rel),
                    after.get(&skipped.rel),
                    "scenario {i} (linked {linked}): {} was skipped but changed",
                    skipped.rel
                );
                assert!(
                    !report.written.contains(&skipped.rel),
                    "scenario {i}: {} is both skipped and written",
                    skipped.rel
                );
            }
        }
    }
}

/// `create`'s regeneration writes over its own scaffold map even when it does
/// not parse (v2's product type writes `description: Sub-folder per project:
/// ...` unquoted), as v2's `_run_dream_on_brain` does; dream keeps such a map.
#[test]
fn create_regenerates_its_own_unparseable_scaffold_map() {
    let map = "---\nauto_generated: true\nparent: ../brain.md\ndescription: Sub-folder per project: branches\nupdated: 2026-10-26\n---\n# Projects\n\nSub-folder per project: branches\n";
    let b = Brain::new(&[("glossary.md", GLOSSARY), ("projects/map.md", map)]);
    let mut store = LocalGitStore::open(&b.root);
    let skipped = kluris_core::dream::regenerate_maps(
        &mut store,
        &DreamOptions {
            name: "demo",
            description: "A demo brain",
            today: today(),
        },
    );
    assert_eq!(skipped, []);
    assert_eq!(
        b.text("projects/map.md"),
        "---\nauto_generated: true\ndescription: 'Sub-folder per project: branches'\nparent: ../brain.md\nsiblings: []\nupdated: '2026-10-26'\n---\n\n# Projects\n\nSub-folder per project: branches\n\nup [brain.md](../brain.md)\n\n## Contents\n\n(empty)\n"
    );
}

// --- temp files an interrupted write left (review T-106) ---------------------------

/// Set a file's modification time `secs` seconds into the past.
fn age(path: &Path, secs: u64) {
    let when = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn dream_removes_only_old_temp_files_of_its_own_writer() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("notes/a.md", &neuron("A", "")),
        // Old: removed.
        ("notes/.a.md.kluris-tmp-123-4", "half a write"),
        (".kluris.yml.kluris-tmp-9-0", "half a config"),
        // Fresh: another process may still be writing it.
        ("notes/.b.md.kluris-tmp-77-1", "being written"),
        // Look-alikes, old: kept.
        ("notes/a.md.kluris-tmp-1-2", "no leading dot"),
        ("notes/.a.md.kluris-tmp-1-2.md", "a suffix after"),
        ("notes/.a.md.kluris-tmp-x-2", "not a pid"),
        ("notes/.a.md.kluris-tmp-1-", "no counter"),
        ("notes/.a.md.kluris-tmp-12", "one number"),
        ("notes/.kluris-tmp-1-2", "no name"),
        ("notes/.A.md.KLURIS-TMP-1-2", "other case"),
        // Under a directory the walker prunes: never looked at.
        (".archive/.x.md.kluris-tmp-1-2", "pruned"),
    ]);
    let old = [
        "notes/.a.md.kluris-tmp-123-4",
        ".kluris.yml.kluris-tmp-9-0",
        "notes/a.md.kluris-tmp-1-2",
        "notes/.a.md.kluris-tmp-1-2.md",
        "notes/.a.md.kluris-tmp-x-2",
        "notes/.a.md.kluris-tmp-1-",
        "notes/.a.md.kluris-tmp-12",
        "notes/.kluris-tmp-1-2",
        "notes/.A.md.KLURIS-TMP-1-2",
        ".archive/.x.md.kluris-tmp-1-2",
    ];
    for rel in old {
        age(&b.path(rel), 2 * 3600);
    }
    // A directory with a temp file's name: kept.
    b.mkdir("notes/.c.md.kluris-tmp-5-6");
    let report = b.dream(&FakeGit::none());
    assert_eq!(
        report.removed_temp_files,
        [".kluris.yml.kluris-tmp-9-0", "notes/.a.md.kluris-tmp-123-4"]
    );
    assert!(!b.exists("notes/.a.md.kluris-tmp-123-4"));
    assert!(!b.exists(".kluris.yml.kluris-tmp-9-0"));
    for rel in &old[2..] {
        assert!(b.exists(rel), "{rel} was removed");
    }
    assert!(b.exists("notes/.b.md.kluris-tmp-77-1"));
    assert!(b.path("notes/.c.md.kluris-tmp-5-6").is_dir());
}

/// A symlink named like a temp file is never removed, even when what it
/// points at is old.
#[cfg(unix)]
#[test]
fn a_symlink_named_like_a_temp_file_is_kept() {
    let b = Brain::new(&[
        ("glossary.md", GLOSSARY),
        ("notes/a.md", &neuron("A", "")),
        ("notes/old.txt", "mine"),
    ]);
    age(&b.path("notes/old.txt"), 2 * 3600);
    std::os::unix::fs::symlink("old.txt", b.path("notes/.a.md.kluris-tmp-1-2")).unwrap();
    let report = b.dream(&FakeGit::none());
    assert_eq!(report.removed_temp_files, Vec::<String>::new());
    assert!(
        fs::symlink_metadata(b.path("notes/.a.md.kluris-tmp-1-2"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(b.text("notes/old.txt"), "mine");
}

#[test]
fn nothing_to_remove_is_an_empty_list() {
    let b = Brain::new(&[("glossary.md", GLOSSARY), ("notes/a.md", &neuron("A", ""))]);
    assert_eq!(
        b.dream(&FakeGit::none()).removed_temp_files,
        Vec::<String>::new()
    );
}
