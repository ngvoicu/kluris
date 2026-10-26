//! What counts as a neuron: v2's `neuron_index.neuron_files` over the
//! shared walk.
//!
//! - a markdown neuron is a file whose name ends in `.md` (case-sensitive, so
//!   `X.MD` is not one, and a file named `.md` is);
//! - a YAML neuron is a file whose name ends in `.yml` or `.yaml`
//!   (case-sensitive) and opens with a complete `#---` block within its first
//!   4 KB ([`frontmatter::has_yaml_opt_in_block`]);
//! - at any depth, names in [`SKIP_FILES`] and `map.md` are not neurons
//!   (`projects/README.md` is not one; `map.yml` can be);
//! - root-level files are neurons like any other.
//!
//! Markdown neurons come first, then YAML neurons, as in v2; each group is in
//! the walk's order (v2: `os.walk` order).

use std::path::PathBuf;

use super::walk::WalkedFile;
use crate::frontmatter;

/// v2's `SKIP_FILES`: never neurons, at any depth.
pub const SKIP_FILES: [&str; 6] = [
    "brain.md",
    "index.md",
    "glossary.md",
    "README.md",
    ".gitignore",
    "kluris.yml",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeuronKind {
    Markdown,
    Yaml,
}

impl NeuronKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NeuronKind::Markdown => "markdown",
            NeuronKind::Yaml => "yaml",
        }
    }
}

/// A neuron file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neuron {
    /// The path relative to the brain root, `/`-separated.
    pub rel: String,
    /// The root joined with the relative path (not resolved).
    pub path: PathBuf,
    pub kind: NeuronKind,
}

/// The neurons among the walked files.
pub fn neuron_files(files: &[WalkedFile]) -> Vec<Neuron> {
    let mut markdown = Vec::new();
    let mut yaml = Vec::new();
    for file in files {
        let name = file.rel.rsplit('/').next().unwrap_or(&file.rel);
        if SKIP_FILES.contains(&name) || name == "map.md" {
            continue;
        }
        let neuron = |kind| Neuron {
            rel: file.rel.clone(),
            path: file.path.clone(),
            kind,
        };
        if name.ends_with(".md") {
            markdown.push(neuron(NeuronKind::Markdown));
        } else if (name.ends_with(".yml") || name.ends_with(".yaml"))
            && frontmatter::has_yaml_opt_in_block(&file.path)
        {
            yaml.push(neuron(NeuronKind::Yaml));
        }
    }
    markdown.extend(yaml);
    markdown
}
