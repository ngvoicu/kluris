//! The brain model: the one walker, the neuron index, glossary parsing,
//! deprecation detection, the path sandbox and the `BrainStore` read side.
//!
//! Expected values come from the v2.28.11 oracle (`parity/tools/
//! brain_model_oracle.py` on the fixture brains under
//! `tests/fixtures/brains/`, symlinks made from `links.symlinks`), with the
//! oracle's deprecation run in v3's neuron order. `parity/test_brain_model.py`
//! re-checks them against the live oracle.

use std::fs;
use std::path::{Path, PathBuf};

use kluris_core::brain::deprecation::{DeprecationIssue, detect_deprecation_issues};
use kluris_core::brain::glossary::{GlossaryEntry, parse_glossary_entries};
use kluris_core::brain::neuron_index::{NeuronKind, neuron_files};
use kluris_core::brain::sandbox::{self, SandboxError};
use kluris_core::brain::store::{BrainStore, LocalGitStore, ReadError};
use kluris_core::brain::walk::walk;
use kluris_core::frontmatter::Problem;
use kluris_core::yaml::Value;
use tempfile::TempDir;

use NeuronKind::{Markdown as Md, Yaml};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/brains")
}

/// Copy a directory tree, iteratively.
fn copy_tree(src: &Path, dst: &Path) {
    let mut stack = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        fs::create_dir_all(&to).unwrap();
        for entry in fs::read_dir(&from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                stack.push((entry.path(), target));
            } else {
                fs::copy(entry.path(), target).unwrap();
            }
        }
    }
}

/// A fixture brain copied into a fresh temp dir (canonical path, as v2's
/// `create`/`register` store it), with `outside/` and `outside-the-brain.md`
/// beside it.
struct Brain {
    _tmp: TempDir,
    root: PathBuf,
}

fn brain(name: &str) -> Brain {
    let tmp = TempDir::new().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let base = strip_verbatim(base);
    let root = base.join(name);
    copy_tree(&fixtures().join(name), &root);
    copy_tree(&fixtures().join("_outside"), &base.join("outside"));
    fs::write(base.join("outside-the-brain.md"), "# outside\n").unwrap();
    Brain { _tmp: tmp, root }
}

#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(not(windows))]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    path
}

