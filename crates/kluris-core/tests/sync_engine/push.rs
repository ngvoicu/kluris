//! The push: git's own push of the pinned branch, so the `pre-push` hooks
//! run as for any push, once per push URL; one the remote turned back
//! because it moved is pulled again and retried, [`PUSH_ATTEMPTS`] times at
//! most.

use std::fs;

use kluris_core::sync::engine::{open, sync};
use kluris_core::sync::{LeftReason, PUSH_ATTEMPTS, SyncRequest};

use crate::common::*;
use crate::sh_path;

/// A third clone, and our `pre-push` hook that makes it push a new commit
/// first: once, or on every push. Returns the file each race appends to.
fn race(w: &World, every_time: bool) -> std::path::PathBuf {
    let third = w.dir.path().join("third");
    w.clone_as(&third, "Cy Third", "cy@example.com");
    let raced = w.dir.path().join("raced");
    let once = if every_time {
        String::new()
    } else {
        format!("[ -e '{}' ] && exit 0\n", sh_path(&raced))
    };
    let body = format!(
        "{once}echo race >> '{r}'\nn=$(wc -l < '{r}' | tr -d ' ')\ncd '{t}' && unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE && printf '# Race %s\\n' \"$n\" > \"ideas/race-$n.md\" && git add -A && git commit -q -m \"race $n\" && git push -q >&2\nexit 0",
        r = sh_path(&raced),
        t = sh_path(&third),
    );
    w.hook(&w.ours, "pre-push", &body);
    raced
}

#[test]
fn a_push_turned_back_because_the_remote_moved_is_pulled_again_and_retried() {
    let w = World::new();
    let raced = race(&w, false);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync();
    assert_eq!(fs::read_to_string(&raced).unwrap(), "race\n");
    assert!(clean(&r), "{r:?}");
    assert_eq!((r.pulled, r.to_push, r.pushed), (1, 0, true), "{r:?}");
    let remote = w.remote_head();
    assert_eq!(remote, w.head(&w.ours));
    assert!(w.read(&w.ours, "ideas/race-1.md").contains("# Race 1"));
    assert!(
        w.show(&w.ours, &remote, "notes/second.md")
            .contains("# Second")
    );
}

#[test]
fn a_remote_that_keeps_moving_is_tried_at_most_the_push_attempts() {
    let w = World::new();
    let raced = race(&w, true);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let r = w.sync();
    let races = fs::read_to_string(&raced).unwrap().lines().count();
    assert_eq!(races, PUSH_ATTEMPTS);
    assert!(left(&r, LeftReason::Refused), "{r:?}");
    assert_eq!(
        detail(&r),
        "the remote kept moving: 5 pushes were rejected; run kluris sync again"
    );
    assert!(!r.pushed, "{r:?}");
    // Each turned-back push pulled the race in but the last.
    assert_eq!(r.pulled, PUSH_ATTEMPTS - 1, "{r:?}");
    assert!(r.to_push > 0, "{r:?}");
    assert_ne!(w.remote_head(), w.head(&w.ours));
}

/// A `pre-push` hook that refuses pushes to `origin` by name: git runs it
/// with the remote's own name, so it stops the push, and the report gives
/// what it printed.
#[test]
fn a_pre_push_hook_that_refuses_origin_stops_the_push() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.hook(
        &w.ours,
        "pre-push",
        "if [ \"$1\" = origin ]; then echo 'origin policy rejected this push' >&2; exit 7; fi",
    );
    let remote = w.remote_head();
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook), "{r:?}");
    assert!(
        detail(&r).starts_with("the pre-push hook refused the push to origin ")
            && detail(&r).ends_with(": origin policy rejected this push"),
        "{}",
        detail(&r)
    );
    assert!(!r.pushed);
    assert_eq!(w.remote_head(), remote);
}

/// `--no-verify` skips the merge's gates, never `pre-push`.
#[test]
fn no_verify_never_skips_pre_push() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.hook(&w.ours, "pre-push", "exit 5");
    let request = SyncRequest {
        no_verify: true,
        ..w.request()
    };
    let r = sync(&open(&w.ours, &w.git(), &request).unwrap());
    assert!(left(&r, LeftReason::Hook), "{r:?}");
    assert!(!r.pushed);
}

/// A second bare repository cloned from the remote, added as the remote's
/// second push URL after the first; its path.
fn second_push_url(w: &World) -> std::path::PathBuf {
    let second = w.dir.path().join("second.git");
    let remote = w.remote.to_string_lossy().into_owned();
    w.run(
        w.dir.path(),
        &["clone", "-q", "--bare", &remote, &second.to_string_lossy()],
    );
    for url in [&remote, &second.to_string_lossy().into_owned()] {
        w.run(
            &w.ours,
            &["remote", "set-url", "--add", "--push", "origin", url],
        );
    }
    second
}

