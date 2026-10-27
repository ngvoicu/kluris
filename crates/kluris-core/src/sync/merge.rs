//! The in-memory merge of `kluris sync` (SPEC Phase 7 conflict rules).
//!
//! `git merge-tree --write-tree` gives the baseline: git's own merge of the
//! two commits, renames included. Every path both sides changed, and every
//! path git could not merge, is then decided here:
//! - the generated `map.md`/`brain.md` are never merged: every one is
//!   rebuilt from the merged files (a [`TreeStore`] over them, through
//!   dream's map step);
//! - the same change on both sides is that change; a change on one side
//!   only is taken;
//! - a deletion on one side and an edit on the other goes to a person
//!   ([`ConflictKind::DeleteEdit`]), unless git merged it as a rename;
//! - a file on one side where the other has a directory, and a rename git
//!   could not settle, go to a person ([`ConflictKind::Structure`]): nothing
//!   of either side is dropped, and a choice keeps one side's whole
//!   structure;
//! - with several merge bases (a criss-cross history) every path changed on
//!   both sides goes to a person ([`ConflictKind::AmbiguousBase`]);
//! - a markdown file changed on both sides is merged by [`merge_markdown`]:
//!   frontmatter field by field ([`super::frontmatter_merge`]), bodies by
//!   `git merge-file` ([`TextMerger`]); an overlapping edit goes to a person;
//! - any other file git could not merge goes to a person
//!   ([`ConflictKind::Content`]).
//!
//! No conflict marker is ever written: a merge with any conflict produces
//! nothing but the list.

use std::collections::{BTreeMap, BTreeSet};

use super::frontmatter_merge::merge_meta;
use super::plumbing::SyncGit;
use super::{Conflict, ConflictKind, ResolvedChoice};
use crate::brain::tree_store::{Node, TreeStore};
use crate::dream::{DreamOptions, map_changes};
use crate::frontmatter::{self, LineEnding, line_ending, write_markdown};
use crate::git::{GitError, Repo, StagedBlob, StagingIndex, TreeEntry};

/// A three-way merge of text (`git merge-file`).
pub trait TextMerger {
    /// The merged text, or `None` when the changes overlap.
    fn merge_text(&self, base: &str, ours: &str, theirs: &str) -> Result<Option<String>, GitError>;
}

/// A file merged without conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub bytes: Vec<u8>,
}

fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn with_ending(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::Crlf => text.replace('\n', "\r\n"),
    }
}

/// Three LF texts merged: one side's when the other did not change it,
/// else `git merge-file`'s.
fn merge_texts(
    base: &str,
    ours: &str,
    theirs: &str,
    merger: &impl TextMerger,
) -> Result<String, ConflictKind> {
    if ours == theirs || theirs == base {
        return Ok(ours.to_string());
    }
    if ours == base {
        return Ok(theirs.to_string());
    }
    match merger.merge_text(base, ours, theirs) {
        Ok(Some(text)) => Ok(text),
        _ => Err(ConflictKind::Content),
    }
}

/// A markdown file changed on both sides, merged (`base` is `None` when
/// both added it): frontmatter field by field, the bodies by `git
/// merge-file`, written as dream writes a neuron (the PyYAML-exact writer)
/// with ours' line endings and BOM. A file without frontmatter on either
/// side is merged as plain text. Text that is not UTF-8, frontmatter that
/// does not parse, and overlapping body edits are
/// [`ConflictKind::Content`].
pub fn merge_markdown(
    base: Option<&[u8]>,
    ours: &[u8],
    theirs: &[u8],
    merger: &impl TextMerger,
) -> Result<Merged, ConflictKind> {
    let text = |bytes: &[u8]| std::str::from_utf8(bytes).map(str::to_string);
    let (Ok(o), Ok(t)) = (text(ours), text(theirs)) else {
        return Err(ConflictKind::Content);
    };
    let b = match base.map(text) {
        Some(Ok(b)) => b,
        Some(Err(_)) => return Err(ConflictKind::Content),
        None => String::new(),
    };
    let has_frontmatter = |text: &str| frontmatter::split(text).0.is_some();
    if !has_frontmatter(&o) || !has_frontmatter(&t) {
        let merged = merge_texts(&lf(&b), &lf(&o), &lf(&t), merger)?;
        return Ok(Merged {
            bytes: with_ending(&merged, line_ending(&o)).into_bytes(),
        });
    }
    let parse = |text: &str| frontmatter::parse(text).map_err(|_| ConflictKind::Content);
    let (fo, ft) = (parse(&o)?, parse(&t)?);
    let fb = if has_frontmatter(&b) {
        parse(&b)?
    } else {
        // Added on both sides, or the base had no frontmatter: its text is
        // the body.
        let mut empty = parse("")?;
        empty.body = b.clone();
        empty
    };
    let meta = merge_meta(&fb.meta, &fo.meta, &ft.meta)?;
    let body = merge_texts(&lf(&fb.body), &lf(&fo.body), &lf(&ft.body), merger)?;
    let mut out = write_markdown(&meta, &body, fo.line_ending);
    if fo.had_bom {
        out.insert(0, '\u{feff}');
    }
    Ok(Merged {
        bytes: out.into_bytes(),
    })
}

