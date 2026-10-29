//! The two claims that let commands share one checkout: the remote claim
//! (one network sync at a time, never waited for) and the tree claim
//! (exclusive to change the work tree, shared to read it, waited for in
//! steps up to a bound). Each is an OS lock on a file in the git directory,
//! so it is taken in this process, in another one, and in a linked worktree
//! alike, and goes with the process that holds it.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use kluris_core::sync::claim::{
    COMMAND_BOUND, ClaimError, READ_BOUND, REMOTE_CLAIM, RemoteClaim, STEP, TREE_CLAIM, TreeClaim,
};
use tempfile::TempDir;

/// A bound short enough for a test, long enough for several steps.
const SHORT: Duration = Duration::from_millis(200);

/// The longest a test waits for another process to do something.
const PATIENCE: Duration = Duration::from_secs(30);

/// The child's environment: the git directory it takes its claim in, and
/// which claim (`remote`, `exclusive` or `shared`).
const CHILD_GIT_DIR: &str = "KLURIS_TEST_CLAIM_GIT_DIR";
const CHILD_CLAIM: &str = "KLURIS_TEST_CLAIM_KIND";
const CHILD_HELD: &str = "KLURIS_TEST_CLAIM_HELD";

/// The two ways to take the tree claim, by name.
type Take = fn(&Path, Duration) -> Result<TreeClaim, ClaimError>;
const TAKERS: [(&str, Take); 2] = [
    ("exclusive", TreeClaim::exclusive),
    ("shared", TreeClaim::shared),
];

const ISOLATED_VARS: [&str; 3] = ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"];

/// A git repository in a temp folder.
struct Repo {
    dir: TempDir,
    work: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("gitconfig"), "").unwrap();
        let work = dir.path().join("brain");
        fs::create_dir_all(&work).unwrap();
        let repo = Repo { dir, work };
        repo.git(&repo.work, &["init", "-q", "-b", "main"]);
        repo.git(&repo.work, &["config", "user.name", "Ana Tester"]);
        repo.git(&repo.work, &["config", "user.email", "ana@example.com"]);
        fs::write(repo.work.join("readme.md"), "# Brain\n").unwrap();
        repo.git(&repo.work, &["add", "-A"]);
        repo.git(&repo.work, &["commit", "-q", "-m", "init"]);
        repo
    }

    fn git(&self, cwd: &Path, args: &[&str]) -> String {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", self.dir.path().join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", self.dir.path());
        for var in ISOLATED_VARS {
            cmd.env_remove(var);
        }
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    }

    /// The git directory of the work tree at `work`.
    fn git_dir(&self, work: &Path) -> PathBuf {
        PathBuf::from(self.git(work, &["rev-parse", "--absolute-git-dir"]))
    }

    /// Where git puts `name` in the git directory of the work tree at
    /// `work` (`git rev-parse --git-path`), as an absolute path.
    fn git_path(&self, work: &Path, name: &str) -> PathBuf {
        let path = PathBuf::from(self.git(work, &["rev-parse", "--git-path", name]));
        if path.is_absolute() {
            path
        } else {
            work.join(path)
        }
    }

    /// A linked worktree of the repository on a new branch.
    fn linked_worktree(&self) -> PathBuf {
        let linked = self.dir.path().join("linked");
        let to = linked.to_string_lossy().into_owned();
        self.git(&self.work, &["worktree", "add", "-q", "-b", "linked", &to]);
        linked
    }
}

fn busy<T>(result: Result<T, ClaimError>) -> bool {
    matches!(result, Err(ClaimError::Busy))
}

