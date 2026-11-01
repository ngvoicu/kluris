//! The sync engine's git layer (`kluris_core::sync::git`) and its fetch
//! (`kluris_core::sync::fetch`), with real git: a bare remote and clones in
//! temp dirs. The fetch writes no ref and no `FETCH_HEAD`, so a fetch
//! stopped at its bound leaves no ref lock, on Windows (where the stop is
//! `TerminateJobObject`, so git cleans up nothing) as on Unix; the tracking
//! ref moves by compare-and-swap, never through a symbolic ref, and never
//! over another git's value. The push moves no tracking ref itself.

use std::ffi::OsString;
use std::fs;
use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kluris_core::git::{Git, Repo, SystemGit};
use kluris_core::sync::engine;
use kluris_core::sync::fetch::{
    BACKOFF_FIRST, BACKOFF_MAX, Fetch, back_off, backed_off, end_backoff, fetch, fetched_recently,
    write_stamp,
};
use kluris_core::sync::git::{PushOutcome, SyncGit, Upstream, remote_env};
use kluris_core::sync::{LeftReason, SyncReport};

mod common;

use common::*;

/// The bound the stopped-step tests give each fetch and push.
const LIMIT: Duration = Duration::from_secs(3);

/// A `reference-transaction` hook that sleeps in the `prepared` state (git
/// holds the ref's lock then) whenever the transaction moves a
/// remote-tracking ref: well past [`LIMIT`].
const SLOW_TRACKING_HOOK: &str =
    "[ \"$1\" = prepared ] || exit 0\nif grep -q ' refs/remotes/'; then sleep 8; fi\nexit 0";

const TRACKING: &str = "refs/remotes/origin/main";

/// A shell path for a script (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn sync_within(w: &World, limit: Duration) -> SyncReport {
    let mut request = w.request();
    request.network_time_limit = limit;
    engine::sync(&engine::open(&w.ours, &w.git(), &request).unwrap())
}

/// Why the brain was refused; empty when it was not.
fn refused(r: &SyncReport) -> String {
    if left(r, LeftReason::Refused) {
        detail(r).to_string()
    } else {
        String::new()
    }
}

/// The git Kluris runs in "ours", its repo and its upstream.
fn ours(w: &World) -> (SystemGit, Repo, Upstream) {
    let git = w.git();
    let repo = git.repo(&w.ours).unwrap();
    let branch = git.branch(&repo).unwrap().unwrap();
    let upstream = git.upstream(&repo, &branch).unwrap().unwrap();
    (git, repo, upstream)
}

/// [`fetch`] in "ours" with a minute to go.
fn fetch_ours(w: &World) -> Fetch {
    let (git, repo, upstream) = ours(w);
    fetch(
        &git,
        &repo,
        &upstream,
        &w.ours.join(".git"),
        Instant::now() + Duration::from_secs(60),
    )
    .unwrap()
}

/// What "ours" has at `refname`; `None` when there is no such ref.
fn ref_at(w: &World, refname: &str) -> Option<String> {
    let out = w
        .command(&w.ours, &["rev-parse", "--verify", "--quiet", refname])
        .output()
        .unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn tracking(w: &World) -> String {
    ref_at(w, TRACKING).unwrap_or_default()
}

/// Whether "ours" has `refname` as a symbolic ref.
fn is_symbolic(w: &World, refname: &str) -> bool {
    w.command(&w.ours, &["symbolic-ref", "--quiet", refname])
        .output()
        .unwrap()
        .status
        .success()
}

/// Every `*.lock` file under `.git/refs`, and `.git/packed-refs.lock`.
fn ref_locks(at: &Path) -> Vec<PathBuf> {
    let git = at.join(".git");
    let mut found = Vec::new();
    let packed = git.join("packed-refs.lock");
    if fs::symlink_metadata(&packed).is_ok() {
        found.push(packed);
    }
    let mut stack = vec![git.join("refs")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "lock") {
                found.push(path);
            }
        }
    }
    found
}

/// Set "ours"'s `remote.origin.uploadpack` to run `script` (a shell
/// script's body), then git's own upload-pack unless it exits.
fn upload_pack(w: &World, at: &Path, name: &str, script: &str) {
    let path = w.dir.path().join(name);
    fs::write(&path, format!("#!/bin/sh\n{script}")).unwrap();
    w.run(
        at,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", sh_path(&path)),
        ],
    );
}

/// The git's version, as `(major, minor)`.
fn git_version(w: &World) -> (u32, u32) {
    let text = w.run(w.dir.path(), &["--version"]);
    let version = kluris_core::git::parse_version(&text).unwrap_or_else(|| panic!("{text}"));
    (version.0, version.1)
}

/// A loopback http server that answers every request with headers and then
/// sends nothing, holding the connection open.
fn stalling_http_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/x-git-upload-pack-advertisement\r\nContent-Length: 100000\r\n\r\n",
            );
            held.push(stream);
        }
    });
    port
}

/// A loopback http server that answers every request with 401 and a Basic
/// challenge, as a host asking for credentials does.
#[cfg(unix)]
fn auth_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0u8; 4096];
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"kluris\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    port
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

// --- the git layer -------------------------------------------------------------------------

#[test]
fn one_upstream_query_names_the_remote_the_tracking_ref_and_the_branch() {
    let w = World::new();
    let (git, repo, upstream) = ours(&w);
    assert_eq!(
        upstream,
        Upstream {
            branch: "refs/heads/main".into(),
            remote: "origin".into(),
            refname: TRACKING.into(),
            remote_ref: "refs/heads/main".into(),
        }
    );
    // A local branch as upstream, no upstream: none.
    let main = "refs/heads/main";
    w.run(&w.ours, &["branch", "base"]);
    w.run(&w.ours, &["branch", "--set-upstream-to=base"]);
    assert_eq!(git.upstream(&repo, main).unwrap(), None);
    w.run(&w.ours, &["branch", "--unset-upstream"]);
    assert_eq!(git.upstream(&repo, main).unwrap(), None);
    // The upstream is the named branch's, whatever HEAD names; a detached
    // HEAD names no branch.
    w.run(&w.ours, &["branch", "--set-upstream-to=origin/main"]);
    w.run(&w.ours, &["checkout", "-q", "base"]);
    assert_eq!(
        git.branch(&repo).unwrap().as_deref(),
        Some("refs/heads/base")
    );
    assert_eq!(git.upstream(&repo, main).unwrap(), Some(upstream.clone()));
    w.run(&w.ours, &["checkout", "-q", "--detach"]);
    assert_eq!(git.branch(&repo).unwrap(), None);
}

