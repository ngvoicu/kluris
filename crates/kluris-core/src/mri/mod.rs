//! `kluris mri`: the brain graph and the standalone HTML page (v2's
//! `core/mri.py`, research-03 §2.8).
//!
//! The graph is v2's `build_graph` over the shared walker:
//! - nodes: every walked `.md` file (maps, `brain.md`, `glossary.md` and
//!   `index.md` too) but `README.md`, then the opted-in YAML neurons, in path
//!   order (PAR-008); a node's id is its index. The walker prunes dot
//!   directories, `node_modules`, `__pycache__` and symlinks that leave the
//!   brain, which v2's MRI walk graphs (PAR-042);
//! - edges: `parent`, each `related` and `replaced_by` whose target resolves
//!   to a node, then inline `[text](target)` links (not `http...`, anchor and
//!   query cut, no self-link) for a pair that has no edge yet.
//!
//! The page is v2's shell, CSS and JS byte for byte
//! (`assets/mri/`, taken from v2.28.11 by `parity/tools/mri_oracle.py`), with
//! the brain name, the header counts and `json.dumps(graph, indent=2)`
//! injected. Differences: the name is the registered brain name, not the
//! directory name (PAR-044), and `</` and `<!--` in the graph JSON are
//! escaped so neuron text cannot end the script (PAR-043).

mod text;
mod uri;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use regex::Regex;
use std::sync::LazyLock;

use crate::brain::neuron_index::{Neuron, NeuronKind};
use crate::brain::store::BrainStore;
use crate::frontmatter::{self, Frontmatter};
use crate::json::{self, Json};
use crate::pyrepr::py_str;
use crate::search::items::{normalized_str, stem_title, universal_newlines};
use crate::yaml::{Map, Value, date_iso, datetime_iso, is_py_space};

pub use text::{content_preview, excerpt};
pub use uri::{file_uri, is_wsl, page_uri, windows_path_if_wsl, wsl_file_uri};

const SHELL: &str = include_str!("../../assets/mri/shell.html");
const CSS: &str = include_str!("../../assets/mri/mri.css");
const JS: &str = include_str!("../../assets/mri/mri.js");

/// The MRI's `SKIP_FILES`: never nodes, at any depth.
const SKIP_FILES: [&str; 3] = [".gitignore", "README.md", "kluris.yml"];

/// v2's `LINK_PATTERN` (`core/linker.py`).
static LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]]*)\]\(([^)]+)\)").unwrap_or_else(|e| panic!("LINK is valid: {e}"))
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Brain,
    Index,
    Glossary,
    Map,
    Neuron,
}

impl NodeType {
    fn of(file_name: &str) -> NodeType {
        match file_name {
            "brain.md" => NodeType::Brain,
            "index.md" => NodeType::Index,
            "glossary.md" => NodeType::Glossary,
            "map.md" => NodeType::Map,
            _ => NodeType::Neuron,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NodeType::Brain => "brain",
            NodeType::Index => "index",
            NodeType::Glossary => "glossary",
            NodeType::Map => "map",
            NodeType::Neuron => "neuron",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    Parent,
    Related,
    Inline,
    ReplacedBy,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Parent => "parent",
            EdgeKind::Related => "related",
            EdgeKind::Inline => "inline",
            EdgeKind::ReplacedBy => "replaced_by",
        }
    }
}

/// One file of the graph, with v2's fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: usize,
    pub label: String,
    /// Brain-relative, `/`-separated.
    pub path: String,
    /// The top-level directory, or `root`.
    pub lobe: String,
    /// The directory path when it is deeper than the lobe, else the lobe.
    pub sublobe: String,
    pub node_type: NodeType,
    /// `"markdown"` or `"yaml"`.
    pub file_type: &'static str,
    pub file_name: String,
    pub title: String,
    pub authored_title: String,
    pub excerpt: String,
    pub content_preview: String,
    pub content_preview_truncated: bool,
    pub content_full: String,
    /// The frontmatter `tags` when it is a list.
    pub tags: Vec<Value>,
    /// The frontmatter values as Python's `str()` gives them (a date as its
    /// ISO text), `""` when absent.
    pub created: String,
    pub updated: String,
    pub parent: String,
    /// The frontmatter `related` when it is a list.
    pub related: Vec<Value>,
    pub status: String,
    pub replaced_by: String,
    /// Distinct neighbours, both directions.
    pub degree: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub source: usize,
    pub target: usize,
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeCounts {
    pub parent: usize,
    pub related: usize,
    pub inline: usize,
    pub replaced_by: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub edge_counts: EdgeCounts,
}

