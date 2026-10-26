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

#[cfg(unix)]
#[test]
fn an_unwritable_neuron_is_skipped_and_its_fix_not_counted() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new().brain(
        "demo",
        &[
            ("lobe/locked.md", NO_PARENT.as_bytes()),
            ("lobe/open.md", NO_PARENT.as_bytes()),
        ],
    );
    let locked = home.path("demo", "lobe/locked.md");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o444)).unwrap();
    if fs::OpenOptions::new().write(true).open(&locked).is_ok() {
        return; // running as root: nothing is unwritable
    }
    let (code, doc) = home.json(&["dream", "--json"]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(code, 1, "{doc}");
    assert_eq!(home.text("demo", "lobe/locked.md"), NO_PARENT);
    assert!(
        home.text("demo", "lobe/open.md")
            .contains("parent: ./map.md")
    );
    let skipped = doc["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{doc}");
    assert_eq!(skipped[0]["file"], "lobe/locked.md");
    let error = skipped[0]["error"].as_str().unwrap();
    assert!(error.starts_with("cannot write the file: "), "{error}");
    assert!(
        !error.contains(home.dir.path().to_str().unwrap()),
        "{error}"
    );
    // Only the fix that was written is counted, as v2's date pass counts.
    assert_eq!(doc["fixes"]["parents_inferred"], 1);
}

// --- write safety (review T-103) ---------------------------------------------------

/// A failed write never truncates the original: under a 1,024-byte file-size
/// limit dream cannot write the neuron's new version, reports it, and the
/// file keeps all of its bytes (finding 2).
#[cfg(unix)]
#[test]
fn a_write_cut_short_leaves_the_original_whole() {
    let body = "Lorem ipsum dolor sit amet.\n".repeat(800);
    let large =
        format!("---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n# Large\n\n{body}");
    let home = Home::new().brain("demo", &[("notes/large.md", large.as_bytes())]);
    let before = home.bytes("demo", "notes/large.md");
    assert!(before.len() > 20_000);
    let bin = assert_cmd::cargo::cargo_bin!("kluris");
    // `ulimit -f` counts 512-byte blocks; SIGXFSZ ignored, so the write
    // fails with EFBIG instead of killing the process.
    let out = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("trap '' XFSZ; ulimit -f 2; exec \"$KLURIS_BIN\" dream --json")
        .env("KLURIS_BIN", bin)
        .env("HOME", home.dir.path())
        .env(
            "KLURIS_CONFIG",
            home.dir.path().join(".kluris").join("config.yml"),
        )
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1")
        .env("GIT_CEILING_DIRECTORIES", home.dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert_eq!(home.bytes("demo", "notes/large.md"), before);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    let skipped = doc["skipped"].as_array().unwrap();
    assert!(
        skipped.iter().any(|s| s["file"] == "notes/large.md"
            && s["error"]
                .as_str()
                .unwrap()
                .starts_with("cannot write the file: ")),
        "{doc}"
    );
    assert_eq!(doc["fixes"]["parents_inferred"], 0, "{doc}");
    // No temp file is left beside it.
    let names: Vec<String> = fs::read_dir(home.path("demo", "notes"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().all(|n| !n.contains("kluris-tmp")), "{names:?}");
}

/// `mri` reports what its dream preflight skipped (finding 10): a
/// `skipped` list per brain in the JSON, only when something was skipped,
/// and dream's `Skipped` lines on stderr.
#[test]
fn mri_reports_the_files_its_preflight_skipped() {
    let home = Home::new().brain(
        "demo",
        &[
            ("notes/bad.md", b"---\ntitle: Foo: bar\n---\n\n# Bad\n"),
            ("notes/good.md", neuron("", "# Good\n").as_bytes()),
        ],
    );
    let before = home.bytes("demo", "notes/bad.md");
    let (code, doc) = home.json(&["mri", "--json"]);
    assert_eq!(code, 0, "{doc}");
    assert_eq!(home.bytes("demo", "notes/bad.md"), before);
    let brain = &doc["brains"][0];
    assert_eq!(
        keys(brain),
        [
            "name",
            "output_path",
            "preflight_fixes",
            "nodes",
            "edges",
            "skipped"
        ]
    );
    let skipped = brain["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1, "{doc}");
    assert_eq!(keys(&skipped[0]), ["brain", "file", "error"]);
    assert_eq!(skipped[0]["brain"], "demo");
    assert_eq!(skipped[0]["file"], "notes/bad.md");
    assert!(
        skipped[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("invalid frontmatter: "),
        "{doc}"
    );

    let (code, stdout, stderr) = home.run(&["mri"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.starts_with("MRI complete"), "{stdout}");
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(lines.len(), 1, "{stderr}");
    assert!(
        lines[0].starts_with("Skipped notes/bad.md: invalid frontmatter: "),
        "{stderr}"
    );
}

#[test]
fn mri_keeps_v2s_shape_when_nothing_was_skipped() {
    let home = Home::new()
        .brain(
            "demo",
            &[("notes/good.md", neuron("", "# Good\n").as_bytes())],
        )
        .brain("calm", &[("notes/bad.md", b"---\n1: x\n---\n\n# Key\n")]);
    let (code, doc) = home.json(&["mri", "--json", "--brain", "all"]);
    assert_eq!(code, 0, "{doc}");
    let brains = doc["brains"].as_array().unwrap();
    let by_name = |name: &str| brains.iter().find(|b| b["name"] == name).unwrap();
    assert_eq!(
        keys(by_name("demo")),
        ["name", "output_path", "preflight_fixes", "nodes", "edges"]
    );
    assert_eq!(by_name("calm")["skipped"][0]["brain"], "calm");
    let (_, _, stderr) = home.run(&["mri", "--brain", "all"]);
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(
        stderr.starts_with("Skipped calm: notes/bad.md: "),
        "{stderr}"
    );
}
