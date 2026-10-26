//! `kluris mri` in the core (`kluris_core::mri`): v2's `build_graph`, its
//! title/excerpt/preview rules and `_render_mri_html` (research-03 §2.8) over
//! the shared walker, with the decided differences (pruned directories and
//! escaping symlinks are not graphed, `</` cannot end the script, the page is
//! named after the registered brain). `parity/test_mri.py` checks the same
//! rules against the live oracle.

use std::fs;
use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::dream::{DreamOptions, sync_brain_state};
use kluris_core::git::SystemGit;
use kluris_core::json::{self, Json};
use kluris_core::mri::{
    EdgeKind, Graph, NodeType, build_graph, content_preview, excerpt, file_uri, generate,
    graph_json, is_wsl, render_html, wsl_file_uri,
};
use kluris_core::yaml::{Date, Value};
use tempfile::TempDir;

const SHELL: &str = include_str!("../assets/mri/shell.html");
const CSS: &str = include_str!("../assets/mri/mri.css");
const JS: &str = include_str!("../assets/mri/mri.js");

fn brain(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (rel, text) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

fn graph_of(dir: &TempDir) -> Graph {
    build_graph(&LocalGitStore::open(dir.path()))
}

fn node<'a>(graph: &'a Graph, path: &str) -> &'a kluris_core::mri::Node {
    graph
        .nodes
        .iter()
        .find(|n| n.path == path)
        .unwrap_or_else(|| panic!("no node {path}"))
}

/// `(source path, target path, kind)` of every edge, in order.
fn edges(graph: &Graph) -> Vec<(String, String, EdgeKind)> {
    graph
        .edges
        .iter()
        .map(|e| {
            (
                graph.nodes[e.source].path.clone(),
                graph.nodes[e.target].path.clone(),
                e.kind,
            )
        })
        .collect()
}

fn e(source: &str, target: &str, kind: EdgeKind) -> (String, String, EdgeKind) {
    (source.to_string(), target.to_string(), kind)
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

// --- which files are nodes ---------------------------------------------------

#[test]
fn every_markdown_file_but_readme_is_a_node_then_opted_in_yaml() {
    let dir = brain(&[
        ("kluris.yml", "name: b\n"),
        (".gitignore", "x\n"),
        ("README.md", "# Readme\n"),
        ("brain.md", "# b\n"),
        ("glossary.md", "# Glossary\n"),
        ("index.md", "# Index\n"),
        ("projects/map.md", "# Projects\n"),
        ("projects/README.md", "# Not a node\n"),
        ("projects/b.md", "# B\n"),
        ("projects/a.md", "# A\n"),
        ("projects/Upper.MD", "# not .md\n"),
        ("projects/spec.yml", "#---\n# title: Spec\n#---\nx: 1\n"),
        ("projects/raw.yaml", "x: 1\n"),
        ("projects/page.html", "<p>\n"),
    ]);
    let graph = graph_of(&dir);
    let paths: Vec<&str> = graph.nodes.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "brain.md",
            "glossary.md",
            "index.md",
            "projects/a.md",
            "projects/b.md",
            "projects/map.md",
            "projects/spec.yml",
        ]
    );
    for (i, n) in graph.nodes.iter().enumerate() {
        assert_eq!(n.id, i);
    }
    let types: Vec<NodeType> = graph.nodes.iter().map(|n| n.node_type).collect();
    assert_eq!(
        types,
        [
            NodeType::Brain,
            NodeType::Glossary,
            NodeType::Index,
            NodeType::Neuron,
            NodeType::Neuron,
            NodeType::Map,
            NodeType::Neuron,
        ]
    );
}

#[test]
fn pruned_directories_and_escaping_symlinks_are_not_graphed() {
    // The escaping symlink is Unix's: elsewhere there is no outside brain.
    #[cfg(unix)]
    let outside = brain(&[("secret.md", "# Secret\n")]);
    let dir = brain(&[
        ("projects/a.md", "# A\n"),
        (".archive/old.md", "# Old\n"),
        ("projects/.hidden/x.md", "# X\n"),
        ("node_modules/pkg/index.md", "# pkg\n"),
        ("projects/__pycache__/c.md", "# c\n"),
    ]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        dir.path().join("projects/leak.md"),
    )
    .unwrap();
    let graph = graph_of(&dir);
    let paths: Vec<&str> = graph.nodes.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(paths, ["projects/a.md"]);
}

