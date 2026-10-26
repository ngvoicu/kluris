//! The git layer (`kluris_core::git`) against real git in temp dirs.
//!
//! Every git run (the layer's and the tests' own) gets an empty global config
//! file, no system config, a ceiling at the temp dir (so no repo above it is
//! found) and pinned dates, so nothing depends on the machine's git setup.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use kluris_core::git::{Commit, Git, SystemGit};
use tempfile::TempDir;

const DATE: &str = "2026-01-01T12:00:00+00:00";
const IDENTITY: [&str; 4] = [
    "GIT_AUTHOR_NAME",
    "GIT_AUTHOR_EMAIL",
    "GIT_COMMITTER_NAME",
    "GIT_COMMITTER_EMAIL",
];

/// A temp root with an empty git config file in it.
struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig-empty"), "").unwrap();
        Sandbox { dir }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn path(&self, rel: &str) -> PathBuf {
        let path = self.root().join(rel);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            (
                "GIT_CONFIG_GLOBAL",
                self.root().join("gitconfig-empty").display().to_string(),
            ),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_CEILING_DIRECTORIES", self.root().display().to_string()),
            ("GIT_AUTHOR_DATE", DATE.into()),
            ("GIT_COMMITTER_DATE", DATE.into()),
        ]
    }

    /// The layer with the sandbox env and a pinned identity.
    fn git(&self) -> SystemGit {
        let mut git = self.anonymous_git();
        for key in IDENTITY {
            let value = if key.ends_with("EMAIL") {
                "test@kluris.invalid"
            } else {
                "Kluris Test"
            };
            git = git.env(key, value);
        }
        git
    }

    /// The layer with the sandbox env and no identity in the environment.
    fn anonymous_git(&self) -> SystemGit {
        let mut git = SystemGit::new();
        for (key, value) in self.env() {
            git = git.env(key, value);
        }
        for key in IDENTITY {
            git = git.env_remove(key);
        }
        git
    }

    /// Run git directly (test setup and inspection); its stdout.
    fn run(&self, dir: &Path, args: &[&str]) -> String {
        self.run_with(dir, args, &[])
    }

    fn run_with(&self, dir: &Path, args: &[&str], extra: &[(&str, &str)]) -> String {
        let mut cmd = Command::new("git");
        cmd.args(args).current_dir(dir);
        for (key, value) in self.env() {
            cmd.env(key, value);
        }
        cmd.env("GIT_AUTHOR_NAME", "Setup")
            .env("GIT_AUTHOR_EMAIL", "setup@kluris.invalid")
            .env("GIT_COMMITTER_NAME", "Setup")
            .env("GIT_COMMITTER_EMAIL", "setup@kluris.invalid");
        for (key, value) in extra {
            cmd.env(key, value);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// `git init` (plus `-c core.autocrlf=false`) at `rel`.
    fn repo(&self, rel: &str) -> PathBuf {
        let dir = self.path(rel);
        self.run(&dir, &["init", "-q"]);
        self.run(&dir, &["config", "core.autocrlf", "false"]);
        dir
    }

    /// Write files (relative to `dir`), stage everything and commit at `date`.
    fn commit(&self, dir: &Path, files: &[(&str, &str)], message: &str, date: &str) {
        for (rel, text) in files {
            let path = dir.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        self.run(dir, &["add", "-A"]);
        self.run_with(
            dir,
            &["commit", "-q", "--allow-empty", "-m", message],
            &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
        );
    }
}

fn subjects(commits: &[Commit]) -> Vec<&str> {
    commits.iter().map(|c| c.subject.as_str()).collect()
}

// --- is the brain in a repo -------------------------------------------------

#[test]
fn a_brain_without_git() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    let git = sb.git();
    assert_eq!(git.repo(&brain), None);
    assert!(!git.is_git_repo(&brain));
}

#[test]
fn a_missing_git_program_is_no_repo_and_an_error() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let git = sb.git().program("kluris-test-no-such-git");
    assert!(!git.is_git_repo(&brain));
    assert!(git.init(&sb.path("other")).is_err());
    assert!(git.version().is_err());
}

#[test]
fn a_repo_at_the_brain_root_is_not_nested() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let repo = sb.git().repo(&brain).expect("a repo");
    assert_eq!(repo.prefix, "");
    assert!(!repo.nested());
    assert_eq!(repo.dir, brain);
}