/// Two push URLs whose `main` hold different commits: git runs the hook
/// once for each, with the value that destination holds, so a hook that
/// guards the second destination's value stops the push there; the first
/// takes it, as `git push` gives it.
#[test]
fn each_push_url_gets_its_own_destinations_value() {
    let w = World::new();
    let second = second_push_url(&w);
    w.ours_commit(&[("notes/deployed.md", &neuron("", "# Deployed\n"))], &[]);
    let deployed = w.head(&w.ours);
    w.run(
        &w.ours,
        &[
            "push",
            "-q",
            &second.to_string_lossy(),
            "main:refs/heads/main",
        ],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let log = w.dir.path().join("pre-push-input");
    w.hook(
        &w.ours,
        "pre-push",
        &format!(
            "input=$(cat)\necho \"$2 $input\" >> '{}'\ncase \"$2\" in *second.git) case \"$input\" in *' {deployed}') echo 'deployed commit is protected' >&2; exit 7;; esac;; esac",
            sh_path(&log)
        ),
    );
    let (remote, at_second) = (w.remote_head(), w.run(&second, &["rev-parse", "main"]));
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook), "{r:?}");
    assert!(
        detail(&r).contains("second.git: deployed commit is protected"),
        "{}",
        detail(&r)
    );
    let seen = fs::read_to_string(&log).unwrap();
    let old: Vec<&str> = seen
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(old, [remote.as_str(), deployed.as_str()], "{seen}");
    assert_eq!(w.remote_head(), pushed);
    assert_eq!(w.run(&second, &["rev-parse", "main"]), at_second);
}

/// The destination moves after the hook ran (the hook moves it itself, to
/// an ancestor of the commit): git's push carries the old value it read,
/// so the remote turns it back; the sync pulls again, and the hook then
/// sees the new value and refuses it.
#[test]
fn a_destination_that_moved_after_the_hook_turns_the_push_back() {
    let w = World::new();
    let base = w.remote_head();
    w.ours_commit(&[("notes/protected.md", &neuron("", "# Protected\n"))], &[]);
    let protected = w.head(&w.ours);
    w.run(
        &w.ours,
        &["push", "-q", "origin", "main:refs/heads/object-source"],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    w.hook(
        &w.ours,
        "pre-push",
        &format!(
            "input=$(cat)\ncase \"$input\" in *' {protected}') echo 'protected destination may not advance' >&2; exit 7;; esac\nunset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE\ngit -C '{remote}' update-ref refs/heads/main {protected} {base}",
            remote = sh_path(&w.remote)
        ),
    );
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook), "{r:?}");
    assert!(
        detail(&r).contains("protected destination may not advance"),
        "{}",
        detail(&r)
    );
    assert_eq!(
        w.remote_head(),
        protected,
        "pushed past the value the hook saw"
    );
}

/// A push URL that already has the commit: git runs the hook with nothing
/// on stdin and pushes nothing there.
#[test]
fn a_destination_that_has_the_commit_gets_no_update_line() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let mirror = w.dir.path().join("mirror.git");
    let remote = w.remote.to_string_lossy().into_owned();
    w.run(
        w.dir.path(),
        &["clone", "-q", "--bare", &remote, &mirror.to_string_lossy()],
    );
    w.run(
        &w.ours,
        &[
            "push",
            "-q",
            &mirror.to_string_lossy(),
            "main:refs/heads/main",
        ],
    );
    w.run(
        &w.ours,
        &["config", "remote.origin.pushurl", &mirror.to_string_lossy()],
    );
    w.hook(
        &w.ours,
        "pre-push",
        "input=$(cat)\n[ \"$1\" = origin ] && [ -z \"$input\" ] || { echo \"unexpected: $input\" >&2; exit 7; }",
    );
    let r = w.sync();
    assert!(clean(&r) && r.pushed && r.to_push == 0, "{r:?}");
    assert_eq!(w.run(&mirror, &["rev-parse", "main"]).trim(), pushed);
}