#[cfg(unix)]
fn symlink(target: &str, link: &Path, _dir: bool) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &str, link: &Path, dir: bool) -> std::io::Result<()> {
    let target = target.replace('/', "\\");
    if dir {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// The `links` brain with the symlinks of `links.symlinks`, or None (with a
/// note) where this machine cannot make symlinks.
fn links_brain() -> Option<Brain> {
    let b = brain("links");
    let manifest = fs::read_to_string(fixtures().join("links.symlinks")).unwrap();
    for line in manifest.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if line.starts_with('#') || fields.len() != 3 {
            continue;
        }
        let link = b.root.join(fields[1]);
        if let Err(e) = symlink(fields[2], &link, fields[0] == "dir") {
            eprintln!("note: cannot create symlinks here ({e}); skipping the symlink cases");
            return None;
        }
    }
    Some(b)
}

/// A brain-relative POSIX path in native form, as v2's `str(Path)` prints it.
fn native(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

fn rels(brain: &Path) -> Vec<String> {
    walk(brain).into_iter().map(|f| f.rel).collect()
}

fn neurons(brain: &Path) -> Vec<(String, NeuronKind)> {
    neuron_files(&walk(brain))
        .into_iter()
        .map(|n| (n.rel, n.kind))
        .collect()
}

fn expect_neurons(list: &[(&str, NeuronKind)]) -> Vec<(String, NeuronKind)> {
    list.iter().map(|(p, k)| (p.to_string(), *k)).collect()
}

fn links(source: &str, target: &str) -> DeprecationIssue {
    DeprecationIssue::ActiveLinksToDeprecated {
        source: source.to_string(),
        target: target.to_string(),
    }
}

fn without(file: &str) -> DeprecationIssue {
    DeprecationIssue::DeprecatedWithoutReplacement {
        file: file.to_string(),
    }
}

fn missing(file: &str, target: &str) -> DeprecationIssue {
    DeprecationIssue::ReplacedByMissing {
        file: file.to_string(),
        target: target.to_string(),
    }
}

fn not_active(file: &str, target: &str) -> DeprecationIssue {
    DeprecationIssue::ReplacedByNotActive {
        file: file.to_string(),
        target: target.to_string(),
    }
}

fn entry(term: &str, definition: &str) -> GlossaryEntry {
    GlossaryEntry {
        term: term.to_string(),
        definition: definition.to_string(),
    }
}

// --- the walker --------------------------------------------------------------

#[test]
fn walker_prunes_dot_dirs_node_modules_and_skip_dirs() {
    let b = brain("full");
    // Every file under the brain except those below `.github`, `.obsidian`,
    // `.vscode`, `.idea`, `node_modules`, `__pycache__` and `.archive`. Dot
    // FILES are kept; only directories are pruned. Sorted by path bytes.
    let expected = [
        ".gitignore",
        ".md",
        "README.md",
        "X.MD",
        "brain.md",
        "glossary.md",
        "index.md",
        "kluris.yml",
        "knowledge-extra/extra.md",
        "knowledge.md",
        "knowledge/bad-block.yml",
        "knowledge/bool-status.md",
        "knowledge/date-replacement.md",
        "knowledge/deprecated-linker.md",
        "knowledge/empty-replacement.md",
        "knowledge/good.md",
        "knowledge/legacy-config.yml",
        "knowledge/linker.md",
        "knowledge/list-replacement.md",
        "knowledge/map.md",
        "knowledge/missing.md",
        "knowledge/null-replacement.md",
        "knowledge/null-status-linker.md",
        "knowledge/null-status.md",
        "knowledge/numeric-replacement.md",
        "knowledge/outside.md",
        "knowledge/string-related.md",
        "knowledge/sub-lobe/map.md",
        "knowledge/sub-lobe/nested.md",
        "knowledge/to-bad-block.md",
        "knowledge/to-bool-status.md",
        "knowledge/to-deprecated.md",
        "knowledge/to-dir.md",
        "knowledge/to-glossary.md",
        "knowledge/to-good-dotted.md",
        "knowledge/to-good.md",
        "knowledge/to-map.md",
        "knowledge/to-null-status.md",
        "knowledge/to-root.md",
        "knowledge/to-yaml.md",
        "knowledge/without.md",
        "notes.markdown",
        "projects/alpha.md",
        "projects/api/README.md",
        "projects/api/bom.yaml",
        "projects/api/deep/deeper/deepest.md",
        "projects/api/early-block.yml",
        "projects/api/endpoints.md",
        "projects/api/indented.yaml",
        "projects/api/kluris.yml",
        "projects/api/late-block.yml",
        "projects/api/map.md",
        "projects/api/map.yml",
        "projects/api/non-comment.yml",
        "projects/api/open-only.yml",
        "projects/api/openapi.yml",
        "projects/api/plain.yaml",
        "projects/api/upper.YML",
        "projects/brain.md",
        "projects/glossary.md",
        "projects/index.md",
        "projects/map.md",
        "projects/readme.md",
        "root-note.md",
        "root-spec.yml",
    ];
    assert_eq!(rels(&b.root), expected);
}

#[test]
fn walker_paths_join_the_root_and_the_posix_relative_path() {
    let b = brain("full");
    let files = walk(&b.root);
    let deep = files
        .iter()
        .find(|f| f.rel == "projects/api/deep/deeper/deepest.md")
        .unwrap();
    assert_eq!(
        deep.path,
        b.root.join(native("projects/api/deep/deeper/deepest.md"))
    );
    assert!(!deep.symlink);
}

#[test]
fn walker_order_is_by_path_bytes() {
    let b = brain("full");
    let all = rels(&b.root);
    let mut sorted = all.clone();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(all, sorted);
    // '-' < '.' < '/': not the order of a depth-first walk over sorted names.
    let pos = |p: &str| all.iter().position(|r| r == p).unwrap();
    assert!(pos("knowledge-extra/extra.md") < pos("knowledge.md"));
    assert!(pos("knowledge.md") < pos("knowledge/bad-block.yml"));
}

#[test]
fn walker_root_that_is_not_a_directory_yields_nothing() {
    let b = brain("full");
    assert!(walk(&b.root.join("root-note.md")).is_empty());
    assert!(walk(&b.root.join("no-such-dir")).is_empty());
}

#[test]
fn walker_is_iterative_on_deep_trees() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("deep");
    let mut dir = root.clone();
    for _ in 0..200 {
        dir.push("d");
    }
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("leaf.md"), "# leaf\n").unwrap();
    // A 64 KiB stack is far too small for 200 recursive frames over paths.
    let found = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || neuron_files(&walk(&root)).len())
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(found, 1);
}

