//! `kluris sync` and `kluris resolve` while another `git fetch` runs in the
//! brain, over real git. The other fetch is of a remote holding commits
//! nobody approved for origin, and origin's upload-pack starts it while
//! sync talks to origin; it rewrites `FETCH_HEAD`, and its remote can even
//! look like origin there: the same URL with another upload-pack, a URL
//! that differs only by `.git`, or an scp-like URL that differs only by its
//! user. Sync learns origin's commit from its own `git ls-remote origin`,
//! so the private commit never reaches origin, the brain's HEAD, index or
//! files, or a conflict record.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

mod sync_home;

use sync_home::*;

const PRIVATE: &str = "PRIVATE: NEVER APPROVED FOR ORIGIN";

/// When the other fetch runs.
#[derive(Clone, Copy)]
enum Concurrent {
    /// Right after origin's upload-pack has served sync.
    Before,
    /// Once origin's commit is in the brain's `FETCH_HEAD`, or 2 s after
    /// origin's upload-pack has served sync: on the first time only, or on
    /// every one.
    After { every_time: bool },
}

/// A shell path (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// The brain's origin, a bare repository.
fn origin(h: &Home) -> PathBuf {
    h.dir.path().join("alpha.git")
}

fn head_of(h: &Home, bare: &Path) -> String {
    h.git(bare, &["rev-parse", "main"]).trim().to_string()
}

/// A clone of `from` at `at`, with a commit adding `notes/private.md`
/// (and, when `conflicting`, changing `notes/first.md` against the brain's
/// own change). Its commit.
fn private_clone(h: &Home, from: &Path, at: &Path, conflicting: bool) -> String {
    h.git(
        h.dir.path(),
        &[
            "clone",
            "-q",
            &from.to_string_lossy(),
            &at.to_string_lossy(),
        ],
    );
    h.configure(at, "Eve Vault", "eve@example.com");
    h.commit(at, "notes/private.md", &neuron("Private", PRIVATE));
    if conflicting {
        h.commit(
            at,
            "notes/first.md",
            &neuron("First", &BODY.replace("Three.", "Three, private.")),
        );
    }
    h.git(at, &["rev-parse", "HEAD"]).trim().to_string()
}

/// The brain's remote `vault`: a private clone of origin at `<home>/vault`.
/// Its commit.
fn vault(h: &Home, root: &Path, conflicting: bool) -> String {
    let at = h.dir.path().join("vault");
    let private = private_clone(h, &origin(h), &at, conflicting);
    h.git(root, &["remote", "add", "vault", &at.to_string_lossy()]);
    private
}

