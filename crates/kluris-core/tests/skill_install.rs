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
    DestinationReport, DestinationRole, InstallReport, InstallRequest, install_skills,
    install_skills_with, read_brain_configs, remove_generated_skills_with,
};
use kluris_core::agents::registry;
use kluris_core::config::{
    BrainConfig, BrainEntry, ConfigError, GlobalConfig, brain_config_to_yaml,
};
use kluris_core::fileops::{FileOps, RealFs};
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

    fn install_with(&self, ops: &dyn FileOps) -> InstallReport {
        let brains: IndexMap<String, Result<BrainConfig, ConfigError>> =
            read_brain_configs(&self.config);
        install_skills_with(
            &InstallRequest {
                config: &self.config,
                brains: &brains,
                home: &self.home,
                copilot_home: self.copilot_home.as_deref(),
            },
            ops,
        )
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

/// A real v1/v2 rendering (`fixtures/skills/legacy/`,
/// `parity/tools/gen_legacy_skills.py`).
fn legacy(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/skills/legacy")
        .join(file);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
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
        Some(&["cursor", "codex", "devin", "kilocode", "junie", "windsurf"]),
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

    // `gemini` is not an agent Kluris knows: an unknown name like any other.
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["gemini"]));
    let report = s.install();
    assert_eq!(skill_files(&s), [".agents/skills/kluris-acme/SKILL.md"]);
    assert!(report.agents.is_empty(), "{:?}", report.agents);
    assert!(report.ok());
}

#[test]
fn gemini_is_not_a_known_agent_name() {
    assert!(!registry::UNIVERSAL_ONLY.contains(&"gemini"));
    assert!(!registry::REGISTRY.contains(&"gemini"));
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
        // Nothing about them looks like Kluris: left alone, not listed.
        let d = dest(&report, &s.h(dir));
        assert!(d.kept.is_empty() && d.removed.is_empty(), "{d:?}");
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
    s.install();
    let v3 = fs::read(s.h(".agents/skills/kluris-acme/SKILL.md")).expect("v3");
    assert_eq!(
        fs::read(s.h(".claude/skills/kluris-acme/SKILL.md")).expect("claude"),
        v3
    );
    let leftovers: Vec<String> = s
        .dirs()
        .into_iter()
        .filter(|d| d.contains("/.kluris"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// A staging name of an earlier run: `.kluris-<brain>.<pid>-<16 hex>.tmp`
/// or `.old`.
fn staged(skill: &str, n: u32, suffix: &str) -> String {
    format!(".{skill}.4242-{n:016x}.{suffix}")
}

/// An earlier run's staging folders are removed when they hold nothing but
/// a Kluris-written SKILL.md; a half-written one is kept and reported.
#[test]
fn stale_staging_folders_from_a_crash_are_cleared() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let tmp = staged("kluris-acme", 1, "tmp");
    let old = staged("kluris-acme", 2, "old");
    let half = staged("kluris-acme", 3, "tmp");
    put(
        &s.h(".claude/skills").join(&tmp).join("SKILL.md"),
        &v2_skill("acme"),
    );
    put(
        &s.h(".claude/skills").join(&old).join("SKILL.md"),
        &v2_skill("acme"),
    );
    put(
        &s.h(".claude/skills").join(&half).join("SKILL.md"),
        "half written",
    );
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(!s.h(".claude/skills").join(&tmp).exists());
    assert!(!s.h(".claude/skills").join(&old).exists());
    assert_eq!(
        fs::read_to_string(s.h(".claude/skills").join(&half).join("SKILL.md")).expect("kept"),
        "half written"
    );
    let claude = dest(&report, &s.h(".claude/skills"));
    assert_eq!(claude.removed, sorted(&[&tmp, &old]));
    assert_eq!(claude.kept, std::slice::from_ref(&half));
    assert!(s.h(".claude/skills/kluris-acme/SKILL.md").is_file());
}

fn sorted(names: &[&String]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    v.sort();
    v
}

fn dest<'a>(report: &'a InstallReport, path: &Path) -> &'a DestinationReport {
    report
        .destinations
        .iter()
        .find(|d| d.path == path)
        .unwrap_or_else(|| panic!("no report for {}: {report:?}", path.display()))
}

