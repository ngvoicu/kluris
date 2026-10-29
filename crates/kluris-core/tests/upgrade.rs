//! What earlier releases left on a user's machine, through
//! `kluris_core::upgrade` directly: the skill copies they wrote and the
//! folders those copies are in (`upgrade::skills`).

use std::fs;
use std::path::{Path, PathBuf};

use kluris_core::upgrade::skills::{
    self, Folder, has_earlier_trait, is_earlier_skill, is_earlier_workflow,
};
use tempfile::TempDir;

fn legacy_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills/legacy")
}

/// Every `fixtures/skills/legacy/` file, an earlier release's own rendering:
/// `(file, the name it is installed under, its text)`. `-bare` and the
/// `v1.*` files are the single-brain `kluris`, the others `kluris-acme`.
fn renderings() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(legacy_fixtures()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap().to_string();
        let stem = file.trim_end_matches(".md").trim_end_matches(".workflow");
        let skill = if stem.ends_with("-bare") || stem.starts_with("v1.") {
            "kluris"
        } else {
            "kluris-acme"
        };
        let name = if file.ends_with(".workflow.md") {
            format!("{skill}.md")
        } else {
            skill.to_string()
        };
        out.push((file, name, fs::read_to_string(&path).unwrap()));
    }
    out.sort();
    assert!(out.len() >= 30, "{}", out.len());
    out
}

#[test]
fn every_earlier_rendering_is_recognised_lf_and_crlf() {
    for (file, name, text) in renderings() {
        for text in [text.clone(), text.replace('\n', "\r\n")] {
            let ok = if file.ends_with(".workflow.md") {
                is_earlier_workflow(&name, &text)
            } else {
                is_earlier_skill(&name, &text)
            };
            assert!(ok, "{file} as {name}");
            assert!(has_earlier_trait(&text), "{file}");
        }
    }
}

#[test]
fn an_earlier_rendering_is_recognised_only_under_its_own_name() {
    for (file, name, text) in renderings() {
        if file.ends_with(".workflow.md") {
            assert!(!is_earlier_workflow("kluris-other.md", &text), "{file}");
            assert!(!is_earlier_workflow(&name.replace(".md", ""), &text));
        } else {
            assert!(!is_earlier_skill("kluris-other", &text), "{file}");
        }
    }
}

/// A user's own skill whose description merely opens like an earlier
/// release's carries the trait (a near miss) but is not an earlier skill.
#[test]
fn a_users_own_skill_is_not_an_earlier_skill() {
    let own = "---\nname: kluris-notes\ndescription: \"You are the team's subject matter expert on notes\"\n---\n\n# Notes\n";
    assert!(has_earlier_trait(own));
    assert!(!is_earlier_skill("kluris-notes", own));
    assert!(!has_earlier_trait("---\nname: notes\n---\n"));
}

fn dirs(home: &Path, rels: &[&str]) {
    for rel in rels {
        fs::create_dir_all(home.join(rel)).unwrap();
    }
}

#[test]
fn the_folders_earlier_releases_wrote_skills_to_in_order() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    dirs(
        home,
        &[
            ".codeium/windsurf-next",
            ".codeium/windsurf",
            ".codeium/other",
        ],
    );
    let windsurf = home.join(".codeium").join("windsurf");
    let next = home.join(".codeium").join("windsurf-next");
    let mut expected = vec![
        Folder::Skills(windsurf.join("skills")),
        Folder::Skills(next.join("skills")),
        Folder::Workflows(windsurf.join("global_workflows")),
        Folder::Workflows(next.join("global_workflows")),
    ];
    let homed = |rel: &str| rel.split('/').fold(home.to_path_buf(), |p, s| p.join(s));
    for rel in [
        ".cursor/skills",
        ".codex/skills",
        ".copilot/skills",
        ".gemini/skills",
        ".kilo/skills",
        ".kilocode/skills",
        ".junie/skills",
    ] {
        expected.push(Folder::Skills(homed(rel)));
    }
    assert_eq!(skills::folders(home, None), expected);

    // With COPILOT_HOME set, ~/.copilot/skills may be the folder Copilot
    // reads: it is not an earlier copy's.
    let copilot = home.join("copilot-home");
    expected.retain(|f| f != &Folder::Skills(homed(".copilot/skills")));
    assert_eq!(skills::folders(home, Some(&copilot)), expected);
}

#[test]
fn without_windsurf_only_the_fixed_folders_are_listed() {
    let tmp = TempDir::new().unwrap();
    let listed = skills::folders(tmp.path(), None);
    assert_eq!(listed.len(), 7, "{listed:?}");
    assert!(listed.iter().all(|f| matches!(f, Folder::Skills(_))));
}
