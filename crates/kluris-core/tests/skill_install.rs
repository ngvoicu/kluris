//! Installing each brain's SKILL.md into the agents' skill folders, in a temp
//! HOME: the `~/.agents/skills`-first matrix (SPEC acceptance criteria and
//! decisions 2026-10-24 / 2026-10-25), stage-then-rename per destination,
//! removal of deselected and stale copies, and the v2 migration. The v2
//! copies laid out here are v2's own renderings
//! (`fixtures/skills/v2/*.md`, `parity/tools/gen_skill_fixtures.py`);
//! `parity/test_skill_install.py` runs the same migration on a HOME the v2
//! oracle wrote itself.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use kluris_core::agents::install::{
    DestinationRole, InstallReport, InstallRequest, install_skills, read_brain_configs,
};
use kluris_core::config::{
    BrainConfig, BrainEntry, ConfigError, GlobalConfig, brain_config_to_yaml,
};
use tempfile::TempDir;

// --- the sandbox -------------------------------------------------------------

struct Sandbox {
    _tmp: TempDir,
    root: PathBuf,
    home: PathBuf,
    config: GlobalConfig,
    copilot_home: Option<PathBuf>,
}

impl Sandbox {
    fn new() -> Sandbox {
        let tmp = TempDir::new().expect("tempdir");
        let root = tmp.path().canonicalize().expect("canonical");
        let home = root.join("home");
        fs::create_dir_all(&home).expect("home");
        Sandbox {
            _tmp: tmp,
            root,
            home,
            config: GlobalConfig::default(),
            copilot_home: None,
        }
    }

    /// Register a brain at `<root>/brains/<name>` with a kluris.yml whose
    /// `commands_for` is `agents` (`None`: no kluris.yml at all).
    fn brain(&mut self, name: &str, agents: Option<&[&str]>) -> PathBuf {
        let path = self.root.join("brains").join(name);
        fs::create_dir_all(path.join(".git")).expect("brain");
        if let Some(agents) = agents {
            let mut cfg = BrainConfig::new(name);
            cfg.agents.commands_for = agents.iter().map(|a| a.to_string()).collect();
            fs::write(path.join("kluris.yml"), brain_config_to_yaml(&cfg)).expect("kluris.yml");
        }
        self.config.brains.insert(
            name.to_string(),
            BrainEntry {
                path: path.to_string_lossy().into_owned(),
                description: format!("{name} knowledge base"),
            },
        );
        path
    }

    fn set_agents(&self, name: &str, agents: &[&str]) {
        let path = self.root.join("brains").join(name);
        let mut cfg = BrainConfig::new(name);
        cfg.agents.commands_for = agents.iter().map(|a| a.to_string()).collect();
        fs::write(path.join("kluris.yml"), brain_config_to_yaml(&cfg)).expect("kluris.yml");
    }

    fn install(&self) -> InstallReport {
        let brains: IndexMap<String, Result<BrainConfig, ConfigError>> =
            read_brain_configs(&self.config);
        install_skills(&InstallRequest {
            config: &self.config,
            brains: &brains,
            home: &self.home,
            copilot_home: self.copilot_home.as_deref(),
        })
    }

    fn h(&self, rel: &str) -> PathBuf {
        self.home.join(rel)
    }

    /// Every file under HOME, relative path -> bytes.
    fn files(&self) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.home.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir") {
                let entry = entry.expect("entry");
                let path = entry.path();
                if entry.file_type().expect("type").is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(&self.home).expect("rel");
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    out.insert(rel, fs::read(&path).expect("read"));
                }
            }
        }
        out
    }

    /// Every directory under HOME, relative.
    fn dirs(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![self.home.clone()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read_dir") {
                let entry = entry.expect("entry");
                if entry.file_type().expect("type").is_dir() {
                    let rel = entry
                        .path()
                        .strip_prefix(&self.home)
                        .expect("rel")
                        .to_path_buf();
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                    stack.push(entry.path());
                }
            }
        }
        out.sort();
        out
    }
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/skills/v2")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// v2's SKILL.md for brain `name` (the `basic` fixture, renamed the way v2
/// names it: `name:` and every brain mention).
fn v2_skill(name: &str) -> String {
    fixture("basic.md").replace("acme", name)
}

fn v2_workflow(name: &str) -> String {
    fixture("basic.workflow.md").replace("acme", name)
}

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, text).expect("write");
}