/// Polls `free` until it holds, at most [`PATIENCE`].
fn eventually(what: &str, mut free: impl FnMut() -> bool) {
    let start = Instant::now();
    while !free() {
        assert!(
            start.elapsed() < PATIENCE,
            "{what}: not within {PATIENCE:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_bounds_and_the_step() {
    assert_eq!(STEP, Duration::from_millis(25));
    assert_eq!(READ_BOUND, Duration::from_secs(3));
    assert_eq!(COMMAND_BOUND, Duration::from_secs(30));
    assert_eq!(REMOTE_CLAIM, "kluris-sync/remote-claim");
    assert_eq!(TREE_CLAIM, "kluris-sync/tree-claim");
}

#[test]
fn the_remote_claim_is_busy_at_once_while_another_holds_it() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let held = RemoteClaim::try_take(&git_dir).unwrap();
    let start = Instant::now();
    assert!(busy(RemoteClaim::try_take(&git_dir)));
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "try_take waited {:?}",
        start.elapsed()
    );
    drop(held);
    let again = RemoteClaim::try_take(&git_dir);
    assert!(again.is_ok(), "{:?}", again.err());
}

#[test]
fn a_dropped_claim_is_free_at_once_while_processes_start() {
    // A process started at the moment a claim is dropped briefly shares
    // its lock file, before it runs and closes it; the claim is free all
    // the same.
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let (stop, stopped) = mpsc::channel::<()>();
    let spawner = thread::spawn(move || {
        while stopped.try_recv().is_err() {
            let _ = Command::new("git")
                .arg("--version")
                .stdout(Stdio::null())
                .status();
        }
    });
    let start = Instant::now();
    let mut rounds = 0;
    while rounds < 2000 && start.elapsed() < Duration::from_secs(10) {
        drop(RemoteClaim::try_take(&git_dir).unwrap());
        drop(TreeClaim::exclusive(&git_dir, Duration::ZERO).unwrap());
        drop(TreeClaim::shared(&git_dir, Duration::ZERO).unwrap());
        rounds += 1;
    }
    stop.send(()).unwrap();
    spawner.join().unwrap();
}

#[test]
fn the_remote_and_tree_claims_are_separate() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let _remote = RemoteClaim::try_take(&git_dir).unwrap();
    let tree = TreeClaim::exclusive(&git_dir, SHORT);
    assert!(tree.is_ok(), "{:?}", tree.err());
}

#[test]
fn a_busy_tree_claim_waits_its_bound_then_is_busy() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let _held = TreeClaim::exclusive(&git_dir, SHORT).unwrap();
    for (what, take) in TAKERS {
        let start = Instant::now();
        assert!(busy(take(&git_dir, SHORT)), "{what} was taken");
        let waited = start.elapsed();
        assert!(waited >= SHORT, "{what} gave up after {waited:?}");
        assert!(
            waited < SHORT + Duration::from_secs(5),
            "{what} waited {waited:?}"
        );
    }
}

#[test]
fn a_zero_bound_tries_once() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let taken = TreeClaim::exclusive(&git_dir, Duration::ZERO);
    assert!(taken.is_ok(), "{:?}", taken.err());
    assert!(busy(TreeClaim::shared(&git_dir, Duration::ZERO)));
}

#[test]
fn a_tree_claim_freed_while_waiting_is_taken() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    for (what, take) in TAKERS {
        let held = TreeClaim::exclusive(&git_dir, SHORT).unwrap();
        let (started, waiting) = mpsc::channel();
        let dir = git_dir.clone();
        let waiter = thread::spawn(move || {
            started.send(()).unwrap();
            let start = Instant::now();
            take(&dir, PATIENCE).map(|_| start.elapsed())
        });
        waiting.recv().unwrap();
        drop(held);
        let waited = waiter.join().unwrap();
        assert!(
            matches!(waited, Ok(waited) if waited < PATIENCE),
            "{what}: {waited:?}"
        );
    }
}

#[test]
fn a_shared_holder_blocks_an_exclusive_taker() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let shared = TreeClaim::shared(&git_dir, SHORT).unwrap();
    assert!(busy(TreeClaim::exclusive(&git_dir, SHORT)));
    drop(shared);
    let exclusive = TreeClaim::exclusive(&git_dir, SHORT);
    assert!(exclusive.is_ok(), "{:?}", exclusive.err());
}