#[cfg(unix)]
#[test]
fn walker_skips_an_unreadable_directory() {
    use std::os::unix::fs::PermissionsExt;
    let b = brain("full");
    let locked = b.root.join("projects/api/deep");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let probe = fs::read_dir(&locked).is_ok(); // root reads it anyway
    let names = rels(&b.root);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if !probe {
        assert!(!names.iter().any(|r| r.starts_with("projects/api/deep/")));
        assert!(names.contains(&"projects/api/endpoints.md".to_string()));
    }
}

// --- the neuron index --------------------------------------------------------

#[test]
fn neuron_index_matches_v2_on_the_full_brain() {
    let b = brain("full");
    // v2's neuron_files as a set; v3 lists markdown then YAML neurons (as v2
    // does), each sorted by path bytes where v2 keeps os.walk order.
    let expected = expect_neurons(&[
        (".md", Md),
        ("knowledge-extra/extra.md", Md),
        ("knowledge.md", Md),
        ("knowledge/bool-status.md", Md),
        ("knowledge/date-replacement.md", Md),
        ("knowledge/deprecated-linker.md", Md),
        ("knowledge/empty-replacement.md", Md),
        ("knowledge/good.md", Md),
        ("knowledge/linker.md", Md),
        ("knowledge/list-replacement.md", Md),
        ("knowledge/missing.md", Md),
        ("knowledge/null-replacement.md", Md),
        ("knowledge/null-status-linker.md", Md),
        ("knowledge/null-status.md", Md),
        ("knowledge/numeric-replacement.md", Md),
        ("knowledge/outside.md", Md),
        ("knowledge/string-related.md", Md),
        ("knowledge/sub-lobe/nested.md", Md),
        ("knowledge/to-bad-block.md", Md),
        ("knowledge/to-bool-status.md", Md),
        ("knowledge/to-deprecated.md", Md),
        ("knowledge/to-dir.md", Md),
        ("knowledge/to-glossary.md", Md),
        ("knowledge/to-good-dotted.md", Md),
        ("knowledge/to-good.md", Md),
        ("knowledge/to-map.md", Md),
        ("knowledge/to-null-status.md", Md),
        ("knowledge/to-root.md", Md),
        ("knowledge/to-yaml.md", Md),
        ("knowledge/without.md", Md),
        ("projects/alpha.md", Md),
        ("projects/api/deep/deeper/deepest.md", Md),
        ("projects/api/endpoints.md", Md),
        ("projects/readme.md", Md),
        ("root-note.md", Md),
        ("knowledge/bad-block.yml", Yaml),
        ("knowledge/legacy-config.yml", Yaml),
        ("projects/api/bom.yaml", Yaml),
        ("projects/api/early-block.yml", Yaml),
        ("projects/api/indented.yaml", Yaml),
        ("projects/api/map.yml", Yaml),
        ("projects/api/openapi.yml", Yaml),
        ("root-spec.yml", Yaml),
    ]);
    assert_eq!(neurons(&b.root), expected);
}