const FOREIGN: &str =
    "---\nname: kluris-notes\ndescription: My own notes about kluris\n---\n\n# Notes\n";

fn skill_files(s: &Sandbox) -> Vec<String> {
    s.files()
        .into_keys()
        .filter(|k| k.ends_with("/SKILL.md"))
        .collect()
}

const ALL_OWN_FOLDERS: [&str; 5] = [
    ".agents/skills/kluris-acme/SKILL.md",
    ".claude/skills/kluris-acme/SKILL.md",
    ".gemini/config/skills/kluris-acme/SKILL.md",
    ".hermes/profiles/work/skills/kluris-acme/SKILL.md",
    ".hermes/skills/kluris-acme/SKILL.md",
];

// --- the matrix --------------------------------------------------------------

#[test]
fn empty_selection_means_every_registry_agent() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    fs::create_dir_all(s.h(".hermes/profiles/work")).expect("profile");
    let report = s.install();
    assert_eq!(skill_files(&s), ALL_OWN_FOLDERS);
    assert!(report.ok(), "{report:?}");
    assert_eq!(report.total_written(), 5);
}

#[test]
fn default_selection_writes_the_same_folders() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&kluris_core::config::DEFAULT_AGENT_COMMANDS));
    fs::create_dir_all(s.h(".hermes/profiles/work")).expect("profile");
    s.install();
    assert_eq!(skill_files(&s), ALL_OWN_FOLDERS);
}

#[test]
fn a_brain_without_kluris_yml_selects_nothing_so_all_agents() {
    let mut s = Sandbox::new();
    s.brain("acme", None);
    s.install();
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".claude/skills/kluris-acme/SKILL.md",
            ".gemini/config/skills/kluris-acme/SKILL.md",
            ".hermes/skills/kluris-acme/SKILL.md",
        ]
    );
}

#[test]
fn claude_only_writes_agents_and_claude_and_creates_no_other_folder() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    s.install();
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".claude/skills/kluris-acme/SKILL.md",
        ]
    );
    assert_eq!(
        s.dirs(),
        [
            ".agents",
            ".agents/skills",
            ".agents/skills/kluris-acme",
            ".claude",
            ".claude/skills",
            ".claude/skills/kluris-acme",
        ]
    );
}

#[test]
fn agents_served_by_the_universal_folder_write_nothing_extra() {
    let mut s = Sandbox::new();
    s.brain(
        "acme",
        Some(&[
            "cursor", "codex", "gemini", "devin", "kilocode", "junie", "windsurf",
        ]),
    );
    let report = s.install();
    assert_eq!(skill_files(&s), [".agents/skills/kluris-acme/SKILL.md"]);
    assert_eq!(
        s.dirs(),
        [".agents", ".agents/skills", ".agents/skills/kluris-acme"]
    );
    assert!(report.ok());
}

#[test]
fn unknown_agent_names_are_skipped_but_still_a_selection() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["bogus-agent"]));
    s.install();
    assert_eq!(skill_files(&s), [".agents/skills/kluris-acme/SKILL.md"]);
}

#[test]
fn hermes_writes_its_home_and_every_profile() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["hermes"]));
    fs::create_dir_all(s.h(".hermes/profiles/b-work")).expect("profile");
    fs::create_dir_all(s.h(".hermes/profiles/a-home")).expect("profile");
    put(&s.h(".hermes/profiles/not-a-profile.txt"), "x");
    let report = s.install();
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".hermes/profiles/a-home/skills/kluris-acme/SKILL.md",
            ".hermes/profiles/b-work/skills/kluris-acme/SKILL.md",
            ".hermes/skills/kluris-acme/SKILL.md",
        ]
    );
    let hermes: Vec<PathBuf> = report
        .destinations
        .iter()
        .filter(|d| d.role == DestinationRole::Agent("hermes"))
        .map(|d| d.path.clone())
        .collect();
    assert_eq!(
        hermes,
        [
            s.h(".hermes/skills"),
            s.h(".hermes/profiles/a-home/skills"),
            s.h(".hermes/profiles/b-work/skills"),
        ]
    );
}

#[test]
fn antigravity_writes_one_gemini_folder_only() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["antigravity"]));
    s.install();
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".gemini/config/skills/kluris-acme/SKILL.md",
        ]
    );
    assert!(!s.h(".gemini/antigravity-cli").exists());
    assert!(!s.h(".gemini/skills").exists());
}