#[test]
fn the_remote_environment_turns_prompts_off_without_overriding_the_users_ssh_command() {
    let none = |_: &str| -> Option<OsString> { None };
    assert_eq!(
        remote_env(none, false),
        [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("GCM_INTERACTIVE", "never"),
            ("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"),
        ]
    );
    // The user's own choices stay: GCM_INTERACTIVE, and an ssh command set
    // by GIT_SSH_COMMAND, GIT_SSH or core.sshCommand.
    let set = |key: &'static str| move |k: &str| (k == key).then(|| OsString::from("mine"));
    assert_eq!(
        remote_env(set("GCM_INTERACTIVE"), false),
        [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("GIT_SSH_COMMAND", "ssh -o BatchMode=yes"),
        ]
    );
    for key in ["GIT_SSH_COMMAND", "GIT_SSH"] {
        assert_eq!(
            remote_env(set(key), false),
            [
                ("GIT_TERMINAL_PROMPT", "0"),
                ("GIT_ASKPASS", ""),
                ("GCM_INTERACTIVE", "never"),
            ],
            "{key}"
        );
    }
    assert_eq!(
        remote_env(none, true),
        [
            ("GIT_TERMINAL_PROMPT", "0"),
            ("GIT_ASKPASS", ""),
            ("GCM_INTERACTIVE", "never"),
        ]
    );
    // An empty variable is not a setting.
    let empty = |k: &str| (k == "GIT_SSH_COMMAND").then(OsString::new);
    assert_eq!(remote_env(empty, false).len(), 4);
}

/// Set by [`a_post_merge_hook_that_reads_stdin_does_not_hang`] for the
/// sync it runs in a child with its stdin held open.
const STDIN_CHILD: &str = "KLURIS_TEST_STDIN_CHILD";

#[test]
#[ignore = "run by a_post_merge_hook_that_reads_stdin_does_not_hang, its stdin held open"]
fn a_sync_whose_post_merge_hook_reads_stdin() {
    assert!(std::env::var_os(STDIN_CHILD).is_some(), "run by its parent");
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let marker = w.dir.path().join("stdin-read");
    w.hook(
        &w.ours,
        "post-merge",
        &format!(
            "if read line; then echo line > '{m}'; else echo eof > '{m}'; fi",
            m = sh_path(&marker)
        ),
    );
    let r = w.sync();
    assert!(r.pulled > 0, "{r:?}");
    assert_eq!(fs::read_to_string(&marker).unwrap(), "eof\n");
}

#[test]
fn a_post_merge_hook_that_reads_stdin_does_not_hang() {
    // The sync runs in a child whose stdin is a pipe this test holds open:
    // a hook that inherited it would wait for a line that never comes.
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "a_sync_whose_post_merge_hook_reads_stdin",
            "--test-threads=1",
        ])
        .env(STDIN_CHILD, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let bound = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() > bound {
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if status.is_none() {
        child.kill().unwrap();
    }
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(status.is_some(), "the sync hung on stdin\n{text}");
    assert!(out.status.success(), "{text}");
    assert!(text.contains("1 passed"), "{text}");
}

/// A fake `ssh` first on `PATH` that serves the remote locally when it runs
/// in batch mode and asks the askpass otherwise, as ssh asks a passphrase;
/// a fake askpass that records each question. Both log to files in the
/// world's folder.
#[cfg(unix)]
struct FakeSsh {
    bin: PathBuf,
    calls: PathBuf,
    asked: PathBuf,
    askpass: PathBuf,
}

#[cfg(unix)]
impl FakeSsh {
    fn new(w: &World) -> FakeSsh {
        let bin = w.dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let calls = w.dir.path().join("ssh-calls");
        let asked = w.dir.path().join("askpass-asked");
        let askpass = w.dir.path().join("askpass.sh");
        fs::write(
            &askpass,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{}'\necho secret\n",
                asked.display()
            ),
        )
        .unwrap();
        make_executable(&askpass);
        let ssh = bin.join("ssh");
        fs::write(
            &ssh,
            format!(
                "#!/bin/sh\necho \"$*\" >> '{calls}'\ncase \" $* \" in\n  *' BatchMode=yes '*) ;;\n  *) '{askpass}' 'passphrase for key:' >/dev/null; exit 255 ;;\nesac\nfor last; do :; done\nexec sh -c \"$last\"\n",
                calls = calls.display(),
                askpass = askpass.display(),
            ),
        )
        .unwrap();
        make_executable(&ssh);
        FakeSsh {
            bin,
            calls,
            asked,
            askpass,
        }
    }

    fn path(&self) -> OsString {
        std::env::join_paths(
            std::iter::once(self.bin.clone()).chain(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            )),
        )
        .unwrap()
    }

    /// `git` with the fake ssh and askpass the user's environment names.
    fn git(&self, w: &World) -> SystemGit {
        w.git()
            .env("PATH", self.path())
            .env("SSH_ASKPASS", &self.askpass)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("GIT_ASKPASS", &self.askpass)
            .env("DISPLAY", ":0")
    }

    fn calls(&self) -> String {
        fs::read_to_string(&self.calls).unwrap_or_default()
    }
}

#[cfg(unix)]
#[test]
fn no_git_the_engine_starts_prompts_through_ssh_or_an_askpass() {
    let w = World::new();
    let ssh = FakeSsh::new(&w);
    let url = format!("ssh://fakehost{}", w.remote.display());
    w.run(&w.ours, &["remote", "set-url", "origin", &url]);
    w.run(
        &w.ours,
        &["config", "core.askPass", &ssh.askpass.to_string_lossy()],
    );
    // Control: a plain git here runs the fake ssh, which asks.
    let control = w
        .command(&w.ours, &["ls-remote", "origin"])
        .env("PATH", ssh.path())
        .env("SSH_ASKPASS", &ssh.askpass)
        .output()
        .unwrap();
    assert!(!control.status.success());
    assert!(
        ssh.asked.exists(),
        "the fake ssh never asks: proves nothing"
    );
    fs::remove_file(&ssh.asked).unwrap();
    fs::remove_file(&ssh.calls).unwrap();

    // Diverged: the fetch, the merge and the push all go through ssh.
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = engine::sync(&engine::open(&w.ours, &ssh.git(&w), &w.request()).unwrap());
    assert!(
        clean(&r) && r.pulled > 0 && r.pushed,
        "{r:?}\n{}",
        ssh.calls()
    );
    assert!(!ssh.asked.exists(), "asked: {}", ssh.calls());

    // A partial clone: the checkout fetches the missing blobs lazily,
    // through ssh too.
    w.run(&w.remote, &["config", "uploadpack.allowFilter", "true"]);
    let partial = w.dir.path().join("partial");
    let clone = w
        .command(
            w.dir.path(),
            &[
                "clone",
                "-q",
                "--filter=blob:none",
                &url,
                &partial.to_string_lossy(),
            ],
        )
        .env("PATH", ssh.path())
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .output()
        .unwrap();
    assert!(clone.status.success(), "{clone:?}");
    w.configure(&partial, "Ana Tester", "ana@example.com");
    w.write(
        &partial,
        "kluris.yml",
        "name: demo\ndescription: A demo brain\n",
    );
    w.run(&w.theirs, &["pull", "-q", "--no-rebase"]);
    w.theirs_push(&[("ideas/later.md", &neuron("", "# Later\n"))], &[]);
    fs::remove_file(&ssh.calls).unwrap();
    let r = engine::sync(&engine::open(&partial, &ssh.git(&w), &w.request()).unwrap());
    assert!(clean(&r) && r.pulled > 0, "{r:?}\n{}", ssh.calls());
    assert!(w.read(&partial, "ideas/later.md").contains("# Later"));
    let calls = ssh.calls();
    assert!(calls.lines().count() >= 3, "{calls}");
    for line in calls.lines() {
        assert!(line.contains("BatchMode=yes"), "{calls}");
    }
    assert!(!ssh.asked.exists(), "asked: {calls}");
}