/// How the in-memory merge ended.
#[derive(Debug)]
pub enum MergeOutcome {
    /// The merged tree, also held in `index` (for the `pre-merge-commit`
    /// hook), and the files merged by the rules.
    Clean {
        tree: String,
        index: StagingIndex,
        auto_merged: Vec<String>,
    },
    /// Something only a person can decide; nothing was written.
    Conflicts {
        base: Option<String>,
        conflicts: Vec<Conflict>,
    },
}

/// The generated files: every directory's `map.md` and the root `brain.md`.
fn is_generated(rel: &str) -> bool {
    rel == "brain.md" || rel.ends_with("/map.md")
}

fn is_markdown(rel: &str) -> bool {
    rel.to_ascii_lowercase().ends_with(".md")
}

/// Whether the walker-facing view needs a file's bytes (the map step reads
/// markdown and YAML files; a symlink's blob is its target).
fn is_text(entry: &TreeEntry) -> bool {
    let lower = entry.rel.to_ascii_lowercase();
    entry.is_symlink() || [".md", ".yml", ".yaml"].iter().any(|s| lower.ends_with(s))
}

type Entries = BTreeMap<String, TreeEntry>;

fn entries<G: SyncGit>(git: &G, repo: &Repo, rev: &str) -> Result<Entries, GitError> {
    Ok(git
        .tree_entries(repo, rev)?
        .into_iter()
        .map(|e| (e.rel.clone(), e))
        .collect())
}

fn same(a: Option<&TreeEntry>, b: Option<&TreeEntry>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.mode == b.mode && a.oid == b.oid,
        (None, None) => true,
        _ => false,
    }
}

/// A file of the merged tree that is not git's.
enum Decided {
    Entry(TreeEntry),
    Bytes(String, Vec<u8>),
    Removed,
}