/// The remote turns the push away (its `pre-receive` hook): the brain is
/// refused with git's message, what the remote said included.
#[test]
fn a_push_the_remote_turns_away_is_refused_with_gits_message() {
    let w = World::new();
    let hooks = w.remote.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let receive = hooks.join("pre-receive");
    fs::write(
        &receive,
        "#!/bin/sh\necho 'the remote refuses' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&receive, fs::Permissions::from_mode(0o755)).unwrap();
    }
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let remote = w.remote_head();
    let r = w.sync();
    assert!(left(&r, LeftReason::Refused), "{r:?}");
    assert!(!r.pushed, "{r:?}");
    for said in [
        "the push was refused: ",
        "[remote rejected] (pre-receive hook declined)",
        "the remote refuses",
    ] {
        assert!(detail(&r).contains(said), "{said}: {}", detail(&r));
    }
    assert_eq!(w.remote_head(), remote);
}

/// With `push.followTags`, git sends the annotated tag the pushed commit
/// carries, and its line is on the hook's stdin with the branch's.
#[test]
fn with_push_follow_tags_the_hook_sees_the_tags_line() {
    let w = World::new();
    w.run(&w.ours, &["config", "push.followTags", "true"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    w.run(&w.ours, &["tag", "-a", "v1", "-m", "v1"]);
    let tag = w.run(&w.ours, &["rev-parse", "refs/tags/v1"]);
    let log = w.dir.path().join("pre-push-input");
    w.hook(&w.ours, "pre-push", &format!("cat >> '{}'", sh_path(&log)));
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    let seen = fs::read_to_string(&log).unwrap();
    let zeros = "0".repeat(pushed.len());
    assert!(
        seen.lines()
            .any(|l| l == format!("refs/tags/v1 {} refs/tags/v1 {zeros}", tag.trim())),
        "{seen}"
    );
    assert!(
        seen.lines()
            .any(|l| l.starts_with(&format!("refs/heads/main {pushed} refs/heads/main "))),
        "{seen}"
    );
    assert_eq!(
        w.run(&w.remote, &["rev-parse", "refs/tags/v1"]).trim(),
        tag.trim()
    );
}

/// A `pre-push` hook that does not finish within the push's time limit is
/// stopped with the push and every process it started that stays in its
/// process group (Unix) or job (Windows): the brain is offline, nothing is
/// pushed, and the remote claim is free, so a retry with a quick hook
/// pushes at once.
#[test]
fn a_hook_past_the_pushs_time_limit_is_stopped() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let (started, late, sleeper) = (
        w.dir.path().join("started"),
        w.dir.path().join("late"),
        w.dir.path().join("sleeper"),
    );
    w.hook(
        &w.ours,
        "pre-push",
        &format!(
            ": > '{s}'\nsleep 3 &\necho $! > '{p}'\nwait\n: > '{l}'",
            s = sh_path(&started),
            p = sh_path(&sleeper),
            l = sh_path(&late)
        ),
    );
    let remote = w.remote_head();
    let request = SyncRequest {
        network_time_limit: std::time::Duration::from_millis(1500),
        ..w.request()
    };
    let ctx = open(&w.ours, &w.git(), &request).unwrap();
    let began = std::time::Instant::now();
    let r = sync(&ctx);
    let took = began.elapsed();
    assert!(started.exists(), "the hook never ran");
    assert!(took < std::time::Duration::from_millis(2900), "{took:?}");
    assert!(left(&r, LeftReason::Offline) && !r.pushed, "{r:?}");
    assert_eq!(w.remote_head(), remote);
    // Its processes are gone: what it would do after the sleep never comes.
    while began.elapsed() < std::time::Duration::from_millis(4000) {
        assert!(!late.exists(), "the hook went on after it was stopped");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    #[cfg(unix)]
    {
        let pid = fs::read_to_string(&sleeper).unwrap();
        let alive = std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .status()
            .unwrap();
        assert!(!alive.success(), "the hook's sleep is still running");
    }
    w.hook(&w.ours, "pre-push", "exit 0");
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
}

/// A `pre-push` hook body that commits on the branch once (the first run),
/// then ends with `end`.
fn committing_hook(w: &World, end: &str) -> String {
    let mark = w.dir.path().join("committed");
    format!(
        "cat > /dev/null\nif [ ! -e '{m}' ]; then\n  : > '{m}'\n  unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE\n  git commit -q --allow-empty -m 'made while the push ran'\nfi\n{end}",
        m = sh_path(&mark)
    )
}

/// A commit the `pre-push` hook makes on the pinned branch: git pushes the
/// commit it read before the hook, and the new one is still to push.
#[test]
fn a_commit_the_pre_push_hook_makes_is_still_to_push() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    w.hook(&w.ours, "pre-push", &committing_hook(&w, "exit 0"));
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    assert_eq!(r.to_push, 1, "{r:?}");
    assert_eq!(w.remote_head(), pushed);
    assert_ne!(w.head(&w.ours), pushed);
}

/// The same hook refusing: nothing is pushed, and both commits are still
/// to push.
#[test]
fn a_commit_a_refusing_pre_push_hook_makes_is_counted() {
    let w = World::new();
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let remote = w.remote_head();
    w.hook(
        &w.ours,
        "pre-push",
        &committing_hook(&w, "echo 'the gate refused' >&2\nexit 7"),
    );
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook) && !r.pushed, "{r:?}");
    assert_eq!(r.to_push, 2, "{r:?}");
    assert_eq!(w.remote_head(), remote);
}

