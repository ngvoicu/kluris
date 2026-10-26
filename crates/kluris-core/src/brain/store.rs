//! `BrainStore`: how every command reads a brain, and the one way it is
//! written: a [`ChangeSet`] given to [`BrainStore::apply`].

use std::fs;
use std::path::{Path, PathBuf};

use super::change::{Applied, ChangeSet, FailedWrite};
use super::glossary::{GlossaryEntry, parse_glossary_entries};
use super::neuron_index::{Neuron, neuron_files};
use super::sandbox::{self, SandboxError};
use super::walk::{WalkedFile, is_pruned_dir, walk, walk_tree};
use crate::frontmatter::{self, Frontmatter, FrontmatterError};

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The path is empty, outside the brain, or missing.
    Sandbox(SandboxError),
    /// The file is there but its frontmatter cannot be read (markdown only:
    /// v2 reads a YAML neuron it cannot parse as no metadata).
    File(FrontmatterError),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Sandbox(e) => e.fmt(f),
            ReadError::File(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ReadError {}

/// A directory's child, as `Path.iterdir()` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    /// `Path.is_file()` / `Path.is_dir()`: symlinks followed.
    pub is_file: bool,
    pub is_dir: bool,
}

/// How a brain is read and written.
pub trait BrainStore {
    /// The brain root, as the store was opened with it.
    fn root(&self) -> &Path;
    /// Every file the walker sees ([`walk`]).
    fn files(&self) -> &[WalkedFile];
    /// The neurons ([`neuron_files`]).
    fn neurons(&self) -> &[Neuron];
    /// A file's frontmatter and body by its brain-relative path, as v2's
    /// `read_frontmatter` reads it, inside the path sandbox
    /// ([`sandbox::resolve_in_brain`]). A malformed neuron is an error value
    /// for this file alone.
    fn read(&self, rel: &str) -> Result<Frontmatter, ReadError>;
    /// One of [`BrainStore::neurons`] read by its own path, as v2's
    /// `read_frontmatter(path)` reads the walked file: its name is never
    /// re-parsed from the `/`-separated `rel` (on POSIX a `\` is part of a
    /// file name).
    fn read_neuron(&self, neuron: &Neuron) -> Result<Frontmatter, ReadError> {
        self.read(&neuron.rel)
    }
    /// Where a link `target` written in the file at `from` (both as the
    /// walker gives paths: brain-relative, `/`-separated) leads: the
    /// brain-relative `/`-separated path of the target, or `None` when it
    /// leaves the brain or cannot be resolved. Whether anything is there is
    /// the caller's question. This default works on the paths alone (`.` and
    /// `..` dropped lexically, an absolute target outside the brain), for a
    /// store without files behind it.
    fn resolve_link(&self, from: &str, target: &str) -> Option<String> {
        let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
        if target.starts_with(separators) {
            return None;
        }
        let mut parts: Vec<&str> = match from.rsplit_once('/') {
            Some((dir, _)) => dir.split('/').collect(),
            None => Vec::new(),
        };
        for part in target.split(separators) {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                name => parts.push(name),
            }
        }
        (!parts.is_empty()).then(|| parts.join("/"))
    }
    /// The glossary entries of `glossary.md`: none when the file does not
    /// exist, an error when its frontmatter cannot be read.
    fn glossary(&self) -> Result<Vec<GlossaryEntry>, FrontmatterError> {
        match self.structure_file("glossary.md") {
            None => Ok(Vec::new()),
            Some(fm) => Ok(parse_glossary_entries(&fm?.body)),
        }
    }
    /// The lobes: the top-level directories (a symlink to a directory is
    /// one) that the walker does not prune, sorted as Python's
    /// `sorted(Path)` sorts them: by code point, and on Windows by
    /// `str.lower()`.
    fn lobe_names(&self) -> Vec<String>;
    /// The neurons of one lobe walked with the lobe as the root, as v2's
    /// `neuron_files(lobe_dir)` does (a symlink is checked against the lobe,
    /// a symlinked lobe is walked).
    fn lobe_neurons(&self, lobe: &str) -> Vec<Neuron>;
    /// One of the brain's own structure files (`brain.md`, `glossary.md`,
    /// `<lobe>/map.md`) read as markdown, as v2 reads them: `None` when it
    /// does not exist (`Path.exists()`, links followed), no path sandbox.
    fn structure_file(&self, rel: &str) -> Option<Result<Frontmatter, FrontmatterError>>;
    /// A structure file's text; `None` when it cannot be read or is not
    /// UTF-8.
    fn structure_text(&self, rel: &str) -> Option<String>;
    /// The directories the walker descends into, the root excluded,
    /// `/`-separated and sorted by bytes.
    fn dirs(&self) -> &[String];
    /// The children of a directory (`""` is the root) in the order of v2's
    /// `sorted(path.iterdir())`: by code point, and on Windows by
    /// `str.lower()`. Nothing when it cannot be read.
    fn entries(&self, rel_dir: &str) -> Vec<DirEntry>;
    /// `Path.exists()` of a brain-relative path (links followed).
    fn exists(&self, rel: &str) -> bool;
    /// A file's bytes; `None` when it cannot be read.
    fn bytes(&self, rel: &str) -> Option<Vec<u8>>;
    /// The brain-relative path a write to `rel` lands on: `rel`, or for a
    /// symlink the path its target resolves to; `None` when that is outside
    /// the brain or loops.
    fn real_rel(&self, rel: &str) -> Option<String>;
    /// A markdown file a brain links to, wherever it is, read as v2's
    /// `read_frontmatter` reads it (no path sandbox: the link checks read
    /// what the links point at).
    fn read_markdown_at(&self, path: &Path) -> Result<Frontmatter, FrontmatterError> {
        frontmatter::read(path)
    }
    /// Write every file of `changes`, then walk the brain again. A write that
    /// fails is reported and the others still happen.
    fn apply(&mut self, changes: &ChangeSet) -> Applied;
}