/// What `kluris mri` reports per brain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MriStats {
    pub nodes: usize,
    pub edges: usize,
}

/// The files that become nodes: markdown first, then opted-in YAML, each in
/// the walk's order.
fn node_files<S: BrainStore>(store: &S) -> Vec<Neuron> {
    let mut markdown = Vec::new();
    let mut yaml = Vec::new();
    for file in store.files() {
        let name = file.rel.rsplit('/').next().unwrap_or(&file.rel);
        if SKIP_FILES.contains(&name) {
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

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// The text v2's `read_frontmatter` hands over as the body: python-frontmatter's
/// (line breaks read as `\n`, stripped) for markdown, the rest of the file as
/// is for a YAML neuron.
fn content(fm: &Frontmatter, kind: NeuronKind) -> String {
    match kind {
        NeuronKind::Markdown => py_strip(&universal_newlines(&fm.body)).to_string(),
        NeuronKind::Yaml => fm.body.clone(),
    }
}

/// `str(meta.get(key, ""))` after v2's date normalization.
fn field(meta: &Map, key: &str) -> String {
    meta.get(key).map(py_str).unwrap_or_default()
}

fn list(meta: &Map, key: &str) -> Vec<Value> {
    match meta.get(key) {
        Some(Value::Seq(items)) => items.clone(),
        _ => Vec::new(),
    }
}

fn make_node(id: usize, file: &Neuron, read: Option<&Frontmatter>) -> Node {
    let parts: Vec<&str> = file.rel.split('/').collect();
    let file_name = parts.last().copied().unwrap_or_default().to_string();
    let lobe = if parts.len() > 1 { parts[0] } else { "root" }.to_string();
    let sublobe = if parts.len() > 2 {
        parts[..parts.len() - 1].join("/")
    } else {
        lobe.clone()
    };
    let node_type = NodeType::of(&file_name);
    let empty = Map::new();
    let meta = read.map_or(&empty, |fm| &fm.meta);
    let body = read.map(|fm| content(fm, file.kind)).unwrap_or_default();

    let display = stem_title(&file_name);
    let (authored, excerpt) = text::excerpt(&file_name, &body);
    let mut authored = if authored == display {
        String::new()
    } else {
        authored
    };
    let mut title = display;
    if node_type != NodeType::Neuron && !authored.is_empty() {
        title = std::mem::take(&mut authored);
    }
    if file.kind == NeuronKind::Yaml
        && let Some(fm_title) = meta.get("title").and_then(normalized_str)
        && !py_strip(&fm_title).is_empty()
    {
        title = py_strip(&fm_title).to_string();
        authored.clear();
    }
    let (content_full, content_preview, truncated) = text::content_preview(&body);
    let authored_title = if !authored.is_empty() && authored != title {
        authored
    } else {
        String::new()
    };
    Node {
        id,
        label: title.clone(),
        path: file.rel.clone(),
        lobe,
        sublobe,
        node_type,
        file_type: file.kind.as_str(),
        file_name,
        title,
        authored_title,
        excerpt,
        content_preview,
        content_preview_truncated: truncated,
        content_full,
        tags: list(meta, "tags"),
        created: field(meta, "created"),
        updated: field(meta, "updated"),
        parent: field(meta, "parent"),
        related: list(meta, "related"),
        status: field(meta, "status"),
        replaced_by: field(meta, "replaced_by"),
        degree: 0,
    }
}

/// A link target as v2 resolves it (`(f.parent / target).resolve()`
/// relative to the resolved brain), when it is a node. A target that is not
/// a string makes v2's `pathlib` raise; it has no edge here.
fn target_node<S: BrainStore>(
    store: &S,
    ids: &HashMap<&str, usize>,
    from: &str,
    target: &str,
) -> Option<usize> {
    let rel = store.resolve_link(from, target)?;
    ids.get(rel.as_str()).copied()
}

/// The edges so far, and their `(source, target)` pairs.
#[derive(Default)]
struct Edges {
    list: Vec<Edge>,
    pairs: HashSet<(usize, usize)>,
}

impl Edges {
    fn push(&mut self, source: usize, target: usize, kind: EdgeKind) {
        self.pairs.insert((source, target));
        self.list.push(Edge {
            source,
            target,
            kind,
        });
    }
}

/// v2's `build_graph(brain_path)` over the store.
pub fn build_graph<S: BrainStore>(store: &S) -> Graph {
    let files = node_files(store);
    let reads: Vec<Option<Frontmatter>> = files
        .iter()
        .map(|file| store.read_neuron(file).ok())
        .collect();
    let nodes: Vec<Node> = files
        .iter()
        .zip(&reads)
        .enumerate()
        .map(|(id, (file, read))| make_node(id, file, read.as_ref()))
        .collect();
    let ids: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.rel.as_str(), i))
        .collect();

    let mut edges = Edges::default();
    for (source, (file, read)) in files.iter().zip(&reads).enumerate() {
        let Some(fm) = read else { continue };
        let from = file.rel.as_str();
        let single = |key: &str| match fm.meta.get(key) {
            Some(Value::Str(s)) if !s.is_empty() => target_node(store, &ids, from, s),
            _ => None,
        };
        if let Some(target) = single("parent") {
            edges.push(source, target, EdgeKind::Parent);
        }
        if let Some(Value::Seq(related)) = fm.meta.get("related") {
            for item in related {
                if let Some(target) = item
                    .as_str()
                    .and_then(|r| target_node(store, &ids, from, r))
                {
                    edges.push(source, target, EdgeKind::Related);
                }
            }
        }
        if let Some(target) = single("replaced_by") {
            edges.push(source, target, EdgeKind::ReplacedBy);
        }
        let body = content(fm, file.kind);
        for captures in LINK.captures_iter(&body) {
            let Some(target) = captures.get(2).map(|m| m.as_str()) else {
                continue;
            };
            if target.starts_with("http") {
                continue;
            }
            let target = target.split('#').next().unwrap_or(target);
            let target = target.split('?').next().unwrap_or(target);
            if target.is_empty() {
                continue;
            }
            if let Some(t) = target_node(store, &ids, from, target)
                && t != source
                && !edges.pairs.contains(&(source, t))
            {
                edges.push(source, t, EdgeKind::Inline);
            }
        }
    }

    let mut neighbours: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); nodes.len()];
    let mut edge_counts = EdgeCounts::default();
    let edges = edges.list;
    for edge in &edges {
        neighbours[edge.source].insert(edge.target);
        neighbours[edge.target].insert(edge.source);
        match edge.kind {
            EdgeKind::Parent => edge_counts.parent += 1,
            EdgeKind::Related => edge_counts.related += 1,
            EdgeKind::Inline => edge_counts.inline += 1,
            EdgeKind::ReplacedBy => edge_counts.replaced_by += 1,
        }
    }
    let nodes = nodes
        .into_iter()
        .zip(&neighbours)
        .map(|(node, n)| Node {
            degree: n.len(),
            ..node
        })
        .collect();
    Graph {
        nodes,
        edges,
        edge_counts,
    }
}

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn string(text: &str) -> Json {
    Json::Str(text.to_string())
}

