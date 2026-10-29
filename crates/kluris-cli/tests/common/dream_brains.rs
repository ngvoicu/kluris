//! The fixture brains of the dream and MRI goldens, built in a [`Sandbox`]
//! under `work/brains/<name>` with commits at pinned dates:
//! - the rich brain holds every dream rule: lobes, a sub-lobe, siblings,
//!   root and uncommitted neurons, the three fixes, many reverse links to
//!   one target, agent-style frontmatter, a YAML neuron, deprecation of
//!   every kind, broken links, a CRLF file and an upper-case suffix;
//! - the healthy brain has nothing to fix but its maps;
//! - the MRI brain is the rich one plus an index, long and non-ASCII text,
//!   HTML that must not end the page's script, anchors, a deep neuron and
//!   inline links of every kind.

use std::path::{Path, PathBuf};

use super::sandbox::{Sandbox, write_tree};

pub const D1: &str = "2026-01-01T12:00:00+00:00";
pub const D2: &str = "2026-02-03T09:30:00+00:00";
pub const D3: &str = "2026-03-04T08:00:00+00:00";

/// Files of a brain: `(relative path, bytes)`.
pub type Files = Vec<(String, Vec<u8>)>;

/// A neuron's text: frontmatter with `created`, `extra` lines, `parent`
/// and `updated`, then `# <title>` and its body (`Body of <title>.` unless
/// given).
#[derive(Clone, Copy)]
pub struct Neuron<'a> {
    title: &'a str,
    parent: &'a str,
    created: &'a str,
    updated: &'a str,
    extra: &'a str,
    body: &'a str,
}