/// Names sorted as Python's `sorted(Path)` sorts siblings.
fn py_sorted(mut names: Vec<String>) -> Vec<String> {
    if cfg!(windows) {
        names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    } else {
        names.sort();
    }
    names
}

/// A `/`-separated path that stays inside the root it is joined to: no
/// empty, `.` or `..` part (and on Windows no `\\` or `:`, which separate
/// paths there).
fn is_plain_rel(rel: &str) -> bool {
    rel.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && !(cfg!(windows) && (part.contains('\\') || part.contains(':')))
    })
}

/// A brain in a local directory (a git checkout or not), walked once when
/// opened.
#[derive(Debug, Clone)]
pub struct LocalGitStore {
    root: PathBuf,
    files: Vec<WalkedFile>,
    dirs: Vec<String>,
    neurons: Vec<Neuron>,
}

impl LocalGitStore {
    pub fn open(root: impl AsRef<Path>) -> LocalGitStore {
        let root = root.as_ref().to_path_buf();
        let tree = walk_tree(&root);
        let neurons = neuron_files(&tree.files);
        LocalGitStore {
            root,
            files: tree.files,
            dirs: tree.dirs,
            neurons,
        }
    }
}

impl BrainStore for LocalGitStore {
    fn root(&self) -> &Path {
        &self.root
    }

    fn files(&self) -> &[WalkedFile] {
        &self.files
    }

    fn neurons(&self) -> &[Neuron] {
        &self.neurons
    }

    fn read(&self, rel: &str) -> Result<Frontmatter, ReadError> {
        sandbox::resolve_in_brain(&self.root, rel).map_err(ReadError::Sandbox)?;
        frontmatter::read_frontmatter(&self.join(&rel.replace('\\', "/"))).map_err(ReadError::File)
    }

    fn read_neuron(&self, neuron: &Neuron) -> Result<Frontmatter, ReadError> {
        frontmatter::read_frontmatter(&neuron.path).map_err(ReadError::File)
    }