/// Staging folders an earlier run left next to skills that no longer
/// exist (a removed brain, or a folder Kluris no longer writes) are
/// removed and reported, in written and cleared folders alike.
#[test]
fn stale_staging_folders_of_gone_skills_are_cleared() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let gone_tmp = staged("kluris-gone", 1, "tmp");
    let gone_old = staged("kluris-gone", 2, "old");
    let acme_tmp = staged("kluris-acme", 3, "tmp");
    put(
        &s.h(".agents/skills").join(&gone_tmp).join("SKILL.md"),
        &v2_skill("gone"),
    );
    put(
        &s.h(".agents/skills").join(&gone_old).join("SKILL.md"),
        &v2_skill("gone"),
    );
    put(
        &s.h(".cursor/skills").join(&acme_tmp).join("SKILL.md"),
        &v2_skill("acme"),
    );
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    for path in [
        s.h(".agents/skills").join(&gone_tmp),
        s.h(".agents/skills").join(&gone_old),
        s.h(".cursor/skills").join(&acme_tmp),
    ] {
        assert!(!path.exists(), "{}", path.display());
    }
    assert_eq!(
        dest(&report, &s.h(".agents/skills")).removed,
        sorted(&[&gone_tmp, &gone_old])
    );
    assert_eq!(dest(&report, &s.h(".cursor/skills")).removed, [acme_tmp]);
    assert!(s.h(".claude/skills/kluris-acme/SKILL.md").is_file());
}

/// Only a real folder holding nothing but (at most) a `SKILL.md` file is a
/// staging folder; anything else with such a name is kept.
#[test]
fn staging_like_entries_that_are_not_kluris_are_kept() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(&s.h(".agents/skills/.kluris-a.tmp/notes.txt"), "mine");
    put(&s.h(".agents/skills/.kluris-b.tmp"), "a file");
    put(
        &s.h(".agents/skills/.kluris-c.old/SKILL.md/inner"),
        "a folder named SKILL.md",
    );
    put(&s.h(".agents/skills/.kluris-d/SKILL.md"), "no suffix");
    put(
        &s.h(".agents/skills/.kluris-e.tmpx/SKILL.md"),
        "other suffix",
    );
    #[cfg(unix)]
    {
        fs::create_dir_all(s.root.join("target")).expect("target");
        std::os::unix::fs::symlink(s.root.join("target"), s.h(".agents/skills/.kluris-f.old"))
            .expect("symlink");
    }
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    for rel in [
        ".agents/skills/.kluris-a.tmp/notes.txt",
        ".agents/skills/.kluris-b.tmp",
        ".agents/skills/.kluris-c.old/SKILL.md/inner",
        ".agents/skills/.kluris-d/SKILL.md",
        ".agents/skills/.kluris-e.tmpx/SKILL.md",
    ] {
        assert!(s.h(rel).exists(), "{rel}");
    }
    #[cfg(unix)]
    assert!(fs::symlink_metadata(s.h(".agents/skills/.kluris-f.old")).is_ok());
    assert!(report.destinations[0].removed.is_empty(), "{report:?}");
    // v2's fixed staging names are near misses; `.kluris-d` and
    // `.kluris-e.tmpx` are not staging names at all.
    let mut near = vec![".kluris-a.tmp", ".kluris-b.tmp", ".kluris-c.old"];
    if cfg!(unix) {
        near.push(".kluris-f.old");
    }
    assert_eq!(report.destinations[0].kept, near);
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
    // v1's bare `kluris` skill, as v1.5.1 rendered it.
    put(&s.h(".cursor/skills/kluris/SKILL.md"), &legacy("v1.5.1.md"));
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

/// The report's absolute paths are in the OS's own separators: the
/// registry's `/`-relative folder names are joined under HOME component by
/// component, and a collision's message names the entry that way
/// (SPEC 2026-10-27; PAR-054's JSON `path` and the `~/` labels stay
/// `/`-joined).
#[test]
fn destination_paths_and_failure_messages_are_os_native() {
    // A collision in the universal folder: the failed message names the
    // entry's absolute path.
    let mut s = Sandbox::new();
    s.brain("acme", None);
    put(&s.h(".agents/skills/kluris-acme/SKILL.md"), FOREIGN);
    let report = s.install();
    let universal = report
        .destinations
        .iter()
        .find(|d| d.role == DestinationRole::Universal)
        .expect("universal destination");
    let skills_dir = s.home.join(".agents").join("skills");
    assert_eq!(universal.path.as_os_str(), skills_dir.as_os_str());
    let blocked = skills_dir.join("kluris-acme");
    assert!(
        universal
            .failed
            .iter()
            .any(|m| m.contains(&blocked.display().to_string())),
        "{:?}",
        universal.failed
    );

    // A migration folder (its registry name is a `/`-string) reports a
    // native path too.
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".cursor/skills/kluris-gone/SKILL.md"),
        &v2_skill("gone"),
    );
    let clean = s.install();
    let migrated = clean
        .destinations
        .iter()
        .find(|d| d.role == DestinationRole::Migration)
        .expect("migration destination");
    assert_eq!(
        migrated.path.as_os_str(),
        s.home.join(".cursor").join("skills").as_os_str()
    );

    #[cfg(windows)]
    for dest in report.destinations.iter().chain(clean.destinations.iter()) {
        assert!(
            !dest.path.to_string_lossy().contains('/'),
            "{}",
            dest.path.display()
        );
    }
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

// --- ownership: only what Kluris wrote is removed (T-103 #5-#7) -------------

