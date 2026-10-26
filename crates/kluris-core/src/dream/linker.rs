//! v2's `core/linker.py`: the two fixes dream applies (steps 2 and 3) and the
//! validators it runs on the written brain.
//!
//! Every path a report holds is brain-relative and `/`-joined
//! (interview-01 #7). A file that cannot be read (strictly: a YAML neuron
//! whose block does not parse too) is skipped and reported where v2 raises,
//! and gets no other report. Links are found by [`super::links`].

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::links::{decoded, markdown_links};
use super::workspace::{Workspace, converged};
use super::{BrokenSynapse, FieldProblem, FrontmatterIssue, OneWaySynapse};
use crate::brain::neuron_index::{Neuron, NeuronKind};
use crate::brain::sandbox;
use crate::brain::store::BrainStore;
use crate::brain::walk::WalkedFile;
use crate::yaml::{Map, Value};

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

/// `Path.resolve()` of a link from `from`; a symlink loop resolves to
/// nothing (v2 raises there).
fn resolve_from(from: &Path, link: &str) -> Option<PathBuf> {
    sandbox::resolve(&parent(from).join(link)).ok()
}

fn strings(value: Option<&Value>) -> Option<Vec<&str>> {
    value
        .and_then(Value::as_seq)
        .map(|items| items.iter().filter_map(Value::as_str).collect())
}

/// `os.path.relpath(source, start=dir(target))` for two brain-relative
/// `/`-separated paths, `/`-joined.
fn relpath(source: &str, target: &str) -> String {
    let from: Vec<&str> = target.split('/').collect();
    let from = &from[..from.len() - 1];
    let to: Vec<&str> = source.split('/').collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    parts.join("/")
}

/// Step 2 (`fix_missing_frontmatter`): every neuron below the root without a
/// `parent` key gets `parent: ./map.md`. Returns the neurons written, one
/// entry each.
pub(crate) fn fix_missing_parents<S: BrainStore>(
    ws: &mut Workspace<'_, S>,
    neurons: &[Neuron],
) -> Vec<String> {
    let mut fixed = Vec::new();
    for neuron in neurons.iter().filter(|n| n.rel.contains('/')) {
        let Some(fm) = ws.read_for_write(&neuron.rel) else {
            continue;
        };
        if fm.meta.contains_key("parent") {
            continue;
        }
        let mut meta = converged(&fm.meta);
        meta.insert("parent".into(), Value::Str("./map.md".into()));
        ws.write(&neuron.rel, Some(&fm), &meta, &fm.body);
        fixed.push(neuron.rel.clone());
    }
    fixed
}

/// Step 3 (`fix_bidirectional_synapses`): for each string `related` entry
/// that resolves to another neuron that does not link back, append the
/// source's path relative to the target's directory to the target's
/// `related` (a `related` that is not a list is replaced). Sources in the
/// store's neuron order (PAR-008); each target is read again for every link,
/// so it collects all its reverse links in one run. Returns the target of
/// each link added.
pub(crate) fn fix_reverse_links<S: BrainStore>(
    ws: &mut Workspace<'_, S>,
    neurons: &[Neuron],
) -> Vec<String> {
    // v2's `{neuron.resolve(): neuron}`: one entry per file, the last walked
    // path of it, at the first one's position.
    let mut by_file: IndexMap<PathBuf, &Neuron> = IndexMap::new();
    for neuron in neurons {
        if let Ok(resolved) = sandbox::resolve(&neuron.path) {
            by_file.insert(resolved, neuron);
        }
    }
    let mut fixed = Vec::new();
    let sources: Vec<(PathBuf, &Neuron)> = by_file.iter().map(|(k, v)| (k.clone(), *v)).collect();
    for (source_file, source) in sources {
        let Some(fm) = ws.read_for_write(&source.rel) else {
            continue;
        };
        let Some(related) = strings(fm.meta.get("related")) else {
            continue;
        };
        for link in related {
            let Some(target) = resolve_from(&source.path, link).and_then(|t| by_file.get(&t))
            else {
                continue;
            };
            let Some(target_fm) = ws.read_for_write(&target.rel) else {
                continue;
            };
            let existing: Vec<Value> = match target_fm.meta.get("related") {
                Some(Value::Seq(items)) => items.clone(),
                _ => Vec::new(),
            };
            let links_back = existing
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|back| resolve_from(&target.path, back))
                .any(|back| back == source_file);
            if links_back {
                continue;
            }
            let mut items = existing;
            items.push(Value::Str(relpath(&source.rel, &target.rel)));
            let mut meta = converged(&target_fm.meta);
            meta.insert("related".into(), Value::Seq(items));
            ws.write(&target.rel, Some(&target_fm), &meta, &target_fm.body);
            fixed.push(target.rel.clone());
        }
    }
    fixed
}