impl<'a> Neuron<'a> {
    pub fn new(title: &'a str) -> Neuron<'a> {
        Neuron {
            title,
            parent: "./map.md",
            created: "'2026-01-01'",
            updated: "'2026-01-01'",
            extra: "",
            body: "",
        }
    }

    pub fn parent(self, parent: &'a str) -> Self {
        Neuron { parent, ..self }
    }

    pub fn created(self, created: &'a str) -> Self {
        Neuron { created, ..self }
    }

    pub fn updated(self, updated: &'a str) -> Self {
        Neuron { updated, ..self }
    }

    pub fn extra(self, extra: &'a str) -> Self {
        Neuron { extra, ..self }
    }

    pub fn body(self, body: &'a str) -> Self {
        Neuron { body, ..self }
    }

    pub fn text(&self) -> String {
        let body = if self.body.is_empty() {
            format!("Body of {}.", self.title)
        } else {
            self.body.to_string()
        };
        format!(
            "---\ncreated: {}\n{}parent: {}\nupdated: {}\n---\n\n# {}\n\n{body}\n",
            self.created, self.extra, self.parent, self.updated, self.title
        )
    }
}

/// [`Neuron::new`]`(title)`'s text.
pub fn neuron(title: &str) -> String {
    Neuron::new(title).text()
}

pub const GLOSSARY: &str =
    "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n\n**JWT** -- JSON Web Token.\n";

pub const AGENT_STYLE: &str = "---
parent: ./map.md
title: Agent written
created: 2026-01-01
updated: 2025-12-31
# a comment the writer drops
tags: [auth, \"jwt\", on, off, 2026, 1.10]
related:
  - ./alpha.md
base: &shared [x, y]
copy: *shared
when: 2026-01-02T10:00:00Z
nested: [2026-01-03]
emoji: \"\u{1F680} launch\"
long: \"a long line that goes past the eighty column limit so that the writer folds it onto two lines\"
---

# Agent written

Text with [alpha](./alpha.md).
";

pub const LINKER_STYLE: &str = "---
created: 2026-01-01
updated: 2026-01-01
tags:
  - linker
when: 2026-01-02T10:00:00Z
---

# Linker written
";

pub const SPEC_YML: &str = "#---
# title: API spec
# updated: '2025-06-01'
# related:
# - ../alpha.md
#---
openapi: 3.0.0
info:
  title: demo
";

/// A neuron complete but for `parent`, so dream rewrites it.
pub const NO_PARENT: &str =
    "---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n# Needs a parent\n";

fn file(rel: &str, text: impl Into<Vec<u8>>) -> (String, Vec<u8>) {
    (rel.to_string(), text.into())
}

pub fn rich_files() -> Files {
    let mut files = vec![
        file(
            "kluris.yml",
            "name: rich\ndescription: The rich fixture brain\n",
        ),
        file("glossary.md", GLOSSARY),
        file(
            "README.md",
            "# Rich\n\nA [link that is not checked](./nowhere.md).\n",
        ),
        file("root-note.md", neuron("Root note")),
        file(
            "projects/alpha.md",
            Neuron::new("Alpha")
                .extra("related:\n- ../knowledge/beta.md\n")
                .text(),
        ),
        file("projects/agent.md", AGENT_STYLE),
        file(
            "projects/no-frontmatter.md",
            "# No frontmatter\n\nJust text.\n",
        ),
        file("projects/no-parent.md", LINKER_STYLE),
        file(
            "projects/api/endpoints.md",
            Neuron::new("Endpoints").extra("tags:\n- api\n").text(),
        ),
        file("projects/api/spec.yml", SPEC_YML),
        file("projects/api/raw.yaml", "not: a neuron\n"),
        file("projects/api/Shout.MD", "# Shouted\n"),
        file("knowledge/beta.md", neuron("Beta")),
        file(
            "knowledge/target.md",
            Neuron::new("Target").extra("related: []\n").text(),
        ),
        file(
            "knowledge/old.md",
            Neuron::new("Old").extra("status: deprecated\n").text(),
        ),
        file(
            "knowledge/older.md",
            Neuron::new("Older")
                .extra("replaced_by: ./gone.md\nstatus: deprecated\n")
                .text(),
        ),
        file(
            "knowledge/newer.md",
            Neuron::new("Newer")
                .extra("replaced_by: ./old.md\nstatus: deprecated\n")
                .text(),
        ),
        file(
            "knowledge/user.md",
            Neuron::new("User")
                .extra("related:\n- ./old.md\n- ./target.md\n")
                .text(),
        ),
        file(
            "knowledge/broken.md",
            Neuron::new("Broken")
                .extra("related:\n- ./gone.md\n- ./beta.md\n")
                .body("See [x](./missing.md#top) and [web](https://x.io).")
                .text(),
        ),
        file(
            "knowledge/crlf.md",
            Neuron::new("Crlf")
                .updated("'2025-01-01'")
                .text()
                .replace('\n', "\r\n"),
        ),
        file("knowledge/deep/deeper/leaf.md", neuron("Leaf")),
        file("my-lobe/thing.md", neuron("Thing")),
        file(
            "uncommitted-lobe/later.md",
            Neuron::new("Later").updated("'2025-05-05'").text(),
        ),
    ];
    for name in ["zeta", "alpha-2", "mid", "Beta", "_u", "a-b", "a.b"] {
        files.push(file(
            &format!("knowledge/src/{name}.md"),
            Neuron::new(name).extra("related:\n- ../target.md\n").text(),
        ));
    }
    files
}

/// The rich brain: committed at [`D1`], `projects/alpha.md` changed and
/// committed at [`D2`], then `uncommitted-lobe/` written and not committed.
pub fn rich_brain(sb: &Sandbox, name: &str) -> PathBuf {
    let brain = sb.work.join("brains").join(name);
    let (later, files): (Files, Files) = rich_files()
        .into_iter()
        .partition(|(rel, _)| rel.starts_with("uncommitted-lobe/"));
    write_tree(&brain, files);
    sb.init(&brain);
    sb.commit(&brain, "initial", D1);
    write_tree(
        &brain,
        [(
            "projects/alpha.md",
            Neuron::new("Alpha")
                .extra("related:\n- ../knowledge/beta.md\n")
                .body("Changed.")
                .text(),
        )],
    );
    sb.commit(&brain, "alpha changed", D2);
    write_tree(&brain, later);
    brain
}

pub fn healthy_files() -> Files {
    vec![
        file("kluris.yml", "name: calm\ndescription: A healthy brain\n"),
        file("glossary.md", GLOSSARY),
        file(
            "projects/alpha.md",
            Neuron::new("Alpha").extra("related:\n- ./beta.md\n").text(),
        ),
        file(
            "projects/beta.md",
            Neuron::new("Beta").extra("related:\n- ./alpha.md\n").text(),
        ),
        file("knowledge/gamma.md", neuron("Gamma")),
    ]
}

/// The healthy brain in `work/brains/<name>`, committed at [`D1`] when
/// `git_repo`.
pub fn healthy_brain(sb: &Sandbox, name: &str, git_repo: bool) -> PathBuf {
    let brain = sb.work.join("brains").join(name);
    write_tree(&brain, healthy_files());
    if git_repo {
        sb.init(&brain);
        sb.commit(&brain, "initial", D1);
    }
    brain
}

/// The healthy brain `calm` plus `files`, all committed at [`D2`].
pub fn resilience_brain(sb: &Sandbox, files: Files) -> PathBuf {
    let brain = healthy_brain(sb, "calm", true);
    write_tree(&brain, files);
    sb.commit(&brain, "resilience case", D2);
    brain
}

pub fn mri_files() -> Files {
    let long_body = (1..70)
        .map(|i| format!("Line {i} of a long neuron."))
        .collect::<Vec<_>>()
        .join("\n");
    let long_body = format!("{long_body}\n\n{}", "x".repeat(3000));
    vec![
        file(
            "index.md",
            "# Index\n\nSee [projects](./projects/map.md).\n",
        ),
        file(
            "projects/script.md",
            Neuron::new("Script")
                .body("Inline </script><script>alert(1)</script> and <!--<script> and </style> here.")
                .text(),
        ),
        file(
            "projects/long-one.md",
            Neuron::new("Long one").body(&long_body).text(),
        ),
        // Dated as committed, so dream leaves its dates alone.
        file(
            "projects/unicode-ăîș.md",
            Neuron::new("Șir de caractere — ünïcødé 🚀")
                .updated("'2026-02-03'")
                .body("Ăla e un text cu diacritice.")
                .text(),
        ),
        file(
            "projects/anchors.md",
            Neuron::new("Anchors")
                .body("<a id=\"top\"></a>First <A ID='b'> </a>line.\n\n## Sub\n\n<a name=\"n\"></a>Kept.")
                .text(),
        ),
        file(
            "projects/links.md",
            Neuron::new("Links")
                .extra("related:\n- ./script.md\n- ./script.md\n- ../nowhere.md\nreplaced_by: ./anchors.md\n")
                .body(
                    "[s](./script.md) [l](./long-one.md#part) [q](long-one.md?x=1) [self](./links.md) \
                     [web](https://example.com) [g](../glossary.md#jwt) [up](../index.md)",
                )
                .text(),
        ),
        file(
            "projects/api/deeper/leaf.md",
            Neuron::new("Leaf").parent("../map.md").text(),
        ),
    ]
}

/// The rich brain plus [`mri_files`], committed at [`D2`].
pub fn mri_brain(sb: &Sandbox, name: &str) -> PathBuf {
    let brain = rich_brain(sb, name);
    write_tree(&brain, mri_files());
    sb.commit(&brain, "mri files", D2);
    brain
}

/// Register `brains` under their folder names, description `fixture`.
pub fn register(sb: &Sandbox, brains: &[&Path]) {
    let entries: Vec<(String, &Path)> = brains
        .iter()
        .map(|b| (b.file_name().unwrap().to_string_lossy().into_owned(), *b))
        .collect();
    let refs: Vec<(&str, &Path, &str)> = entries
        .iter()
        .map(|(name, path)| (name.as_str(), *path, "fixture"))
        .collect();
    sb.write_config(&refs);
}