/// T-103 `description-prefix`: a user's own `kluris-notes` whose
/// description opens like Kluris's is kept, in written, deselected and v2
/// folders alike, and reported as left in place.
#[test]
fn a_personal_skill_whose_description_opens_like_kluris_is_kept() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let own = "---\nname: kluris-notes\ndescription: \"You are the team's subject matter expert on my personal notes\"\n---\n\n# Notes\n";
    let dirs = [
        ".agents/skills",
        ".claude/skills",
        ".hermes/skills",
        ".codex/skills",
    ];
    for dir in dirs {
        put(&s.h(dir).join("kluris-notes/SKILL.md"), own);
    }
    put(&s.h(".agents/skills/kluris-notes/private.md"), "my notes");
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    for dir in dirs {
        assert_eq!(
            fs::read_to_string(s.h(dir).join("kluris-notes/SKILL.md")).expect(dir),
            own
        );
        assert_eq!(dest(&report, &s.h(dir)).kept, ["kluris-notes"], "{dir}");
        assert!(dest(&report, &s.h(dir)).removed.is_empty(), "{dir}");
    }
    assert!(s.h(".agents/skills/kluris-notes/private.md").is_file());
}

/// T-103 `foreign-staging`: entries named like v2's or an older v3's fixed
/// staging folders (`.kluris-<brain>.tmp` / `.old`) are not this run's or
/// any run's: kept, whatever they hold, and reported. A case variant
/// (`.KLURIS-ACME.OLD`) is not even looked at.
#[test]
fn staging_lookalikes_are_kept_and_reported() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let skills = s.h(".agents/skills");
    put(&skills.join(".kluris-acme.tmp/SKILL.md"), &v2_skill("acme"));
    put(&skills.join(".kluris-acme.old/SKILL.md"), "my draft");
    put(
        &skills.join(".kluris-notes.old/SKILL.md"),
        &v2_skill("notes"),
    );
    put(&skills.join(".KLURIS-ACME.OLD/my-file"), "mine");
    let before = s.files();
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    let after = s.files();
    for rel in [
        ".agents/skills/.kluris-acme.tmp/SKILL.md",
        ".agents/skills/.kluris-acme.old/SKILL.md",
        ".agents/skills/.kluris-notes.old/SKILL.md",
    ] {
        assert!(before.contains_key(rel), "{rel}");
        assert_eq!(after.get(rel), before.get(rel), "{rel}");
    }
    // On a case-insensitive file system this is the folder above.
    assert_eq!(
        fs::read_to_string(skills.join(".KLURIS-ACME.OLD/my-file")).expect("kept"),
        "mine"
    );
    let universal = dest(&report, &skills);
    assert!(universal.removed.is_empty(), "{universal:?}");
    assert_eq!(
        universal.kept,
        [".kluris-acme.old", ".kluris-acme.tmp", ".kluris-notes.old"]
    );
}

/// A v3 staging name holding anything but one Kluris-written SKILL.md is
/// kept: other files, a SKILL.md that is a folder, a token that is not a
/// pid and 16 hex digits.
#[test]
fn v3_staging_names_holding_anything_else_are_kept() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let skills = s.h(".agents/skills");
    let extra = staged("kluris-gone", 1, "old");
    put(&skills.join(&extra).join("SKILL.md"), &v2_skill("gone"));
    put(&skills.join(&extra).join("notes.txt"), "mine");
    let other = staged("kluris-gone", 2, "tmp");
    put(&skills.join(&other).join("SKILL.md"), &v2_skill("other"));
    let bad_token = ".kluris-gone.4242-xyz.tmp".to_string();
    put(&skills.join(&bad_token).join("SKILL.md"), &v2_skill("gone"));
    let upper = ".kluris-gone.4242-00000000000000AB.tmp".to_string();
    put(&skills.join(&upper).join("SKILL.md"), &v2_skill("gone"));
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    for name in [&extra, &other, &bad_token, &upper] {
        assert!(skills.join(name).join("SKILL.md").is_file(), "{name}");
    }
    let universal = dest(&report, &skills);
    assert!(universal.removed.is_empty(), "{universal:?}");
    assert_eq!(
        universal.kept,
        sorted(&[&extra, &other, &bad_token, &upper])
    );
}

/// A skill folder holding anything besides SKILL.md is not Kluris's: kept
/// where Kluris would replace it (reported as not written) and where it
/// would remove it.
#[test]
fn a_skill_folder_holding_more_than_skill_md_is_kept() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    for dir in [".claude/skills", ".cursor/skills"] {
        put(&s.h(dir).join("kluris-acme/SKILL.md"), &v2_skill("acme"));
        put(&s.h(dir).join("kluris-acme/extra.txt"), "left by someone");
    }
    let report = s.install();
    for dir in [".claude/skills", ".cursor/skills"] {
        assert_eq!(
            fs::read_to_string(s.h(dir).join("kluris-acme/SKILL.md")).expect(dir),
            v2_skill("acme")
        );
        assert!(s.h(dir).join("kluris-acme/extra.txt").is_file(), "{dir}");
    }
    let claude = dest(&report, &s.h(".claude/skills"));
    assert!(claude.written.is_empty(), "{claude:?}");
    assert_eq!(claude.failed.len(), 1, "{claude:?}");
    assert_eq!(dest(&report, &s.h(".cursor/skills")).kept, ["kluris-acme"]);
}

