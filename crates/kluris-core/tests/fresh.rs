//! Fresh reads (`kluris_core::sync::fresh`): the decisions before `wake-up`
//! and `search` read a brain, with real git ("ours" reads, "theirs" pushes).
//! The fetch's own cases, its throttle and its back-off are in
//! `sync_fetch.rs`; the commands' reads in `crates/kluris-cli/tests/fresh_reads.rs`.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kluris_core::sync::fetch::{BACKOFF, STAMP};
use kluris_core::sync::fresh::{CLAIM, Fresh, FreshRequest, NO_HOOKS, fresh_read};

mod common;

use common::*;

fn git_path(w: &World, name: &str) -> PathBuf {
    World::path(&w.ours.join(".git"), name)
}

/// Whether "ours" has the remote's latest commit (a fetch brought it).
fn has_remote_head(w: &World) -> bool {
    w.command(
        &w.ours,
        &["cat-file", "-e", &format!("{}^{{commit}}", w.remote_head())],
    )
    .output()
    .unwrap()
    .status
    .success()
}

/// A read of "ours" at `at` (Unix seconds), or now.
fn read(w: &World, at: Option<u64>) -> Fresh {
    let mut req = FreshRequest::new(&w.ours, false);
    if let Some(at) = at {
        req.now = UNIX_EPOCH + Duration::from_secs(at);
    }
    fresh_read(&w.git(), &req)
}

/// "ours"'s refs and HEAD, which a read never changes.
fn refs(w: &World) -> String {
    w.run(&w.ours, &["for-each-ref"]) + &w.head(&w.ours)
}

#[test]
fn the_brain_is_fetched_and_nothing_else_changes() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let before = (refs(&w), w.snapshot());
    let started = Instant::now();
    assert_eq!(read(&w, None), Fresh::Fetched);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(has_remote_head(&w), "not fetched");
    assert!(git_path(&w, STAMP).is_file(), "stamped");
    assert!(!w.ours.join(".git").join("FETCH_HEAD").exists());
    let (refs_now, snapshot) = (refs(&w), w.snapshot());
    assert_eq!(refs_now, before.0, "a ref changed");
    assert_eq!(snapshot.0, before.1.0);
    assert_eq!(snapshot.3, before.1.3);
}

#[test]
fn turned_off_by_the_environment_or_kluris_yml_without_fetching() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    assert_eq!(
        fresh_read(&w.git(), &FreshRequest::new(&w.ours, true)),
        Fresh::Disabled
    );
    w.write(
        &w.ours,
        "kluris.yml",
        "name: b\nsync:\n  auto_fetch: false\n",
    );
    assert_eq!(read(&w, None), Fresh::Disabled);
    assert!(!has_remote_head(&w));
    assert!(!git_path(&w, STAMP).exists());

    // `true`, a kluris.yml that does not read, and none at all: on.
    for text in [
        Some("name: b\nsync:\n  auto_fetch: true\n"),
        Some("name: [unclosed\n"),
        None,
    ] {
        match text {
            Some(text) => w.write(&w.ours, "kluris.yml", text),
            None => fs::remove_file(w.ours.join("kluris.yml")).unwrap(),
        }
        let _ = fs::remove_file(git_path(&w, STAMP));
        assert_eq!(read(&w, None), Fresh::Fetched, "{text:?}");
    }
}

#[test]
fn outside_a_work_tree_nothing_is_fetched() {
    let w = World::new();
    let plain = w.dir.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let req = FreshRequest::new(&plain, false);
    assert_eq!(fresh_read(&w.git(), &req), Fresh::NotARepo);
}

#[test]
fn a_brain_below_its_work_tree_root_is_never_touched() {
    // A brain nested in a larger repo: no fetch, no stamp.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let nested = w.ours.join("ideas");
    let req = FreshRequest::new(&nested, false);
    assert_eq!(fresh_read(&w.git(), &req), Fresh::Nested);
    assert!(!has_remote_head(&w));
    assert!(!git_path(&w, STAMP).exists());
}

#[test]
fn a_shallow_brain_is_never_fetched() {
    // Deepening it would write `.git/shallow` under `shallow.lock`.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let remote = w.remote.to_string_lossy().replace('\\', "/");
    let url = if remote.starts_with('/') {
        format!("file://{remote}")
    } else {
        format!("file:///{remote}")
    };
    let shallow = w.dir.path().join("shallow");
    w.run(
        w.dir.path(),
        &["clone", "-q", "--depth=1", &url, &shallow.to_string_lossy()],
    );
    w.theirs_push(&[("ideas/later.md", &neuron("", "# Later\n"))], &[]);
    let req = FreshRequest::new(&shallow, false);
    assert_eq!(fresh_read(&w.git(), &req), Fresh::Shallow);
    assert!(!World::path(&shallow.join(".git"), STAMP).exists());
}

