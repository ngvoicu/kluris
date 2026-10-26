//! The Specmint companion playbooks (v2 `core/companions.py`).
//!
//! The four companions are embedded from `vendored/` at build time and
//! installed on opt-in to `~/.kluris/companions/<name>/`: `SKILL.md` plus the
//! `references/` and `assets/` folders, nothing else ([`install`]). A brain
//! opts in through the `companions` list of its `kluris.yml` ([`add`],
//! [`remove`]).

use std::borrow::Cow;
use std::path::{Component, Path};

use include_dir::{Dir, DirEntry, include_dir};

mod install;
mod optin;
pub mod prompt;

pub use install::{
    CompanionError, install, install_tree, installed, installed_dir, is_installed, referenced,
    refresh, verify_runtime_dir,
};
pub use optin::{BrainOptIns, OptInBrain, add, enabled, listing, load_brains, remove};

/// Known companions, in canonical order (v2 `companions.KNOWN`).
pub const KNOWN: [&str; 4] = [
    "specmint-core",
    "specmint-tdd",
    "specmint-core-html",
    "specmint-tdd-html",
];

/// Known companion names, de-duplicated, in canonical order; unknown names
/// are dropped (v2 `companions.normalize`).
pub fn normalize<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|known| names.iter().any(|n| n.as_ref() == **known))
        .map(|known| known.to_string())
        .collect()
}

/// `vendored/`, as it was when this crate was built.
static VENDORED: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../vendored");

/// The files and folders of one companion, at POSIX paths relative to its
/// folder (`SKILL.md`, `references/spec-format.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompanionTree {
    files: Vec<(String, Cow<'static, [u8]>)>,
    dirs: Vec<String>,
}

impl CompanionTree {
    pub fn new() -> CompanionTree {
        CompanionTree::default()
    }

    /// The tree with a file at `rel`.
    pub fn file(mut self, rel: impl Into<String>, bytes: impl Into<Cow<'static, [u8]>>) -> Self {
        self.files.push((rel.into(), bytes.into()));
        self
    }

    /// The tree with a folder at `rel` (folders holding files need not be
    /// listed).
    pub fn dir(mut self, rel: impl Into<String>) -> Self {
        self.dirs.push(rel.into());
        self
    }

    /// Every file, in the order added.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(rel, bytes)| (rel.as_str(), bytes.as_ref()))
    }

    /// Every listed folder, in the order added.
    pub fn dirs(&self) -> impl Iterator<Item = &str> {
        self.dirs.iter().map(String::as_str)
    }
}

/// `path` relative to `base`, with `/` separators.
fn posix_rel(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    Some(parts.join("/"))
}

/// The embedded copy of companion `name`; `None` for an unknown name.
pub fn embedded(name: &str) -> Option<CompanionTree> {
    if !KNOWN.contains(&name) {
        return None;
    }
    let root = VENDORED.get_dir(name)?;
    let mut tree = CompanionTree::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in dir.entries() {
            let rel = posix_rel(entry.path(), root.path())?;
            match entry {
                DirEntry::Dir(sub) => {
                    tree = tree.dir(rel);
                    stack.push(sub);
                }
                DirEntry::File(file) => tree = tree.file(rel, Cow::Borrowed(file.contents())),
            }
        }
    }
    Some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_filters_dedupes_and_orders() {
        let names = [
            "specmint-tdd-html",
            "bogus",
            "specmint-core",
            "specmint-core",
        ];
        assert_eq!(
            normalize(&names),
            vec!["specmint-core", "specmint-tdd-html"]
        );
    }

    #[test]
    fn normalize_empty() {
        assert!(normalize::<&str>(&[]).is_empty());
    }
}