#[test]
fn the_git_dir_itself_is_not_a_work_tree() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    assert!(!sb.git().is_git_repo(&brain.join(".git")));
}

#[test]
fn a_brain_inside_a_bigger_repo_is_nested() {
    let sb = Sandbox::new();
    let outer = sb.repo("outer");
    let brain = sb.path("outer/docs/brain");
    let repo = sb.git().repo(&brain).expect("the outer repo");
    assert_eq!(repo.prefix, "docs/brain/");
    assert!(repo.nested());
    assert!(sb.git().is_git_repo(&outer));
}

#[test]
fn version_names_git() {
    let sb = Sandbox::new();
    assert!(sb.git().version().unwrap().starts_with("git version "));
}

// --- init, branch, remote, add, commit --------------------------------------

#[test]
fn init_creates_a_repo_on_main() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    sb.git().init(&brain).unwrap();
    assert!(brain.join(".git").is_dir());
    assert_eq!(
        sb.run(&brain, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
}

#[test]
fn init_on_main_when_main_is_already_the_default() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    fs::write(
        sb.root().join("gitconfig-empty"),
        "[init]\n\tdefaultBranch = main\n",
    )
    .unwrap();
    sb.git().init(&brain).unwrap();
    assert_eq!(
        sb.run(&brain, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/main"
    );
}

#[test]
fn checkout_new_branch_and_remote_add() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    let git = sb.git();
    git.init(&brain).unwrap();
    git.checkout_new_branch(&brain, "trunk").unwrap();
    git.remote_add(&brain, "origin", "git@example.invalid:team/brain.git")
        .unwrap();
    assert_eq!(
        sb.run(&brain, &["symbolic-ref", "HEAD"]).trim(),
        "refs/heads/trunk"
    );
    assert_eq!(
        sb.run(&brain, &["remote", "get-url", "origin"]).trim(),
        "git@example.invalid:team/brain.git"
    );
    assert!(git.remote_add(&brain, "origin", "x").is_err(), "exists");
}

#[test]
fn add_and_commit() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    let git = sb.git();
    git.init(&brain).unwrap();
    fs::write(brain.join("brain.md"), "# demo\n").unwrap();
    fs::create_dir(brain.join("lobe")).unwrap();
    fs::write(brain.join("lobe").join("n.md"), "x\n").unwrap();
    git.add(&brain, "-A").unwrap();
    git.commit(&brain, "brain: initialize demo").unwrap();
    assert_eq!(
        sb.run(&brain, &["log", "--format=%s|%an|%ae|%aI"]),
        "brain: initialize demo|Kluris Test|test@kluris.invalid|2026-01-01T12:00:00Z\n"
    );
    assert_eq!(
        sb.run(&brain, &["ls-files"]),
        "brain.md\nlobe/n.md\n",
        "add -A staged everything"
    );
}

#[test]
fn commit_with_nothing_to_commit_is_an_error() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    let git = sb.git();
    git.init(&brain).unwrap();
    let err = git.commit(&brain, "empty").unwrap_err();
    assert!(err.to_string().contains("git commit"), "{err}");
}

// --- the commit identity fallback (v2 `_commit_env`) ------------------------

fn identity(sb: &Sandbox, dir: &Path) -> String {
    sb.run(dir, &["log", "-1", "--format=%an <%ae> / %cn <%ce>"])
        .trim()
        .to_string()
}

fn commit_one(sb: &Sandbox, git: &SystemGit, dir: &Path) {
    fs::write(dir.join("a.md"), "a\n").unwrap();
    git.add(dir, "-A").unwrap();
    git.commit(dir, "one").unwrap();
    let _ = sb;
}

#[test]
fn identity_fallback_without_any_identity() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let git = sb.anonymous_git();
    commit_one(&sb, &git, &brain);
    assert_eq!(
        identity(&sb, &brain),
        "kluris <kluris@local> / kluris <kluris@local>"
    );
}

#[test]
fn identity_from_repo_config_is_kept() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.run(&brain, &["config", "user.name", "Ann"]);
    sb.run(&brain, &["config", "user.email", "ann@kluris.invalid"]);
    let git = sb.anonymous_git();
    commit_one(&sb, &git, &brain);
    assert_eq!(
        identity(&sb, &brain),
        "Ann <ann@kluris.invalid> / Ann <ann@kluris.invalid>"
    );
}