#[test]
fn a_fetch_within_the_last_minute_is_not_repeated() {
    let w = World::new();
    let now = 1_000_000_000u64;
    assert_eq!(read(&w, Some(now)), Fresh::Fetched);
    assert_eq!(
        fs::read_to_string(git_path(&w, STAMP)).unwrap(),
        format!("{now}\n")
    );
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    assert_eq!(read(&w, Some(now + 59)), Fresh::Throttled);
    assert!(!has_remote_head(&w));
    assert_eq!(read(&w, Some(now + 61)), Fresh::Fetched);
    assert!(has_remote_head(&w));
}

#[test]
fn an_operation_in_progress_or_an_open_conflict_does_not_stop_the_fetch() {
    // A fetch writes only objects: a merge, a rebase or an open `kluris
    // sync` conflict (whose commits are recorded) is unaffected.
    let w = World::new();
    for marker in [
        "MERGE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "kluris-sync/pending.json",
    ] {
        w.theirs_push(
            &[("ideas/new.md", &neuron("", &format!("# {marker}\n")))],
            &[],
        );
        let _ = fs::remove_file(git_path(&w, STAMP));
        let path = git_path(&w, marker);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        if marker.starts_with("rebase") {
            fs::create_dir_all(&path).unwrap();
        } else {
            fs::write(&path, "0123\n").unwrap();
        }
        assert_eq!(read(&w, None), Fresh::Fetched, "{marker}");
        assert!(has_remote_head(&w), "{marker}");
        if marker.starts_with("rebase") {
            fs::remove_dir_all(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
    }
}

#[test]
fn without_an_upstream_nothing_is_fetched_and_the_check_is_throttled() {
    let w = World::new();
    w.run(&w.ours, &["branch", "--unset-upstream"]);
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    assert_eq!(read(&w, None), Fresh::NoUpstream);
    assert!(!has_remote_head(&w));
    assert!(git_path(&w, STAMP).exists());
    assert_eq!(read(&w, None), Fresh::Throttled);
}

#[test]
fn a_failed_or_timed_out_fetch_ends_the_step_and_is_throttled() {
    let w = World::new();
    let gone = w.dir.path().join("gone.git").to_string_lossy().into_owned();
    let slow = w.dir.path().join("slow-upload-pack.sh");
    fs::write(&slow, "#!/bin/sh\nsleep 10\nexec git upload-pack \"$@\"\n").unwrap();
    let slow = format!("sh '{}'", slow.display().to_string().replace('\\', "/"));
    for (setting, expected) in [
        (("remote.origin.url", gone.as_str()), Fresh::FetchFailed),
        (("remote.origin.uploadpack", slow.as_str()), Fresh::TimedOut),
    ] {
        let url = w.remote.to_string_lossy().into_owned();
        w.run(&w.ours, &["config", "remote.origin.url", &url]);
        w.run(&w.ours, &["config", setting.0, setting.1]);
        let _ = fs::remove_file(git_path(&w, STAMP));
        let mut req = FreshRequest::new(&w.ours, false);
        req.time_limit = Duration::from_secs(1);
        assert_eq!(fresh_read(&w.git(), &req), expected);
        // Stamped before the fetch: an offline brain costs this once a minute.
        assert!(git_path(&w, STAMP).exists());
        assert_eq!(read(&w, None), Fresh::Throttled);
    }
    // Only the deadline backs off.
    assert!(git_path(&w, BACKOFF).exists());
}

#[test]
fn a_read_that_fetches_ends_the_back_off_and_one_inside_it_fetches_nothing() {
    let w = World::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let backoff = git_path(&w, BACKOFF);
    fs::create_dir_all(backoff.parent().unwrap()).unwrap();
    fs::write(&backoff, format!("{} 3600\n", now + 1800)).unwrap();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    assert_eq!(read(&w, Some(now)), Fresh::BackedOff);
    assert!(!has_remote_head(&w));
    assert_eq!(read(&w, Some(now + 1800)), Fresh::Fetched);
    assert!(has_remote_head(&w));
    assert!(!backoff.exists());
}

#[test]
fn every_git_after_rev_parse_runs_with_hooks_off() {
    // `core.hooksPath` points at `<git dir>/kluris-sync/no-hooks`, which is
    // never made; `rev-parse`, which finds the git dir, runs no hook.
    let w = World::new();
    let marker = w.dir.path().join("hook-ran");
    let touch = format!(
        "touch '{}'",
        marker.display().to_string().replace('\\', "/")
    );
    for hook in ["reference-transaction", "post-index-change", "pre-auto-gc"] {
        w.hook(&w.ours, hook, &touch);
    }
    // Control: git runs them here.
    w.run(&w.ours, &["update-ref", "refs/kluris-test", "HEAD"]);
    assert!(
        marker.exists(),
        "the hooks never run: the test proves nothing"
    );
    fs::remove_file(&marker).unwrap();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    assert_eq!(read(&w, None), Fresh::Fetched);
    assert!(has_remote_head(&w));
    assert!(!marker.exists(), "a hook ran during a read");
    assert!(!git_path(&w, NO_HOOKS).exists());
}

#[test]
fn something_at_the_no_hooks_path_stops_the_fetch() {
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let no_hooks = git_path(&w, NO_HOOKS);
    fs::create_dir_all(&no_hooks).unwrap();
    assert_eq!(read(&w, None), Fresh::HooksPathTaken);
    assert!(!git_path(&w, STAMP).exists());
    fs::remove_dir(&no_hooks).unwrap();
    fs::write(&no_hooks, "").unwrap();
    assert_eq!(read(&w, None), Fresh::HooksPathTaken);
    assert!(!has_remote_head(&w));
}

#[test]
fn a_read_while_another_holds_the_claim_reads_the_brain_as_it_is() {
    // Agents starting together: while one read fetches, holding the claim,
    // another reads the brain as it is.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let claim = git_path(&w, CLAIM);
    fs::create_dir_all(claim.parent().unwrap()).unwrap();
    let held = fs::File::create(&claim).unwrap();
    held.lock().unwrap();
    assert_eq!(read(&w, None), Fresh::Throttled);
    assert!(!has_remote_head(&w));
    assert!(!git_path(&w, STAMP).exists());
    drop(held);
    // The claim file stays; the lock went with its holder.
    assert_eq!(read(&w, None), Fresh::Fetched);
    assert!(claim.is_file());
}

#[cfg(unix)]
#[test]
fn a_claim_that_cannot_be_taken_skips_the_fetch() {
    use std::os::unix::fs::PermissionsExt;
    // `kluris-sync/` exists but cannot be written: no claim file can be made.
    let w = World::new();
    w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
    let folder = w.ours.join(".git").join("kluris-sync");
    fs::create_dir_all(&folder).unwrap();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o500)).unwrap();
    let fresh = read(&w, None);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(fresh, Fresh::NoClaim);
    assert!(!has_remote_head(&w));
}