#[test]
fn an_exclusive_holder_blocks_a_shared_taker() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let exclusive = TreeClaim::exclusive(&git_dir, SHORT).unwrap();
    assert!(busy(TreeClaim::shared(&git_dir, SHORT)));
    drop(exclusive);
    let shared = TreeClaim::shared(&git_dir, SHORT);
    assert!(shared.is_ok(), "{:?}", shared.err());
}

#[test]
fn two_shared_holders_coexist() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let first = TreeClaim::shared(&git_dir, SHORT).unwrap();
    let start = Instant::now();
    let second = TreeClaim::shared(&git_dir, SHORT);
    assert!(second.is_ok(), "{:?}", second.err());
    assert!(start.elapsed() < SHORT, "waited {:?}", start.elapsed());
    drop(first);
    // One shared holder left still keeps an exclusive taker out.
    assert!(busy(TreeClaim::exclusive(&git_dir, Duration::ZERO)));
}

#[test]
fn the_claim_files_are_made_in_the_git_dir_and_never_removed() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let remote = git_dir.join("kluris-sync").join("remote-claim");
    let tree = git_dir.join("kluris-sync").join("tree-claim");
    assert!(!git_dir.join("kluris-sync").exists());
    drop(RemoteClaim::try_take(&git_dir).unwrap());
    drop(TreeClaim::exclusive(&git_dir, SHORT).unwrap());
    assert!(remote.is_file(), "{} is gone", remote.display());
    assert!(tree.is_file(), "{} is gone", tree.display());
    drop(TreeClaim::shared(&git_dir, SHORT).unwrap());
    let busy_remote = RemoteClaim::try_take(&git_dir).unwrap();
    assert!(busy(RemoteClaim::try_take(&git_dir)));
    drop(busy_remote);
    assert!(remote.is_file() && tree.is_file());
    // Where git itself puts these names, so callers that ask git find them.
    for (name, file) in [(REMOTE_CLAIM, &remote), (TREE_CLAIM, &tree)] {
        assert_eq!(
            fs::canonicalize(repo.git_path(&repo.work, name)).unwrap(),
            fs::canonicalize(file).unwrap()
        );
    }
}

#[test]
fn a_claim_that_cannot_be_made_is_an_error_not_busy() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    // A file where the claims' folder goes: nothing can be made in it.
    fs::write(git_dir.join("kluris-sync"), "").unwrap();
    assert!(matches!(
        RemoteClaim::try_take(&git_dir),
        Err(ClaimError::Io(_))
    ));
    assert!(matches!(
        TreeClaim::exclusive(&git_dir, SHORT),
        Err(ClaimError::Io(_))
    ));
    assert!(matches!(
        TreeClaim::shared(&git_dir, SHORT),
        Err(ClaimError::Io(_))
    ));
}

#[test]
fn a_linked_worktree_has_claims_of_its_own() {
    let repo = Repo::new();
    let main = repo.git_dir(&repo.work);
    let linked_work = repo.linked_worktree();
    let linked = repo.git_dir(&linked_work);
    assert_ne!(
        fs::canonicalize(&main).unwrap(),
        fs::canonicalize(&linked).unwrap()
    );

    let _main_remote = RemoteClaim::try_take(&main).unwrap();
    let _main_tree = TreeClaim::exclusive(&main, SHORT).unwrap();
    let linked_remote = RemoteClaim::try_take(&linked);
    assert!(linked_remote.is_ok(), "{:?}", linked_remote.err());
    let linked_tree = TreeClaim::exclusive(&linked, SHORT);
    assert!(linked_tree.is_ok(), "{:?}", linked_tree.err());

    // In the linked worktree's git directory, where git puts the names,
    // and they hold there as anywhere.
    for name in [REMOTE_CLAIM, TREE_CLAIM] {
        assert_eq!(
            fs::canonicalize(repo.git_path(&linked_work, name)).unwrap(),
            fs::canonicalize(linked.join(name)).unwrap()
        );
    }
    assert!(busy(RemoteClaim::try_take(&linked)));
    assert!(busy(TreeClaim::shared(&linked, Duration::ZERO)));
}

