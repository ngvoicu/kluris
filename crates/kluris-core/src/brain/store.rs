//! `BrainStore`: how every command reads a brain. Only the read side exists so
//! far; write methods come with the commands that write.

use std::path::{Path, PathBuf};

use super::glossary::{GlossaryEntry, parse_glossary_entries};
use super::neuron_index::{Neuron, neuron_files};
use super::sandbox::{self, SandboxError};
use super::walk::{WalkedFile, walk};
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
    /// The glossary entries of `glossary.md`: none when the file does not
    /// exist, an error when its frontmatter cannot be read.
    fn glossary(&self) -> Result<Vec<GlossaryEntry>, FrontmatterError>;
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
        let path = rel
            .replace('\\', "/")
            .split('/')
            .filter(|part| !part.is_empty())
            .fold(self.root.clone(), |path, part| path.join(part));
        frontmatter::read_frontmatter(&path).map_err(ReadError::File)
    }

    fn glossary(&self) -> Result<Vec<GlossaryEntry>, FrontmatterError> {
        let path = self.root.join("glossary.md");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let fm = frontmatter::read(&path)?;
        Ok(parse_glossary_entries(&fm.body))
    }
}