#[test]
fn identity_from_global_config_is_kept() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    fs::write(
        sb.root().join("gitconfig-empty"),
        "[user]\n\tname = Glob\n\temail = glob@kluris.invalid\n",
    )
    .unwrap();
    let git = sb.anonymous_git();
    commit_one(&sb, &git, &brain);
    assert_eq!(
        identity(&sb, &brain),
        "Glob <glob@kluris.invalid> / Glob <glob@kluris.invalid>"
    );
}

#[test]
fn a_half_configured_identity_gets_the_whole_fallback() {
    // v2 injects all four variables unless BOTH user.name and user.email are
    // set; the environment then wins over the configured name.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.run(&brain, &["config", "user.name", "Ann"]);
    let git = sb.anonymous_git();
    commit_one(&sb, &git, &brain);
    assert_eq!(
        identity(&sb, &brain),
        "kluris <kluris@local> / kluris <kluris@local>"
    );
}

#[test]
fn identity_variables_already_set_win_over_the_fallback() {
    // `setdefault`: a set author name stays, the committer name defaults to it.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let git = sb.anonymous_git().env("GIT_AUTHOR_NAME", "Env Author");
    commit_one(&sb, &git, &brain);
    assert_eq!(
        identity(&sb, &brain),
        "Env Author <kluris@local> / Env Author <kluris@local>"
    );
}

#[test]
fn init_sets_no_identity() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    sb.anonymous_git().init(&brain).unwrap();
    let config = fs::read_to_string(brain.join(".git").join("config")).unwrap();
    assert!(!config.contains("[user]"), "{config}");
}

// --- log --------------------------------------------------------------------

#[test]
fn log_lists_the_newest_commits_first_up_to_the_limit() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    for i in 1..=12 {
        let day = format!("2026-01-{i:02}T08:30:00+02:00");
        sb.commit(&brain, &[("n.md", &i.to_string())], &format!("c{i}"), &day);
    }
    let git = sb.git();
    let repo = git.repo(&brain).unwrap();
    let log = git.log(&repo, 10).unwrap();
    assert_eq!(
        subjects(&log),
        [
            "c12", "c11", "c10", "c9", "c8", "c7", "c6", "c5", "c4", "c3"
        ]
    );
    assert_eq!(log[0].date, "2026-01-12T08:30:00+02:00", "%aI, author zone");
    assert_eq!(log[0].hash.len(), 40);
    assert_eq!(
        log[0].hash,
        sb.run(&brain, &["rev-parse", "HEAD"]).trim(),
        "full hash"
    );
}

#[test]
fn log_keeps_a_subject_with_pipes() {
    // v2 split `%H|%s|%aI` on `|`: this subject became "has " and its date
    // " pipe | char|2026-..." (research-03 §5 item 29).
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("a.md", "a")], "has | pipe | char", DATE);
    let git = sb.git();
    let log = git.log(&git.repo(&brain).unwrap(), 10).unwrap();
    assert_eq!(subjects(&log), ["has | pipe | char"]);
    assert_eq!(log[0].date, "2026-01-01T12:00:00Z");
}

#[test]
fn log_keeps_empty_and_non_ascii_subjects() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("a.md", "a")], "Grüße — ✓", DATE);
    sb.run(
        &brain,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "--allow-empty-message",
            "-m",
            "",
        ],
    );
    let git = sb.git();
    let log = git.log(&git.repo(&brain).unwrap(), 10).unwrap();
    assert_eq!(subjects(&log), ["", "Grüße — ✓"]);
}

#[test]
fn log_joins_a_multi_line_message_into_the_subject() {
    // %s: the first paragraph, its lines joined with spaces (as in v2).
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("a.md", "a")], "line one\nline two\n\nbody", DATE);
    let git = sb.git();
    let log = git.log(&git.repo(&brain).unwrap(), 10).unwrap();
    assert_eq!(subjects(&log), ["line one line two"]);
}

#[test]
fn log_of_a_repo_without_commits_is_empty() {
    // v2's `git log` fails there (no HEAD) and `status` ends in a traceback.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let git = sb.git();
    assert_eq!(git.log(&git.repo(&brain).unwrap(), 10).unwrap(), []);
}

#[test]
fn log_keeps_empty_commits_at_the_repo_root() {
    // No pathspec at the repo root: `git log -- .` would drop these.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("a.md", "a")], "first", DATE);
    sb.commit(&brain, &[], "empty", DATE);
    let git = sb.git();
    let log = git.log(&git.repo(&brain).unwrap(), 10).unwrap();
    assert_eq!(subjects(&log), ["empty", "first"]);
}