#[cfg(unix)]
#[test]
fn a_host_asking_for_credentials_is_refused_without_asking_the_askpass() {
    let w = World::new();
    let ssh = FakeSsh::new(&w);
    let port = auth_server();
    let url = format!("http://127.0.0.1:{port}/remote.git");
    w.run(&w.ours, &["remote", "set-url", "origin", &url]);
    w.run(
        &w.ours,
        &["config", "core.askPass", &ssh.askpass.to_string_lossy()],
    );
    // Control: a plain git asks the askpass.
    let _ = w
        .command(&w.ours, &["ls-remote", "origin"])
        .output()
        .unwrap();
    assert!(ssh.asked.exists(), "git never asks: proves nothing");
    fs::remove_file(&ssh.asked).unwrap();
    let before = w.snapshot();
    let r = engine::sync(&engine::open(&w.ours, &ssh.git(&w), &w.request()).unwrap());
    assert!(!left(&r, LeftReason::Offline), "{r:?}");
    let why = refused(&r);
    assert!(why.contains("could not read Username"), "{why}");
    assert!(!ssh.asked.exists(), "the askpass was asked");
    assert_eq!(w.snapshot(), before);
}

/// A partial clone (`--filter=blob:none`) of the remote whose listed commit
/// is not here after the fetch: the remote's upload-pack makes a commit to
/// advertise to `git ls-remote`, then moves `main` to a sibling, which the
/// fetch brings. Each contact with the remote is logged with its
/// `GIT_TERMINAL_PROMPT` and `GIT_NO_LAZY_FETCH`, and one that could prompt,
/// or that comes from a check meant to stay local, stalls for 20 s, so a
/// lazy fetch of the missing commit shows. Sync checks the commit locally
/// and inside the bound: it fetches nothing for the check, lists and
/// fetches again, and fast-forwards to the sibling.
#[test]
fn a_listed_commit_missing_from_a_partial_clone_is_reported_absent_not_fetched() {
    let w = World::new();
    for (key, value) in [
        ("uploadpack.allowFilter", "true"),
        ("uploadpack.allowAnySHA1InWant", "true"),
    ] {
        w.run(&w.remote, &["config", key, value]);
    }
    let remote = sh_path(&w.remote);
    let url = if remote.starts_with('/') {
        format!("file://{remote}")
    } else {
        format!("file:///{remote}")
    };
    let partial = w.dir.path().join("partial");
    w.run(
        w.dir.path(),
        &[
            "clone",
            "-q",
            "--filter=blob:none",
            &url,
            &partial.to_string_lossy(),
        ],
    );
    w.configure(&partial, "Ana Tester", "ana@example.com");
    w.write(
        &partial,
        "kluris.yml",
        "name: demo\ndescription: A demo brain\n",
    );
    let counter = w.dir.path().join("upload-pack-runs");
    let log = w.dir.path().join("upload-pack-contacts");
    let commit = |message: &str| {
        format!(
            "git -C '{remote}' -c user.name=Remote -c user.email=r@example.com commit-tree \"$t\" -p \"$p\" -m '{message}'"
        )
    };
    upload_pack(
        &w,
        &partial,
        "partial-upload-pack.sh",
        &format!(
            "n=$(($(cat '{counter}' 2>/dev/null || echo 0) + 1))\necho $n > '{counter}'\necho \"$n prompt=${{GIT_TERMINAL_PROMPT-}} nolazy=${{GIT_NO_LAZY_FETCH-}}\" >> '{log}'\nif [ \"${{GIT_TERMINAL_PROMPT-}}\" != 0 ] || [ \"${{GIT_NO_LAZY_FETCH-}}\" = 1 ]; then sleep 20; fi\nif [ $n = 1 ]; then\n  t=$(git -C '{remote}' rev-parse 'main^{{tree}}')\n  p=$(git -C '{remote}' rev-parse main)\n  git -C '{remote}' update-ref refs/heads/main \"$({advertised})\"\n  git upload-pack \"$@\" || exit $?\n  git -C '{remote}' update-ref refs/heads/main \"$({sibling})\"\n  exit 0\nfi\nexec git upload-pack \"$@\"\n",
            counter = sh_path(&counter),
            log = sh_path(&log),
            advertised = commit("advertised"),
            sibling = commit("sibling"),
        ),
    );
    let limit = Duration::from_secs(5);
    let mut request = w.request();
    request.network_time_limit = limit;
    let started = Instant::now();
    let report = engine::sync(&engine::open(&partial, &w.git(), &request).unwrap());
    let took = started.elapsed();
    let contacts = fs::read_to_string(&log).unwrap();
    // Both rounds within the one bound.
    assert!(
        took < limit + Duration::from_secs(2),
        "{took:?}\n{contacts}"
    );
    for line in contacts.lines() {
        assert!(
            line.contains("prompt=0"),
            "a contact that could prompt\n{contacts}"
        );
    }
    if git_version(&w) >= (2, 45) {
        // `GIT_NO_LAZY_FETCH`: the missing commit is reported absent.
        assert!(!contacts.contains("nolazy=1"), "a lazy fetch\n{contacts}");
        assert_eq!(contacts.lines().count(), 4, "{contacts}");
        assert!(clean(&report) && report.pulled > 0, "{report:?}");
        assert_eq!(w.head(&partial), w.remote_head());
    } else {
        // Older git fetches it lazily, never prompting, and the bound stops
        // it: offline.
        assert!(left(&report, LeftReason::Offline), "{report:?}\n{contacts}");
    }
}

// --- the fetch -----------------------------------------------------------------------------