/// A remote `vault` with origin's very URL and an upload-pack that serves
/// a private clone of origin (at `<home>/vault`). Its commit.
fn same_url_vault(h: &Home, root: &Path, conflicting: bool) -> String {
    let at = h.dir.path().join("vault");
    let private = private_clone(h, &origin(h), &at, conflicting);
    let url = h.git(root, &["remote", "get-url", "origin"]);
    h.git(root, &["remote", "add", "vault", url.trim_end()]);
    let script = h.dir.path().join("vault-upload-pack.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\nexec git upload-pack '{}'\n", sh_path(&at)),
    )
    .unwrap();
    h.git(
        root,
        &[
            "config",
            "remote.vault.uploadpack",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
    private
}

/// The `git fetch vault` origin's upload-pack starts, `when`.
fn other_fetch(h: &Home, root: &Path, when: Concurrent) -> String {
    let fetch = format!(
        "git -C '{}' fetch -q --no-tags --refmap= vault refs/heads/main >&2\n",
        sh_path(root)
    );
    match when {
        Concurrent::Before => fetch,
        Concurrent::After { every_time } => {
            let marker = h.dir.path().join("other-fetch-ran");
            let once = if every_time {
                String::new()
            } else {
                format!(
                    "[ -e '{m}' ] && exit 0\ntouch '{m}'\n",
                    m = sh_path(&marker)
                )
            };
            format!(
                "{once}i=0\nwhile ! grep -q '{oid}' '{fh}' 2>/dev/null; do\n  i=$((i+1)); [ $i -gt 20 ] && break; sleep 0.1\ndone\n{fetch}",
                oid = head_of(h, &origin(h)),
                fh = sh_path(&root.join(".git").join("FETCH_HEAD")),
            )
        }
    }
}

/// Origin's upload-pack serves sync, then starts the other fetch, `when`.
fn concurrent_fetch(h: &Home, root: &Path, when: Concurrent) {
    let script = h.dir.path().join("concurrent-upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ngit upload-pack \"$@\" || exit $?\n{}",
            other_fetch(h, root, when)
        ),
    )
    .unwrap();
    h.git(
        root,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
}

/// The brain already has origin's objects, and no `FETCH_HEAD`.
fn prefetched(h: &Home, root: &Path) {
    h.git(
        root,
        &[
            "fetch",
            "-q",
            "--no-tags",
            "--refmap=",
            "origin",
            "refs/heads/main",
        ],
    );
    fs::remove_file(root.join(".git").join("FETCH_HEAD")).unwrap();
}

/// `notes/private.md` (and the private commit) is in neither `bare` (the
/// brain's origin), the brain's HEAD, index or files, nor a conflict
/// record.
fn assert_not_published(h: &Home, bare: &Path, root: &Path, private: &str) {
    let published = h.git(bare, &["ls-tree", "-r", "--name-only", "main"]);
    assert!(
        !published.contains("notes/private.md"),
        "origin: {published}"
    );
    let head = h.git(root, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(!head.contains("notes/private.md"), "HEAD: {head}");
    let index = h.git(root, &["ls-files"]);
    assert!(!index.contains("notes/private.md"), "index: {index}");
    assert!(!root.join("notes").join("private.md").exists());
    let first = h.read(root, "notes/first.md");
    assert!(!first.contains("private"), "{first}");
    let pending = root.join(".git").join("kluris-sync").join("pending.json");
    if let Ok(text) = fs::read_to_string(pending) {
        assert!(!text.contains(private), "{text}");
    }
    let log = h.git(root, &["log", "--format=%H", "HEAD"]);
    assert!(!log.contains(private), "{log}");
}

fn sync(h: &Home) -> (i32, Value) {
    let (code, doc) = h.json(&["sync", "--brain", "alpha", "--json"]);
    (code, doc["brains"][0].clone())
}

/// Sync merged origin's commit (`notes/remote.md`) and pushed the merge.
fn assert_merged_and_pushed(code: i32, entry: &Value, root: &Path) {
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["conflicts"], serde_json::json!([]), "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    assert_eq!(entry["pushed"], true, "{entry}");
    assert!(root.join("notes").join("remote.md").is_file());
}

/// A brain with an approved commit of its own (a change to
/// `notes/first.md` when `conflicting`) and a teammate's
/// `notes/remote.md` on origin.
fn diverged(h: &mut Home, conflicting: bool) -> PathBuf {
    let root = h.brain("alpha");
    if conflicting {
        h.commit(
            &root,
            "notes/first.md",
            &neuron("First", &BODY.replace("Three.", "Three, ours.")),
        );
    } else {
        h.commit(&root, "notes/local.md", &neuron("Local", "Approved.\n"));
    }
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    root
}

#[test]
fn a_merge_with_another_fetch_running_merges_origin_alone() {
    let mut h = Home::new();
    let root = diverged(&mut h, false);
    let private = vault(&h, &root, false);
    concurrent_fetch(&h, &root, Concurrent::Before);
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &origin(&h), &root, &private);
}

#[test]
fn a_fast_forward_with_another_fetch_running_goes_to_origin() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    let private = vault(&h, &root, false);
    prefetched(&h, &root);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: false });
    let (code, entry) = sync(&h);
    assert!(h.dir.path().join("other-fetch-ran").exists());
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), head_of(&h, &origin(&h)));
    assert_not_published(&h, &origin(&h), &root, &private);
}

#[test]
fn a_merge_that_would_conflict_with_the_other_fetch_merges_origin() {
    let mut h = Home::new();
    let root = diverged(&mut h, true);
    let private = vault(&h, &root, true);
    prefetched(&h, &root);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: false });
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &origin(&h), &root, &private);
}

