//! A read-only brain over a git tree plus overlay files ([`TreeStore`]):
//! the view `kluris commit` builds its maps from (the base revision's files,
//! with the given paths taken from the work tree), and the one `sync`
//! rebuilds a merged tree's maps from. Nothing else is in it: a file that is
//! uncommitted, untracked or ignored in the work tree is invisible.
//!
//! It answers as [`LocalGitStore`](super::store::LocalGitStore) answers for
//! a brain on disk holding exactly these files:
//! - directories are the ancestors of files (git keeps no empty one);
//! - the walk prunes the directories the disk walker prunes ([`is_pruned_dir`]);
//! - a symlink is followed only inside the view: its target is resolved
//!   against its directory, `..` applied lexically, at most 40 links deep;
//!   one that leads out of the view (an absolute target, `..` past the root,
//!   a path with nothing there) or loops is neither a file nor a directory;
//! - contents are known for the files a map reads (`.md`, `.yml`, `.yaml`)
//!   and the overlay files; any other file is there with no bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::change::{Applied, ChangeSet, FailedWrite};
use super::neuron_index::{Neuron, NeuronKind, SKIP_FILES};
use super::sandbox::SandboxError;
use super::store::{BrainStore, DirEntry, ReadError, py_sorted};
use super::walk::{WalkedFile, is_pruned_dir};
use crate::frontmatter::{self, Frontmatter, FrontmatterError, Problem};

/// One file of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A file; `None` when its contents were not loaded.
    File(Option<Vec<u8>>),
    /// A symlink, by its target as written.
    Link(String),
}

/// How deep a chain of symlinks is followed.
const MAX_LINKS: usize = 40;

/// What a path in the view leads to.
enum Target<'a> {
    File(&'a Option<Vec<u8>>, String),
    Dir(String),
}

fn parent_dir(rel: &str) -> &str {
    rel.rfind('/').map_or("", |i| &rel[..i])
}