// --- node fields ----------------------------------------------------------------

#[test]
fn neuron_fields_follow_v2() {
    let dir = brain(&[(
        "projects/auth-flow.md",
        "---\ncreated: 2026-01-02\nupdated: '2026-02-03'\nparent: ./map.md\ntags: [auth, 2026-03-04, 7]\nrelated:\n- ./other.md\nstatus: deprecated\nreplaced_by: ./new.md\nextra: null\n---\n\n# OAuth & sessions\n\n## Why\n\n- a list item\nFirst real line.\n\nMore.\n",
    )]);
    let graph = graph_of(&dir);
    let n = node(&graph, "projects/auth-flow.md");
    assert_eq!(n.label, "Auth Flow");
    assert_eq!(n.title, "Auth Flow");
    assert_eq!(n.authored_title, "OAuth & sessions");
    assert_eq!(n.lobe, "projects");
    assert_eq!(n.sublobe, "projects");
    assert_eq!(n.node_type, NodeType::Neuron);
    assert_eq!(n.file_type, "markdown");
    assert_eq!(n.file_name, "auth-flow.md");
    assert_eq!(n.excerpt, "First real line.");
    assert_eq!(
        n.content_full,
        "## Why\n\n- a list item\nFirst real line.\n\nMore."
    );
    assert_eq!(n.content_preview, n.content_full);
    assert!(!n.content_preview_truncated);
    // A nested date is written as its ISO text (v2's json.dumps raises).
    assert_eq!(
        n.tags,
        vec![
            s("auth"),
            Value::Date(Date::new(2026, 3, 4).unwrap()),
            Value::Int(7.into())
        ]
    );
    assert_eq!(n.created, "2026-01-02");
    assert_eq!(n.updated, "2026-02-03");
    assert_eq!(n.parent, "./map.md");
    assert_eq!(n.related, vec![s("./other.md")]);
    assert_eq!(n.status, "deprecated");
    assert_eq!(n.replaced_by, "./new.md");
}

#[test]
fn missing_fields_are_empty_and_non_strings_are_python_str() {
    let dir = brain(&[
        ("root.md", "Just text.\n"),
        (
            "a/b/c/deep.md",
            "---\nparent: [x, 1]\nupdated: null\ntags: nope\nrelated: ./x.md\nstatus: 3\n---\nBody\n",
        ),
    ]);
    let graph = graph_of(&dir);
    let root = node(&graph, "root.md");
    assert_eq!(
        (root.lobe.as_str(), root.sublobe.as_str()),
        ("root", "root")
    );
    assert_eq!(root.title, "Root");
    assert_eq!(root.authored_title, "");
    assert_eq!(root.excerpt, "Just text.");
    assert_eq!((root.created.as_str(), root.parent.as_str()), ("", ""));
    let deep = node(&graph, "a/b/c/deep.md");
    assert_eq!((deep.lobe.as_str(), deep.sublobe.as_str()), ("a", "a/b/c"));
    assert_eq!(deep.parent, "['x', 1]");
    assert_eq!(deep.updated, "None");
    assert_eq!(deep.status, "3");
    assert!(deep.tags.is_empty());
    assert!(deep.related.is_empty());
}