#[test]
fn log_of_a_nested_brain_lists_only_its_own_commits() {
    // v2 listed the whole outer repo's log (research-03 §5 item 26).
    let sb = Sandbox::new();
    let outer = sb.repo("outer");
    sb.commit(&outer, &[("README.md", "r")], "outer only", DATE);
    sb.commit(&outer, &[("brain/a.md", "a")], "brain one", DATE);
    sb.commit(&outer, &[("src/x.rs", "x")], "outer again", DATE);
    sb.commit(
        &outer,
        &[("brain/b.md", "b"), ("src/y.rs", "y")],
        "both",
        DATE,
    );
    let git = sb.git();
    let repo = git.repo(&outer.join("brain")).unwrap();
    assert_eq!(
        subjects(&git.log(&repo, 10).unwrap()),
        ["both", "brain one"]
    );
    let root = git.repo(&outer).unwrap();
    assert_eq!(git.log(&root, 10).unwrap().len(), 4);
}

// --- status -----------------------------------------------------------------

#[test]
fn status_of_a_clean_repo_is_empty() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("a.md", "a")], "first", DATE);
    let git = sb.git();
    assert_eq!(git.status_short(&git.repo(&brain).unwrap()).unwrap(), "");
}

#[test]
fn status_is_short_status_stripped_like_python() {
    // `.strip()` of the whole text: the first line loses its leading space.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("a.md", "a"), ("b.md", "b")], "first", DATE);
    fs::write(brain.join("a.md"), "changed").unwrap();
    fs::write(brain.join("b.md"), "changed").unwrap();
    fs::write(brain.join("new.md"), "n").unwrap();
    let git = sb.git();
    assert_eq!(
        git.status_short(&git.repo(&brain).unwrap()).unwrap(),
        "M a.md\n M b.md\n?? new.md"
    );
}

#[test]
fn status_shows_non_ascii_names_unquoted() {
    // `-c core.quotePath=false`; v2 printed `"\303\251.md"`. A name with a
    // space is still quoted by git, as in v2.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(&brain, &[("keep.md", "k")], "first", DATE);
    fs::write(brain.join("é.md"), "e").unwrap();
    fs::write(brain.join("sp ace.md"), "s").unwrap();
    let git = sb.git();
    assert_eq!(
        git.status_short(&git.repo(&brain).unwrap()).unwrap(),
        "?? \"sp ace.md\"\n?? é.md"
    );
}

#[test]
fn status_of_a_repo_without_commits() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    fs::write(brain.join("a.md"), "a").unwrap();
    let git = sb.git();
    assert_eq!(
        git.status_short(&git.repo(&brain).unwrap()).unwrap(),
        "?? a.md"
    );
}

#[test]
fn status_of_a_nested_brain_is_scoped_and_brain_relative() {
    // v2 showed the outer repo's whole status, paths relative to the brain
    // (`../top.md`).
    let sb = Sandbox::new();
    let outer = sb.repo("outer");
    sb.commit(
        &outer,
        &[("top.md", "t"), ("brain/lobe/n.md", "n")],
        "first",
        DATE,
    );
    fs::write(outer.join("top.md"), "changed").unwrap();
    fs::write(outer.join("brain").join("lobe").join("n.md"), "changed").unwrap();
    fs::write(outer.join("brain").join("new.md"), "new").unwrap();
    let git = sb.git();
    let repo = git.repo(&outer.join("brain")).unwrap();
    assert_eq!(git.status_short(&repo).unwrap(), "M lobe/n.md\n?? new.md");
}

// --- the batched file dates (v2 `git_log_file_dates`) ----------------------

#[test]
fn file_dates_latest_and_created_per_path() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(
        &brain,
        &[("a.md", "1"), ("lobe/b.md", "1")],
        "one",
        "2026-01-01T10:00:00+02:00",
    );
    sb.commit(&brain, &[("a.md", "2")], "two", "2026-02-01T10:00:00-05:00");
    sb.commit(&brain, &[], "empty", "2026-03-01T10:00:00+00:00");
    sb.commit(
        &brain,
        &[("a.md", "3"), ("lobe/c.md", "1")],
        "three | piped",
        "2026-04-01T10:00:00+00:00",
    );
    let git = sb.git();
    let dates = git.file_dates(&git.repo(&brain).unwrap());
    let pairs = |map: &std::collections::BTreeMap<String, String>| {
        map.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        pairs(&dates.latest),
        [
            "a.md=2026-04-01T10:00:00Z",
            "lobe/b.md=2026-01-01T10:00:00+02:00",
            "lobe/c.md=2026-04-01T10:00:00Z",
        ]
    );
    assert_eq!(
        pairs(&dates.created),
        [
            "a.md=2026-01-01T10:00:00+02:00",
            "lobe/b.md=2026-01-01T10:00:00+02:00",
            "lobe/c.md=2026-04-01T10:00:00Z",
        ]
    );
}

