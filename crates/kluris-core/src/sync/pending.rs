//! The waiting merge ([`PENDING`] in the worktree's own git directory): a
//! conflict a pull found, kept until a person settles it with `kluris
//! resolve`. It pins the teammate's commit (`theirs`) the conflict was shown
//! against, so the merge is finished against that commit, never a newer
//! one. It blocks nothing: each pull compares it with HEAD and the upstream
//! ([`super::engine::pull_fetched`]).
//!
//! Each record has an id ([`Pending::id`]), shown with every conflict it
//! reports. A command that finishes the merge names the id the person
//! answered, and [`check_answer`] refuses one that is not the current
//! record's: a record replaced since (its `ours` went stale, the teammate
//! changed the file again) is a different conflict, never settled by a
//! choice made for the one before it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::git::GitDirs;
use super::{Conflict, ConflictKind, PENDING, ResolvedChoice};
use crate::atomic;
use crate::json::{Json, to_string};

/// Where the worktree's waiting merge is.
pub fn path(dirs: &GitDirs) -> PathBuf {
    PENDING
        .split('/')
        .fold(dirs.own.clone(), |path, part| path.join(part))
}

fn opt(value: &Option<String>) -> Json {
    value.clone().map_or(Json::Null, Json::Str)
}

impl Conflict {
    /// As the waiting merge writes it: `{path, kind, base, ours, theirs}`,
    /// each side its blob.
    pub fn to_json(&self) -> Json {
        Json::Object(vec![
            ("path".into(), Json::Str(self.path.clone())),
            ("kind".into(), Json::Str(self.kind.as_str().into())),
            ("base".into(), opt(&self.base)),
            ("ours".into(), opt(&self.ours)),
            ("theirs".into(), opt(&self.theirs)),
        ])
    }
}

/// The waiting merge: the two commits, their merge base, the conflicts,
/// when it was opened, and the choices made so far.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub base: Option<String>,
    pub ours: String,
    pub theirs: String,
    pub conflicts: Vec<Conflict>,
    /// RFC 3339.
    pub created: String,
    /// In the order they were made; one per path.
    pub resolved: Vec<(String, ResolvedChoice)>,
}

/// A merge id that is not the current waiting merge's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrongMerge {
    /// The id given.
    pub given: String,
    /// The current record's id.
    pub current: String,
}

/// The check every command that finishes a waiting merge makes first, once
/// it holds the tree claim: `merge`, the id the person answered, is the
/// current record's.
pub fn check_answer(pending: &Pending, merge: &str) -> Result<(), WrongMerge> {
    let current = pending.id();
    if merge == current {
        Ok(())
    } else {
        Err(WrongMerge {
            given: merge.to_string(),
            current,
        })
    }
}

/// What [`Pending::read`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    None,
    Unreadable,
    Waiting(Pending),
}

fn str_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn opt_field(value: &serde_json::Value, key: &str) -> Option<Option<String>> {
    match value.get(key)? {
        serde_json::Value::Null => Some(None),
        serde_json::Value::String(s) => Some(Some(s.clone())),
        _ => None,
    }
}

impl Pending {
    /// A new waiting merge of `ours` and `theirs`, opened now.
    pub fn new(base: Option<String>, ours: &str, theirs: &str, conflicts: Vec<Conflict>) -> Self {
        Pending {
            base,
            ours: ours.to_string(),
            theirs: theirs.to_string(),
            conflicts,
            created: jiff::Timestamp::now().to_string(),
            resolved: Vec::new(),
        }
    }

    /// The record's id: the first 12 hex characters of SHA-256 over its
    /// base, ours and theirs, one per line (an empty line for no base).
    pub fn id(&self) -> String {
        let text = format!(
            "{}\n{}\n{}\n",
            self.base.as_deref().unwrap_or(""),
            self.ours,
            self.theirs
        );
        Sha256::digest(text.as_bytes())
            .iter()
            .take(6)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// `{base, ours, theirs, conflicts, created}`, then `resolved` once a
    /// choice was made (`[{path, choice, blob}]`).
    pub fn to_json(&self) -> Json {
        let mut doc = vec![
            ("base".into(), opt(&self.base)),
            ("ours".into(), Json::Str(self.ours.clone())),
            ("theirs".into(), Json::Str(self.theirs.clone())),
            (
                "conflicts".into(),
                Json::Array(self.conflicts.iter().map(Conflict::to_json).collect()),
            ),
            ("created".into(), Json::Str(self.created.clone())),
        ];
        if !self.resolved.is_empty() {
            let items = self
                .resolved
                .iter()
                .map(|(path, choice)| {
                    let blob = match choice {
                        ResolvedChoice::Blob(oid) => Json::Str(oid.clone()),
                        _ => Json::Null,
                    };
                    Json::Object(vec![
                        ("path".into(), Json::Str(path.clone())),
                        ("choice".into(), Json::Str(choice.as_str().into())),
                        ("blob".into(), blob),
                    ])
                })
                .collect();
            doc.push(("resolved".into(), Json::Array(items)));
        }
        Json::Object(doc)
    }

    /// The waiting merge from its JSON; `None` when it is not one.
    pub fn parse(text: &str) -> Option<Pending> {
        let doc: serde_json::Value = serde_json::from_str(text).ok()?;
        let mut conflicts = Vec::new();
        for c in doc.get("conflicts")?.as_array()? {
            conflicts.push(Conflict {
                path: str_field(c, "path")?,
                kind: ConflictKind::from_name(c.get("kind")?.as_str()?)?,
                base: opt_field(c, "base")?,
                ours: opt_field(c, "ours")?,
                theirs: opt_field(c, "theirs")?,
            });
        }
        let mut resolved = Vec::new();
        if let Some(items) = doc.get("resolved").and_then(|r| r.as_array()) {
            for item in items {
                let choice = match item.get("choice")?.as_str()? {
                    "ours" => ResolvedChoice::Ours,
                    "theirs" => ResolvedChoice::Theirs,
                    "file" => ResolvedChoice::Blob(str_field(item, "blob")?),
                    "removed" => ResolvedChoice::Removed,
                    _ => return None,
                };
                resolved.push((str_field(item, "path")?, choice));
            }
        }
        Some(Pending {
            base: opt_field(&doc, "base")?,
            ours: str_field(&doc, "ours")?,
            theirs: str_field(&doc, "theirs")?,
            conflicts,
            created: str_field(&doc, "created").unwrap_or_default(),
            resolved,
        })
    }

    /// The waiting merge at `path`.
    pub fn read(path: &Path) -> Found {
        match fs::read_to_string(path) {
            Ok(text) => Pending::parse(&text).map_or(Found::Unreadable, Found::Waiting),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Found::None,
            Err(_) => Found::Unreadable,
        }
    }

    /// Write it to `path` atomically.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut text = to_string(&self.to_json());
        text.push('\n');
        atomic::write_atomic(path, text.as_bytes())
    }
}

/// Remove the waiting merge at `path`; none there is fine.
pub fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        done => done,
    }
}