#[test]
fn the_fetch_moves_the_tracking_ref_to_the_listed_commit_and_writes_no_fetch_head() {
    let w = World::new();
    let old = tracking(&w);
    assert_eq!(fetch_ours(&w), Fetch::Same { at: old.clone() });
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let t = &w.theirs;
    w.run(t, &["tag", "v9"]);
    w.run(t, &["push", "-q", "origin", "v9"]);
    let head = w.head(&w.ours);
    let new = w.remote_head();
    assert_eq!(
        fetch_ours(&w),
        Fetch::Moved {
            old: Some(old),
            new: new.clone()
        }
    );
    assert_eq!(tracking(&w), new);
    assert!(!w.ours.join(".git").join("FETCH_HEAD").exists());
    assert_eq!(w.run(&w.ours, &["tag"]), "", "a tag was fetched");
    assert_eq!(w.head(&w.ours), head, "the branch moved");
    let reflog = w.run(&w.ours, &["reflog", "show", TRACKING]);
    assert!(reflog.contains("kluris sync: fetch origin"), "{reflog}");
    assert_eq!(fetch_ours(&w), Fetch::Same { at: new });
}

#[test]
fn an_upstream_with_no_tracking_ref_yet_is_moved_from_none() {
    let w = World::new();
    w.run(&w.ours, &["update-ref", "-d", TRACKING]);
    let new = w.remote_head();
    assert_eq!(
        fetch_ours(&w),
        Fetch::Moved {
            old: None,
            new: new.clone()
        }
    );
    assert_eq!(tracking(&w), new);
}

/// The remote has no such branch: no commit is the upstream's, and the
/// tracking ref, with a value or without, is left as it is.
#[test]
fn a_remote_without_the_branch_is_no_branch_and_its_tracking_ref_stays() {
    let w = World::new();
    let held = tracking(&w);
    w.run(&w.remote, &["update-ref", "-d", "refs/heads/main"]);
    assert_eq!(fetch_ours(&w), Fetch::NoBranch);
    assert_eq!(ref_at(&w, TRACKING), Some(held));
    w.run(&w.ours, &["update-ref", "-d", TRACKING]);
    assert_eq!(fetch_ours(&w), Fetch::NoBranch);
    assert_eq!(ref_at(&w, TRACKING), None);
}

#[test]
fn the_listed_commit_is_used_when_fetch_head_or_the_tracking_ref_is_rewritten_meanwhile() {
    // Another git in the brain writes `FETCH_HEAD` and moves the tracking
    // ref to a commit of its own while Kluris fetches: Kluris goes on from
    // its own `ls-remote`'s commit.
    let w = World::new();
    let other = w
        .run(
            &w.ours,
            &[
                "commit-tree",
                w.run(&w.ours, &["rev-parse", "HEAD^{tree}"]).trim(),
                "-m",
                "other",
            ],
        )
        .trim()
        .to_string();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let ours = sh_path(&w.ours);
    upload_pack(
        &w,
        &w.ours,
        "rewriting-upload-pack.sh",
        &format!(
            "git upload-pack \"$@\" || exit $?\nprintf '{other}\\t\\tbranch main of x\\n' > '{ours}/.git/FETCH_HEAD'\ngit -C '{ours}' update-ref {TRACKING} {other}\n"
        ),
    );
    let new = w.remote_head();
    let Fetch::Moved { new: fetched, .. } = fetch_ours(&w) else {
        panic!("not moved");
    };
    assert_eq!(fetched, new);
    // The report and what sync merges are the listed commit.
    upload_pack(&w, &w.ours, "plain.sh", "exec git upload-pack \"$@\"\n");
    w.run(&w.ours, &["update-ref", TRACKING, &other]);
    fs::write(
        w.ours.join(".git").join("FETCH_HEAD"),
        format!("{other}\t\tbranch 'main' of x\n"),
    )
    .unwrap();
    let r = w.sync();
    assert!(r.pulled > 0, "{r:?}");
    assert_eq!(w.head(&w.ours), new);
}

#[test]
fn a_tracking_ref_outside_refs_remotes_or_symbolic_is_refused_before_anything_changes() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let (git, repo, upstream) = ours(&w);
    let head = w.head(&w.ours);
    let deadline = Instant::now() + Duration::from_secs(60);
    // Mapped onto the checked-out branch itself.
    let outside = Upstream {
        refname: "refs/heads/main".into(),
        ..upstream.clone()
    };
    let common = w.ours.join(".git");
    let Fetch::Refused { detail } = fetch(&git, &repo, &outside, &common, deadline).unwrap() else {
        panic!("not refused");
    };
    assert!(detail.contains("outside refs/remotes/"), "{detail}");
    assert_eq!(w.head(&w.ours), head);
    // A symbolic ref to another branch.
    w.run(&w.ours, &["branch", "other"]);
    w.run(&w.ours, &["symbolic-ref", TRACKING, "refs/heads/other"]);
    let Fetch::Refused { detail } = fetch(&git, &repo, &upstream, &common, deadline).unwrap()
    else {
        panic!("not refused");
    };
    assert!(
        detail.contains("is a symbolic ref to refs/heads/other"),
        "{detail}"
    );
    assert!(
        detail.contains("git symbolic-ref --delete refs/remotes/origin/main"),
        "{detail}"
    );
    assert_eq!(ref_at(&w, "refs/heads/other"), Some(head.clone()));
    assert!(is_symbolic(&w, TRACKING));
}

#[test]
fn a_tracking_ref_made_symbolic_during_the_fetch_is_refused_and_its_branch_kept() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.run(&w.ours, &["branch", "other"]);
    let other = ref_at(&w, "refs/heads/other");
    let ours = sh_path(&w.ours);
    upload_pack(
        &w,
        &w.ours,
        "symbolic-upload-pack.sh",
        &format!(
            "git upload-pack \"$@\" || exit $?\ngit -C '{ours}' symbolic-ref {TRACKING} refs/heads/other\n"
        ),
    );
    let Fetch::Refused { detail } = fetch_ours(&w) else {
        panic!("not refused");
    };
    assert!(
        detail.contains("is a symbolic ref to refs/heads/other"),
        "{detail}"
    );
    assert_eq!(ref_at(&w, "refs/heads/other"), other);
    assert!(is_symbolic(&w, TRACKING));
}