#[test]
fn neuron_index_rules_one_by_one() {
    let b = brain("full");
    let list: Vec<String> = neurons(&b.root).into_iter().map(|(p, _)| p).collect();
    let has = |p: &str| list.iter().any(|r| r == p);
    // `.md` is matched case-sensitively, on the name (a file named `.md` counts).
    assert!(has(".md"));
    assert!(!has("X.MD") && !has("notes.markdown"));
    // SKIP_FILES and map.md at any depth; lower-case readme.md is a neuron.
    for skipped in [
        "brain.md",
        "glossary.md",
        "index.md",
        "README.md",
        "projects/brain.md",
        "projects/glossary.md",
        "projects/index.md",
        "projects/map.md",
        "projects/api/README.md",
        "knowledge/sub-lobe/map.md",
    ] {
        assert!(!has(skipped), "{skipped} is not a neuron");
    }
    assert!(has("projects/readme.md"));
    // kluris.yml never, even with a complete block; map.yml is a neuron.
    assert!(!has("kluris.yml") && !has("projects/api/kluris.yml"));
    assert!(has("projects/api/map.yml"));
    // The YAML gate: a complete `#---` block within the first 4 KB.
    assert!(has("projects/api/openapi.yml") && has("projects/api/bom.yaml"));
    assert!(has("projects/api/indented.yaml") && has("projects/api/early-block.yml"));
    for not_yaml in [
        "projects/api/plain.yaml",
        "projects/api/open-only.yml",
        "projects/api/non-comment.yml",
        "projects/api/late-block.yml",
        "projects/api/upper.YML",
    ] {
        assert!(!has(not_yaml), "{not_yaml} is not a YAML neuron");
    }
    // Root-level neurons are neurons; nothing under a pruned directory is.
    assert!(has("root-note.md") && has("root-spec.yml"));
    assert!(!list.iter().any(|r| r.starts_with('.') && r != ".md"));
}

#[test]
fn neuron_index_keeps_malformed_neurons() {
    let b = brain("malformed");
    let expected = expect_neurons(&[
        ("lobe/active.md", Md),
        ("lobe/bad-utf8.md", Md),
        ("lobe/bad-yaml.md", Md),
        ("lobe/to-bad.md", Md),
        ("lobe/bad-block.yml", Yaml),
    ]);
    assert_eq!(neurons(&b.root), expected);
}

// --- symlinks ----------------------------------------------------------------

#[test]
fn walker_symlinks_as_v2() {
    let Some(b) = links_brain() else { return };
    let files = walk(&b.root);
    let names: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
    // Kept: links that resolve inside the brain, a broken one included (v2's
    // non-strict resolve() keeps it). Dropped: links that escape, broken
    // links that point outside, loops. Directory links are not descended:
    // an inside target is walked once, at its real path.
    assert_eq!(
        names,
        [
            "lobe/a.md",
            "lobe/broken.md",
            "lobe/good.md",
            "lobe/inlink.md",
            "lobe/map.md",
            "lobe/real/r.md",
            "lobe/via-dir.md",
            "to-broken.md",
            "to-escape.md",
            "to-inlink.md",
        ]
    );
    let link = files.iter().find(|f| f.rel == "lobe/inlink.md").unwrap();
    assert!(link.symlink);
}

#[test]
fn neuron_index_with_symlinks_matches_v2() {
    let Some(b) = links_brain() else { return };
    let expected = expect_neurons(&[
        ("lobe/a.md", Md),
        ("lobe/broken.md", Md),
        ("lobe/good.md", Md),
        ("lobe/inlink.md", Md),
        ("lobe/real/r.md", Md),
        ("lobe/via-dir.md", Md),
        ("to-broken.md", Md),
        ("to-escape.md", Md),
        ("to-inlink.md", Md),
    ]);
    assert_eq!(neurons(&b.root), expected);
}

#[test]
fn deprecation_with_symlinks_matches_v2() {
    let Some(b) = links_brain() else { return };
    let store = LocalGitStore::open(&b.root);
    let report = detect_deprecation_issues(&store);
    // A link and its target are one neuron for statuses (keyed by resolved
    // path) but each walked path reports its own issue; a link target is
    // reported as the resolved target.
    assert_eq!(
        report.issues,
        [
            without("lobe/a.md"),
            without("lobe/inlink.md"),
            missing("to-broken.md", "./lobe/broken.md"),
            missing("to-escape.md", "./lobe/escape.md"),
            not_active("to-inlink.md", "./lobe/inlink.md"),
            links("lobe/good.md", "lobe/a.md"),
        ]
    );
    let skipped: Vec<&str> = report.skipped.iter().map(|s| s.rel.as_str()).collect();
    assert_eq!(skipped, ["lobe/broken.md"]);
}

