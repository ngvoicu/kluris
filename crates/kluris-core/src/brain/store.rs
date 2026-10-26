//! `BrainStore`: how every command reads a brain. Only the read side exists so
//! far; write methods come with the commands that write.

use std::fs;
use std::path::{Path, PathBuf};

use super::glossary::{GlossaryEntry, parse_glossary_entries};
use super::neuron_index::{Neuron, neuron_files};
use super::sandbox::{self, SandboxError};
use super::walk::{WalkedFile, is_pruned_dir, walk};
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

/// The read side of a brain.
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
}

/// A brain in a local directory (a git checkout or not), walked once when
/// opened.
#[derive(Debug, Clone)]
pub struct LocalGitStore {
    root: PathBuf,
    files: Vec<WalkedFile>,
    neurons: Vec<Neuron>,
}

impl LocalGitStore {
    pub fn open(root: impl AsRef<Path>) -> LocalGitStore {
        let root = root.as_ref().to_path_buf();
        let files = walk(&root);
        let neurons = neuron_files(&files);
        LocalGitStore {
            root,
            files,
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
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| fs::metadata(entry.path()).is_ok_and(|m| m.is_dir()))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !is_pruned_dir(name))
            .collect();
        if cfg!(windows) {
            names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
        } else {
            names.sort();
        }
        names
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