#[test]
fn a_tracking_ref_made_symbolic_after_a_lost_compare_and_swap_is_refused() {
    // Kluris's own update of the tracking ref is turned away (a
    // `reference-transaction` hook aborts it), and once the lock is gone
    // the ref is made symbolic to another branch: checked again before its
    // value is used, it is refused, never merged.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.run(&w.ours, &["branch", "other"]);
    let other = ref_at(&w, "refs/heads/other");
    w.hook(
        &w.ours,
        "reference-transaction",
        &format!(
            "[ -n \"$INNER\" ] && exit 0\ngrep -q ' {TRACKING}$' || exit 0\ncase \"$1\" in\n  prepared) exit 1 ;;\n  aborted) INNER=1 git symbolic-ref {TRACKING} refs/heads/other ;;\nesac\nexit 0"
        ),
    );
    let head = w.head(&w.ours);
    let r = w.sync();
    let why = refused(&r);
    assert!(
        why.contains("is a symbolic ref to refs/heads/other"),
        "{why}"
    );
    assert!(r.pulled == 0, "merged");
    assert_eq!(w.head(&w.ours), head);
    assert_eq!(ref_at(&w, "refs/heads/other"), other);
    assert!(is_symbolic(&w, TRACKING));
}

#[test]
fn moving_the_tracking_ref_replaces_a_symbolic_ref_and_never_follows_it() {
    let w = World::new();
    let (git, repo, _) = ours(&w);
    let main = w.head(&w.ours);
    let tree = w.run(&w.ours, &["rev-parse", "HEAD^{tree}"]);
    let other = w
        .run(
            &w.ours,
            &["commit-tree", tree.trim(), "-p", &main, "-m", "other"],
        )
        .trim()
        .to_string();
    w.run(&w.ours, &["symbolic-ref", TRACKING, "refs/heads/main"]);
    git.move_ref(&repo, TRACKING, &other, Some(&main), "test")
        .unwrap();
    assert_eq!(ref_at(&w, "refs/heads/main"), Some(main));
    assert_eq!(tracking(&w), other);
    assert!(!is_symbolic(&w, TRACKING), "still symbolic");
}

#[test]
fn a_concurrent_fetch_that_moved_the_tracking_ref_is_kept() {
    // While Kluris's fetch waits for its upload-pack to end, the teammate
    // pushes again and a plain `git fetch` in the brain moves the tracking
    // ref past what Kluris fetched: Kluris's compare-and-swap fails, and it
    // goes on from where that fetch left the ref.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.write(&w.theirs, "ideas/later.md", &neuron("", "# Later\n"));
    w.commit(&w.theirs, &["ideas/later.md"]);
    upload_pack(
        &w,
        &w.ours,
        "racing-upload-pack.sh",
        &format!(
            "[ -n \"$RACING_FETCH\" ] && exec git upload-pack \"$@\"\ngit upload-pack \"$@\" || exit $?\ngit -C '{theirs}' push -q origin main >&2\nRACING_FETCH=1 git -C '{ours}' fetch -q origin >&2\n",
            theirs = sh_path(&w.theirs),
            ours = sh_path(&w.ours),
        ),
    );
    let r = sync_within(&w, Duration::from_secs(60));
    let latest = w.remote_head();
    assert_eq!(w.head(&w.theirs), latest, "the teammate's second push");
    assert!(clean(&r) && r.pulled > 0, "{r:?}");
    assert!(!left(&r, LeftReason::Refused), "{r:?}");
    assert_eq!(
        tracking(&w),
        latest,
        "the concurrent fetch's ref was moved back"
    );
    let reflog = w.run(&w.ours, &["reflog", "show", TRACKING]);
    assert!(!reflog.contains("kluris sync"), "{reflog}");
    assert_eq!(w.head(&w.ours), latest);
    assert!(w.read(&w.ours, "ideas/later.md").contains("# Later"));
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
}

#[test]
fn a_remote_that_moves_its_branch_away_on_each_try_is_refused() {
    // Each `ls-remote` is given a commit the remote then drops from its
    // branch before the fetch: absent after the fetch, fetched again, then
    // refused.
    let w = World::new();
    let counter = w.dir.path().join("upload-pack-runs");
    let remote = sh_path(&w.remote);
    upload_pack(
        &w,
        &w.ours,
        "moving-upload-pack.sh",
        &format!(
            "n=$(($(cat '{counter}' 2>/dev/null || echo 0) + 1))\necho $n > '{counter}'\nif [ $((n % 2)) = 1 ]; then\n  p=$(git -C '{remote}' rev-parse main)\n  t=$(git -C '{remote}' rev-parse 'main^{{tree}}')\n  git -C '{remote}' update-ref refs/heads/main \"$(git -C '{remote}' -c user.name=R -c user.email=r@x commit-tree \"$t\" -p \"$p\" -m listed)\"\n  git upload-pack \"$@\" || exit $?\n  git -C '{remote}' update-ref refs/heads/main \"$p\"\n  exit 0\nfi\nexec git upload-pack \"$@\"\n",
            counter = sh_path(&counter),
        ),
    );
    let held = tracking(&w);
    let Fetch::Refused { detail } = fetch_ours(&w) else {
        panic!("not refused");
    };
    assert_eq!(
        detail,
        "origin's main moved while Kluris fetched it, on each try; nothing was merged or pushed: run kluris sync again"
    );
    assert_eq!(fs::read_to_string(&counter).unwrap().trim(), "4");
    assert_eq!(tracking(&w), held);
}

#[test]
fn a_host_that_does_not_resolve_is_offline_with_nothing_changed() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let url = "ssh://git@nonexistent.invalid/x.git";
    w.run(&w.ours, &["remote", "set-url", "origin", url]);
    let before = w.snapshot();
    assert_eq!(fetch_ours(&w), Fetch::Offline);
    let r = w.sync();
    assert!(
        left(&r, LeftReason::Offline) && r.pulled == 0 && !r.pushed,
        "{r:?}"
    );
    assert!(!left(&r, LeftReason::Refused), "{r:?}");
    assert_eq!(w.snapshot(), before);
}

/// A remote path with no repository is not network trouble: git's
/// message and the fix.
#[test]
fn a_remote_repository_that_does_not_exist_is_refused_with_nothing_changed() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let gone = w.dir.path().join("gone.git").to_string_lossy().into_owned();
    w.run(&w.ours, &["remote", "set-url", "origin", &gone]);
    let before = w.snapshot();
    let Fetch::Refused { detail } = fetch_ours(&w) else {
        panic!("not refused");
    };
    for why in [detail, refused(&w.sync())] {
        assert!(why.starts_with("fatal: '"), "{why}");
        assert!(
            why.contains("gone.git' does not appear to be a git repository"),
            "{why}"
        );
        assert!(
            why.ends_with(
                "; check that this machine can run `git fetch origin` in this brain without a prompt"
            ),
            "{why}"
        );
    }
    assert_eq!(w.snapshot(), before);
}

#[cfg(unix)]
#[test]
fn a_missing_remote_path_holding_a_newline_then_ssh_words_is_a_refusal() {
    let w = World::new();
    let path = format!(
        "{}/a\nssh: connect to host h port 22: Connection refused",
        w.dir.path().display()
    );
    w.run(&w.ours, &["config", "remote.origin.url", &path]);
    let before = w.snapshot();
    let r = w.sync();
    assert!(!left(&r, LeftReason::Offline), "{r:?}");
    let why = refused(&r);
    assert!(
        why.contains("does not appear to be a git repository"),
        "{why}"
    );
    assert_eq!(w.snapshot(), before);
}