#[test]
fn store_reads_through_inside_links_and_refuses_the_rest() {
    let Some(b) = links_brain() else { return };
    let store = LocalGitStore::open(&b.root);
    let fm = store.read("lobe/inlink.md").unwrap();
    assert_eq!(
        fm.meta.get("status"),
        Some(&Value::Str("deprecated".into()))
    );
    assert!(matches!(
        store.read("lobe/escape.md"),
        Err(ReadError::Sandbox(SandboxError::Outside))
    ));
    assert!(matches!(
        store.read("lobe/broken.md"),
        Err(ReadError::Sandbox(SandboxError::NotFound))
    ));
    assert!(matches!(
        store.read("lobe/loop1.md"),
        Err(ReadError::Sandbox(SandboxError::Outside))
    ));
}

#[test]
fn resolve_follows_python_path_resolve() {
    let Some(b) = links_brain() else { return };
    let r = &b.root;
    // On POSIX `..` after a symlink is physical: lobe/up -> .. is the brain
    // root, so lobe/up/.. is the brain's parent. Python's ntpath.realpath
    // normalizes lexically first, so on Windows lobe/up/.. is lobe/.
    let up = sandbox::resolve(&r.join(native("lobe/up/../links/to-inlink.md"))).unwrap();
    #[cfg(not(windows))]
    assert_eq!(up, r.join("to-inlink.md"));
    #[cfg(windows)]
    assert_eq!(up, r.join(native("lobe/links/to-inlink.md")));
    assert_eq!(
        sandbox::resolve(&r.join(native("lobe/inlink.md"))).unwrap(),
        r.join(native("lobe/a.md"))
    );
    // A broken link resolves to its target's path; a missing tail is kept.
    assert_eq!(
        sandbox::resolve(&r.join(native("lobe/broken.md"))).unwrap(),
        r.join(native("lobe/missing.md"))
    );
    assert_eq!(
        sandbox::resolve(&r.join(native("lobe/nope/../a.md"))).unwrap(),
        r.join(native("lobe/a.md"))
    );
    assert!(sandbox::resolve(&r.join(native("lobe/loop1.md"))).is_err());
    assert!(sandbox::resolve(&r.join(native("lobe/self.md"))).is_err());
    assert!(!sandbox::is_within_brain(
        &sandbox::resolve(&r.join(native("lobe/escape.md"))).unwrap(),
        r
    ));
}

// --- the sandbox -------------------------------------------------------------

#[test]
fn is_within_brain_is_inclusive_and_component_wise() {
    let b = brain("full");
    let r = &b.root;
    assert!(sandbox::is_within_brain(r, r));
    assert!(sandbox::is_within_brain(&r.join("knowledge"), r));
    assert!(!sandbox::is_within_brain(r.parent().unwrap(), r));
    let sibling = PathBuf::from(format!("{}-extra", r.display()));
    assert!(!sandbox::is_within_brain(&sibling, r));
}

#[test]
fn resolve_in_brain_rules() {
    let b = brain("full");
    let r = &b.root;
    assert_eq!(
        sandbox::resolve_in_brain(r, "knowledge/good.md").unwrap(),
        r.join(native("knowledge/good.md"))
    );
    // Backslashes become slashes and leading slashes are dropped.
    assert_eq!(
        sandbox::resolve_in_brain(r, "/knowledge\\good.md").unwrap(),
        r.join(native("knowledge/good.md"))
    );
    assert_eq!(
        sandbox::resolve_in_brain(r, "").unwrap_err(),
        SandboxError::Empty
    );
    assert_eq!(
        sandbox::resolve_in_brain(r, "../outside-the-brain.md").unwrap_err(),
        SandboxError::Outside
    );
    assert_eq!(
        sandbox::resolve_in_brain(r, "knowledge/nope.md").unwrap_err(),
        SandboxError::NotFound
    );
    // The root itself and directories are inside and exist.
    assert_eq!(sandbox::resolve_in_brain(r, ".").unwrap(), r.clone());
}