/// `json.dumps` of a frontmatter value. A date or datetime, which v2 can only
/// meet nested in `tags` or `related` (where its `json.dumps` raises), is
/// written as its ISO text.
fn value_json(value: &Value) -> Json {
    enum Task<'a> {
        Value(&'a Value),
        Array(usize),
        Object(Vec<&'a str>),
    }
    let mut tasks = vec![Task::Value(value)];
    let mut done: Vec<Json> = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            Task::Value(value) => match value {
                Value::Null => done.push(Json::Null),
                Value::Bool(b) => done.push(Json::Bool(*b)),
                Value::Int(n) => done.push(
                    i64::try_from(n)
                        .map(Json::Int)
                        .unwrap_or_else(|_| Json::BigInt(n.to_string())),
                ),
                Value::Float(x) => done.push(Json::Float(*x)),
                Value::Str(s) => done.push(string(s)),
                Value::Date(d) => done.push(Json::Str(date_iso(d))),
                Value::DateTime(dt) => done.push(Json::Str(datetime_iso(dt, 'T'))),
                Value::Seq(items) => {
                    tasks.push(Task::Array(items.len()));
                    tasks.extend(items.iter().rev().map(Task::Value));
                }
                Value::Map(map) => {
                    tasks.push(Task::Object(map.keys().map(String::as_str).collect()));
                    tasks.extend(map.values().rev().map(Task::Value));
                }
            },
            Task::Array(n) => {
                let items = done.split_off(done.len() - n);
                done.push(Json::Array(items));
            }
            Task::Object(keys) => {
                let values = done.split_off(done.len() - keys.len());
                done.push(Json::Object(
                    keys.into_iter().map(str::to_string).zip(values).collect(),
                ));
            }
        }
    }
    done.pop().unwrap_or(Json::Null)
}