/// Staging folders get a new name on every run: `.kluris-<brain>.<pid>-
/// <16 hex>.tmp`, so no user folder can be taken for one.
#[test]
fn staging_names_are_unique_per_run() {
    use std::cell::RefCell;
    struct Spy(RefCell<Vec<String>>);
    impl FileOps for Spy {
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            if to.file_name().is_some_and(|n| n == "kluris-acme") {
                let name = from.file_name().expect("name").to_string_lossy();
                self.0.borrow_mut().push(name.into_owned());
            }
            RealFs.rename(from, to)
        }
    }
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    let spy = Spy(RefCell::new(Vec::new()));
    assert!(s.install_with(&spy).ok());
    assert!(s.install_with(&spy).ok());
    let names = spy.0.into_inner();
    let pattern = regex::Regex::new(&format!(
        r"^\.kluris-acme\.{}-[0-9a-f]{{16}}\.tmp$",
        std::process::id()
    ))
    .expect("regex");
    assert!(names.len() >= 2, "{names:?}");
    assert!(names.iter().all(|n| pattern.is_match(n)), "{names:?}");
    let unique: std::collections::BTreeSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
}

/// The file system's own answer to "is this name taken" for a name that
/// differs only in case: true on APFS and NTFS, false on Linux.
fn case_insensitive(dir: &Path) -> bool {
    let probe = dir.join("Case-Probe");
    fs::create_dir_all(&probe).expect("probe");
    let insensitive = dir.join("case-probe").exists();
    fs::remove_dir(&probe).expect("probe");
    insensitive
}

/// A user's `KLURIS-ACME` folder is never taken for `kluris-acme`: on a
/// case-insensitive file system Kluris cannot write its skill there
/// without touching the user's, so that destination fails; on Linux both
/// exist side by side.
#[test]
fn a_case_variant_of_the_skill_folder_is_kept() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let mine = "---\nname: KLURIS-ACME\ndescription: mine\n---\n";
    put(&s.h(".claude/skills/KLURIS-ACME/SKILL.md"), mine);
    put(&s.h(".cursor/skills/KLURIS-ACME/SKILL.md"), mine);
    let report = s.install();
    for dir in [".claude/skills", ".cursor/skills"] {
        let names: Vec<String> = fs::read_dir(s.h(dir))
            .expect(dir)
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.contains(&"KLURIS-ACME".to_string()),
            "{dir}: {names:?}"
        );
        assert_eq!(
            fs::read_to_string(s.h(dir).join("KLURIS-ACME/SKILL.md")).expect(dir),
            mine
        );
    }
    let claude = dest(&report, &s.h(".claude/skills"));
    if case_insensitive(&s.home) {
        assert!(claude.written.is_empty(), "{claude:?}");
        assert_eq!(claude.failed.len(), 1, "{claude:?}");
        assert!(
            claude.failed[0].contains("move or delete it, then run `kluris doctor`"),
            "{}",
            claude.failed[0]
        );
    } else {
        assert_eq!(claude.written, ["kluris-acme"]);
    }
}

/// On a case-insensitive file system, `COPILOT_HOME=~/.Claude` names the
/// same folder as `~/.claude`: it is written once and reported as an
/// alias. On Linux they are two folders.
#[test]
fn a_case_variant_destination_is_the_same_folder() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude", "copilot"]));
    s.copilot_home = Some(s.h(".Claude"));
    fs::create_dir_all(s.h(".claude/skills")).expect("claude");
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    let copilot = dest(&report, &s.h(".Claude/skills"));
    if case_insensitive(&s.home) {
        assert_eq!(
            copilot.alias_of.as_deref(),
            Some(s.h(".claude/skills").as_path())
        );
        assert!(copilot.written.is_empty(), "{copilot:?}");
    } else {
        assert_eq!(copilot.alias_of, None);
        assert_eq!(copilot.written, ["kluris-acme"]);
    }
    assert!(s.h(".claude/skills/kluris-acme/SKILL.md").is_file());
}

#[cfg(unix)]
fn link(target: &str, at: &Path) {
    fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
    std::os::unix::fs::symlink(target, at).expect("symlink");
}