// --- the store ---------------------------------------------------------------

#[test]
fn store_lists_the_walk() {
    let b = brain("full");
    let store = LocalGitStore::open(&b.root);
    assert_eq!(store.root(), b.root.as_path());
    let files: Vec<&str> = store.files().iter().map(|f| f.rel.as_str()).collect();
    assert_eq!(files, rels(&b.root));
    let listed: Vec<(String, NeuronKind)> = store
        .neurons()
        .iter()
        .map(|n| (n.rel.clone(), n.kind))
        .collect();
    assert_eq!(listed, neurons(&b.root));
}

#[test]
fn store_read_markdown_and_yaml_neurons() {
    let b = brain("full");
    let store = LocalGitStore::open(&b.root);
    let fm = store.read("projects/alpha.md").unwrap();
    assert_eq!(fm.meta.get("parent"), Some(&Value::Str("./map.md".into())));
    assert!(fm.body.starts_with("# Alpha"));
    let yml = store.read("projects/api/openapi.yml").unwrap();
    assert_eq!(yml.meta.get("title"), Some(&Value::Str("OpenAPI".into())));
    assert_eq!(yml.body, "openapi: 3.0.0\n");
}

#[test]
fn store_read_yaml_neuron_with_a_bad_block_is_empty_metadata_as_in_v2() {
    // v2's _read_yaml_neuron returns ({}, whole text) on a YAML error.
    let b = brain("full");
    let store = LocalGitStore::open(&b.root);
    let fm = store.read("knowledge/bad-block.yml").unwrap();
    assert!(fm.meta.is_empty());
    assert_eq!(
        fm.body,
        "#---\n# title: [unclosed\n# status: deprecated\n#---\nk: v\n"
    );
}

#[test]
fn store_read_reports_malformed_markdown_per_file() {
    let b = brain("malformed");
    let store = LocalGitStore::open(&b.root);
    match store.read("lobe/bad-yaml.md") {
        Err(ReadError::File(e)) => assert!(matches!(e.problem, Problem::Yaml(_))),
        other => panic!("expected a YAML error, got {other:?}"),
    }
    match store.read("lobe/bad-utf8.md") {
        Err(ReadError::File(e)) => assert_eq!(e.problem, Problem::NotUtf8),
        other => panic!("expected a UTF-8 error, got {other:?}"),
    }
    assert!(store.read("lobe/active.md").is_ok());
    assert!(matches!(
        store.read("../full/root-note.md"),
        Err(ReadError::Sandbox(SandboxError::Outside))
    ));
}

// --- the glossary ------------------------------------------------------------

fn full_glossary() -> Vec<GlossaryEntry> {
    vec![
        entry("API", "Application programming interface"),
        entry("SLA", "service level agreement (en dash)"),
        entry("SLO", "service level objective (em dash)"),
        entry("RPO", "recovery point objective, three dashes"),
        entry("RTO", "no spaces around the em dash"),
        entry("a**b", "term closes at the first double star"),
        entry("x", "-"),
        entry("API", "duplicate term, kept"),
        entry("Indented", "leading whitespace is stripped"),
        entry("NBSP", "non-breaking spaces around the dash"),
        entry("US", "definition after unit separators"),
        entry("Bold with *emphasis*", "it"),
        entry("Cut", "bold line rejected | not | a table"),
        entry("GA", "General availability"),
        entry("QA", "Quality assurance"),
        entry("dash def", "----"),
        entry("GA", "duplicate table term"),
        entry("caf\u{e9}", "unicode term \u{65e5}\u{672c}"),
        entry("CRLF", "line ends with CRLF"),
        entry("LS", "a line separator"),
        entry("Next", "splits the line"),
    ]
}

#[test]
fn glossary_matches_v2() {
    let b = brain("full");
    let store = LocalGitStore::open(&b.root);
    assert_eq!(store.glossary().unwrap(), full_glossary());
}