/// Resolve the conflict `h.conflict` opened, choosing ours: the merge is
/// committed and pushed.
fn assert_resolved_and_pushed(h: &Home) {
    let (code, doc) = h.json(&[
        "resolve",
        "--brain",
        "alpha",
        "notes/first.md",
        "--ours",
        "--json",
    ]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(doc["committed"], true, "{doc}");
    assert_eq!(doc["pushed"], true, "{doc}");
}

/// A brain with a conflict open against origin on `notes/first.md`.
fn conflicted(h: &mut Home) -> PathBuf {
    let root = h.brain("alpha");
    h.conflict("alpha", &root);
    let (code, entry) = sync(h);
    assert_eq!(code, 1, "{entry}");
    assert_eq!(entry["conflicts"][0]["path"], "notes/first.md", "{entry}");
    root
}

#[test]
fn resolve_pushes_the_merge_with_origin_alone() {
    let mut h = Home::new();
    let root = conflicted(&mut h);
    let private = vault(&h, &root, false);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: false });
    assert_resolved_and_pushed(&h);
    assert_not_published(&h, &origin(&h), &root, &private);
}

#[test]
fn another_fetch_on_every_one_of_syncs_changes_nothing_sync_merges() {
    let mut h = Home::new();
    let root = diverged(&mut h, false);
    let private = vault(&h, &root, false);
    prefetched(&h, &root);
    concurrent_fetch(&h, &root, Concurrent::After { every_time: true });
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &origin(&h), &root, &private);
}

// The other remote looks like origin in `FETCH_HEAD`: the same URL with
// another upload-pack.

#[test]
fn a_merge_with_a_same_url_remote_fetched_meanwhile_merges_origin_alone() {
    let mut h = Home::new();
    let root = diverged(&mut h, false);
    let private = same_url_vault(&h, &root, false);
    concurrent_fetch(&h, &root, Concurrent::Before);
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &origin(&h), &root, &private);
}

#[test]
fn a_fast_forward_with_a_same_url_remote_fetched_meanwhile_goes_to_origin() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    h.teammate_pushes("alpha", "notes/remote.md", &neuron("Remote", ""));
    let private = same_url_vault(&h, &root, false);
    concurrent_fetch(&h, &root, Concurrent::Before);
    let (code, entry) = sync(&h);
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), head_of(&h, &origin(&h)));
    assert_not_published(&h, &origin(&h), &root, &private);
}

#[test]
fn a_conflict_with_a_same_url_remote_fetched_meanwhile_is_never_recorded() {
    let mut h = Home::new();
    let root = diverged(&mut h, true);
    let private = same_url_vault(&h, &root, true);
    concurrent_fetch(&h, &root, Concurrent::Before);
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &origin(&h), &root, &private);
}

#[test]
fn resolve_with_a_same_url_remote_fetched_meanwhile_pushes_origin_alone() {
    let mut h = Home::new();
    let root = conflicted(&mut h);
    let private = same_url_vault(&h, &root, false);
    concurrent_fetch(&h, &root, Concurrent::Before);
    assert_resolved_and_pushed(&h);
    assert_not_published(&h, &origin(&h), &root, &private);
}

// The other remote's URL differs from origin's only by `.git`.

#[test]
fn a_merge_with_a_remote_named_like_origin_without_git_merges_origin_alone() {
    let mut h = Home::new();
    let root = diverged(&mut h, false);
    // Origin at `<home>/approved.git`, the other remote at
    // `<home>/approved`.
    let approved = h.dir.path().join("approved.git");
    h.git(
        h.dir.path(),
        &[
            "clone",
            "-q",
            "--bare",
            &origin(&h).to_string_lossy(),
            &approved.to_string_lossy(),
        ],
    );
    h.git(
        &root,
        &["remote", "set-url", "origin", &approved.to_string_lossy()],
    );
    let at = h.dir.path().join("approved");
    let private = private_clone(&h, &approved, &at, false);
    h.git(&root, &["remote", "add", "vault", &at.to_string_lossy()]);
    concurrent_fetch(&h, &root, Concurrent::Before);
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &approved, &root, &private);
}

// The other remote's scp-like URL differs from origin's only by its user.