// --- validators ------------------------------------------------------------------

/// Reads for the validators: as dream reads a file it may rewrite, a file
/// that does not read skipped and reported (PAR-038).
struct Reader<'a, S: BrainStore> {
    store: &'a S,
    skipped: &'a mut BTreeMap<String, String>,
}

impl<S: BrainStore> Reader<'_, S> {
    fn walked(&mut self, rel: &str) -> Option<(Map, String)> {
        let Some(bytes) = self.store.bytes(rel) else {
            self.skipped
                .entry(rel.into())
                .or_insert_with(|| "cannot read the file".into());
            return None;
        };
        match super::workspace::parse(rel, &bytes, super::workspace::Read::Strict) {
            Ok(fm) => Some((fm.meta, fm.body)),
            Err(e) => {
                self.skipped.entry(rel.into()).or_insert(e);
                None
            }
        }
    }

    fn neuron(&mut self, neuron: &Neuron) -> Option<(Map, String)> {
        self.walked(&neuron.rel)
    }
}

/// A resolved path relative to the resolved root, `/`-joined; the whole
/// path when it is outside.
fn brain_rel(path: &Path, root: Option<&Path>) -> String {
    let inside = root.and_then(|root| path.strip_prefix(root).ok());
    match inside {
        Some(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        None => path.to_string_lossy().into_owned(),
    }
}

fn name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// v2's `_all_md_files`: every walked `.md` file (map.md, brain.md and the
/// rest included), then the YAML neurons.
fn all_md_files<S: BrainStore>(store: &S) -> Vec<(String, PathBuf)> {
    let md = store
        .files()
        .iter()
        .filter(|f: &&WalkedFile| name(&f.rel).ends_with(".md"))
        .map(|f| (f.rel.clone(), f.path.clone()));
    let yaml = store
        .neurons()
        .iter()
        .filter(|n| n.kind == NeuronKind::Yaml)
        .map(|n| (n.rel.clone(), n.path.clone()));
    md.chain(yaml).collect()
}

pub(crate) struct Validation {
    pub broken: Vec<BrokenSynapse>,
    pub one_way: Vec<OneWaySynapse>,
    pub orphans: Vec<String>,
    pub frontmatter: Vec<FrontmatterIssue>,
}

/// `validate_synapses`: body links of every file but `README.md`, and the
/// string `related` entries of neurons, that do not exist or resolve outside
/// the brain.
fn broken_synapses<S: BrainStore>(store: &S, read: &mut Reader<'_, S>) -> Vec<BrokenSynapse> {
    let root = sandbox::resolve(store.root()).ok();
    let neurons: HashSet<PathBuf> = store
        .neurons()
        .iter()
        .filter_map(|n| sandbox::resolve(&n.path).ok())
        .collect();
    let exists = |from: &Path, link: &str| {
        resolve_from(from, link).is_some_and(|t| {
            t.exists()
                && root
                    .as_deref()
                    .is_some_and(|root| sandbox::is_within(&t, root))
        })
    };
    // A body link may be URL-encoded: either reading may name the file.
    let link_ok = |from: &Path, link: &str| {
        exists(from, link) || decoded(link).is_some_and(|d| exists(from, &d))
    };
    let mut broken = Vec::new();
    for (rel, path) in all_md_files(store) {
        if name(&rel) == "README.md" {
            continue;
        }
        let Some((meta, body)) = read.walked(&rel) else {
            continue;
        };
        for link in markdown_links(&body) {
            if !link_ok(&path, &link) {
                broken.push(BrokenSynapse {
                    file: rel.clone(),
                    target: link,
                });
            }
        }
        if !sandbox::resolve(&path).is_ok_and(|p| neurons.contains(&p)) {
            continue;
        }
        for link in strings(meta.get("related")).unwrap_or_default() {
            if !exists(&path, link) {
                broken.push(BrokenSynapse {
                    file: rel.clone(),
                    target: link.to_string(),
                });
            }
        }
    }
    broken
}

/// Python's `PurePath.suffix`.
fn py_suffix(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 && i < name.len() - 1 => &name[i..],
        _ => "",
    }
}