#[test]
fn glossary_line_rules() {
    let parse = |text: &str| parse_glossary_entries(text);
    assert_eq!(parse(""), vec![]);
    // A bold header term and a blank bold term are skipped.
    assert_eq!(parse("**Term** - x\n**  ** - y"), vec![]);
    assert_eq!(parse("**y** - "), vec![]);
    assert_eq!(
        parse("| a | b |\r\n| c | d |"),
        vec![entry("a", "b"), entry("c", "d")]
    );
    // `\x1c`-`\x1e` and U+0085 end a line for str.splitlines.
    assert_eq!(
        parse("**a** - b\u{1c}**c** - d\u{85}| e | f |"),
        vec![entry("a", "b"), entry("c", "d"), entry("e", "f")]
    );
    // Header terms are matched case-insensitively in both forms.
    assert_eq!(
        parse("| TERM | x |\n| x | MEANING |\n**definition** - z"),
        vec![]
    );
    assert_eq!(parse("| : - : | x |"), vec![]);
}

#[test]
fn glossary_missing_is_empty_and_unreadable_is_an_error() {
    let b = brain("links");
    assert_eq!(LocalGitStore::open(&b.root).glossary().unwrap(), vec![]);
    let m = brain("malformed");
    assert!(LocalGitStore::open(&m.root).glossary().is_err());
}

// --- deprecation -------------------------------------------------------------

#[test]
fn deprecation_matches_v2_on_the_full_brain() {
    let b = brain("full");
    let report = detect_deprecation_issues(&LocalGitStore::open(&b.root));
    assert_eq!(
        report.issues,
        [
            missing("knowledge/date-replacement.md", "2026-01-01"),
            without("knowledge/empty-replacement.md"),
            missing("knowledge/missing.md", "./nope.md"),
            without("knowledge/null-replacement.md"),
            missing("knowledge/outside.md", "../../outside-the-brain.md"),
            not_active("knowledge/to-bool-status.md", "bool-status.md"),
            not_active("knowledge/to-deprecated.md", "./without.md"),
            not_active("knowledge/to-dir.md", "../projects"),
            not_active("knowledge/to-glossary.md", "../glossary.md"),
            not_active("knowledge/to-map.md", "./map.md"),
            not_active("knowledge/to-null-status.md", "./null-status.md"),
            not_active("knowledge/to-yaml.md", "./legacy-config.yml"),
            without("knowledge/without.md"),
            without("knowledge/legacy-config.yml"),
            links("knowledge-extra/extra.md", "knowledge/to-good.md"),
            links("knowledge.md", "knowledge/without.md"),
            links("knowledge/linker.md", "knowledge/without.md"),
            links("knowledge/linker.md", "knowledge/missing.md"),
            links("knowledge/linker.md", "knowledge/to-map.md"),
            links("knowledge/linker.md", "knowledge/legacy-config.yml"),
            links("knowledge/null-status-linker.md", "knowledge/without.md"),
        ]
    );
    assert!(report.skipped.is_empty());
}

#[test]
fn deprecation_skips_malformed_neurons_and_reports_them() {
    let b = brain("malformed");
    let report = detect_deprecation_issues(&LocalGitStore::open(&b.root));
    assert_eq!(
        report.issues,
        [
            not_active("lobe/to-bad.md", "./bad-yaml.md"),
            links("lobe/active.md", "lobe/to-bad.md"),
        ]
    );
    let skipped: Vec<&str> = report.skipped.iter().map(|s| s.rel.as_str()).collect();
    assert_eq!(skipped, ["lobe/bad-utf8.md", "lobe/bad-yaml.md"]);
    assert!(matches!(report.skipped[0].error, ReadError::File(_)));
}

#[test]
fn deprecation_issue_kinds_are_v2_names() {
    assert_eq!(links("a", "b").kind(), "active_links_to_deprecated");
    assert_eq!(without("a").kind(), "deprecated_without_replacement");
    assert_eq!(missing("a", "b").kind(), "replaced_by_missing");
    assert_eq!(not_active("a", "b").kind(), "replaced_by_not_active");
}