/// A git that stalls on the step whose arguments hold `step`, for ten
/// seconds at most, and runs every other one.
#[cfg(unix)]
fn stalling_git(w: &World, step: &str) -> kluris_core::git::SystemGit {
    use std::os::unix::fs::PermissionsExt;
    let script = w.dir.path().join("stalling-git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \" $* \" in\n *'{step}'*) n=0; while [ $n -lt 1000 ]; do sleep 0.01; n=$((n+1)); done;;\nesac\nexec git \"$@\"\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    w.git().program(&script)
}

/// Every git the read runs has the read's one deadline: whichever local
/// step stalls, the read ends near the time limit, not when the step does.
#[cfg(unix)]
#[test]
fn every_step_has_the_reads_one_deadline() {
    for step in [
        " symbolic-ref --quiet HEAD ",
        " for-each-ref --format=%(upstream:remotename)",
        " config --get core.sshCommand ",
    ] {
        let w = World::new();
        w.theirs_push(&[("ideas/new.md", &neuron("", "# New\n"))], &[]);
        let mut req = FreshRequest::new(&w.ours, false);
        req.time_limit = Duration::from_millis(1500);
        let started = Instant::now();
        let fresh = fresh_read(&stalling_git(&w, step), &req);
        let took = started.elapsed();
        assert!(took < Duration::from_millis(4500), "{step}: {took:?}");
        assert_ne!(fresh, Fresh::Fetched, "{step}");
        assert!(!has_remote_head(&w), "{step}: fetched after the stall");
    }
}