/// `validate_bidirectional`: a neuron's `related` target that is a neuron
/// not linking back, or another existing `.md` file whose list `related`
/// does not link back. v2 raises on a non-string entry and on a target
/// outside the brain; Kluris 3 ignores both here (they are a frontmatter
/// issue and broken synapses, PAR-039).
fn one_way_synapses<S: BrainStore>(store: &S, read: &mut Reader<'_, S>) -> Vec<OneWaySynapse> {
    let root = sandbox::resolve(store.root()).ok();
    let mut related: IndexMap<PathBuf, Vec<PathBuf>> = IndexMap::new();
    for neuron in store.neurons() {
        let Some((meta, _)) = read.neuron(neuron) else {
            continue;
        };
        // `meta.get("related", [])`: a neuron without the key links nowhere.
        let links = match meta.get("related") {
            None => Some(Vec::new()),
            related => strings(related),
        };
        let (Some(links), Ok(file)) = (links, sandbox::resolve(&neuron.path)) else {
            continue;
        };
        let targets = links
            .into_iter()
            .filter_map(|link| resolve_from(&neuron.path, link))
            .collect();
        related.insert(file, targets);
    }
    let inside = |path: &Path| {
        root.as_deref()
            .is_some_and(|root| sandbox::is_within(path, root))
    };
    let mut one_way = Vec::new();
    for (source, targets) in &related {
        for target in targets.iter().filter(|t| inside(t)) {
            let back = match related.get(target) {
                Some(back) => back.contains(source),
                None => {
                    let markdown = target.exists()
                        && py_suffix(&target.file_name().unwrap_or_default().to_string_lossy())
                            == ".md";
                    if !markdown {
                        continue;
                    }
                    let Ok(fm) = store.read_markdown_at(target) else {
                        read.skipped
                            .entry(brain_rel(target, root.as_deref()))
                            .or_insert_with(|| "cannot read the file".into());
                        continue;
                    };
                    // v2's `get("related", [])`: missing is an empty list.
                    let links = match fm.meta.get("related") {
                        None => Vec::new(),
                        related => match strings(related) {
                            Some(links) => links,
                            None => continue,
                        },
                    };
                    links
                        .into_iter()
                        .filter_map(|link| resolve_from(target, link))
                        .any(|back| back == *source)
                }
            };
            if !back {
                one_way.push(OneWaySynapse {
                    source: brain_rel(source, root.as_deref()),
                    target: brain_rel(target, root.as_deref()),
                });
            }
        }
    }
    one_way
}

