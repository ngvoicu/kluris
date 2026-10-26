//! The one brain walker every Kluris 3 command uses.
//!
//! It prunes `.git`, every directory whose name starts with `.`,
//! `node_modules` and the rest of v2's `SKIP_DIRS` (`__pycache__`), at any
//! depth below the root. That is exactly what v2's neuron index prunes
//! (`neuron_index.all_neuron_files`); v2's maps and MRI walkers prune less
//! (research-03 §5 items 1-2), and their differences are recorded where those
//! commands are ported.
//!
//! Symlinks are taken as v2's `os.walk` plus `_escapes_brain` take them:
//! - a symlink to a file is listed when it resolves inside the brain; one
//!   that resolves outside, a broken one that points outside, and a loop are
//!   dropped. A broken link that points inside is listed (Python's non-strict
//!   `resolve()` gives its target's path), and reading it then fails.
//! - a symlink to a directory is never descended. One that resolves inside
//!   the brain names a directory the walk already visits at its real path
//!   (or prunes), so each directory is walked once and a link back to an
//!   ancestor cannot loop; one that resolves outside is never followed.
//!
//! The walk is iterative and the result is sorted by the bytes of the POSIX
//! relative path. v2 keeps `os.walk`'s directory-read order, which depends on
//! the filesystem, so where v2's output order comes from the walk, parity
//! compares it as a set (research-03 §3.2).

use std::fs;
use std::path::{Path, PathBuf};

use super::sandbox;

/// v2's `SKIP_DIRS`; every other dot-directory is pruned too.
pub const SKIP_DIRS: [&str; 6] = [
    ".git",
    ".github",
    ".vscode",
    ".idea",
    "node_modules",
    "__pycache__",
];

/// A file the walker sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkedFile {
    /// The path relative to the brain root, `/`-separated.
    pub rel: String,
    /// The root joined with the relative path (not resolved).
    pub path: PathBuf,
    /// Whether the entry is a symlink (one that resolves inside the brain).
    pub symlink: bool,
}

/// Whether the walker skips a directory of this name.
pub fn is_pruned_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// Whether a symlinked file resolves outside the brain (or cannot be
/// resolved), v2's `_escapes_brain`.
fn escapes(path: &Path, resolved_root: Option<&Path>) -> bool {
    match (sandbox::resolve(path), resolved_root) {
        (Ok(target), Some(root)) => !sandbox::is_within(&target, root),
        _ => true,
    }
}

/// What the walker sees under a root.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    pub files: Vec<WalkedFile>,
    /// The directories it descends into, the root excluded, by their
    /// `/`-separated relative paths, sorted by bytes (dream writes a `map.md`
    /// into each).
    pub dirs: Vec<String>,
}

/// Every file under `root`, pruned and sorted as the module docs say. A root
/// that is not a readable directory, and any directory that cannot be read,
/// contribute nothing.
pub fn walk(root: &Path) -> Vec<WalkedFile> {
    walk_tree(root).files
}

/// The files and the directories of the walk ([`walk`]).
pub fn walk_tree(root: &Path) -> Tree {
    let resolved_root = sandbox::resolve(root).ok();
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, rel_dir)) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let rel = if rel_dir.is_empty() {
                name.to_string()
            } else {
                format!("{rel_dir}/{name}")
            };
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if !is_pruned_dir(&name) {
                    dirs.push(rel.clone());
                    pending.push((path, rel));
                }
            } else if file_type.is_symlink() {
                if fs::metadata(&path).is_ok_and(|m| m.is_dir())
                    || escapes(&path, resolved_root.as_deref())
                {
                    continue;
                }
                files.push(WalkedFile {
                    rel,
                    path,
                    symlink: true,
                });
            } else {
                files.push(WalkedFile {
                    rel,
                    path,
                    symlink: false,
                });
            }
        }
    }
    files.sort_by(|a, b| a.rel.as_bytes().cmp(b.rel.as_bytes()));
    dirs.sort();
    Tree { files, dirs }
}