#[test]
fn copilot_writes_copilot_home_only_when_it_is_set() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["copilot"]));
    s.install();
    assert_eq!(skill_files(&s), [".agents/skills/kluris-acme/SKILL.md"]);
    assert!(!s.h(".copilot").exists());

    s.copilot_home = Some(s.h("custom-copilot"));
    s.install();
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            "custom-copilot/skills/kluris-acme/SKILL.md",
        ]
    );
}

#[test]
fn copilot_home_without_copilot_selected_is_not_created() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    s.copilot_home = Some(s.h("custom-copilot"));
    s.install();
    assert!(!s.h("custom-copilot").exists());
}

#[test]
fn every_copy_is_byte_identical_and_only_a_skill_md() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    fs::create_dir_all(s.h(".hermes/profiles/work")).expect("profile");
    s.copilot_home = Some(s.h("cop"));
    s.brain("beta", Some(&[]));
    s.install();
    let files = s.files();
    for brain in ["acme", "beta"] {
        let copies: Vec<&Vec<u8>> = files
            .iter()
            .filter(|(k, _)| k.contains(&format!("/kluris-{brain}/")))
            .map(|(_, v)| v)
            .collect();
        assert_eq!(copies.len(), 6, "{brain}");
        assert!(copies.iter().all(|c| *c == copies[0]), "{brain}");
        let text = String::from_utf8(copies[0].clone()).expect("utf-8");
        assert!(text.starts_with(&format!("---\nname: kluris-{brain}\n")));
    }
    for k in files.keys() {
        assert!(k.ends_with("/SKILL.md"), "unexpected file {k}");
    }
}

#[test]
fn the_selection_is_the_union_over_brains() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    s.brain("beta", Some(&["antigravity"]));
    let report = s.install();
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".agents/skills/kluris-beta/SKILL.md",
            ".claude/skills/kluris-acme/SKILL.md",
            ".claude/skills/kluris-beta/SKILL.md",
            ".gemini/config/skills/kluris-acme/SKILL.md",
            ".gemini/config/skills/kluris-beta/SKILL.md",
        ]
    );
    assert_eq!(report.agents, ["antigravity", "claude"]);
    assert_eq!(report.skills, ["kluris-acme", "kluris-beta"]);
}

#[test]
fn rendered_skill_reflects_the_brain() {
    let mut s = Sandbox::new();
    let path = s.brain("acme", Some(&["claude"]));
    let mut cfg = BrainConfig::new("acme");
    cfg.agents.commands_for = vec!["claude".into()];
    cfg.companions = vec!["specmint-tdd".into(), "bogus".into()];
    fs::write(path.join("kluris.yml"), brain_config_to_yaml(&cfg)).expect("kluris.yml");
    s.install();
    let text = fs::read_to_string(s.h(".claude/skills/kluris-acme/SKILL.md")).expect("skill");
    let brain_posix = kluris_core::agents::render::posix_path(&path.to_string_lossy());
    assert!(text.contains(&format!("The brain lives at `{brain_posix}` (git).")));
    assert!(text.contains("Description: acme knowledge base\n"));
    let companions = kluris_core::agents::render::posix_path(
        &s.home.join(".kluris").join("companions").to_string_lossy(),
    );
    assert!(text.contains(&format!("read `{companions}/specmint-tdd/SKILL.md`")));
    assert!(!text.contains("bogus"));
}

// --- removal -----------------------------------------------------------------

#[test]
fn deselected_agents_lose_their_copies_and_keep_their_folders() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    fs::create_dir_all(s.h(".hermes/profiles/work")).expect("profile");
    s.copilot_home = Some(s.h("cop"));
    s.install();
    for dir in [
        ".claude/skills",
        ".hermes/skills",
        ".gemini/config/skills",
        "cop/skills",
    ] {
        put(&s.h(dir).join("kluris-notes/SKILL.md"), FOREIGN);
    }

    s.set_agents("acme", &["cursor"]);
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".claude/skills/kluris-notes/SKILL.md",
            ".gemini/config/skills/kluris-notes/SKILL.md",
            ".hermes/skills/kluris-notes/SKILL.md",
            "cop/skills/kluris-notes/SKILL.md",
        ]
    );
    // The profile's folder is emptied, not removed.
    assert!(s.h(".hermes/profiles/work/skills").is_dir());
    let removed: Vec<String> = report
        .destinations
        .iter()
        .filter(|d| matches!(d.role, DestinationRole::Deselected(_)))
        .flat_map(|d| d.removed.clone())
        .collect();
    assert_eq!(removed, ["kluris-acme"; 5]);
}