/// A missing local path that holds an apostrophe, then a newline, then
/// words that read like ssh's: git quotes it, and the apostrophe looks
/// like the field's end.
#[cfg(unix)]
fn quoted_missing_path(w: &World) -> String {
    format!(
        "{}/missing'\nssh: Could not resolve hostname this-is-path-text.git",
        w.dir.path().display()
    )
}

#[cfg(unix)]
#[test]
fn a_missing_remote_path_holding_an_apostrophe_a_newline_then_ssh_words_is_a_refusal() {
    let w = World::new();
    w.run(
        &w.ours,
        &["config", "remote.origin.url", &quoted_missing_path(&w)],
    );
    let before = w.snapshot();
    let Fetch::Refused { detail } = fetch_ours(&w) else {
        panic!("not refused");
    };
    let r = w.sync();
    assert!(!left(&r, LeftReason::Offline), "{r:?}");
    for why in [detail, refused(&r)] {
        assert!(
            why.contains("does not appear to be a git repository"),
            "{why}"
        );
    }
    assert_eq!(w.snapshot(), before);
}

#[cfg(unix)]
#[test]
fn a_push_to_a_missing_path_holding_an_apostrophe_a_newline_then_ssh_words_is_a_refusal() {
    let w = World::new();
    w.run(
        &w.ours,
        &["config", "remote.origin.pushurl", &quoted_missing_path(&w)],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.remote_head();
    let r = w.sync();
    assert!(!left(&r, LeftReason::Offline) && !r.pushed, "{r:?}");
    let why = refused(&r);
    assert!(
        why.contains("does not appear to be a git repository"),
        "{why}"
    );
    assert_eq!(w.remote_head(), before);
}

/// A `file://` URL, as git takes it on each OS.
fn file_url(path: &Path) -> String {
    let path = sh_path(path);
    match path.strip_prefix('/') {
        Some(rest) => format!("file:///{rest}"),
        None => format!("file:///{path}"),
    }
}

#[test]
fn a_file_url_to_a_missing_repository_named_with_network_words_is_refused() {
    let w = World::new();
    let gone = w
        .dir
        .path()
        .join("Could not resolve host")
        .join("Connection refused timed out.git");
    w.run(&w.ours, &["config", "remote.origin.url", &file_url(&gone)]);
    let before = w.snapshot();
    let Fetch::Refused { detail } = fetch_ours(&w) else {
        panic!("not refused");
    };
    let r = w.sync();
    assert!(!left(&r, LeftReason::Offline), "{r:?}");
    for why in [detail, refused(&r)] {
        assert!(why.contains("Connection refused timed out.git"), "{why}");
    }
    assert_eq!(w.snapshot(), before);
}

/// Make the remote's upload-pack print ssh's words for a host it cannot
/// reach and fail, as no local git does.
fn upload_pack_says_ssh_cannot_connect(w: &World) {
    upload_pack(
        w,
        &w.ours,
        "ssh-words-upload-pack.sh",
        "echo 'ssh: connect to host h port 22: Connection refused' >&2\nexit 128\n",
    );
}

#[test]
fn a_local_remote_that_fails_with_ssh_words_is_refused() {
    let w = World::new();
    upload_pack_says_ssh_cannot_connect(&w);
    let before = w.snapshot();
    let Fetch::Refused { detail } = fetch_ours(&w) else {
        panic!("not refused");
    };
    let r = w.sync();
    assert!(!left(&r, LeftReason::Offline), "{r:?}");
    for why in [detail, refused(&r)] {
        assert!(why.contains("ssh: connect to host h port 22"), "{why}");
    }
    assert_eq!(w.snapshot(), before);
}

#[test]
fn an_ssh_url_that_insteadof_makes_a_local_path_is_local() {
    let w = World::new();
    upload_pack_says_ssh_cannot_connect(&w);
    let remote = w.remote.to_string_lossy().into_owned();
    w.run(
        &w.ours,
        &[
            "config",
            &format!("url.{remote}.insteadOf"),
            "ssh://h/brain.git",
        ],
    );
    w.run(
        &w.ours,
        &["config", "remote.origin.url", "ssh://h/brain.git"],
    );
    let before = w.snapshot();
    let r = w.sync();
    assert!(!left(&r, LeftReason::Offline), "{r:?}");
    assert!(
        refused(&r).contains("ssh: connect to host h port 22"),
        "{r:?}"
    );
    assert_eq!(w.snapshot(), before);
}

/// Point "ours"'s ssh at a script that says the host refused the
/// connection, as OpenSSH does, and fails with ssh's 255. Returns the file
/// each run of it appends to.
fn ssh_cannot_connect(w: &World) -> PathBuf {
    let calls = w.dir.path().join("ssh-calls");
    let script = w.dir.path().join("ssh-down.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\necho 'ssh: connect to host fakehost port 22: Connection refused' >&2\nexit 255\n",
            sh_path(&calls)
        ),
    )
    .unwrap();
    w.run(
        &w.ours,
        &[
            "config",
            "core.sshCommand",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
    w.run(&w.ours, &["config", "ssh.variant", "ssh"]);
    calls
}

#[test]
fn an_ssh_host_that_refuses_the_connection_is_offline() {
    // ssh's `host:path`, `ssh://`, and a path that insteadOf makes ssh's.
    for setup in ["scp", "ssh-url", "instead-of"] {
        let w = World::new();
        let calls = ssh_cannot_connect(&w);
        let url = match setup {
            "scp" => "fakehost:brain.git".to_string(),
            "ssh-url" => "ssh://fakehost/brain.git".to_string(),
            _ => {
                let local = w.dir.path().join("elsewhere.git");
                let local = local.to_string_lossy().into_owned();
                w.run(
                    &w.ours,
                    &["config", "url.ssh://fakehost/brain.git.insteadOf", &local],
                );
                local
            }
        };
        w.run(&w.ours, &["config", "remote.origin.url", &url]);
        let before = w.snapshot();
        assert_eq!(fetch_ours(&w), Fetch::Offline, "{setup}");
        let r = w.sync();
        assert!(left(&r, LeftReason::Offline), "{setup}: {r:?}");
        assert!(!left(&r, LeftReason::Refused), "{setup}: {r:?}");
        assert!(
            fs::read_to_string(&calls)
                .unwrap_or_default()
                .contains("fakehost"),
            "{setup}: ssh never ran"
        );
        assert_eq!(w.snapshot(), before, "{setup}");
    }
}

#[test]
fn a_push_to_an_ssh_host_that_refuses_the_connection_is_unreachable() {
    let w = World::new();
    let calls = ssh_cannot_connect(&w);
    w.run(
        &w.ours,
        &["config", "remote.origin.pushurl", "fakehost:brain.git"],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let before = w.remote_head();
    let (git, repo, upstream) = ours(&w);
    let deadline = Instant::now() + Duration::from_secs(60);
    assert_eq!(
        git.push(&repo, &upstream, deadline),
        PushOutcome::Unreachable
    );
    let r = w.sync();
    assert!(left(&r, LeftReason::Offline) && !r.pushed, "{r:?}");
    assert!(fs::read_to_string(&calls).unwrap().contains("fakehost"));
    assert_eq!(w.remote_head(), before);
}

#[test]
fn a_transfer_slower_than_http_low_speed_limit_is_offline() {
    let w = World::new();
    let port = stalling_http_server();
    let url = format!("http://127.0.0.1:{port}/r.git");
    w.run(&w.ours, &["config", "remote.origin.url", &url]);
    w.run(&w.ours, &["config", "http.lowSpeedLimit", "1000"]);
    w.run(&w.ours, &["config", "http.lowSpeedTime", "1"]);
    let before = w.snapshot();
    let r = sync_within(&w, Duration::from_secs(30));
    assert!(left(&r, LeftReason::Offline), "{r:?}");
    assert!(!left(&r, LeftReason::Refused), "{r:?}");
    assert_eq!(w.snapshot(), before);
}

/// A fetch stopped at the time limit is offline (an upload-pack that
/// starts late holds it past a 1 s limit).
#[test]
fn a_fetch_held_past_the_time_limit_is_offline() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    upload_pack(
        &w,
        &w.ours,
        "slow-upload-pack.sh",
        "sleep 10\nexec git upload-pack \"$@\"\n",
    );
    let before = w.snapshot();
    let r = sync_within(&w, Duration::from_secs(1));
    assert!(
        left(&r, LeftReason::Offline) && r.pulled == 0 && !r.pushed,
        "{r:?}"
    );
    assert!(!left(&r, LeftReason::Refused), "{r:?}");
    assert_eq!(w.snapshot(), before);
}

#[test]
fn a_fetch_stopped_while_git_would_hold_the_tracking_ref_leaves_no_lock() {
    // The fetch inside the bound writes no ref, so the slow hook runs under
    // Kluris's own update of the tracking ref, outside the bound.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    w.hook(&w.ours, "reference-transaction", SLOW_TRACKING_HOOK);
    let r = sync_within(&w, LIMIT);
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new(), "{r:?}");
    assert!(clean(&r) && r.pulled > 0, "{r:?}");
    assert!(!left(&r, LeftReason::Refused), "{r:?}");
    assert_eq!(tracking(&w), w.remote_head());
    assert_eq!(w.head(&w.ours), w.remote_head());
    assert!(w.read(&w.ours, "ideas/new.md").contains("# New"));
}

#[test]
fn a_fetch_held_past_the_bound_by_a_slow_upload_pack_leaves_no_lock() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    upload_pack(
        &w,
        &w.ours,
        "slow-upload-pack.sh",
        "sleep 10\nexec git upload-pack \"$@\"\n",
    );
    let before = tracking(&w);
    let started = Instant::now();
    let r = sync_within(&w, LIMIT);
    let took = started.elapsed();
    assert!(took < Duration::from_secs(8), "{took:?}");
    assert!(left(&r, LeftReason::Offline) && r.pulled == 0, "{r:?}");
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
    assert_eq!(tracking(&w), before);
    // The remote fast again: the next sync fetches and merges.
    w.run(&w.ours, &["config", "--unset", "remote.origin.uploadpack"]);
    let r = sync_within(&w, LIMIT);
    assert!(clean(&r) && r.pulled > 0, "{r:?}");
    assert_eq!(tracking(&w), w.remote_head());
    assert_eq!(w.head(&w.ours), w.remote_head());
}