fn base_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// `target` joined to `dir`, `.` and `..` applied; `None` when it is
/// absolute or leaves the root.
fn join_link(dir: &str, target: &str) -> Option<String> {
    if target.starts_with('/') || target.starts_with('\\') || target.contains(':') {
        return None;
    }
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

/// Whether a file name is a markdown neuron's or a YAML neuron's suffix
/// (case-sensitive, as the neuron index has it).
fn neuron_kind(name: &str) -> Option<NeuronKind> {
    if SKIP_FILES.contains(&name) || name == "map.md" {
        None
    } else if name.ends_with(".md") {
        Some(NeuronKind::Markdown)
    } else if name.ends_with(".yml") || name.ends_with(".yaml") {
        Some(NeuronKind::Yaml)
    } else {
        None
    }
}

/// A brain view over files held in memory (see the module docs).
#[derive(Debug, Clone)]
pub struct TreeStore {
    root: PathBuf,
    nodes: BTreeMap<String, Node>,
    /// Every directory, pruned ones included.
    all_dirs: BTreeSet<String>,
    /// The names in each directory (`""` is the root).
    children: BTreeMap<String, BTreeSet<String>>,
    files: Vec<WalkedFile>,
    dirs: Vec<String>,
    neurons: Vec<Neuron>,
}

impl TreeStore {
    /// The view of `nodes` (brain-relative, `/`-separated paths), as if the
    /// brain were at `root`.
    pub fn new(root: impl AsRef<Path>, nodes: BTreeMap<String, Node>) -> TreeStore {
        let root = root.as_ref().to_path_buf();
        let mut all_dirs = BTreeSet::new();
        let mut children: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for rel in nodes.keys() {
            let mut child = rel.as_str();
            loop {
                let dir = parent_dir(child);
                children
                    .entry(dir.to_string())
                    .or_default()
                    .insert(base_name(child).to_string());
                if dir.is_empty() || !all_dirs.insert(dir.to_string()) {
                    break;
                }
                child = dir;
            }
        }
        let mut store = TreeStore {
            root,
            nodes,
            all_dirs,
            children,
            files: Vec::new(),
            dirs: Vec::new(),
            neurons: Vec::new(),
        };
        let walked = |rel: &str| !parent_dir(rel).split('/').any(is_pruned_dir);
        store.dirs = store
            .all_dirs
            .iter()
            .filter(|dir| !dir.split('/').any(is_pruned_dir))
            .cloned()
            .collect();
        store.files = store
            .nodes
            .iter()
            .filter(|(rel, node)| {
                walked(rel)
                    && match node {
                        Node::File(_) => true,
                        Node::Link(_) => matches!(store.resolve(rel), Some(Target::File(..))),
                    }
            })
            .map(|(rel, node)| WalkedFile {
                rel: rel.clone(),
                path: store.join(rel),
                symlink: matches!(node, Node::Link(_)),
            })
            .collect();
        let mut markdown = Vec::new();
        let mut yaml = Vec::new();
        for file in &store.files {
            let neuron = |kind| Neuron {
                rel: file.rel.clone(),
                path: file.path.clone(),
                kind,
            };
            match neuron_kind(base_name(&file.rel)) {
                Some(NeuronKind::Markdown) => markdown.push(neuron(NeuronKind::Markdown)),
                Some(NeuronKind::Yaml)
                    if store
                        .bytes(&file.rel)
                        .is_some_and(|b| frontmatter::has_yaml_opt_in_head(&b)) =>
                {
                    yaml.push(neuron(NeuronKind::Yaml))
                }
                _ => {}
            }
        }
        markdown.extend(yaml);
        store.neurons = markdown;
        store
    }

    /// The files of the view.
    pub fn nodes(&self) -> &BTreeMap<String, Node> {
        &self.nodes
    }

    fn join(&self, rel: &str) -> PathBuf {
        rel.split('/')
            .filter(|part| !part.is_empty())
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    /// Where `rel` leads, symlinks followed inside the view.
    fn resolve(&self, rel: &str) -> Option<Target<'_>> {
        let mut current = rel.to_string();
        for _ in 0..=MAX_LINKS {
            match self.nodes.get(&current) {
                Some(Node::File(bytes)) => return Some(Target::File(bytes, current)),
                Some(Node::Link(target)) => {
                    current = join_link(parent_dir(&current), target)?;
                }
                None if current.is_empty() || self.all_dirs.contains(&current) => {
                    return Some(Target::Dir(current));
                }
                None => return None,
            }
        }
        None
    }

    fn file_error(&self, rel: &str, problem: Problem) -> FrontmatterError {
        FrontmatterError {
            path: self.join(rel),
            problem,
        }
    }

    /// A file read as markdown, as `frontmatter::read` reads one on disk.
    fn read_markdown(&self, rel: &str) -> Result<Frontmatter, FrontmatterError> {
        let bytes = self
            .bytes(rel)
            .ok_or_else(|| self.file_error(rel, Problem::Io("not in the view".into())))?;
        let text = String::from_utf8(bytes).map_err(|_| self.file_error(rel, Problem::NotUtf8))?;
        frontmatter::parse(&text).map_err(|e| self.file_error(rel, Problem::Yaml(e)))
    }
}

impl BrainStore for TreeStore {
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
        let rel = rel.replace('\\', "/");
        if rel.is_empty() {
            return Err(ReadError::Sandbox(SandboxError::Empty));
        }
        match self.resolve(&rel) {
            Some(Target::File(..)) => {}
            None if self.nodes.contains_key(&rel) => {
                return Err(ReadError::Sandbox(SandboxError::Outside));
            }
            _ => return Err(ReadError::Sandbox(SandboxError::NotFound)),
        }
        if frontmatter::has_yaml_suffix(Path::new(&rel)) {
            let bytes = self.bytes(&rel).unwrap_or_default();
            return Ok(frontmatter::parse_yaml_neuron_lenient(
                &String::from_utf8_lossy(&bytes),
            ));
        }
        self.read_markdown(&rel).map_err(ReadError::File)
    }

    fn lobe_names(&self) -> Vec<String> {
        py_sorted(
            self.entries("")
                .into_iter()
                .filter(|entry| entry.is_dir && !is_pruned_dir(&entry.name))
                .map(|entry| entry.name)
                .collect(),
        )
    }

    fn lobe_neurons(&self, lobe: &str) -> Vec<Neuron> {
        let prefix = format!("{lobe}/");
        self.neurons
            .iter()
            .filter_map(|neuron| {
                let rel = neuron.rel.strip_prefix(&prefix)?;
                Some(Neuron {
                    rel: rel.to_string(),
                    path: neuron.path.clone(),
                    kind: neuron.kind,
                })
            })
            .collect()
    }

    fn structure_file(&self, rel: &str) -> Option<Result<Frontmatter, FrontmatterError>> {
        self.exists(rel).then(|| self.read_markdown(rel))
    }

    fn structure_text(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.bytes(rel)?).ok()
    }

    fn dirs(&self) -> &[String] {
        &self.dirs
    }

    fn entries(&self, rel_dir: &str) -> Vec<DirEntry> {
        let Some(names) = self.children.get(rel_dir) else {
            return Vec::new();
        };
        py_sorted(names.iter().cloned().collect())
            .into_iter()
            .map(|name| {
                let rel = if rel_dir.is_empty() {
                    name.clone()
                } else {
                    format!("{rel_dir}/{name}")
                };
                let target = self.resolve(&rel);
                DirEntry {
                    name,
                    is_file: matches!(target, Some(Target::File(..))),
                    is_dir: matches!(target, Some(Target::Dir(_))),
                }
            })
            .collect()
    }

    fn exists(&self, rel: &str) -> bool {
        self.resolve(rel).is_some()
    }

    fn bytes(&self, rel: &str) -> Option<Vec<u8>> {
        match self.resolve(rel)? {
            Target::File(bytes, _) => bytes.clone(),
            Target::Dir(_) => None,
        }
    }

    fn real_rel(&self, rel: &str) -> Option<String> {
        match self.nodes.get(rel) {
            Some(Node::Link(_)) => match self.resolve(rel)? {
                Target::File(_, real) | Target::Dir(real) => Some(real),
            },
            _ => Some(rel.to_string()),
        }
    }

    fn apply(&mut self, changes: &ChangeSet) -> Applied {
        Applied {
            written: Vec::new(),
            failed: changes
                .writes
                .iter()
                .map(|write| FailedWrite {
                    rel: write.rel.clone(),
                    error: "a view of a git tree is read-only".into(),
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::brain::change::{ChangeSet, FileWrite};
    use crate::brain::store::BrainStore;

    fn file(text: &str) -> Node {
        Node::File(Some(text.as_bytes().to_vec()))
    }

    fn store(nodes: &[(&str, Node)]) -> TreeStore {
        let nodes: BTreeMap<String, Node> = nodes
            .iter()
            .map(|(rel, node)| (rel.to_string(), node.clone()))
            .collect();
        TreeStore::new("/brain", nodes)
    }

    const NEURON: &str = "---\nparent: ./map.md\n---\n\n# A\n";

    #[test]
    fn dirs_files_and_lobes_follow_the_walker() {
        let s = store(&[
            ("top.md", file(NEURON)),
            ("notes/a.md", file(NEURON)),
            ("notes/deep/b.md", file(NEURON)),
            ("notes/map.md", file("# Notes\n")),
            (".github/x.md", file(NEURON)),
            ("node_modules/y.md", file(NEURON)),
            ("assets/logo.png", Node::File(None)),
        ]);
        assert_eq!(s.dirs(), ["assets", "notes", "notes/deep"]);
        assert_eq!(s.lobe_names(), ["assets", "notes"]);
        let files: Vec<&str> = s.files().iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            files,
            [
                "assets/logo.png",
                "notes/a.md",
                "notes/deep/b.md",
                "notes/map.md",
                "top.md"
            ]
        );
        let neurons: Vec<&str> = s.neurons().iter().map(|n| n.rel.as_str()).collect();
        assert_eq!(neurons, ["notes/a.md", "notes/deep/b.md", "top.md"]);
        let lobe: Vec<String> = s.lobe_neurons("notes").into_iter().map(|n| n.rel).collect();
        assert_eq!(lobe, ["a.md", "deep/b.md"]);
    }

    #[test]
    fn entries_exists_and_bytes() {
        let s = store(&[
            ("notes/a.md", file(NEURON)),
            ("notes/deep/b.md", file(NEURON)),
            ("assets/logo.png", Node::File(None)),
        ]);
        let entries = s.entries("notes");
        let names: Vec<(&str, bool, bool)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.is_file, e.is_dir))
            .collect();
        assert_eq!(names, [("a.md", true, false), ("deep", false, true)]);
        assert!(s.exists("notes") && s.exists("notes/a.md") && s.exists("assets/logo.png"));
        assert!(!s.exists("notes/none.md") && !s.exists("no"));
        assert_eq!(s.bytes("notes/a.md").as_deref(), Some(NEURON.as_bytes()));
        assert_eq!(s.bytes("assets/logo.png"), None);
        assert_eq!(s.bytes("notes"), None);
        assert_eq!(s.structure_text("notes/a.md").as_deref(), Some(NEURON));
        let fm = s.read("notes/a.md").unwrap();
        assert!(fm.body.contains("# A"));
        assert!(s.read("notes/none.md").is_err());
        assert!(s.structure_file("notes/none.md").is_none());
        assert!(s.structure_file("notes/a.md").is_some_and(|r| r.is_ok()));
    }

    #[test]
    fn yaml_neurons_need_the_opt_in_block() {
        let s = store(&[
            ("api/in.yml", file("#---\n# title: In\n#---\nk: 1\n")),
            ("api/out.yaml", file("k: 1\n")),
        ]);
        let neurons: Vec<&str> = s.neurons().iter().map(|n| n.rel.as_str()).collect();
        assert_eq!(neurons, ["api/in.yml"]);
    }

    #[test]
    fn links_resolve_inside_the_view_only() {
        let s = store(&[
            ("notes/a.md", file(NEURON)),
            ("notes/l.md", Node::Link("a.md".into())),
            ("notes/up.md", Node::Link("../notes/./a.md".into())),
            ("notes/out.md", Node::Link("../../outside.md".into())),
            ("notes/abs.md", Node::Link("/etc/passwd".into())),
            ("notes/loop1.md", Node::Link("loop2.md".into())),
            ("notes/loop2.md", Node::Link("loop1.md".into())),
            ("notes/dir", Node::Link("../other".into())),
            ("other/c.md", file(NEURON)),
        ]);
        assert_eq!(s.real_rel("notes/l.md").as_deref(), Some("notes/a.md"));
        assert_eq!(s.real_rel("notes/up.md").as_deref(), Some("notes/a.md"));
        assert_eq!(s.real_rel("notes/a.md").as_deref(), Some("notes/a.md"));
        assert_eq!(s.real_rel("notes/out.md"), None);
        assert_eq!(s.real_rel("notes/abs.md"), None);
        assert_eq!(s.real_rel("notes/loop1.md"), None);
        assert_eq!(s.bytes("notes/l.md").as_deref(), Some(NEURON.as_bytes()));
        assert_eq!(s.bytes("notes/out.md"), None);
        let kinds: BTreeMap<String, (bool, bool)> = s
            .entries("notes")
            .into_iter()
            .map(|e| (e.name, (e.is_file, e.is_dir)))
            .collect();
        assert_eq!(kinds["l.md"], (true, false));
        assert_eq!(kinds["out.md"], (false, false));
        assert_eq!(kinds["loop1.md"], (false, false));
        assert_eq!(kinds["dir"], (false, true));
        // The walker lists a link to a file inside, never one leading out.
        let files: Vec<&str> = s.files().iter().map(|f| f.rel.as_str()).collect();
        assert!(files.contains(&"notes/l.md") && !files.contains(&"notes/out.md"));
        assert!(!s.dirs().contains(&"notes/dir".to_string()));
    }

    #[test]
    fn the_view_is_read_only() {
        let mut s = store(&[("notes/a.md", file(NEURON))]);
        let applied = s.apply(&ChangeSet {
            writes: vec![FileWrite::replace("notes/a.md", "x")],
            ..ChangeSet::default()
        });
        assert!(applied.written.is_empty());
        assert_eq!(applied.failed.len(), 1);
        assert_eq!(s.bytes("notes/a.md").as_deref(), Some(NEURON.as_bytes()));
    }
}