#[test]
fn removed_brains_lose_their_skills_everywhere() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    s.brain("beta", Some(&["claude"]));
    s.install();
    put(&s.h(".agents/skills/kluris-notes/SKILL.md"), FOREIGN);
    put(
        &s.h(".agents/skills/other-skill/SKILL.md"),
        "---\nname: other-skill\n---\n",
    );
    put(&s.h(".agents/skills/kluris-readme.txt"), "a file");

    s.config.brains.shift_remove("beta");
    let report = s.install();
    assert!(report.ok());
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".agents/skills/kluris-notes/SKILL.md",
            ".agents/skills/other-skill/SKILL.md",
            ".claude/skills/kluris-acme/SKILL.md",
        ]
    );
    assert!(s.h(".agents/skills/kluris-readme.txt").is_file());
}

#[test]
fn no_brains_left_clears_every_kluris_skill() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    s.install();
    s.config.brains.clear();
    let report = s.install();
    assert!(report.ok());
    assert!(skill_files(&s).is_empty(), "{:?}", skill_files(&s));
    assert_eq!(report.total_written(), 0);
}

#[test]
fn a_users_own_kluris_skill_is_never_touched() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    for dir in [
        ".agents/skills",
        ".claude/skills",
        ".cursor/skills",
        ".codex/skills",
    ] {
        put(&s.h(dir).join("kluris-notes/SKILL.md"), FOREIGN);
        put(&s.h(dir).join("kluris-empty/README.md"), "no SKILL.md here");
    }
    let report = s.install();
    assert!(report.ok());
    for dir in [
        ".agents/skills",
        ".claude/skills",
        ".cursor/skills",
        ".codex/skills",
    ] {
        let text = fs::read_to_string(s.h(dir).join("kluris-notes/SKILL.md")).expect("kept");
        assert_eq!(text, FOREIGN);
        assert!(s.h(dir).join("kluris-empty/README.md").is_file());
    }
}

/// A brain whose skill name collides with a user's own folder: the folder
/// is kept and the destination reports the skill as not written.
#[test]
fn a_foreign_folder_with_the_skill_name_is_kept_and_reported() {
    let mut s = Sandbox::new();
    s.brain("notes", Some(&["claude"]));
    put(&s.h(".claude/skills/kluris-notes/SKILL.md"), FOREIGN);
    let report = s.install();
    assert!(!report.ok());
    let text = fs::read_to_string(s.h(".claude/skills/kluris-notes/SKILL.md")).expect("kept");
    assert_eq!(text, FOREIGN);
    let claude = report
        .destinations
        .iter()
        .find(|d| d.role == DestinationRole::Agent("claude"))
        .expect("claude");
    assert!(claude.written.is_empty());
    assert_eq!(claude.failed.len(), 1, "{claude:?}");
    assert!(
        claude.failed[0].contains("kluris-notes"),
        "{:?}",
        claude.failed
    );
    // Other destinations are unaffected.
    assert!(s.h(".agents/skills/kluris-notes/SKILL.md").is_file());
    assert!(!s.h(".claude/skills/.kluris-notes.tmp").exists());
}

#[test]
fn an_old_copy_is_replaced_in_place() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".claude/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    put(
        &s.h(".claude/skills/kluris-acme/extra.txt"),
        "left by someone",
    );
    s.install();
    let v3 = fs::read(s.h(".agents/skills/kluris-acme/SKILL.md")).expect("v3");
    assert_eq!(
        fs::read(s.h(".claude/skills/kluris-acme/SKILL.md")).expect("claude"),
        v3
    );
    assert!(!s.h(".claude/skills/kluris-acme/extra.txt").exists());
    let leftovers: Vec<String> = s
        .dirs()
        .into_iter()
        .filter(|d| d.contains("/.kluris"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn stale_staging_folders_from_a_crash_are_cleared() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".claude/skills/.kluris-acme.tmp/SKILL.md"),
        "half written",
    );
    put(&s.h(".claude/skills/.kluris-acme.old/SKILL.md"), "old");
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(!s.h(".claude/skills/.kluris-acme.tmp").exists());
    assert!(!s.h(".claude/skills/.kluris-acme.old").exists());
    assert!(s.h(".claude/skills/kluris-acme/SKILL.md").is_file());
}