#[test]
fn a_merge_with_a_remote_whose_url_differs_by_its_user_merges_origin_alone() {
    let mut h = Home::new();
    let root = diverged(&mut h, false);
    let at = h.dir.path().join("vault");
    let private = private_clone(&h, &origin(&h), &at, false);
    // An ssh that serves `private@` from the private clone and anyone else
    // from origin, where it also starts the other fetch.
    let ssh = h.dir.path().join("fake-ssh.sh");
    fs::write(
        &ssh,
        format!(
            "#!/bin/sh\ncase $2 in git-receive-pack*) exec git receive-pack '{origin}' ;; esac\ncase $1 in private@*) exec git upload-pack '{vault}' ;; esac\ngit upload-pack '{origin}' || exit $?\n{fetch}",
            origin = sh_path(&origin(&h)),
            vault = sh_path(&at),
            fetch = other_fetch(&h, &root, Concurrent::Before),
        ),
    )
    .unwrap();
    for (key, value) in [
        ("core.sshCommand", format!("sh '{}'", sh_path(&ssh))),
        ("ssh.variant", "simple".to_string()),
    ] {
        h.git(&root, &["config", key, &value]);
    }
    let url = "approved@example.invalid:brain.git";
    h.git(&root, &["remote", "set-url", "origin", url]);
    let url = "private@example.invalid:brain.git";
    h.git(&root, &["remote", "add", "vault", url]);
    let (code, entry) = sync(&h);
    assert_merged_and_pushed(code, &entry, &root);
    assert_not_published(&h, &origin(&h), &root, &private);
}

/// Origin's upload-pack moves `main` away from the commit it advertises
/// to `git ls-remote` (a commit made then and replaced right after by a
/// sibling), the first time only or every time: the fetch that follows
/// never brings that commit.
fn moves_away_after_ls_remote(h: &Home, root: &Path, every_time: bool) {
    let counter = h.dir.path().join("upload-pack-runs");
    let commit = |message: &str| {
        format!(
            "git -C '{bare}' -c user.name=Remote -c user.email=r@example.com commit-tree \"$t\" -p \"$p\" -m '{message}'",
            bare = sh_path(&origin(h)),
        )
    };
    let first = if every_time { "" } else { " && [ $n = 1 ]" };
    let script = h.dir.path().join("moving-upload-pack.sh");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nn=$(($(cat '{counter}' 2>/dev/null || echo 0) + 1))\necho $n > '{counter}'\nif [ $((n % 2)) = 1 ]{first}; then\n  t=$(git -C '{bare}' rev-parse 'main^{{tree}}')\n  p=$(git -C '{bare}' rev-parse main)\n  git -C '{bare}' update-ref refs/heads/main \"$({advertised})\"\n  git upload-pack \"$@\" || exit $?\n  git -C '{bare}' update-ref refs/heads/main \"$({replaced})\"\n  exit 0\nfi\nexec git upload-pack \"$@\"\n",
            counter = sh_path(&counter),
            bare = sh_path(&origin(h)),
            advertised = commit("advertised"),
            replaced = commit("replaced"),
        ),
    )
    .unwrap();
    h.git(
        root,
        &[
            "config",
            "remote.origin.uploadpack",
            &format!("sh '{}'", sh_path(&script)),
        ],
    );
}

#[test]
fn a_commit_the_remote_moved_away_from_once_is_fetched_again() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    moves_away_after_ls_remote(&h, &root, false);
    let (code, entry) = sync(&h);
    assert_eq!(code, 0, "{entry}");
    assert_eq!(entry["merged"], true, "{entry}");
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), head_of(&h, &origin(&h)));
    assert_eq!(h.git(&root, &["status", "--porcelain"]), "");
}

#[test]
fn a_commit_the_remote_moves_away_from_every_time_refuses_the_brain() {
    let mut h = Home::new();
    let root = h.brain("alpha");
    moves_away_after_ls_remote(&h, &root, true);
    let head = h.git(&root, &["rev-parse", "HEAD"]);
    let tracking = h.git(&root, &["rev-parse", "refs/remotes/origin/main"]);
    let (code, entry) = sync(&h);
    assert_eq!(code, 1, "{entry}");
    assert_eq!(
        entry["refused"],
        "origin's main moved while Kluris fetched it, on each try; nothing was merged or pushed: run kluris sync again",
        "{entry}"
    );
    assert_eq!(entry["merged"], false, "{entry}");
    assert_eq!(h.git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        h.git(&root, &["rev-parse", "refs/remotes/origin/main"]),
        tracking
    );
    assert_eq!(h.git(&root, &["status", "--porcelain"]), "");
}