/// Merge `theirs` into `ours` in memory (see the module docs). A path in
/// `resolutions` takes the person's choice instead of the rules.
pub fn merge_commits<G: SyncGit>(
    git: &G,
    repo: &Repo,
    ours: &str,
    theirs: &str,
    maps: &DreamOptions,
    resolutions: &BTreeMap<String, ResolvedChoice>,
) -> Result<MergeOutcome, GitError> {
    // A criss-cross history has several merge bases. merge-tree merges with
    // a virtual base of them all, but no single base says who changed what:
    // with several, every path changed on both sides goes to a person
    // (`ambiguous_base`) and only one-sided changes merge, as merge-tree
    // has them (decision of T-146, replacing the first-base rule).
    let bases = git.merge_bases(repo, ours, theirs)?;
    let ambiguous = bases.len() > 1;
    let base = bases.first().cloned();
    let merged = git.merge_tree(repo, ours, theirs)?;
    let mut ebs = Vec::new();
    for b in &bases {
        ebs.push(entries(git, repo, b)?);
    }
    let no_base = Entries::new();
    let eb = ebs.first().unwrap_or(&no_base);
    let (eo, et) = (entries(git, repo, ours)?, entries(git, repo, theirs)?);
    let em = entries(git, repo, &merged.tree)?;

    // Structural conflicts (`structure`). A file/directory conflict leaves a
    // synthetic `<path>~<commit>` in merge-tree's result, a path neither side
    // has. A path git could not merge without a file on both sides, nor a
    // base and one side (a delete/edit), is a rename conflict.
    let known = |p: &str| eb.contains_key(p) || eo.contains_key(p) || et.contains_key(p);
    let mut structural: BTreeSet<String> = BTreeSet::new();
    let mut synthetic: Vec<(String, String)> = Vec::new();
    for path in em.keys().chain(merged.conflicted.iter()) {
        if known(path) {
            continue;
        }
        let original = path.rsplit_once('~').map_or(path.as_str(), |(p, _)| p);
        structural.insert(original.to_string());
        synthetic.push((path.clone(), original.to_string()));
    }
    for path in &merged.conflicted {
        let (b, o, t) = (eb.get(path), eo.get(path), et.get(path));
        let triple = o.is_some() && t.is_some();
        let delete_edit = b.is_some() && (o.is_some() != t.is_some());
        if known(path) && !triple && !delete_edit && !is_generated(path) {
            structural.insert(path.clone());
        }
    }
    // Everything a structural conflict covers: the path, its synthetic copy
    // and whatever either side has below it.
    let covered = |p: &str| -> Vec<String> {
        let below = format!("{p}/");
        let mut all: BTreeSet<String> = BTreeSet::from([p.to_string()]);
        all.extend(
            synthetic
                .iter()
                .filter(|(_, original)| original == p)
                .map(|(s, _)| s.clone()),
        );
        for key in eo.keys().chain(et.keys()).chain(em.keys()) {
            if key.starts_with(&below) {
                all.insert(key.clone());
            }
        }
        all.into_iter().collect()
    };
    let mut owned: BTreeSet<String> = BTreeSet::new();
    let mut decided: BTreeMap<String, Decided> = BTreeMap::new();
    let mut conflicts = Vec::new();
    let take =
        |side: Option<&TreeEntry>| side.map_or(Decided::Removed, |e| Decided::Entry(e.clone()));
    for p in &structural {
        let all = covered(p);
        owned.extend(all.iter().cloned());
        let is_synthetic = |x: &String| synthetic.iter().any(|(s, _)| s == x);
        match resolutions.get(p.as_str()) {
            // A person's choice keeps that side's whole structure.
            Some(ResolvedChoice::Ours) | Some(ResolvedChoice::Theirs) => {
                let side = match resolutions.get(p.as_str()) {
                    Some(ResolvedChoice::Ours) => &eo,
                    _ => &et,
                };
                for x in &all {
                    let chosen = if is_synthetic(x) {
                        Decided::Removed
                    } else {
                        take(side.get(x))
                    };
                    decided.insert(x.clone(), chosen);
                }
            }
            Some(ResolvedChoice::Blob(oid)) => {
                for x in all.iter().filter(|x| is_synthetic(x)) {
                    decided.insert(x.clone(), Decided::Removed);
                }
                let mode = eo
                    .get(p)
                    .or(et.get(p))
                    .map_or_else(|| "100644".to_string(), |e| e.mode.clone());
                decided.insert(
                    p.clone(),
                    Decided::Entry(TreeEntry {
                        rel: p.clone(),
                        mode,
                        oid: oid.clone(),
                    }),
                );
            }
            None => conflicts.push(Conflict {
                path: p.clone(),
                kind: ConflictKind::Structure,
                base: eb.get(p).map(|e| e.oid.clone()),
                ours: eo.get(p).map(|e| e.oid.clone()),
                theirs: et.get(p).map(|e| e.oid.clone()),
            }),
        }
    }

    let mut paths: BTreeSet<&String> = merged
        .conflicted
        .iter()
        .filter(|p| !owned.contains(*p))
        .collect();
    for path in eo.keys().chain(et.keys()).chain(eb.keys()) {
        let (b, o, t) = (eb.get(path), eo.get(path), et.get(path));
        let candidate = if ambiguous {
            !same(o, t)
        } else {
            !same(o, b) && !same(t, b)
        };
        if candidate && !owned.contains(path) {
            paths.insert(path);
        }
    }

    // The blobs the markdown merges read.
    let wanted: Vec<String> = paths
        .iter()
        .filter(|p| is_markdown(p) && !is_generated(p))
        .flat_map(|p| [eb.get(*p), eo.get(*p), et.get(*p)])
        .flatten()
        .map(|e| e.oid.clone())
        .collect();
    let blobs = git.read_blobs(repo, &wanted)?;
    let bytes_of = |entry: Option<&TreeEntry>| entry.and_then(|e| blobs.get(&e.oid));

    let mut auto_merged = Vec::new();
    for path in paths {
        let (b, o, t) = (eb.get(path), eo.get(path), et.get(path));
        if is_generated(path) {
            // Rebuilt below; ours (or theirs) until then, never git's merge.
            decided.insert(path.clone(), take(o.or(t)));
            continue;
        }
        if let Some(choice) = resolutions.get(path.as_str()) {
            // A person's choice (`kluris resolve`): their version, or the
            // side's, a deletion included.
            let chosen = match choice {
                ResolvedChoice::Ours => take(o),
                ResolvedChoice::Theirs => take(t),
                ResolvedChoice::Blob(oid) => Decided::Entry(TreeEntry {
                    rel: path.clone(),
                    mode: o
                        .or(t)
                        .map_or_else(|| "100644".to_string(), |e| e.mode.clone()),
                    oid: oid.clone(),
                }),
            };
            decided.insert(path.clone(), chosen);
            continue;
        }
        let conflict = |kind| Conflict {
            path: path.clone(),
            kind,
            base: b.map(|e| e.oid.clone()),
            ours: o.map(|e| e.oid.clone()),
            theirs: t.map(|e| e.oid.clone()),
        };
        if ambiguous {
            if same(o, t) {
                decided.insert(path.clone(), take(o));
            } else if merged.conflicted.contains(path)
                || ebs
                    .iter()
                    .all(|eb| !same(o, eb.get(path)) && !same(t, eb.get(path)))
            {
                conflicts.push(conflict(ConflictKind::AmbiguousBase));
            }
            // Otherwise one side changed it: merge-tree's result stands.
            continue;
        }
        if same(o, t) || same(t, b) {
            decided.insert(path.clone(), take(o));
            continue;
        }
        if same(o, b) {
            decided.insert(path.clone(), take(t));
            continue;
        }
        match (o, t) {
            (Some(oe), Some(_)) if is_markdown(path) => {
                let (Some(ob), Some(tb)) = (bytes_of(o), bytes_of(t)) else {
                    conflicts.push(conflict(ConflictKind::Content));
                    continue;
                };
                match merge_markdown(bytes_of(b).map(Vec::as_slice), ob, tb, git) {
                    Ok(m) => {
                        decided.insert(path.clone(), Decided::Bytes(oe.mode.clone(), m.bytes));
                        auto_merged.push(path.clone());
                    }
                    Err(kind) => conflicts.push(conflict(kind)),
                }
            }
            (Some(_), Some(_)) if !merged.conflicted.contains(path) => {
                // git merged it cleanly (not markdown): its result stands.
            }
            (Some(_), Some(_)) => conflicts.push(conflict(ConflictKind::Content)),
            _ if merged.conflicted.contains(path) => {
                conflicts.push(conflict(ConflictKind::DeleteEdit))
            }
            // Deleted on one side, edited on the other, and git merged it:
            // a rename it followed. Its result stands.
            _ => {}
        }
    }
    if !conflicts.is_empty() {
        conflicts.sort_by(|a, b| a.path.cmp(&b.path));
        return Ok(MergeOutcome::Conflicts { base, conflicts });
    }

    // The merged files, then every generated file rebuilt from them.
    let mut files: BTreeMap<String, TreeEntry> = em;
    let mut bytes: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (path, d) in &decided {
        match d {
            Decided::Entry(e) => {
                files.insert(path.clone(), e.clone());
            }
            Decided::Bytes(mode, b) => {
                files.insert(
                    path.clone(),
                    TreeEntry {
                        rel: path.clone(),
                        mode: mode.clone(),
                        oid: String::new(),
                    },
                );
                bytes.insert(path.clone(), b.clone());
            }
            Decided::Removed => {
                files.remove(path);
            }
        }
    }
    let to_read: Vec<String> = files
        .values()
        .filter(|e| is_text(e) && !bytes.contains_key(&e.rel))
        .map(|e| e.oid.clone())
        .collect();
    let read = git.read_blobs(repo, &to_read)?;
    let nodes: BTreeMap<String, Node> = files
        .values()
        .map(|e| {
            let content = bytes
                .get(&e.rel)
                .cloned()
                .or_else(|| is_text(e).then(|| read.get(&e.oid).cloned()).flatten());
            let node = if e.is_symlink() {
                Node::Link(String::from_utf8_lossy(&content.unwrap_or_default()).into_owned())
            } else {
                Node::File(content)
            };
            (e.rel.clone(), node)
        })
        .collect();
    let view = TreeStore::new(&repo.dir, nodes);
    let (changes, _skipped) = map_changes(&view, maps);
    for write in changes.writes {
        let mode = files
            .get(&write.rel)
            .map_or_else(|| "100644".to_string(), |e| e.mode.clone());
        decided.insert(write.rel.clone(), Decided::Bytes(mode, write.contents));
    }

    let index = git.staging_index(repo, Some(&merged.tree))?;
    let mut by_oid = Vec::new();
    let mut by_bytes = Vec::new();
    for (path, d) in decided {
        match d {
            Decided::Entry(e) => by_oid.push((path, e.mode, e.oid)),
            Decided::Bytes(mode, b) => by_bytes.push(StagedBlob {
                rel: path,
                mode,
                bytes: Some(b),
            }),
            Decided::Removed => by_bytes.push(StagedBlob {
                rel: path,
                mode: String::new(),
                bytes: None,
            }),
        }
    }
    // Removals first: a file can replace a directory of the merged tree (a
    // structural choice), and the other way round.
    let (removals, writes): (Vec<StagedBlob>, Vec<StagedBlob>) =
        by_bytes.into_iter().partition(|b| b.bytes.is_none());
    git.stage_blobs(repo, &index, &removals)?;
    git.stage_entries(repo, &index, &by_oid)?;
    git.stage_blobs(repo, &index, &writes)?;
    let tree = git.write_tree(repo, &index)?;
    Ok(MergeOutcome::Clean {
        tree,
        index,
        auto_merged,
    })
}