/// A destination that cannot be written keeps its old skill; the others are
/// still written and the failure is reported for that destination.
#[cfg(unix)]
#[test]
fn a_failed_destination_keeps_its_old_skill() {
    use std::os::unix::fs::PermissionsExt;
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude", "antigravity"]));
    let old = v2_skill("acme");
    put(&s.h(".claude/skills/kluris-acme/SKILL.md"), &old);
    let claude = s.h(".claude/skills");
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o555)).expect("chmod");
    let report = s.install();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).expect("chmod back");

    assert!(!report.ok());
    assert_eq!(
        fs::read_to_string(claude.join("kluris-acme/SKILL.md")).expect("old kept"),
        old
    );
    let failed: Vec<&PathBuf> = report
        .destinations
        .iter()
        .filter(|d| !d.failed.is_empty())
        .map(|d| &d.path)
        .collect();
    assert_eq!(failed, [&claude]);
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());
    assert!(s.h(".gemini/config/skills/kluris-acme/SKILL.md").is_file());
    assert_eq!(report.failures().len(), 1);
}

/// A skills folder that is a file (or anything that is not a folder) fails
/// that destination only; on every OS.
#[test]
fn a_destination_that_is_a_file_fails_alone() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(&s.h(".claude/skills"), "not a folder");
    let report = s.install();
    assert!(!report.ok());
    assert_eq!(
        fs::read_to_string(s.h(".claude/skills")).expect("file"),
        "not a folder"
    );
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());
}

#[test]
fn a_brain_with_an_invalid_kluris_yml_is_reported_and_still_rendered() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let bad = s.brain("bad", Some(&["claude"]));
    fs::write(bad.join("kluris.yml"), "name: [unclosed\n").expect("bad yml");
    let report = s.install();
    assert_eq!(report.brain_errors.len(), 1, "{:?}", report.brain_errors);
    assert_eq!(report.brain_errors[0].0, "bad");
    assert_eq!(
        skill_files(&s),
        [
            ".agents/skills/kluris-acme/SKILL.md",
            ".agents/skills/kluris-bad/SKILL.md",
            ".claude/skills/kluris-acme/SKILL.md",
            ".claude/skills/kluris-bad/SKILL.md",
        ]
    );
}

// --- v2 migration ------------------------------------------------------------

/// The folders v2 wrote and v3 no longer uses (research-07: they make real
/// duplicates), with `~/.copilot/skills` only while `COPILOT_HOME` is unset.
const V2_ONLY_SKILL_FOLDERS: [&str; 10] = [
    ".codeium/windsurf/skills",
    ".codeium/windsurf-next/skills",
    ".codeium/windsurf-insiders/skills",
    ".cursor/skills",
    ".codex/skills",
    ".copilot/skills",
    ".gemini/skills",
    ".kilo/skills",
    ".kilocode/skills",
    ".junie/skills",
];

