//! `kluris dream`'s resilience and the decided v2-bug fixes (TEST-K3-21):
//! malformed files are skipped and reported, a BOM file keeps one block, the
//! first body line keeps its indentation, line endings are kept, odd
//! `related` values and a missing `kluris.yml` never crash, and link checking
//! ignores code, other schemes and titles and reads `%20`. Each case is one
//! where v2 crashes or writes the wrong thing (`parity/test_dream.py` shows
//! v2's side).

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use assert_fs::TempDir;
use serde_json::Value;

fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1")
        .env("GIT_CEILING_DIRECTORIES", home);
    cmd
}

fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

const GLOSSARY: &str = "---\nupdated: '2026-01-01'\n---\n\n# Glossary\n";

/// A neuron that is complete but for `parent`, so dream rewrites it.
const NO_PARENT: &str = "---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n# Good\n";

fn neuron(extra: &str, body: &str) -> String {
    format!(
        "---\ncreated: '2026-01-01'\n{extra}parent: ./map.md\nupdated: '2026-01-01'\n---\n\n{body}"
    )
}

struct Home {
    dir: TempDir,
    brains: Vec<String>,
}

impl Home {
    fn new() -> Home {
        Home {
            dir: TempDir::new().unwrap(),
            brains: Vec::new(),
        }
    }