#[test]
fn structure_files_take_their_heading_and_yaml_neurons_their_title() {
    let dir = brain(&[
        (
            "projects/map.md",
            "---\nauto_generated: true\n---\n\n# Projects\n\nup [brain.md](../brain.md)\n",
        ),
        ("glossary.md", "# Glossary\n\n**JWT** -- token.\n"),
        ("brain.md", "No heading here.\n"),
        (
            "projects/openapi.yml",
            "#---\n# title: '  Payments API  '\n# updated: 2026-01-01\n#---\nopenapi: 3.0.0\n",
        ),
        ("projects/notitle.yaml", "#---\n# title: ''\n#---\nk: v\n"),
        ("projects/same-name.md", "# Same Name\n\nText.\n"),
    ]);
    let graph = graph_of(&dir);
    let map = node(&graph, "projects/map.md");
    assert_eq!(
        (map.title.as_str(), map.authored_title.as_str()),
        ("Projects", "")
    );
    assert_eq!(map.excerpt, "");
    assert_eq!(node(&graph, "glossary.md").title, "Glossary");
    assert_eq!(node(&graph, "glossary.md").excerpt, "**JWT** -- token.");
    // No H1: the stem title.
    assert_eq!(node(&graph, "brain.md").title, "Brain");
    let api = node(&graph, "projects/openapi.yml");
    assert_eq!(
        (api.title.as_str(), api.file_type),
        ("Payments API", "yaml")
    );
    assert_eq!(api.updated, "2026-01-01");
    assert_eq!(api.content_full, "openapi: 3.0.0");
    assert_eq!(node(&graph, "projects/notitle.yaml").title, "Notitle");
    // An H1 equal to the stem title is not repeated as the authored title.
    assert_eq!(node(&graph, "projects/same-name.md").authored_title, "");
}

#[test]
fn a_file_that_does_not_read_is_a_node_without_fields_or_edges() {
    let dir = brain(&[
        ("projects/a.md", "---\nparent: ./b.md\n---\n# A\n"),
        (
            "projects/b.md",
            "---\nparent: [unclosed\n---\n# B\n\n[a](./a.md)\n",
        ),
    ]);
    let graph = graph_of(&dir);
    let b = node(&graph, "projects/b.md");
    assert_eq!(
        (
            b.title.as_str(),
            b.excerpt.as_str(),
            b.content_full.as_str()
        ),
        ("B", "", "")
    );
    assert_eq!(b.parent, "");
    assert_eq!(
        edges(&graph),
        [e("projects/a.md", "projects/b.md", EdgeKind::Parent)]
    );
}

// --- excerpt and preview ----------------------------------------------------------

#[test]
fn excerpt_rules() {
    let cases: &[(&str, (&str, &str))] = &[
        (
            "# T\n\n## Sub\n- item\n* star\n```\n---\nup x\nsideways y\nReal.\n",
            ("T", "Real."),
        ),
        (
            "  \n<a id=\"x\"></a># Title\nLine<a name='y'></a> two\n",
            ("Title", "Line two"),
        ),
        ("Before\n# Late title\n", ("Note", "Before")),
        ("#nospace\n", ("Note", "#nospace")),
        ("", ("Note", "")),
    ];
    for (body, (title, text)) in cases {
        assert_eq!(
            excerpt("note.md", body),
            (title.to_string(), text.to_string()),
            "{body:?}"
        );
    }
    // Capped at 220 characters, not bytes.
    let long = "é".repeat(300);
    let (_, got) = excerpt("x.md", &long);
    assert_eq!(got.chars().count(), 220);
}

#[test]
fn preview_rules() {
    assert_eq!(
        content_preview("  \n "),
        (String::new(), String::new(), false)
    );
    assert_eq!(
        content_preview("# Only a title\n"),
        (String::new(), String::new(), false)
    );
    // The first `# ` line is dropped (wherever it is), blank lines before the
    // body too; id anchors are stripped case-insensitively.
    let (full, preview, truncated) =
        content_preview("Intro\n# Title\n\nText <A ID='x'> </a>here  \n# Second\n");
    assert_eq!(full, "Intro\n\nText here\n# Second");
    assert_eq!((preview.as_str(), truncated), (full.as_str(), false));
    let (full, _, _) = content_preview("<a ıd=\"k\"></a>dotless\n");
    assert_eq!(full, "dotless");
    let (full, _, _) = content_preview("<a name=\"k\"></a>kept\n");
    assert_eq!(full, "<a name=\"k\"></a>kept");

    // 48 lines at most.
    let lines: Vec<String> = (1..=60).map(|i| format!("line {i}")).collect();
    let (full, preview, truncated) = content_preview(&lines.join("\n"));
    assert!(truncated);
    assert_eq!(full, lines.join("\n"));
    assert_eq!(preview, format!("{}\n\n...", lines[..48].join("\n")));

    // 2800 characters at most, cut back to a line end.
    let body = format!("{}\n{}", "é".repeat(2000), "ü".repeat(1000));
    let (_, preview, truncated) = content_preview(&body);
    assert!(truncated);
    assert_eq!(preview, format!("{}\n\n...", "é".repeat(2000)));
    let one_line = "x".repeat(3000);
    let (_, preview, _) = content_preview(&one_line);
    assert_eq!(preview, format!("{}\n\n...", "x".repeat(2800)));
}