/// Lay out a v2.28 install for brains acme and gone (no longer registered)
/// plus a user's own skills, workflows and files next to them.
fn v2_home(s: &Sandbox) {
    let v2_dirs: Vec<&str> = V2_ONLY_SKILL_FOLDERS
        .iter()
        .copied()
        .chain([".claude/skills", ".hermes/skills", ".agents/skills"])
        .collect();
    for dir in v2_dirs {
        for brain in ["acme", "gone"] {
            put(
                &s.h(dir).join(format!("kluris-{brain}/SKILL.md")),
                &v2_skill(brain),
            );
        }
        put(&s.h(dir).join("kluris-notes/SKILL.md"), FOREIGN);
        put(
            &s.h(dir).join("my-skill/SKILL.md"),
            "---\nname: my-skill\n---\n",
        );
    }
    // v1's bare `kluris` skill.
    put(
        &s.h(".cursor/skills/kluris/SKILL.md"),
        "---\nname: kluris\ndescription: \"You are the team's subject matter expert. Your knowledge\"\n---\n\nbody\n",
    );
    for wf in [
        ".codeium/windsurf/global_workflows",
        ".codeium/windsurf-next/global_workflows",
    ] {
        for brain in ["acme", "gone"] {
            put(
                &s.h(wf).join(format!("kluris-{brain}.md")),
                &v2_workflow(brain),
            );
        }
        put(&s.h(wf).join("kluris-notes.md"), "# my own workflow\n");
        put(&s.h(wf).join("deploy.md"), "# deploy\n");
    }
    // Not a v2 folder: never looked at.
    put(
        &s.h(".codeium/other/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    put(&s.h(".claude/commands/kluris.md"), "# v1 command\n");
}

#[test]
fn v2_install_is_migrated_and_nothing_else_is_touched() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude", "cursor", "windsurf", "codex"]));
    v2_home(&s);
    let before = s.files();
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    let after = s.files();

    let mut expected = before.clone();
    for dir in V2_ONLY_SKILL_FOLDERS {
        expected.remove(&format!("{dir}/kluris-acme/SKILL.md"));
        expected.remove(&format!("{dir}/kluris-gone/SKILL.md"));
    }
    expected.remove(".cursor/skills/kluris/SKILL.md");
    for wf in [
        ".codeium/windsurf/global_workflows",
        ".codeium/windsurf-next/global_workflows",
    ] {
        expected.remove(&format!("{wf}/kluris-acme.md"));
        expected.remove(&format!("{wf}/kluris-gone.md"));
    }
    // The kept folders: acme rewritten, gone removed; hermes is not selected.
    let v3 = after[".agents/skills/kluris-acme/SKILL.md"].clone();
    for dir in [".agents/skills", ".claude/skills"] {
        expected.insert(format!("{dir}/kluris-acme/SKILL.md"), v3.clone());
        expected.remove(&format!("{dir}/kluris-gone/SKILL.md"));
    }
    expected.remove(".hermes/skills/kluris-acme/SKILL.md");
    expected.remove(".hermes/skills/kluris-gone/SKILL.md");

    let added: Vec<&String> = after.keys().filter(|k| !before.contains_key(*k)).collect();
    assert!(added.is_empty(), "added {added:?}");
    let diff: Vec<&String> = expected
        .keys()
        .chain(after.keys())
        .filter(|k| expected.get(*k) != after.get(*k))
        .collect();
    assert!(diff.is_empty(), "differs from the expected HOME: {diff:?}");
    // Folders are emptied, never removed.
    for dir in V2_ONLY_SKILL_FOLDERS {
        assert!(s.h(dir).is_dir(), "{dir}");
    }
    let migrated: usize = report
        .destinations
        .iter()
        .filter(|d| d.role == DestinationRole::Migration)
        .map(|d| d.removed.len())
        .sum();
    assert_eq!(migrated, 10 * 2 + 1 + 2 * 2);
}

#[test]
fn copilot_folder_is_kept_when_copilot_home_is_set() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".copilot/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    s.copilot_home = Some(s.h(".copilot-custom"));
    s.install();
    assert!(s.h(".copilot/skills/kluris-acme/SKILL.md").is_file());

    s.copilot_home = None;
    s.install();
    assert!(!s.h(".copilot/skills/kluris-acme").exists());
}

/// The migration runs after the new copies exist: a failure writing
/// `~/.agents/skills` leaves the v2 copies the agents still read.
#[test]
fn migration_waits_for_the_universal_copy() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["cursor"]));
    put(
        &s.h(".cursor/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    put(&s.h(".agents/skills"), "not a folder");
    let report = s.install();
    assert!(!report.ok());
    assert!(s.h(".cursor/skills/kluris-acme/SKILL.md").is_file());
}

#[test]
fn report_order_is_universal_agents_deselected_migration() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    fs::create_dir_all(s.h(".hermes/skills")).expect("hermes");
    fs::create_dir_all(s.h(".cursor/skills")).expect("cursor");
    let report = s.install();
    let roles: Vec<&DestinationRole> = report.destinations.iter().map(|d| &d.role).collect();
    assert_eq!(roles[0], &DestinationRole::Universal);
    assert_eq!(roles[1], &DestinationRole::Agent("claude"));
    assert!(roles.contains(&&DestinationRole::Deselected("hermes")));
    assert!(roles.contains(&&DestinationRole::Migration));
    let first_cleanup = roles
        .iter()
        .position(|r| !matches!(r, DestinationRole::Universal | DestinationRole::Agent(_)))
        .expect("cleanup");
    assert!(
        roles[first_cleanup..]
            .iter()
            .all(|r| !matches!(r, DestinationRole::Universal | DestinationRole::Agent(_)))
    );
    // Cleanup entries are listed only for folders that exist.
    assert!(report.destinations.iter().all(|d| d.path.is_dir()));
}