/// A bare repository's hook, executable.
fn remote_hook(repo: &std::path::Path, name: &str, body: &str) {
    let hooks = repo.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let path = hooks.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A `pre-push` hook that passes but prints no newline, so its text joins
/// git's `To <url>` line, and a remote that turns the tag `push.followTags`
/// sends away: the remote refused, not the hook.
#[test]
fn a_passing_hook_joined_to_gits_report_is_not_taken_for_a_refusal() {
    let w = World::new();
    w.run(&w.ours, &["config", "push.followTags", "true"]);
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    w.run(&w.ours, &["tag", "-a", "note-tag", "-m", "note tag"]);
    remote_hook(
        &w.remote,
        "update",
        "case \"$1\" in refs/tags/*) echo 'the remote refused the tag' >&2; exit 1;; esac",
    );
    w.hook(
        &w.ours,
        "pre-push",
        "cat > /dev/null\nprintf 'gate check: '",
    );
    let r = w.sync();
    assert!(left(&r, LeftReason::Refused) && !r.pushed, "{r:?}");
    for said in [
        "gate check: To ",
        "[remote rejected]",
        "the remote refused the tag",
    ] {
        assert!(detail(&r).contains(said), "{said}: {}", detail(&r));
    }
    // The remote took the branch: nothing is left to push.
    assert_eq!(w.remote_head(), pushed);
    assert_eq!(r.to_push, 0, "{r:?}");
}

/// A refusing `pre-push` hook that prints a nested push's rejection (the
/// remote moved, for another destination): the hook refused, once, and
/// the push is not tried again.
#[test]
fn a_rejection_a_refusing_hook_prints_is_not_the_remotes() {
    let w = World::new();
    let other = w.dir.path().join("other.git");
    let remote = w.remote.to_string_lossy().into_owned();
    w.run(
        w.dir.path(),
        &["clone", "-q", "--bare", &remote, &other.to_string_lossy()],
    );
    w.run(
        &w.theirs,
        &["commit", "-q", "--allow-empty", "-m", "elsewhere"],
    );
    w.run(
        &w.theirs,
        &[
            "push",
            "-q",
            &other.to_string_lossy(),
            "HEAD:refs/heads/main",
        ],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let log = w.dir.path().join("pre-push-runs");
    w.hook(
        &w.ours,
        "pre-push",
        &format!(
            "cat > /dev/null\necho run >> '{l}'\ngit push --no-verify --dry-run --porcelain '{o}' refs/heads/main:refs/heads/main\necho 'the gate refused' >&2\nexit 7",
            l = sh_path(&log),
            o = sh_path(&other)
        ),
    );
    let before = w.remote_head();
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook), "{r:?}");
    assert!(detail(&r).contains("the gate refused"), "{}", detail(&r));
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 1);
    assert_eq!(w.remote_head(), before);
}

/// A push that failed (git died: the push URL is no repository) and whose
/// classification would ask git for the push URLs past the push's time
/// limit (a git that stalls on that question): reported at the limit, as a
/// timeout (`offline`).
#[cfg(unix)]
#[test]
fn a_failed_push_classified_past_the_time_limit_is_a_timeout() {
    use std::os::unix::fs::PermissionsExt;
    let w = World::new();
    let missing = w.dir.path().join("missing.git");
    w.run(
        &w.ours,
        &[
            "config",
            "remote.origin.pushurl",
            &missing.to_string_lossy(),
        ],
    );
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let stalling = w.dir.path().join("stalling-git");
    fs::write(
        &stalling,
        "#!/bin/sh\ncase \" $* \" in *' get-url '*) sleep 5;; esac\nexec git \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&stalling, fs::Permissions::from_mode(0o755)).unwrap();
    let request = SyncRequest {
        network_time_limit: std::time::Duration::from_millis(1500),
        ..w.request()
    };
    let ctx = open(&w.ours, &w.git().program(&stalling), &request).unwrap();
    let began = std::time::Instant::now();
    let r = sync(&ctx);
    let took = began.elapsed();
    assert!(left(&r, LeftReason::Offline) && !r.pushed, "{r:?}");
    assert!(took < std::time::Duration::from_millis(4000), "{took:?}");
}