// --- the push ------------------------------------------------------------------------------

/// git moves the tracking ref after a push the remote took, as after any
/// push of yours (`update by push`), and leaves no lock.
#[test]
fn git_moves_the_tracking_ref_after_a_push_the_remote_took() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), pushed);
    assert_eq!(tracking(&w), pushed);
    assert_eq!(ref_locks(&w.ours), Vec::<PathBuf>::new());
    let reflog = w.run(&w.ours, &["reflog", "show", "-1", "--format=%gs", TRACKING]);
    assert_eq!(reflog.trim(), "update by push");
}

/// A `pre-push` hook that makes the tracking ref symbolic while the push
/// runs: git follows it, as `git push` does, and the next sync refuses the
/// ref with the fix.
#[test]
fn a_tracking_ref_a_pre_push_hook_makes_symbolic_is_refused_by_the_next_sync() {
    let w = World::new();
    w.run(&w.ours, &["branch", "other"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    w.hook(
        &w.ours,
        "pre-push",
        &format!("git symbolic-ref {TRACKING} refs/heads/other"),
    );
    let r = w.sync();
    assert!(r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), pushed);
    assert_eq!(w.head(&w.ours), pushed);
    assert!(is_symbolic(&w, TRACKING));
    let why = refused(&w.sync());
    assert!(
        why.contains("is a symbolic ref to refs/heads/other"),
        "{why}"
    );
}

#[test]
fn the_pre_push_hook_gets_the_remote_its_push_url_and_the_ref_line() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let remote = w.remote_head();
    let log = w.dir.path().join("pre-push-args");
    w.hook(
        &w.ours,
        "pre-push",
        &format!(
            "printf '%s|%s|%s\\n' \"$1\" \"$2\" \"$(git config \"remote.$1.url\")\" > '{l}'\ncat >> '{l}'",
            l = sh_path(&log)
        ),
    );
    let r = w.sync();
    assert!(r.pushed, "{r:?}");
    let url = w.run(&w.ours, &["remote", "get-url", "--push", "origin"]);
    let url = url.trim();
    let configured = w.run(&w.ours, &["config", "remote.origin.url"]);
    assert_eq!(
        fs::read_to_string(&log).unwrap(),
        format!(
            "origin|{url}|{}\nrefs/heads/main {pushed} refs/heads/main {remote}\n",
            configured.trim()
        )
    );
}