// --- edges ----------------------------------------------------------------------

#[test]
fn edges_follow_v2() {
    let dir = brain(&[
        ("projects/map.md", "# Projects\n"),
        (
            "projects/a.md",
            "---\nparent: ./map.md\nrelated:\n- ./b.md\n- ./b.md\n- ../outside/none.md\n- 5\n- ../../escape.md\nreplaced_by: ./b.md\n---\n# A\n\n[b](./b.md) [map](map.md#top) [self](./a.md) [c](./c.md?x=1) [web](https://x.io) [anchor](#here) [gone](./gone.md) [c again](c.md)\n",
        ),
        (
            "projects/b.md",
            "---\nparent: 7\nrelated: [./a.md]\n---\n# B\n",
        ),
        ("projects/c.md", "# C\n\n[glossary](../glossary.md#jwt)\n"),
        ("glossary.md", "# Glossary\n"),
    ]);
    let graph = graph_of(&dir);
    assert_eq!(
        edges(&graph),
        [
            e("projects/a.md", "projects/map.md", EdgeKind::Parent),
            e("projects/a.md", "projects/b.md", EdgeKind::Related),
            e("projects/a.md", "projects/b.md", EdgeKind::Related),
            e("projects/a.md", "projects/b.md", EdgeKind::ReplacedBy),
            e("projects/a.md", "projects/c.md", EdgeKind::Inline),
            e("projects/b.md", "projects/a.md", EdgeKind::Related),
            e("projects/c.md", "glossary.md", EdgeKind::Inline),
        ]
    );
    let counts = graph.edge_counts;
    assert_eq!(
        (
            counts.parent,
            counts.related,
            counts.inline,
            counts.replaced_by
        ),
        (1, 3, 2, 1)
    );
    // Distinct neighbours, both directions.
    assert_eq!(node(&graph, "projects/a.md").degree, 3);
    assert_eq!(node(&graph, "projects/b.md").degree, 1);
    assert_eq!(node(&graph, "projects/map.md").degree, 1);
    assert_eq!(node(&graph, "glossary.md").degree, 1);
}

#[test]
fn an_absolute_link_inside_the_brain_is_an_edge() {
    let dir = brain(&[("projects/b.md", "# B\n")]);
    let target = dir.path().join("projects/b.md");
    let text = format!(
        "---\nrelated:\n- {}\n---\n# A\n",
        target.to_string_lossy().replace('\\', "/")
    );
    fs::write(dir.path().join("projects/a.md"), text).unwrap();
    let graph = graph_of(&dir);
    assert_eq!(
        edges(&graph),
        [e("projects/a.md", "projects/b.md", EdgeKind::Related)]
    );
}

// --- JSON and HTML ---------------------------------------------------------------