/// This test binary run again as a child that holds a claim in
/// [`CHILD_GIT_DIR`] until its stdin closes; it writes [`CHILD_HELD`]'s
/// file once the claim is held. Without those variables it does nothing.
#[test]
#[ignore = "run only as the child of the cross-process tests"]
fn child_holding_a_claim() {
    let (Some(git_dir), Some(kind), Some(held)) = (
        std::env::var_os(CHILD_GIT_DIR),
        std::env::var(CHILD_CLAIM).ok(),
        std::env::var_os(CHILD_HELD),
    ) else {
        return;
    };
    let git_dir = PathBuf::from(git_dir);
    let _claim: Box<dyn std::any::Any> = match kind.as_str() {
        "remote" => Box::new(RemoteClaim::try_take(&git_dir).unwrap()),
        "exclusive" => Box::new(TreeClaim::exclusive(&git_dir, Duration::ZERO).unwrap()),
        "shared" => Box::new(TreeClaim::shared(&git_dir, Duration::ZERO).unwrap()),
        other => panic!("no claim {other}"),
    };
    fs::write(held, "").unwrap();
    let mut rest = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut rest);
}

/// A child process holding `kind` in `git_dir`, once it holds it.
struct Holder {
    child: Child,
}

impl Holder {
    fn start(git_dir: &Path, kind: &str, scratch: &Path) -> Holder {
        let held = scratch.join(format!("{kind}-held"));
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "child_holding_a_claim", "--quiet"])
            .env(CHILD_GIT_DIR, git_dir)
            .env(CHILD_CLAIM, kind)
            .env(CHILD_HELD, &held)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut holder = Holder { child };
        eventually(&format!("the child holding {kind}"), || {
            if let Ok(Some(status)) = holder.child.try_wait() {
                panic!("the child holding {kind} ended: {status}");
            }
            held.exists()
        });
        holder
    }

    /// Kills the child (by its own handle) and waits for it to end.
    fn die(mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

#[test]
fn a_remote_claim_held_by_another_process_is_busy_until_it_dies() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let holder = Holder::start(&git_dir, "remote", repo.dir.path());
    assert!(busy(RemoteClaim::try_take(&git_dir)));
    holder.die();
    eventually("the remote claim after its holder died", || {
        RemoteClaim::try_take(&git_dir).is_ok()
    });
}

#[test]
fn an_exclusive_tree_claim_held_by_another_process_blocks_until_it_dies() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let holder = Holder::start(&git_dir, "exclusive", repo.dir.path());
    assert!(busy(TreeClaim::shared(&git_dir, SHORT)));
    assert!(busy(TreeClaim::exclusive(&git_dir, SHORT)));
    holder.die();
    eventually("the tree claim after its holder died", || {
        TreeClaim::exclusive(&git_dir, Duration::ZERO).is_ok()
    });
}

#[test]
fn a_shared_tree_claim_held_by_another_process_blocks_only_exclusive() {
    let repo = Repo::new();
    let git_dir = repo.git_dir(&repo.work);
    let holder = Holder::start(&git_dir, "shared", repo.dir.path());
    assert!(busy(TreeClaim::exclusive(&git_dir, SHORT)));
    let shared = TreeClaim::shared(&git_dir, SHORT);
    assert!(shared.is_ok(), "{:?}", shared.err());
    drop(shared);
    holder.die();
    eventually("the tree claim after its holder died", || {
        TreeClaim::exclusive(&git_dir, Duration::ZERO).is_ok()
    });
}
