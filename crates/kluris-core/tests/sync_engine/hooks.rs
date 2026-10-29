//! The hooks a pull and a push run, as `git merge` and `git push` run them:
//! a failing gate leaves the brain as it is for the hook, a hook that
//! changes the merge stops it, and the message hooks shape the message.

use std::fs;

use kluris_core::sync::LeftReason;
use kluris_core::sync::SyncRequest;
use kluris_core::sync::engine::{self, open};

use crate::common::*;
use crate::{diverged, sh_path};

/// The engine with `no_verify`.
fn sync_no_verify(w: &World) -> kluris_core::sync::SyncReport {
    let request = SyncRequest {
        no_verify: true,
        ..w.request()
    };
    engine::sync(&open(&w.ours, &w.git(), &request).unwrap())
}

/// Left for `hook` with `detail`; nothing changed.
fn stopped_by(w: &World, hook: &str, detail_text: &str) {
    let before = w.snapshot();
    let r = w.sync();
    assert!(left(&r, LeftReason::Hook), "{hook}: {r:?}");
    assert_eq!(detail(&r), detail_text, "{hook}");
    assert_eq!((r.pulled, r.pushed), (0, false), "{hook}");
    assert_eq!(w.snapshot(), before, "{hook}");
}

#[test]
fn a_failing_gate_leaves_the_brain_as_is_with_the_hook_and_its_exit() {
    for (hook, code) in [
        ("pre-merge-commit", 1),
        ("prepare-commit-msg", 4),
        ("commit-msg", 23),
    ] {
        let w = World::new();
        diverged(&w);
        w.hook(&w.ours, hook, &format!("echo 'not now' >&2\nexit {code}"));
        stopped_by(
            &w,
            hook,
            &format!("the {hook} hook failed (exit {code}); nothing was merged"),
        );
    }
}

#[test]
fn no_verify_skips_the_gates_but_not_prepare_commit_msg_and_post_merge_runs() {
    let w = World::new();
    diverged(&w);
    w.hook(&w.ours, "commit-msg", "exit 23");
    w.hook(&w.ours, "pre-merge-commit", "exit 1");
    w.hook(
        &w.ours,
        "prepare-commit-msg",
        "echo 'Prepared: yes' >> \"$1\"",
    );
    let ran = w.dir.path().join("post-merge-ran");
    w.hook(&w.ours, "post-merge", &format!("touch '{}'", sh_path(&ran)));
    let r = sync_no_verify(&w);
    assert!(clean(&r) && r.pulled == 1 && r.pushed, "{r:?}");
    assert!(
        w.run(&w.ours, &["log", "-1", "--format=%B"])
            .contains("\nPrepared: yes\n")
    );
    assert!(ran.exists(), "post-merge did not run");
}

#[test]
fn the_message_hooks_shape_the_merge_message() {
    let w = World::new();
    diverged(&w);
    let seen = w.dir.path().join("prepare-args");
    w.hook(
        &w.ours,
        "prepare-commit-msg",
        &format!(
            "echo \"$2\" > '{}'\necho 'Prepared: yes' >> \"$1\"",
            sh_path(&seen)
        ),
    );
    w.hook(
        &w.ours,
        "commit-msg",
        "sed 's/^kluris: sync$/kluris: sync (checked)/' \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"",
    );
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    assert_eq!(
        w.run(&w.ours, &["log", "-1", "--format=%B"]),
        "kluris: sync (checked)\n\nKluris-Sync: auto\nPrepared: yes\n\n"
    );
    assert_eq!(fs::read_to_string(&seen).unwrap().trim(), "merge");
}

#[test]
fn message_hooks_that_empty_the_message_stop_the_merge() {
    let w = World::new();
    diverged(&w);
    w.hook(&w.ours, "commit-msg", ": > \"$1\"");
    stopped_by(
        &w,
        "commit-msg",
        "the message hooks left the merge message empty; nothing was merged",
    );
}

/// A hook that stages anything into the merge (here `git add -A`, which
/// stages the old work tree over the incoming files) stops it.
#[test]
fn a_hook_that_changes_the_merge_stops_it() {
    for hook in ["pre-merge-commit", "prepare-commit-msg", "commit-msg"] {
        let w = World::new();
        w.theirs_push(
            &[("ideas/idea.md", &neuron("", "# Idea, remote approved\n"))],
            &[],
        );
        w.ours_commit(&[("notes/second.md", &neuron("", "# Second\n"))], &[]);
        w.hook(&w.ours, hook, "git add -A");
        stopped_by(
            &w,
            hook,
            &format!(
                "the {hook} hook changed the merge; nothing was merged (a merge hook may change the message, not the files)"
            ),
        );
    }
}

/// Every hook a sync runs, in the order it runs them: the fetch's tracking
/// ref, the merge's gates and message hooks, the land, the push and the
/// push's tracking ref. `reference-transaction` is logged when committed,
/// for refs under `refs/` (which pseudo-refs git also writes, `ORIG_HEAD`
/// and `AUTO_MERGE`, depends on its version).
#[test]
fn every_hook_runs_in_order() {
    let w = World::new();
    diverged(&w);
    let log = w.dir.path().join("hooks.log");
    let log_path = sh_path(&log);
    for hook in [
        "pre-merge-commit",
        "prepare-commit-msg",
        "commit-msg",
        "post-merge",
        "pre-push",
    ] {
        w.hook(&w.ours, hook, &format!("echo {hook} >> '{log_path}'"));
    }
    w.hook(
        &w.ours,
        "reference-transaction",
        &format!(
            "[ \"$1\" = committed ] || exit 0\nwhile read old new ref; do case $ref in refs/*) echo \"reference-transaction $ref\" >> '{log_path}';; esac; done"
        ),
    );
    let r = w.sync();
    assert!(clean(&r) && r.pushed, "{r:?}");
    let ran = fs::read_to_string(&log).unwrap();
    assert_eq!(
        ran.lines().collect::<Vec<_>>(),
        [
            "reference-transaction refs/remotes/origin/main",
            "pre-merge-commit",
            "prepare-commit-msg",
            "commit-msg",
            "reference-transaction refs/heads/main",
            "post-merge",
            "pre-push",
            "reference-transaction refs/remotes/origin/main",
        ],
        "{ran}"
    );
}