#[test]
fn graph_json_has_v2s_shape() {
    let dir = brain(&[
        (
            "projects/a.md",
            "---\nparent: ./b.md\ntags: [x]\n---\n# A\n\nÉcole.\n",
        ),
        ("projects/b.md", "# B\n"),
    ]);
    let doc = json::to_string_indented(&graph_json(&graph_of(&dir)), 2);
    let expected = r#"{
  "nodes": [
    {
      "id": 0,
      "label": "A",
      "path": "projects/a.md",
      "lobe": "projects",
      "sublobe": "projects",
      "type": "neuron",
      "file_type": "markdown",
      "file_name": "a.md",
      "title": "A",
      "authored_title": "",
      "excerpt": "\u00c9cole.",
      "content_preview": "\u00c9cole.",
      "content_preview_truncated": false,
      "content_full": "\u00c9cole.",
      "tags": [
        "x"
      ],
      "created": "",
      "updated": "",
      "parent": "./b.md",
      "related": [],
      "status": "",
      "replaced_by": "",
      "degree": 1
    },
    {
      "id": 1,
      "label": "B",
      "path": "projects/b.md",
      "lobe": "projects",
      "sublobe": "projects",
      "type": "neuron",
      "file_type": "markdown",
      "file_name": "b.md",
      "title": "B",
      "authored_title": "",
      "excerpt": "",
      "content_preview": "",
      "content_preview_truncated": false,
      "content_full": "",
      "tags": [],
      "created": "",
      "updated": "",
      "parent": "",
      "related": [],
      "status": "",
      "replaced_by": "",
      "degree": 1
    }
  ],
  "edges": [
    {
      "source": 0,
      "target": 1,
      "type": "parent"
    }
  ],
  "meta": {
    "edge_counts": {
      "parent": 1,
      "related": 0,
      "inline": 0,
      "replaced_by": 0
    }
  }
}"#;
    assert_eq!(doc, expected);
}

#[test]
fn indented_json_matches_python() {
    // python3 -c 'import json; print(json.dumps({"a": [], "b": {}, "c": [1, [2, {"d": None}]], "e": "\u2028"}, indent=2))'
    let value = Json::Object(vec![
        ("a".into(), Json::Array(vec![])),
        ("b".into(), Json::Object(vec![])),
        (
            "c".into(),
            Json::Array(vec![
                Json::Int(1),
                Json::Array(vec![
                    Json::Int(2),
                    Json::Object(vec![("d".into(), Json::Null)]),
                ]),
            ]),
        ),
        ("e".into(), Json::Str("\u{2028}".into())),
    ]);
    assert_eq!(
        json::to_string_indented(&value, 2),
        "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    [\n      2,\n      {\n        \"d\": null\n      }\n    ]\n  ],\n  \"e\": \"\\u2028\"\n}"
    );
    // Deep nesting needs no deep call stack (indented output grows with the
    // square of the depth, so that one is shallower).
    let nest = |depth: usize| {
        let mut deep = Json::Array(vec![]);
        for _ in 0..depth {
            deep = Json::Array(vec![deep]);
        }
        deep
    };
    let deep = nest(100_000);
    assert!(json::to_string(&deep).starts_with("[[[["));
    std::mem::forget(deep);
    let deep = nest(3_000);
    assert!(json::to_string_indented(&deep, 2).starts_with("[\n  [\n"));
    std::mem::forget(deep);
}

/// The page with the injected graph taken out, and that graph's JSON.
fn split_page(html: &str) -> (String, String) {
    let start = html.find("const graph = ").unwrap() + "const graph = ".len();
    let end = html.find(";\nconst BRAIN_NAME = ").unwrap();
    (
        format!("{}{}", &html[..start], &html[end..]),
        html[start..end].to_string(),
    )
}

#[test]
fn the_page_is_v2s_shell_css_and_js_with_the_data_injected() {
    let dir = brain(&[
        ("projects/a.md", "# A\n"),
        ("projects/sub/b.md", "---\nparent: ../a.md\n---\n# B\n"),
        ("glossary.md", "# Glossary\n"),
        ("projects/map.md", "# Projects\n"),
    ]);
    let graph = graph_of(&dir);
    let html = render_html("my-brain", &graph);
    let expected_shell = SHELL
        .replace("{{brain_name}}", "my-brain")
        .replace("{{lobe_count}}", "1")
        .replace("{{neuron_count}}", "2")
        .replace("{{edge_count}}", "1")
        .replace("{{css}}", CSS)
        .replace("{{brain_name_json}}", "\"my-brain\"")
        .replace("{{js}}", JS);
    let (rest, data) = split_page(&html);
    assert_eq!(rest, expected_shell.replace("{{graph_json}}", ""));
    assert_eq!(data, json::to_string_indented(&graph_json(&graph), 2));
    assert!(html.contains("<title>my-brain Brain MRI</title>"));
}