fn node_json(n: &Node) -> Json {
    Json::Object(vec![
        ("id".into(), count(n.id)),
        ("label".into(), string(&n.label)),
        ("path".into(), string(&n.path)),
        ("lobe".into(), string(&n.lobe)),
        ("sublobe".into(), string(&n.sublobe)),
        ("type".into(), string(n.node_type.as_str())),
        ("file_type".into(), string(n.file_type)),
        ("file_name".into(), string(&n.file_name)),
        ("title".into(), string(&n.title)),
        ("authored_title".into(), string(&n.authored_title)),
        ("excerpt".into(), string(&n.excerpt)),
        ("content_preview".into(), string(&n.content_preview)),
        (
            "content_preview_truncated".into(),
            Json::Bool(n.content_preview_truncated),
        ),
        ("content_full".into(), string(&n.content_full)),
        (
            "tags".into(),
            Json::Array(n.tags.iter().map(value_json).collect()),
        ),
        ("created".into(), string(&n.created)),
        ("updated".into(), string(&n.updated)),
        ("parent".into(), string(&n.parent)),
        (
            "related".into(),
            Json::Array(n.related.iter().map(value_json).collect()),
        ),
        ("status".into(), string(&n.status)),
        ("replaced_by".into(), string(&n.replaced_by)),
        ("degree".into(), count(n.degree)),
    ])
}

/// The graph as v2's dict: `{nodes, edges, meta: {edge_counts}}`.
pub fn graph_json(graph: &Graph) -> Json {
    let edges = graph
        .edges
        .iter()
        .map(|e| {
            Json::Object(vec![
                ("source".into(), count(e.source)),
                ("target".into(), count(e.target)),
                ("type".into(), string(e.kind.as_str())),
            ])
        })
        .collect();
    let c = graph.edge_counts;
    Json::Object(vec![
        (
            "nodes".into(),
            Json::Array(graph.nodes.iter().map(node_json).collect()),
        ),
        ("edges".into(), Json::Array(edges)),
        (
            "meta".into(),
            Json::Object(vec![(
                "edge_counts".into(),
                Json::Object(vec![
                    ("parent".into(), count(c.parent)),
                    ("related".into(), count(c.related)),
                    ("inline".into(), count(c.inline)),
                    ("replaced_by".into(), count(c.replaced_by)),
                ]),
            )]),
        ),
    ])
}

/// JSON text that is safe inside `<script>`: `</` cannot close the element
/// and `<!--` cannot start the escaped state that would keep the real
/// `</script>` from closing it. Both rewrites are escapes inside JSON
/// strings, the only place `<` can occur.
fn script_safe(json_text: &str) -> String {
    json_text
        .replace("</", "<\\/")
        .replace("<!--", "\\u003c!--")
}

/// v2's `_render_mri_html` for the brain called `brain_name`.
pub fn render_html(brain_name: &str, graph: &Graph) -> String {
    let lobes: HashSet<&str> = graph
        .nodes
        .iter()
        .map(|n| n.lobe.as_str())
        .filter(|l| !l.is_empty() && *l != "root")
        .collect();
    let neurons = graph
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Neuron)
        .count();
    let graph_text = script_safe(&json::to_string_indented(&graph_json(graph), 2));
    let name_json = script_safe(&json::to_string(&string(brain_name)));
    let lobe_count = lobes.len().to_string();
    let neuron_count = neurons.to_string();
    let edge_count = graph.edges.len().to_string();
    let mut out = String::with_capacity(SHELL.len() + CSS.len() + JS.len() + graph_text.len());
    let mut rest = SHELL;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let value = match &after[..end] {
            "brain_name" => brain_name,
            "lobe_count" => &lobe_count,
            "neuron_count" => &neuron_count,
            "edge_count" => &edge_count,
            "css" => CSS,
            "graph_json" => &graph_text,
            "brain_name_json" => &name_json,
            "js" => JS,
            other => {
                out.push_str("{{");
                out.push_str(other);
                out.push_str("}}");
                rest = &after[end + 2..];
                continue;
            }
        };
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Build the graph of the brain in `store` and write its page to `output`,
/// creating the missing parent directories. The page is a generated
/// artifact (gitignored in the brain, or anywhere `--output` puts it), not
/// brain content, so it is written directly rather than as a change set.
pub fn generate<S: BrainStore>(
    store: &S,
    brain_name: &str,
    output: &Path,
) -> std::io::Result<MriStats> {
    let graph = build_graph(store);
    let html = render_html(brain_name, &graph);
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, html)?;
    Ok(MriStats {
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
    })
}