#[test]
fn push_instead_of_and_a_chained_instead_of_apply_once() {
    // `first:` is rewritten to the remote's folder, and a second rewrite of
    // that folder would make the URL `broken:`; for the push, the remote's
    // URL goes to a second bare repo, which a second rewrite would also
    // make `broken:`.
    let w = World::new();
    let folder = w.dir.path().to_string_lossy().into_owned();
    let remote = w.remote.to_string_lossy().into_owned();
    let pushes = w.dir.path().join("pushes.git");
    w.run(
        w.dir.path(),
        &["clone", "-q", "--bare", &remote, &pushes.to_string_lossy()],
    );
    let pushes = pushes.to_string_lossy().into_owned();
    let sep = std::path::MAIN_SEPARATOR;
    for (key, value) in [
        (format!("url.{folder}{sep}.insteadOf"), "first:".to_string()),
        (
            "url.broken:.insteadOf".to_string(),
            format!("{folder}{sep}"),
        ),
        (
            format!("url.{pushes}.pushInsteadOf"),
            "first:remote.git".to_string(),
        ),
        ("url.broken:.pushInsteadOf".to_string(), pushes.clone()),
    ] {
        w.run(&w.ours, &["config", "--add", &key, &value]);
    }
    w.run(
        &w.ours,
        &["remote", "set-url", "origin", "first:remote.git"],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let log = w.dir.path().join("pre-push-url");
    w.hook(
        &w.ours,
        "pre-push",
        &format!("echo \"$2\" > '{}'", sh_path(&log)),
    );
    // git itself: the fetch goes to the remote, the push to the second repo.
    let get = |args: &[&str]| w.run(&w.ours, args).trim().to_string();
    assert_eq!(get(&["remote", "get-url", "origin"]), remote);
    assert_eq!(get(&["remote", "get-url", "--push", "origin"]), pushes);
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    let at = |repo: &str| {
        w.run(Path::new(repo), &["rev-parse", "main"])
            .trim()
            .to_string()
    };
    assert_eq!(at(&pushes), pushed);
    assert_ne!(at(&remote), pushed);
    // The hook sees the push URL rewritten once, as git would push to it.
    assert_eq!(fs::read_to_string(&log).unwrap().trim(), pushes);
}

/// The push is git's own to the remote: its `receivepack` runs.
#[test]
fn the_remotes_receivepack_runs_for_the_push() {
    let w = World::new();
    let received = w.dir.path().join("received");
    let script = w.dir.path().join("receive-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho ran > '{}'\nexec git receive-pack \"$@\"\n",
            sh_path(&received)
        ),
    )
    .unwrap();
    let receivepack = format!("sh '{}'", sh_path(&script));
    w.run(
        &w.ours,
        &["config", "remote.origin.receivepack", &receivepack],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    assert!(received.exists(), "the remote's receivepack did not run");
    assert_eq!(w.remote_head(), pushed);
    assert_eq!(tracking(&w), pushed);
}

// --- the read throttle and the back-off ----------------------------------------------------

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

#[test]
fn a_fetch_within_the_last_minute_is_not_repeated() {
    let tmp = tempfile::TempDir::new().unwrap();
    let stamp = tmp.path().join("kluris-sync").join("fetch-stamp");
    let throttle = Duration::from_secs(60);
    let now = 1_000_000_000u64;
    assert!(!fetched_recently(&stamp, at(now), throttle), "no stamp");
    write_stamp(&stamp, at(now));
    assert_eq!(fs::read_to_string(&stamp).unwrap(), format!("{now}\n"));
    for (age, throttled) in [
        (0i64, true),
        (59, true),
        (61, false),
        // A stamp from the future (the clock went back) never throttles.
        (-3600, false),
    ] {
        let stamped = (now as i64 - age) as u64;
        fs::write(&stamp, format!("{stamped}\n")).unwrap();
        assert_eq!(
            fetched_recently(&stamp, at(now), throttle),
            throttled,
            "age {age}"
        );
    }
    // A stamp that does not read is no stamp.
    fs::write(&stamp, "yesterday").unwrap();
    assert!(!fetched_recently(&stamp, at(now), throttle));
}

#[test]
fn a_fetch_stopped_at_the_deadline_backs_off_an_hour_doubling_to_a_day() {
    assert_eq!(BACKOFF_FIRST, Duration::from_secs(3600));
    assert_eq!(BACKOFF_MAX, Duration::from_secs(24 * 3600));
    let tmp = tempfile::TempDir::new().unwrap();
    let backoff = tmp.path().join("kluris-sync").join("fetch-backoff");
    let mut now = 1_000_000_000u64;
    for window in [3600u64, 7200, 14400, 28800, 57600, 86400, 86400] {
        back_off(&backoff, at(now));
        assert_eq!(
            fs::read_to_string(&backoff).unwrap(),
            format!("{} {window}\n", now + window)
        );
        for later in [61, window / 2, window - 1] {
            assert!(
                backed_off(&backoff, at(now + later)),
                "{later} s into {window}"
            );
        }
        now += window;
        assert!(!backed_off(&backoff, at(now)), "past {window}");
    }
}

#[test]
fn a_fetch_that_succeeds_ends_the_back_off() {
    let w = World::new();
    let backoff = w
        .ours
        .join(".git")
        .join("kluris-sync")
        .join("fetch-backoff");
    let now = 1_000_000_000u64;
    back_off(&backoff, at(now));
    back_off(&backoff, at(now + 3600));
    assert!(backed_off(&backoff, at(now + 3601)));
    // A fetch that brought the upstream ends it, a sync's too.
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    assert!(matches!(fetch_ours(&w), Fetch::Moved { .. }));
    assert!(!backoff.exists());
    // The next stop starts over at one hour.
    back_off(&backoff, at(now));
    assert_eq!(
        fs::read_to_string(&backoff).unwrap(),
        format!("{} 3600\n", now + 3600)
    );
    // A fetch that failed otherwise (offline) leaves it as it is.
    let url = "ssh://git@nonexistent.invalid/x.git";
    w.run(&w.ours, &["remote", "set-url", "origin", url]);
    assert_eq!(fetch_ours(&w), Fetch::Offline);
    assert!(backoff.exists());
    end_backoff(&backoff);
    assert!(!backoff.exists());
}

#[test]
fn a_back_off_that_does_not_read_or_lies_far_ahead_never_blocks_longer() {
    let tmp = tempfile::TempDir::new().unwrap();
    let backoff = tmp.path().join("fetch-backoff");
    let now = 1_000_000_000u64;
    for (text, blocked) in [
        ("yesterday\n".to_string(), false),
        (String::new(), false),
        (format!("{}\n", now + 100), false),
        // Further ahead than its own window (the clock went back).
        (format!("{} 3600\n", now + 3601), false),
        // A window past the cap counts as the cap.
        (format!("{} 999999999\n", now + 86_400), true),
        (format!("{} 999999999\n", now + 86_401), false),
        (format!("{} 3600\n", now + 3600), true),
    ] {
        fs::write(&backoff, &text).unwrap();
        assert_eq!(backed_off(&backoff, at(now)), blocked, "{text:?}");
    }
}