#[test]
fn neuron_text_cannot_end_the_script() {
    let dir = brain(&[(
        "projects/x.md",
        "# X\n\nA </script><script>alert(1)</script> and <!--<script> here.\n",
    )]);
    let html = render_html("b", &graph_of(&dir));
    assert_eq!(html.matches("</script>").count(), 1);
    assert_eq!(html.matches("<!--").count(), 0);
    let (_, data) = split_page(&html);
    assert!(data.contains(r"A <\/script><script>alert(1)<\/script> and \u003c!--<script> here."));
    // `\/` and `\u003c` are JSON escapes: the data still parses to the text.
    let parsed: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_eq!(
        parsed["nodes"][0]["excerpt"],
        "A </script><script>alert(1)</script> and <!--<script> here."
    );
}

#[test]
fn generate_writes_the_page_and_creates_parent_directories() {
    let dir = brain(&[
        ("projects/a.md", "# A\n"),
        ("projects/b.md", "[a](./a.md)\n"),
    ]);
    let out = dir.path().join("out/deeper/page.html");
    let stats = generate(&LocalGitStore::open(dir.path()), "b", &out).unwrap();
    assert_eq!((stats.nodes, stats.edges), (2, 1));
    let html = fs::read_to_string(&out).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>\n"));
    assert!(!html.contains('\r'));
}

// --- preflight -------------------------------------------------------------------

#[test]
fn preflight_runs_dreams_fixes_without_validating() {
    let dir = brain(&[
        ("kluris.yml", "name: b\n"),
        ("projects/a.md", "---\nrelated: [./b.md]\n---\n# A\n"),
        (
            "projects/b.md",
            "---\nparent: ./map.md\n---\n# B\n\n[gone](./gone.md)\n",
        ),
    ]);
    let mut store = LocalGitStore::open(dir.path());
    let report = sync_brain_state(
        &mut store,
        &SystemGit::new(),
        &DreamOptions {
            name: "b",
            description: "d",
            today: Date::new(2026, 10, 26).unwrap(),
        },
    );
    assert_eq!(report.fixes.parents_inferred, 1);
    assert_eq!(report.fixes.reverse_synapses_added, 1);
    assert_eq!(report.fixes.orphan_references_added, 2);
    assert_eq!(report.fixes.total, 4);
    assert!(dir.path().join("projects/map.md").is_file());
    assert!(dir.path().join("brain.md").is_file());
    assert!(
        fs::read_to_string(dir.path().join("projects/b.md"))
            .unwrap()
            .contains("related:\n- a.md\n")
    );
}

// --- the file:// link ---------------------------------------------------------------

#[test]
fn file_uris_are_python_as_uri() {
    if cfg!(windows) {
        assert_eq!(
            file_uri(Path::new(r"C:\Users\me\my brain\é page.html")),
            "file:///C:/Users/me/my%20brain/%C3%A9%20page.html"
        );
        assert_eq!(
            file_uri(Path::new(r"\\server\share\a b.html")),
            "file://server/share/a%20b.html"
        );
    } else {
        assert_eq!(
            file_uri(Path::new("/home/me/my brain/é page.html")),
            "file:///home/me/my%20brain/%C3%A9%20page.html"
        );
        assert_eq!(
            file_uri(Path::new("/a/b-c_d.e~f/g%h#i?j@k+l,m;n=o&p:q'r")),
            "file:///a/b-c_d.e~f/g%25h%23i%3Fj%40k%2Bl%2Cm%3Bn%3Do%26p%3Aq%27r"
        );
    }
}

#[test]
fn wsl_detection_and_link() {
    assert!(is_wsl(Some("Ubuntu".as_ref()), None));
    assert!(!is_wsl(Some("".as_ref()), None));
    assert!(is_wsl(
        None,
        Some("Linux version 5.15.90.1-Microsoft-standard-WSL2")
    ));
    assert!(!is_wsl(None, Some("Linux version 6.1.0 (debian)")));
    assert!(!is_wsl(None, None));
    assert_eq!(
        wsl_file_uri(r"\\wsl.localhost\Ubuntu\home\me\brain-mri.html"),
        "file://wsl.localhost/Ubuntu/home/me/brain-mri.html"
    );
}
