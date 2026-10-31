//! The sync engine (`kluris_core::sync::engine`) with real git: a bare
//! remote and two clones in temp dirs, "ours" (the brain) and "theirs" (a
//! teammate). `pull` brings the teammate's commits in, `push` shares ours,
//! `sync` does both, and `finish_merge` (through `kluris resolve`) lands a
//! merge a person settled.

#[path = "../common/mod.rs"]
mod common;

mod branch;
mod claimed;
mod conflicts;
mod finish;
mod hooks;
mod matrix;
mod pull;
mod push;
mod rules;
mod safety;
mod waiting;

use common::World;

/// Two clones that each merged the other: `git merge-base --all` lists two
/// bases. Returns ours' and theirs' commits before the merges, and the
/// first base.
pub fn criss_cross(w: &World) -> (String, String, String) {
    w.ours_commit(
        &[(
            "notes/first.md",
            &w.read(&w.ours, "notes/first.md")
                .replace("One.", "P_APPROVED"),
        )],
        &[],
    );
    let p = w.head(&w.ours);
    let q_text = w
        .read(&w.theirs, "notes/first.md")
        .replace("Five.", "Q_APPROVED");
    w.theirs_push(&[("notes/first.md", &q_text)], &[]);
    let q = w.remote_head();
    w.run(&w.ours, &["fetch", "-q"]);
    let ours_path = w.ours.to_string_lossy().into_owned();
    w.run(&w.theirs, &["fetch", "-q", &ours_path, "main:refs/ours"]);
    w.run(&w.ours, &["merge", "-q", "--no-edit", &q]);
    w.run(&w.theirs, &["merge", "-q", "--no-edit", "refs/ours"]);
    w.run(&w.theirs, &["push", "-q"]);
    w.run(&w.ours, &["fetch", "-q"]);
    let bases = w.run(&w.ours, &["merge-base", "--all", "HEAD", "origin/main"]);
    assert_eq!(bases.lines().count(), 2, "{bases}");
    let first = bases.lines().next().unwrap_or_default().to_string();
    (p, q, first)
}

/// The overlapping edit of `notes/first.md` (ours committed, theirs
/// pushed); returns (base, ours, theirs).
pub fn overlapping(w: &World) -> (String, String, String) {
    let base = w.read(&w.ours, "notes/first.md");
    let theirs = base.replace("Three.", "Three, theirs.");
    let ours = base.replace("Three.", "Three, ours.");
    w.theirs_push(&[("notes/first.md", &theirs)], &[]);
    w.ours_commit(&[("notes/first.md", &ours)], &[]);
    (base, ours, theirs)
}

/// A teammate's commit and one of ours on different files.
pub fn diverged(w: &World) {
    w.theirs_push(&[("ideas/new.md", &common::neuron("", "# New\n"))], &[]);
    w.ours_commit(
        &[("notes/second.md", &common::neuron("", "# Second\n"))],
        &[],
    );
}

/// A path as a hook's shell takes it (Git for Windows' sh takes `/`).
pub fn sh_path(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Ours and theirs both edit `notes/first.md` and `ideas/idea.md`: a
/// waiting merge with two conflicts.
pub fn two_conflicts(w: &World) {
    let first = w.read(&w.ours, "notes/first.md");
    let idea = common::neuron("", "# Idea\n\nA.\n");
    w.ours_commit(&[("ideas/idea.md", &idea)], &[]);
    w.run(&w.ours, &["push", "-q"]);
    w.run(&w.theirs, &["pull", "-q"]);
    for (side, at) in [("theirs", &w.theirs), ("ours", &w.ours)] {
        w.write(
            at,
            "notes/first.md",
            &first.replace("Three.", &format!("Three, {side}.")),
        );
        w.write(
            at,
            "ideas/idea.md",
            &idea.replace("A.", &format!("A, {side}.")),
        );
        w.commit(at, &["notes/first.md", "ideas/idea.md"]);
    }
    w.run(&w.theirs, &["push", "-q"]);
    assert_eq!(w.sync().conflicts.len(), 2);
}

/// The paths the waiting merge holds a choice for.
pub fn chosen(w: &World) -> Vec<String> {
    let pending = w.ours.join(".git").join(kluris_core::sync::PENDING);
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(pending).unwrap()).unwrap();
    doc["resolved"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| item["path"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}