/// T-103 `aliased-skill-dir`: `~/.codex/skills -> ../.agents/skills` is the
/// universal folder; the migration skips it (an alias) instead of removing
/// the skill it just wrote.
#[cfg(unix)]
#[test]
fn a_v2_folder_linked_to_the_universal_one_keeps_the_new_skill() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    fs::create_dir_all(s.h(".agents/skills")).expect("agents");
    link("../.agents/skills", &s.h(".codex/skills"));
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());
    let codex = dest(&report, &s.h(".codex/skills"));
    assert_eq!(
        codex.alias_of.as_deref(),
        Some(s.h(".agents/skills").as_path())
    );
    assert!(codex.removed.is_empty(), "{codex:?}");
}

/// A selected agent's folder linked to the universal one is written once;
/// a deselected one linked to it is not cleared.
#[cfg(unix)]
#[test]
fn agent_folders_linked_to_the_universal_one_are_aliases() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    fs::create_dir_all(s.h(".agents/skills")).expect("agents");
    link("../.agents/skills", &s.h(".claude/skills"));
    link("../.agents/skills", &s.h(".hermes/skills"));
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());
    for dir in [".claude/skills", ".hermes/skills"] {
        let d = dest(&report, &s.h(dir));
        assert_eq!(
            d.alias_of.as_deref(),
            Some(s.h(".agents/skills").as_path()),
            "{dir}"
        );
        assert!(d.written.is_empty() && d.removed.is_empty(), "{d:?}");
    }
    assert_eq!(report.total_written(), 1);
}

/// `self-uninstall --purge` clears a folder once, however many names it has.
#[cfg(unix)]
#[test]
fn purge_skips_linked_folders() {
    let s = Sandbox::new();
    put(
        &s.h(".agents/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    link("../.agents/skills", &s.h(".codex/skills"));
    let reports = remove_generated_skills_with(&s.home, None, &RealFs);
    let universal = reports
        .iter()
        .find(|d| d.path == s.h(".agents/skills"))
        .expect("universal");
    assert_eq!(universal.removed, ["kluris-acme"]);
    let codex = reports
        .iter()
        .find(|d| d.path == s.h(".codex/skills"))
        .expect("codex");
    assert_eq!(
        codex.alias_of.as_deref(),
        Some(s.h(".agents/skills").as_path())
    );
    assert!(
        codex.failed.is_empty() && codex.removed.is_empty(),
        "{codex:?}"
    );
}

/// A `kluris-*` entry that is a link is never followed for deletion: the
/// link goes when it points at a Kluris-written skill (whose folder stays),
/// and stays otherwise.
#[cfg(unix)]
#[test]
fn a_linked_skill_entry_is_unlinked_never_followed() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let elsewhere = s.root.join("elsewhere");
    put(&elsewhere.join("kluris-gone/SKILL.md"), &v2_skill("gone"));
    put(&elsewhere.join("kluris-mine/SKILL.md"), &v2_skill("mine"));
    put(&elsewhere.join("kluris-mine/notes.txt"), "mine");
    let gone = elsewhere.join("kluris-gone");
    let mine = elsewhere.join("kluris-mine");
    link(
        gone.to_str().expect("utf-8"),
        &s.h(".codex/skills/kluris-gone"),
    );
    link(
        mine.to_str().expect("utf-8"),
        &s.h(".codex/skills/kluris-mine"),
    );
    link(
        "/nonexistent/kluris-dangling",
        &s.h(".codex/skills/kluris-dangling"),
    );
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(fs::symlink_metadata(s.h(".codex/skills/kluris-gone")).is_err());
    assert!(gone.join("SKILL.md").is_file(), "the link's target stays");
    assert!(fs::symlink_metadata(s.h(".codex/skills/kluris-mine")).is_ok());
    assert!(mine.join("notes.txt").is_file());
    assert!(fs::symlink_metadata(s.h(".codex/skills/kluris-dangling")).is_ok());
    let codex = dest(&report, &s.h(".codex/skills"));
    assert_eq!(codex.removed, ["kluris-gone"]);
    // The dangling link carries nothing of Kluris's: not listed.
    assert_eq!(codex.kept, ["kluris-mine"]);
}

/// In a folder Kluris writes, a `kluris-<brain>` link to a Kluris-written
/// skill is replaced by a real copy; the link's target is not touched.
#[cfg(unix)]
#[test]
fn a_linked_skill_entry_in_a_written_folder_is_replaced_by_a_copy() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".agents/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    link(
        "../../.agents/skills/kluris-acme",
        &s.h(".claude/skills/kluris-acme"),
    );
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    let meta = fs::symlink_metadata(s.h(".claude/skills/kluris-acme")).expect("entry");
    assert!(meta.is_dir(), "a real folder now");
    let universal = fs::read(s.h(".agents/skills/kluris-acme/SKILL.md")).expect("universal");
    assert_eq!(
        fs::read(s.h(".claude/skills/kluris-acme/SKILL.md")).expect("claude"),
        universal
    );
    assert_eq!(
        dest(&report, &s.h(".claude/skills")).written,
        ["kluris-acme"]
    );
}