    /// Add a brain `name` (with `kluris.yml` and a glossary) holding `files`.
    fn brain(mut self, name: &str, files: &[(&str, &[u8])]) -> Home {
        let root = self.root(name);
        fs::create_dir_all(&root).unwrap();
        self.write(
            name,
            "kluris.yml",
            format!("name: {name}\ndescription: d\n").as_bytes(),
        );
        self.write(name, "glossary.md", GLOSSARY.as_bytes());
        for (rel, bytes) in files {
            self.write(name, rel, bytes);
        }
        self.brains.push(name.to_string());
        let mut config = String::from("brains:\n");
        for brain in &self.brains {
            config.push_str(&format!(
                "  {brain}:\n    path: {}\n    description: ''\n",
                yaml_path(&self.root(brain))
            ));
        }
        let path = self.dir.path().join(".kluris").join("config.yml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, config).unwrap();
        self
    }

    fn root(&self, brain: &str) -> PathBuf {
        self.dir.path().join(brain)
    }

    fn path(&self, brain: &str, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(self.root(brain), |p, part| p.join(part))
    }

    fn write(&self, brain: &str, rel: &str, bytes: &[u8]) {
        let path = self.path(brain, rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn bytes(&self, brain: &str, rel: &str) -> Vec<u8> {
        fs::read(self.path(brain, rel)).unwrap()
    }

    fn text(&self, brain: &str, rel: &str) -> String {
        String::from_utf8(self.bytes(brain, rel)).unwrap()
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = kluris(self.dir.path()).args(args).output().unwrap();
        (
            out.status.code().unwrap(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8(out.stderr).unwrap(),
        )
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let (code, stdout, stderr) = self.run(args);
        assert_eq!(stderr, "", "{stdout}");
        (code, serde_json::from_str(&stdout).unwrap())
    }
}

fn keys(doc: &Value) -> Vec<String> {
    doc.as_object().unwrap().keys().cloned().collect()
}

// --- 1. malformed files are skipped and reported ------------------------------------

fn malformed() -> Home {
    Home::new().brain(
        "demo",
        &[
            ("lobe/good.md", NO_PARENT.as_bytes()),
            (
                "lobe/invalid.md",
                b"---\ntitle: Foo: bar\n---\n\n# Invalid\n",
            ),
            ("lobe/key.md", b"---\n1: x\n---\n\n# Key\n"),
            (
                "lobe/latin1.md",
                b"---\ncreated: '2026-01-01'\n---\n\n# Caf\xe9\n",
            ),
            ("lobe/block.yml", b"#---\n# title: Foo: bar\n#---\nk: 1\n"),
        ],
    )
}

const MALFORMED: [&str; 4] = [
    "lobe/block.yml",
    "lobe/invalid.md",
    "lobe/key.md",
    "lobe/latin1.md",
];

#[test]
fn malformed_files_are_skipped_reported_and_left_alone() {
    let home = malformed();
    let before: Vec<Vec<u8>> = MALFORMED.iter().map(|r| home.bytes("demo", r)).collect();
    let (code, doc) = home.json(&["dream", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["healthy"], false);
    for (rel, old) in MALFORMED.iter().zip(&before) {
        assert_eq!(&home.bytes("demo", rel), old, "{rel} was written");
    }
    // The other files are processed.
    assert!(
        home.text("demo", "lobe/good.md")
            .contains("parent: ./map.md")
    );
    assert!(home.path("demo", "lobe/map.md").exists());

    assert_eq!(keys(&doc).last().unwrap(), "skipped");
    let skipped = doc["skipped"].as_array().unwrap();
    let files: Vec<&str> = skipped
        .iter()
        .map(|s| s["file"].as_str().unwrap())
        .collect();
    assert_eq!(files, MALFORMED);
    for entry in skipped {
        assert_eq!(keys(entry), ["brain", "file", "error"]);
        assert_eq!(entry["brain"], "demo");
        let error = entry["error"].as_str().unwrap();
        assert!(
            !error.is_empty() && !error.contains(home.dir.path().to_str().unwrap()),
            "{error}"
        );
    }
    assert!(
        skipped[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("invalid frontmatter")
    );
    assert_eq!(skipped[3]["error"], "the file is not valid UTF-8");
    // A skipped file gets no other report (no "missing updated" for block.yml).
    assert_eq!(doc["frontmatter_issues"], 0, "{doc}");
}

#[test]
fn broken_only_lists_the_skipped_files_too() {
    let home = malformed();
    let (code, doc) = home.json(&["dream", "--json", "--broken-only"]);
    assert_eq!(code, 1);
    assert_eq!(
        keys(&doc),
        ["ok", "broken_synapses_count", "broken_synapses", "skipped"]
    );
    assert_eq!(doc["skipped"].as_array().unwrap().len(), 4);
}

#[test]
fn text_mode_prints_one_skipped_line_per_file_on_stderr() {
    let home = malformed();
    let (code, stdout, stderr) = home.run(&["dream"]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("Remaining issues need manual attention."),
        "{stdout}"
    );
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 4, "{stderr}");
    assert!(
        lines[0].starts_with("Skipped lobe/block.yml: invalid frontmatter"),
        "{stderr}"
    );
    assert_eq!(
        lines[3],
        "Skipped lobe/latin1.md: the file is not valid UTF-8"
    );
}

#[test]
fn skipped_lines_name_the_brain_under_brain_all() {
    let home = malformed().brain("calm", &[("lobe/a.md", neuron("", "# A\n").as_bytes())]);
    let (code, _, stderr) = home.run(&["dream", "--brain", "all"]);
    assert_eq!(code, 1);
    assert_eq!(stderr.lines().count(), 4, "{stderr}");
    assert!(
        stderr.lines().all(|l| l.starts_with("Skipped demo: lobe/")),
        "{stderr}"
    );
    let (_, doc) = home.json(&["dream", "--json", "--brain", "all"]);
    assert!(
        doc["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["brain"] == "demo")
    );
}

#[test]
fn no_skipped_key_when_nothing_was_skipped() {
    let home = Home::new().brain("demo", &[("lobe/a.md", neuron("", "# A\n").as_bytes())]);
    let (code, doc) = home.json(&["dream", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert!(doc.get("skipped").is_none());
    let (_, doc) = home.json(&["dream", "--json", "--broken-only"]);
    assert!(doc.get("skipped").is_none());
}

// --- 2-4. BOM, first-line indentation, line endings ------------------------------------

#[test]
fn a_bom_file_keeps_its_bom_and_gets_one_block() {
    let home = Home::new().brain(
        "demo",
        &[(
            "lobe/bom.md",
            "\u{feff}---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n# Bom\n".as_bytes(),
        )],
    );
    home.run(&["dream", "--json"]);
    assert_eq!(
        home.text("demo", "lobe/bom.md"),
        "\u{feff}---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n# Bom\n"
    );
}

#[test]
fn the_first_body_line_keeps_its_indentation() {
    let body = "    let x = 1;\n    let y = 2;\n\nText after the code.\n";
    let home = Home::new().brain(
        "demo",
        &[(
            "lobe/code.md",
            format!("---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n{body}").as_bytes(),
        )],
    );
    home.run(&["dream", "--json"]);
    assert_eq!(
        home.text("demo", "lobe/code.md"),
        format!(
            "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n{body}"
        )
    );
}

#[test]
fn crlf_files_stay_crlf_and_new_files_are_lf() {
    let crlf = |t: &str| t.replace('\n', "\r\n");
    let home = Home::new().brain(
        "demo",
        &[
            ("lobe/a.md", crlf(NO_PARENT).as_bytes()),
            (
                "lobe/n.yml",
                crlf("#---\n# updated: '2026-01-01'\n#---\nk: 1\n").as_bytes(),
            ),
            ("other/map.md", crlf("# Other\n\nOld body.\n").as_bytes()),
            ("other/b.md", neuron("", "# B\n").as_bytes()),
        ],
    );
    home.run(&["dream", "--json"]);
    assert_eq!(
        home.text("demo", "lobe/a.md"),
        crlf(
            "---\ncreated: '2026-01-01'\nparent: ./map.md\nupdated: '2026-01-01'\n---\n\n# Good\n"
        )
    );
    assert_eq!(
        home.text("demo", "lobe/n.yml"),
        crlf("#---\n# parent: ./map.md\n# updated: '2026-01-01'\n#---\nk: 1\n")
    );
    let other = home.text("demo", "other/map.md");
    assert!(other.contains("## Contents\r\n") && !other.replace("\r\n", "").contains('\n'));
    for new in ["lobe/map.md", "brain.md"] {
        assert!(!home.text("demo", new).contains('\r'), "{new} is not LF");
    }
}

// --- 5. odd related values, a missing kluris.yml ---------------------------------

#[test]
fn a_non_string_related_entry_is_a_type_issue() {
    let home = Home::new().brain(
        "demo",
        &[
            (
                "lobe/r.md",
                neuron("related:\n- 1\n- ./b.md\n", "# R\n").as_bytes(),
            ),
            (
                "lobe/b.md",
                neuron("related:\n- ./r.md\n", "# B\n").as_bytes(),
            ),
        ],
    );
    let (code, doc) = home.json(&["dream", "--json"]);
    assert_eq!(code, 1, "{doc}");
    assert_eq!(doc["frontmatter_issues"], 1);
    assert_eq!(doc["one_way_synapses"], 0);
    assert_eq!(doc["broken_synapses"], 0);
    assert!(doc.get("skipped").is_none());
}

#[test]
fn related_paths_outside_the_brain_are_broken_not_one_way() {
    let home = Home::new().brain(
        "demo",
        &[(
            "lobe/r.md",
            neuron("related:\n- ../../outside.md\n- /etc/hosts.md\n", "# R\n").as_bytes(),
        )],
    );
    fs::write(home.dir.path().join("outside.md"), neuron("", "# Out\n")).unwrap();
    let (code, doc) = home.json(&["dream", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(doc["broken_synapses"], 2, "{doc}");
    assert_eq!(doc["one_way_synapses"], 0);
    let targets: Vec<&str> = doc["broken_synapses_detail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["target"].as_str().unwrap())
        .collect();
    assert_eq!(targets, ["../../outside.md", "/etc/hosts.md"]);
}

#[test]
fn a_missing_kluris_yml_is_a_clean_error_and_nothing_is_written() {
    let home = Home::new()
        .brain("calm", &[("lobe/a.md", NO_PARENT.as_bytes())])
        .brain("bare", &[("lobe/b.md", NO_PARENT.as_bytes())]);
    fs::remove_file(home.path("bare", "kluris.yml")).unwrap();
    let missing = home.path("bare", "kluris.yml");

    let (code, stdout, stderr) = home.run(&["dream", "--brain", "all"]);
    assert_eq!((code, stdout.as_str()), (1, ""));
    assert_eq!(
        stderr,
        format!(
            "Error: Brain bare has no kluris.yml: {}\n",
            missing.display()
        )
    );
    let (code, stdout, _) = home.run(&["dream", "--brain", "all", "--json"]);
    assert_eq!(code, 1);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["ok"], false);
    assert!(doc["error"].as_str().unwrap().contains("kluris.yml"));
    // No brain was dreamed, the healthy one included.
    assert_eq!(home.text("calm", "lobe/a.md"), NO_PARENT);
    assert!(!home.path("calm", "lobe/map.md").exists());
    assert_eq!(home.text("bare", "lobe/b.md"), NO_PARENT);
}

// --- 6. link checking ---------------------------------------------------------------

#[test]
fn link_checking_ignores_code_other_schemes_and_titles_and_decodes_paths() {
    let body = "# L\n\n```\n[fenced](./nope1.md)\n```\n\n~~~md\n[tilde](./nope2.md)\n~~~\n\nSee `[inline](./nope3.md)` and ``[double `tick`](./nope4.md)``.\n\n[mail](mailto:a@b.c) [ftp](ftp://x/y) [tel](tel:123) [file](file:///etc/x)\n\n[titled](./good.md \"Good\") [single](./good.md 'Good') [paren](./good.md (Good))\n\n[encoded](./my%20note.md) [literal](./a%20b.md)\n\n![img](./pic.png) ![gone](./nopic.png)\n\n[still broken](./missing.md)\n";
    let home = Home::new().brain(
        "demo",
        &[
            ("lobe/l.md", neuron("", body).as_bytes()),
            ("lobe/good.md", neuron("", "# Good\n").as_bytes()),
            ("lobe/my note.md", neuron("", "# Note\n").as_bytes()),
            ("lobe/a%20b.md", neuron("", "# Literal\n").as_bytes()),
            ("lobe/pic.png", b"png"),
        ],
    );
    let (code, doc) = home.json(&["dream", "--json"]);
    assert_eq!(code, 1);
    let targets: Vec<&str> = doc["broken_synapses_detail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["target"].as_str().unwrap())
        .collect();
    // A missing image stays broken, as in v2.
    assert_eq!(targets, ["./nopic.png", "./missing.md"], "{doc}");
}