/// `detect_orphans`: neurons no link in a `map.md` or `brain.md` body points
/// at.
pub(crate) fn orphans<S: BrainStore>(
    store: &S,
    skipped: &mut BTreeMap<String, String>,
) -> Vec<String> {
    let mut read = Reader { store, skipped };
    let mut referenced = HashSet::new();
    for (rel, path) in all_md_files(store) {
        if !matches!(name(&rel), "map.md" | "brain.md") {
            continue;
        }
        let Some((_, body)) = read.walked(&rel) else {
            continue;
        };
        for link in markdown_links(&body) {
            referenced.extend(resolve_from(&path, &link));
            if let Some(decoded) = decoded(&link) {
                referenced.extend(resolve_from(&path, &decoded));
            }
        }
    }
    store
        .neurons()
        .iter()
        .filter(|n| !sandbox::resolve(&n.path).is_ok_and(|p| referenced.contains(&p)))
        .map(|n| n.rel.clone())
        .collect()
}

/// `check_frontmatter`: the required fields (markdown: `parent`, `created`,
/// `updated`; YAML neurons: `updated`), then `related` and `tags` that are not
/// lists, a `related` list with an entry that is not a string (where v2
/// raises, PAR-039) and a `replaced_by` that is not a string or null.
fn frontmatter_issues<S: BrainStore>(store: &S, read: &mut Reader<'_, S>) -> Vec<FrontmatterIssue> {
    let mut issues = Vec::new();
    for neuron in store.neurons() {
        let Some((meta, _)) = read.neuron(neuron) else {
            continue;
        };
        let mut issue = |field: &'static str, problem| {
            issues.push(FrontmatterIssue {
                file: neuron.rel.clone(),
                field,
                problem,
            })
        };
        let yaml = crate::frontmatter::has_yaml_suffix(&neuron.path);
        if !yaml {
            for field in ["parent", "created"] {
                if !meta.contains_key(field) {
                    issue(field, FieldProblem::Missing);
                }
            }
        }
        if !meta.contains_key("updated") {
            issue("updated", FieldProblem::Missing);
        }
        let wrong = |field: &str| match meta.get(field) {
            None => false,
            Some(Value::Seq(items)) => {
                field == "related" && items.iter().any(|v| v.as_str().is_none())
            }
            Some(_) => true,
        };
        for field in ["related", "tags"] {
            if wrong(field) {
                issue(field, FieldProblem::Type);
            }
        }
        // Top-level dates read as their ISO text in v2, so they are strings.
        let replaced_by_ok = |v: &Value| {
            matches!(
                v,
                Value::Null | Value::Str(_) | Value::Date(_) | Value::DateTime(_)
            )
        };
        if meta.get("replaced_by").is_some_and(|v| !replaced_by_ok(v)) {
            issue("replaced_by", FieldProblem::Type);
        }
    }
    issues
}

/// The validators dream reports, run on the brain as written.
pub(crate) fn validate<S: BrainStore>(
    store: &S,
    skipped: &mut BTreeMap<String, String>,
) -> Validation {
    let mut read = Reader { store, skipped };
    let broken = broken_synapses(store, &mut read);
    let one_way = one_way_synapses(store, &mut read);
    let frontmatter = frontmatter_issues(store, &mut read);
    let orphans = orphans(store, read.skipped);
    Validation {
        broken,
        one_way,
        orphans,
        frontmatter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relpath_is_relative_to_the_target_directory() {
        assert_eq!(relpath("lobe/s3.md", "lobe/s1.md"), "s3.md");
        assert_eq!(relpath("lobe/s1.md", "other/t.md"), "../lobe/s1.md");
        assert_eq!(relpath("a/b/c.md", "x.md"), "a/b/c.md");
        assert_eq!(relpath("root.md", "a/b/t.md"), "../../root.md");
    }

    #[test]
    fn python_suffix() {
        assert_eq!(py_suffix("a.md"), ".md");
        assert_eq!(py_suffix(".md"), "");
        assert_eq!(py_suffix("a."), "");
        assert_eq!(py_suffix("a.b.MD"), ".MD");
    }
}