/// The file operations of a run in which `target` is replaced by
/// `replacement` right before Kluris renames it (T-103 `faults.c`:
/// "replace-after-check"); with `fail_back`, putting it back fails too.
struct SwapBeforeRename {
    target: PathBuf,
    replacement: PathBuf,
    saved: PathBuf,
    fired: std::cell::Cell<bool>,
    fail_back: bool,
}

impl FileOps for SwapBeforeRename {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        if !self.fired.get() && from == self.target {
            self.fired.set(true);
            fs::rename(&self.target, &self.saved).expect("swap out");
            fs::rename(&self.replacement, &self.target).expect("swap in");
        } else if self.fail_back && self.fired.get() && to == self.target {
            return Err(std::io::Error::other("injected: cannot put back"));
        }
        RealFs.rename(from, to)
    }
}

fn swap(s: &Sandbox, target: PathBuf, fail_back: bool) -> SwapBeforeRename {
    let replacement = s.root.join("user-folder");
    put(&replacement.join("valuable.txt"), "irreplaceable");
    SwapBeforeRename {
        target,
        replacement,
        saved: s.root.join("checked-folder"),
        fired: std::cell::Cell::new(false),
        fail_back,
    }
}

/// T-103 #7 in the sweep: a removed brain's skill swapped for a user's
/// folder between the check and the removal is put back, not deleted.
#[test]
fn an_entry_swapped_after_its_check_is_put_back() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".agents/skills/kluris-gone/SKILL.md"),
        &v2_skill("gone"),
    );
    let ops = swap(&s, s.h(".agents/skills/kluris-gone"), false);
    let report = s.install_with(&ops);
    assert!(ops.fired.get(), "the swap ran");
    assert_eq!(
        fs::read_to_string(s.h(".agents/skills/kluris-gone/valuable.txt")).expect("put back"),
        "irreplaceable"
    );
    let universal = dest(&report, &s.h(".agents/skills"));
    assert!(universal.removed.is_empty(), "{universal:?}");
    assert_eq!(universal.kept, ["kluris-gone"]);
}

/// T-103 #7 when replacing: an old copy swapped for a user's folder right
/// before it is moved aside is put back; the skill is reported not written.
#[test]
fn a_target_swapped_after_its_check_is_put_back() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".claude/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    let ops = swap(&s, s.h(".claude/skills/kluris-acme"), false);
    let report = s.install_with(&ops);
    assert!(ops.fired.get(), "the swap ran");
    assert_eq!(
        fs::read_to_string(s.h(".claude/skills/kluris-acme/valuable.txt")).expect("put back"),
        "irreplaceable"
    );
    let claude = dest(&report, &s.h(".claude/skills"));
    assert!(claude.written.is_empty(), "{claude:?}");
    assert_eq!(claude.failed.len(), 1, "{claude:?}");
    assert!(!report.ok());
}

/// When the swapped folder cannot be put back it stays under its
/// quarantine name, which the report names; nothing is deleted.
#[test]
fn a_swapped_entry_that_cannot_be_put_back_stays_in_quarantine() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".agents/skills/kluris-gone/SKILL.md"),
        &v2_skill("gone"),
    );
    let ops = swap(&s, s.h(".agents/skills/kluris-gone"), true);
    let report = s.install_with(&ops);
    assert!(ops.fired.get(), "the swap ran");
    let quarantined: Vec<PathBuf> = fs::read_dir(s.h(".agents/skills"))
        .expect("read")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.join("valuable.txt").is_file())
        .collect();
    assert_eq!(quarantined.len(), 1, "{quarantined:?}");
    let name = quarantined[0]
        .file_name()
        .expect("name")
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with(".kluris-gone.") && name.ends_with(".old"),
        "{name}"
    );
    let universal = dest(&report, &s.h(".agents/skills"));
    assert!(universal.removed.is_empty(), "{universal:?}");
    assert!(
        universal.failed.iter().any(|f| f.contains(&name)),
        "{universal:?}"
    );
    assert!(!report.ok());
}

/// A removal whose first move fails leaves the entry where it is.
#[test]
fn a_quarantine_that_cannot_start_leaves_the_entry() {
    struct NoRename;
    impl FileOps for NoRename {
        fn rename(&self, from: &Path, _to: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other(format!(
                "injected: {}",
                from.display()
            )))
        }
    }
    let s = Sandbox::new();
    put(
        &s.h(".agents/skills/kluris-gone/SKILL.md"),
        &v2_skill("gone"),
    );
    let reports = remove_generated_skills_with(&s.home, None, &NoRename);
    assert!(s.h(".agents/skills/kluris-gone/SKILL.md").is_file());
    let universal = reports
        .iter()
        .find(|d| d.path == s.h(".agents/skills"))
        .expect("universal");
    assert!(universal.removed.is_empty());
    assert_eq!(universal.failed.len(), 1, "{universal:?}");
}

// --- T-105 follow-up (chief's decisions m-4606) -----------------------------

