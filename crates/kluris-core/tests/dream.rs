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

const ALPHA_MAP: &str = "---\nauto_generated: true\ndescription: Alpha described here.\nparent: ../brain.md\nsiblings:\n- ../beta/map.md\nupdated: '2026-10-26'\n---\n\n# Alpha Lobe\n\nAlpha described here.\n\nup [brain.md](../brain.md)\nsideways [beta](../beta/map.md)\n\n## Contents\n\n- [sub/](./sub/map.md)\n- [Upper.MD](./Upper.MD) — Up\n- [one.md](./one.md) — One Title\n- [spec.yml](./spec.yml) — Spec T\n- [two-words.md](./two-words.md) — Two Words\n";

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
