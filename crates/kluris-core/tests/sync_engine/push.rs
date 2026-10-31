//! The push: the `pre-push` hook runs as git runs it, once per push URL
//! and before anything is pushed; one the remote turned back because it
//! moved is pulled again and retried, [`PUSH_ATTEMPTS`] times at most.

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

/// A `pre-push` hook that refuses pushes to `origin` by name: the engine
/// runs it as git would, with the remote's own name, so it stops the push.
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
        detail(&r).starts_with("the pre-push hook failed (exit 7) for origin ")
            && detail(&r).ends_with("; nothing was pushed: origin policy rejected this push"),
        "{}",
        detail(&r)
    );
    assert!(!r.pushed);
    assert_eq!(w.remote_head(), remote);
}

/// A remote with two push URLs: the hook runs once for each, as git runs
/// it, and all of them before anything is pushed, so one refusal stops the
/// push to every URL.
#[test]
fn the_pre_push_hook_runs_once_per_push_url_before_anything_is_pushed() {
    let w = World::new();
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
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let log = w.dir.path().join("pre-push-urls");
    w.hook(
        &w.ours,
        "pre-push",
        &format!(
            "echo \"$2\" >> '{}'\ncase \"$2\" in *second.git) exit 3;; esac",
            sh_path(&log)
        ),
    );
    let (first_before, second_before) = (w.remote_head(), w.run(&second, &["rev-parse", "main"]));
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook), "{r:?}");
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 2);
    assert_eq!(w.remote_head(), first_before, "pushed to the first URL");
    assert_eq!(w.run(&second, &["rev-parse", "main"]), second_before);
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

/// Two push URLs whose `main` hold different commits: each hook run gets
/// the value its own destination holds, as `git push` gives it, so a hook
/// that guards the second destination's actual value stops the push.
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
    let seen = fs::read_to_string(&log).unwrap();
    let old: Vec<&str> = seen
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(old, [remote.as_str(), deployed.as_str()], "{seen}");
    assert_eq!(w.remote_head(), remote);
    assert_eq!(w.run(&second, &["rev-parse", "main"]), at_second);
}

/// The destination moves after the hook ran (the hook moves it itself, to
/// an ancestor of the commit): the push carries a lease on the value the
/// hook saw, so it is turned back, pulled again, and the hook then sees the
/// new value and refuses it.
#[test]
fn the_push_goes_only_where_the_hook_saw_it() {
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
/// on stdin and pushes nothing there, and so does the engine.
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

/// The first push URL takes the commit and the second turns it away (its
/// `pre-receive` hook): the report says which took it and which did not,
/// and the sync fails.
#[test]
fn a_push_that_some_urls_took_says_which() {
    let w = World::new();
    let second = second_push_url(&w);
    let hooks = second.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    let receive = hooks.join("pre-receive");
    fs::write(&receive, "#!/bin/sh\necho 'second refuses' >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&receive, fs::Permissions::from_mode(0o755)).unwrap();
    }
    w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
    let pushed = w.head(&w.ours);
    let r = w.sync();
    assert!(left(&r, LeftReason::Refused), "{r:?}");
    assert!(!r.pushed, "{r:?}");
    let remote = w.remote.to_string_lossy().into_owned();
    let second_url = second.to_string_lossy().into_owned();
    assert!(
        detail(&r).contains(&format!("{remote} took it"))
            && detail(&r).contains(&format!("{second_url} did not")),
        "{}",
        detail(&r)
    );
    assert_eq!(w.remote_head(), pushed);
}