/// Finder's `.DS_Store`, Explorer's `Thumbs.db` and `desktop.ini` (exact
/// names, regular files) do not make a Kluris folder someone else's: it is
/// replaced and removed with them.
#[test]
fn os_metadata_files_go_with_a_kluris_folder() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    put(
        &s.h(".claude/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    put(&s.h(".claude/skills/kluris-acme/.DS_Store"), "finder");
    put(
        &s.h(".cursor/skills/kluris-acme/SKILL.md"),
        &v2_skill("acme"),
    );
    put(&s.h(".cursor/skills/kluris-acme/Thumbs.db"), "explorer");
    put(
        &s.h(".cursor/skills/kluris-acme/desktop.ini"),
        "[.ShellClassInfo]",
    );
    put(
        &s.h(".agents/skills/kluris-gone/SKILL.md"),
        &v2_skill("gone"),
    );
    put(&s.h(".agents/skills/kluris-gone/.DS_Store"), "finder");
    let stale = staged("kluris-gone", 7, "old");
    put(
        &s.h(".agents/skills").join(&stale).join("SKILL.md"),
        &v2_skill("gone"),
    );
    put(
        &s.h(".agents/skills").join(&stale).join(".DS_Store"),
        "finder",
    );
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    // Replaced: the new folder holds the new SKILL.md only.
    let names: Vec<String> = fs::read_dir(s.h(".claude/skills/kluris-acme"))
        .expect("claude")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["SKILL.md"]);
    assert_eq!(
        fs::read(s.h(".claude/skills/kluris-acme/SKILL.md")).expect("claude"),
        fs::read(s.h(".agents/skills/kluris-acme/SKILL.md")).expect("universal")
    );
    assert_eq!(
        dest(&report, &s.h(".claude/skills")).written,
        ["kluris-acme"]
    );
    // Swept.
    assert!(!s.h(".cursor/skills/kluris-acme").exists());
    assert!(!s.h(".agents/skills/kluris-gone").exists());
    assert!(!s.h(".agents/skills").join(&stale).exists());
    assert_eq!(
        dest(&report, &s.h(".cursor/skills")).removed,
        ["kluris-acme"]
    );
    assert_eq!(
        dest(&report, &s.h(".agents/skills")).removed,
        sorted(&[&stale, &"kluris-gone".to_string()])
    );
}

/// Only those exact names, as regular files, next to the one SKILL.md.
#[test]
fn other_files_beside_skill_md_still_keep_the_folder() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let skills = s.h(".cursor/skills");
    put(&skills.join("kluris-a/SKILL.md"), &v2_skill("a"));
    put(&skills.join("kluris-a/.DS_Store/inner"), "a folder");
    put(&skills.join("kluris-b/SKILL.md"), &v2_skill("b"));
    put(&skills.join("kluris-b/thumbs.db"), "wrong case");
    put(&skills.join("kluris-c/SKILL.md"), &v2_skill("c"));
    put(&skills.join("kluris-c/Thumbs.db"), "explorer");
    put(&skills.join("kluris-c/notes.txt"), "mine");
    put(&skills.join("kluris-d/.DS_Store"), "no SKILL.md");
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    for rel in [
        "kluris-a/.DS_Store/inner",
        "kluris-b/thumbs.db",
        "kluris-c/notes.txt",
        "kluris-d/.DS_Store",
    ] {
        assert!(skills.join(rel).is_file(), "{rel}");
    }
    let cursor = dest(&report, &skills);
    assert!(cursor.removed.is_empty(), "{cursor:?}");
    // kluris-d has no SKILL.md, so nothing of Kluris's: not listed.
    assert_eq!(cursor.kept, ["kluris-a", "kluris-b", "kluris-c"]);
}

/// `kept` lists near misses only: a `kluris*` entry whose SKILL.md (or
/// workflow) carries the marker's opening or a v1/v2 description opening.
/// Entries with nothing of Kluris's are left alone and not listed.
#[test]
fn only_near_misses_are_listed_as_kept() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let skills = s.h(".cursor/skills");
    put(
        &skills.join("kluris-marked/SKILL.md"),
        "---\nname: kluris-marked\ndescription: x\n---\n\n# Mine\n\n<!-- kluris:generated -->\n",
    );
    put(
        &skills.join("kluris-v1/SKILL.md"),
        "---\nname: kluris-v1\ndescription: Your team's shared AI brain, my way\n---\n",
    );
    put(
        &skills.join("kluris-sme/SKILL.md"),
        "---\nname: kluris-sme\ndescription: \"You are the team's subject matter expert on cats\"\n---\n",
    );
    put(&skills.join("kluris-plain/SKILL.md"), FOREIGN);
    put(&skills.join("kluris-file"), "a file");
    let workflows = s.h(".codeium/windsurf/global_workflows");
    put(
        &workflows.join("kluris-old.md"),
        "---\ndescription: You are the team's subject matter expert, edited\n---\n",
    );
    put(&workflows.join("kluris-notes.md"), "# my own workflow\n");
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert_eq!(
        dest(&report, &skills).kept,
        ["kluris-marked", "kluris-sme", "kluris-v1"]
    );
    assert_eq!(dest(&report, &workflows).kept, ["kluris-old.md"]);
    for rel in [
        "kluris-marked",
        "kluris-v1",
        "kluris-sme",
        "kluris-plain",
        "kluris-file",
    ] {
        assert!(skills.join(rel).exists(), "{rel}");
    }
}