#[test]
fn file_dates_keep_non_ascii_and_odd_names_as_written() {
    // v2 read `git log --name-only` lines: non-ASCII names came C-quoted
    // (`"lobe/\303\251.md"`) and never matched a neuron.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let mut files = vec![
        ("lobe/é.md", "e"),
        ("sp ace.md", "s"),
        ("2026-01-01", "a name that looks like a date"),
    ];
    if !cfg!(windows) {
        // Names Windows cannot create.
        files.extend([("quo\"te.md", "q"), ("2026-01-01T00:00:00Z", "a date")]);
    }
    sb.commit(&brain, &files, "one", DATE);
    let git = sb.git();
    let dates = git.file_dates(&git.repo(&brain).unwrap());
    let names: Vec<&str> = dates.latest.keys().map(String::as_str).collect();
    let mut expected: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
    expected.sort();
    assert_eq!(names, expected);
    assert!(dates.latest.values().all(|d| d == "2026-01-01T12:00:00Z"));
}

#[test]
fn file_dates_of_a_nested_brain_are_brain_relative() {
    // v2's paths were relative to the outer repo (`brain/a.md`) and never
    // matched the brain's own `a.md`.
    let sb = Sandbox::new();
    let outer = sb.repo("outer");
    sb.commit(
        &outer,
        &[("brain/a.md", "1"), ("top.md", "t")],
        "one",
        "2026-01-01T10:00:00+00:00",
    );
    sb.commit(
        &outer,
        &[("top.md", "only outside")],
        "outside",
        "2026-02-01T10:00:00+00:00",
    );
    sb.commit(
        &outer,
        &[("brain/lobe/b.md", "b")],
        "two",
        "2026-03-01T10:00:00+00:00",
    );
    let git = sb.git();
    let dates = git.file_dates(&git.repo(&outer.join("brain")).unwrap());
    let latest: Vec<String> = dates
        .latest
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    assert_eq!(
        latest,
        [
            "a.md=2026-01-01T10:00:00Z",
            "lobe/b.md=2026-03-01T10:00:00Z"
        ]
    );
    assert_eq!(dates.created, dates.latest);
}

#[test]
fn file_dates_of_a_repo_without_commits_or_without_git_are_empty() {
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    let git = sb.git();
    let dates = git.file_dates(&git.repo(&brain).unwrap());
    assert!(dates.latest.is_empty() && dates.created.is_empty());

    let gone = sb.git().program("kluris-test-no-such-git");
    let dates = gone.file_dates(&git.repo(&brain).unwrap());
    assert!(dates.latest.is_empty() && dates.created.is_empty());
}

#[test]
fn file_dates_drop_deleted_files_nowhere() {
    // Like v2: a path that a commit deleted still has its dates (the log
    // lists it); callers look up only the files they have.
    let sb = Sandbox::new();
    let brain = sb.repo("brain");
    sb.commit(
        &brain,
        &[("gone.md", "g")],
        "add",
        "2026-01-01T10:00:00+00:00",
    );
    fs::remove_file(brain.join("gone.md")).unwrap();
    sb.commit(&brain, &[], "delete", "2026-02-01T10:00:00+00:00");
    let git = sb.git();
    let dates = git.file_dates(&git.repo(&brain).unwrap());
    assert_eq!(dates.latest["gone.md"], "2026-02-01T10:00:00Z");
    assert_eq!(dates.created["gone.md"], "2026-01-01T10:00:00Z");
}

#[test]
fn git_errors_name_the_command() {
    let sb = Sandbox::new();
    let brain = sb.path("brain");
    let err = sb.git().add(&brain, "-A").unwrap_err();
    let text = err.to_string();
    assert!(text.contains("git add -A"), "{text}");
    assert!(text.contains("not a git repository"), "{text}");
}