    /// On disk, as v2's deprecation check resolves `related`: the target
    /// joined to `from`'s directory and resolved like the walker
    /// ([`sandbox::resolve`]: symlinks followed, then `..`), kept when it is
    /// inside the resolved brain root; a symlink loop resolves to nothing.
    fn resolve_link(&self, from: &str, target: &str) -> Option<String> {
        let dir = match from.rsplit_once('/') {
            Some((dir, _)) => self.join(dir),
            None => self.root.clone(),
        };
        let resolved_root = sandbox::resolve(&self.root).ok()?;
        let resolved = sandbox::resolve(&dir.join(target)).ok()?;
        if !sandbox::is_within(&resolved, &resolved_root) {
            return None;
        }
        let rel = resolved.strip_prefix(&resolved_root).ok()?;
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        (!parts.is_empty()).then(|| parts.join("/"))
    }

    fn lobe_names(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        py_sorted(
            entries
                .flatten()
                .filter(|entry| fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| !is_pruned_dir(name))
                .collect(),
        )
    }

    fn lobe_neurons(&self, lobe: &str) -> Vec<Neuron> {
        neuron_files(&walk(&self.root.join(lobe)))
    }

    fn structure_file(&self, rel: &str) -> Option<Result<Frontmatter, FrontmatterError>> {
        let path = self.join(rel);
        path.exists().then(|| frontmatter::read(&path))
    }

    fn structure_text(&self, rel: &str) -> Option<String> {
        fs::read_to_string(self.join(rel)).ok()
    }

    fn dirs(&self) -> &[String] {
        &self.dirs
    }

    fn entries(&self, rel_dir: &str) -> Vec<DirEntry> {
        let Ok(read) = fs::read_dir(self.join(rel_dir)) else {
            return Vec::new();
        };
        let mut found: Vec<(String, PathBuf)> = read
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .collect();
        let order = py_sorted(found.iter().map(|(name, _)| name.clone()).collect());
        order
            .into_iter()
            .filter_map(|name| {
                let at = found.iter().position(|(n, _)| *n == name)?;
                let (name, path) = found.swap_remove(at);
                let meta = fs::metadata(&path).ok();
                Some(DirEntry {
                    name,
                    is_file: meta.as_ref().is_some_and(|m| m.is_file()),
                    is_dir: meta.as_ref().is_some_and(|m| m.is_dir()),
                })
            })
            .collect()
    }

    fn exists(&self, rel: &str) -> bool {
        self.join(rel).exists()
    }

    fn bytes(&self, rel: &str) -> Option<Vec<u8>> {
        fs::read(self.join(rel)).ok()
    }

    fn real_rel(&self, rel: &str) -> Option<String> {
        let path = self.join(rel);
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Some(rel.to_string());
        }
        let target = sandbox::resolve(&path).ok()?;
        let root = sandbox::resolve(&self.root).ok()?;
        let inside = target.strip_prefix(&root).ok()?;
        let parts: Vec<String> = inside
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        (!parts.is_empty()).then(|| parts.join("/"))
    }

    fn apply(&mut self, changes: &ChangeSet) -> Applied {
        let mut applied = Applied::default();
        for write in &changes.writes {
            let failed = |error: String| FailedWrite {
                rel: write.rel.clone(),
                error,
            };
            if changes.base.is_some() {
                applied.failed.push(failed(
                    "a local store applies changes to the working tree only".into(),
                ));
                continue;
            }
            if !is_plain_rel(&write.rel) {
                applied
                    .failed
                    .push(failed("not a path inside the brain".into()));
                continue;
            }
            match fs::write(self.join(&write.rel), &write.contents) {
                Ok(()) => applied.written.push(write.rel.clone()),
                Err(e) => applied.failed.push(failed(e.to_string())),
            }
        }
        *self = LocalGitStore::open(&self.root);
        applied
    }
}

impl LocalGitStore {
    /// The root joined with a `/`-separated relative path (split at `/`
    /// only, so a `\\` in a POSIX file name stays part of it).
    fn join(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|part| !part.is_empty())
            .fold(self.root.clone(), |path, part| path.join(part))
    }
}