/// A same-named entry that is not proven Kluris's (here v2's copy with a
/// file of the user's beside it) is never overwritten and never passes
/// silently: the folder reports the skill as not written, says how to fix
/// it, and lists the entry as kept.
#[test]
fn a_same_named_entry_in_the_way_of_a_skill_is_not_overwritten() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    for dir in [".agents/skills", ".claude/skills"] {
        put(&s.h(dir).join("kluris-acme/SKILL.md"), &v2_skill("acme"));
        put(&s.h(dir).join("kluris-acme/mine.txt"), "mine");
    }
    let report = s.install();
    assert!(!report.ok());
    for dir in [".agents/skills", ".claude/skills"] {
        assert_eq!(
            fs::read_to_string(s.h(dir).join("kluris-acme/SKILL.md")).expect(dir),
            v2_skill("acme")
        );
        assert!(s.h(dir).join("kluris-acme/mine.txt").is_file());
        let d = dest(&report, &s.h(dir));
        assert!(d.written.is_empty(), "{d:?}");
        assert_eq!(d.kept, ["kluris-acme"], "{dir}");
        assert_eq!(d.failed.len(), 1, "{d:?}");
        let path = s.h(dir).join("kluris-acme").display().to_string();
        assert!(d.failed[0].contains(&path), "{}", d.failed[0]);
        assert!(
            d.failed[0].contains("move or delete it, then run `kluris doctor`"),
            "{}",
            d.failed[0]
        );
    }
    assert_eq!(report.total_written(), 0);
}

/// A Windows junction made the way users make them (`mklink /J`, no admin
/// rights), from a Sandbox path (`\\?\` dropped for `cmd`).
#[cfg(windows)]
fn junction(target: &Path, at: &Path) {
    let plain = |p: &Path| {
        p.display()
            .to_string()
            .trim_start_matches(r"\\?\")
            .to_string()
    };
    fs::create_dir_all(at.parent().expect("parent")).expect("mkdir");
    let out = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(plain(at))
        .arg(plain(target))
        .output()
        .expect("cmd");
    assert!(
        out.status.success(),
        "mklink /J: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `~/.codex/skills` junctioned to `~/.agents/skills` is one folder: the
/// migration skips it as an alias instead of removing the new skill.
#[cfg(windows)]
#[test]
fn a_junctioned_v2_folder_is_an_alias_of_the_universal_one() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&[]));
    fs::create_dir_all(s.h(".agents/skills")).expect("agents");
    junction(&s.h(".agents/skills"), &s.h(".codex/skills"));
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(s.h(".agents/skills/kluris-acme/SKILL.md").is_file());
    let codex = dest(&report, &s.h(".codex/skills"));
    assert_eq!(
        codex.alias_of.as_deref(),
        Some(s.h(".agents/skills").as_path())
    );
    assert!(codex.removed.is_empty(), "{codex:?}");
}

/// A junctioned `kluris-<brain>` entry is removed as a junction only, and
/// only when it points at a Kluris skill; its target stays.
#[cfg(windows)]
#[test]
fn a_junctioned_skill_entry_is_unlinked_never_followed() {
    let mut s = Sandbox::new();
    s.brain("acme", Some(&["claude"]));
    let elsewhere = s.root.join("elsewhere");
    put(&elsewhere.join("kluris-gone/SKILL.md"), &v2_skill("gone"));
    put(&elsewhere.join("kluris-mine/SKILL.md"), &v2_skill("mine"));
    put(&elsewhere.join("kluris-mine/notes.txt"), "mine");
    junction(
        &elsewhere.join("kluris-gone"),
        &s.h(".codex/skills/kluris-gone"),
    );
    junction(
        &elsewhere.join("kluris-mine"),
        &s.h(".codex/skills/kluris-mine"),
    );
    let report = s.install();
    assert!(report.ok(), "{report:?}");
    assert!(fs::symlink_metadata(s.h(".codex/skills/kluris-gone")).is_err());
    assert!(
        elsewhere.join("kluris-gone/SKILL.md").is_file(),
        "the target stays"
    );
    assert!(fs::symlink_metadata(s.h(".codex/skills/kluris-mine")).is_ok());
    assert!(elsewhere.join("kluris-mine/notes.txt").is_file());
    let codex = dest(&report, &s.h(".codex/skills"));
    assert_eq!(codex.removed, ["kluris-gone"]);
    assert_eq!(codex.kept, ["kluris-mine"]);
}
