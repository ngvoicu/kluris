//! Installing every registered brain's skill into the agents' skill folders
//! (v2 `_do_install`, with the `~/.agents/skills`-first matrix and the
//! narrowed sweep: `parity/allowlist.md`).
//!
//! 1. `~/.agents/skills` always gets one `kluris-<brain>/SKILL.md` per brain.
//! 2. Each selected registry agent's own folders get the same bytes
//!    ([`registry::skill_folders`]). The selection is the union of
//!    `commands_for` over the brains with a `kluris.yml`; an empty union
//!    selects the whole registry. Only selected folders are ever created.
//! 3. Each written folder is staged then renamed, skill by skill: a failure
//!    leaves that folder's old skills in place and is reported for it.
//!    Kluris-written skills of brains no longer registered are removed.
//! 4. The folders of deselected registry agents lose every Kluris-written
//!    skill (the folders stay).
//! 5. Once `~/.agents/skills` is written, the v2 migration removes the
//!    Kluris-written skills and workflows v2 left in folders v3 no longer
//!    uses ([`registry::V2_ONLY_SKILL_FOLDERS`], `~/.codeium/windsurf*`).
//!
//! "Kluris-written" is [`render::is_generated_skill`] /
//! [`render::is_generated_workflow`]: a user's own `kluris-notes` skill, or
//! anything else in these folders, is never touched.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::registry::{self, REGISTRY};
use super::render::{self, SkillInput};
use crate::companions;
use crate::config::{BrainConfig, ConfigError, ConfigErrorKind, GlobalConfig, read_brain_config};

/// What [`install_skills`] works from.
#[derive(Debug, Clone, Copy)]
pub struct InstallRequest<'a> {
    pub config: &'a GlobalConfig,
    /// Each registered brain's `kluris.yml` ([`read_brain_configs`]).
    pub brains: &'a IndexMap<String, Result<BrainConfig, ConfigError>>,
    /// The home directory (`config::home_dir`).
    pub home: &'a Path,
    /// `COPILOT_HOME` when set and non-empty.
    pub copilot_home: Option<&'a Path>,
}

/// Why a folder appears in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationRole {
    /// `~/.agents/skills`.
    Universal,
    /// A folder of a selected registry agent.
    Agent(&'static str),
    /// A folder of a registry agent that is not selected: cleared.
    Deselected(&'static str),
    /// A folder only v2 wrote to: cleared.
    Migration,
}

/// What happened in one folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationReport {
    pub path: PathBuf,
    pub role: DestinationRole,
    /// Skill names written, in brain order.
    pub written: Vec<String>,
    /// Entry names removed (skill folders, or `kluris-*.md` workflows).
    pub removed: Vec<String>,
    /// One message per failure; empty when everything worked.
    pub failed: Vec<String>,
}

impl DestinationReport {
    fn new(path: PathBuf, role: DestinationRole) -> DestinationReport {
        DestinationReport {
            path,
            role,
            written: Vec::new(),
            removed: Vec::new(),
            failed: Vec::new(),
        }
    }
}

/// The outcome of [`install_skills`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallReport {
    /// `kluris-<brain>` for every registered brain, in config order.
    pub skills: Vec<String>,
    /// The registry agents whose folders were written, sorted.
    pub agents: Vec<String>,
    /// Written folders first (universal, then agents), then cleared ones;
    /// a cleared folder is listed only when it exists.
    pub destinations: Vec<DestinationReport>,
    /// `(brain, error)` for each `kluris.yml` that exists but does not read;
    /// that brain selects no agents and renders without companions.
    pub brain_errors: Vec<(String, String)>,
}

impl InstallReport {
    /// No folder reported a failure.
    pub fn ok(&self) -> bool {
        self.destinations.iter().all(|d| d.failed.is_empty())
    }

    /// Skill copies written, over every folder.
    pub fn total_written(&self) -> usize {
        self.destinations.iter().map(|d| d.written.len()).sum()
    }

    /// `(folder, message)` for every failure.
    pub fn failures(&self) -> Vec<(PathBuf, String)> {
        self.destinations
            .iter()
            .flat_map(|d| d.failed.iter().map(|f| (d.path.clone(), f.clone())))
            .collect()
    }
}

/// Every registered brain's `kluris.yml`, in config order.
pub fn read_brain_configs(
    config: &GlobalConfig,
) -> IndexMap<String, Result<BrainConfig, ConfigError>> {
    config
        .brains
        .iter()
        .map(|(name, entry)| (name.clone(), read_brain_config(Path::new(&entry.path))))
        .collect()
}

/// Write every brain's skill where the selected agents read it and remove
/// the Kluris-written copies nothing reads any more. Never fails as a whole:
/// each folder's problems are in its report entry.
pub fn install_skills(req: &InstallRequest) -> InstallReport {
    let mut report = InstallReport::default();

    // The selection: the union of commands_for (v2), empty means all.
    let mut selected: BTreeSet<&str> = BTreeSet::new();
    for (name, brain) in req.brains {
        match brain {
            Ok(cfg) => selected.extend(cfg.agents.commands_for.iter().map(String::as_str)),
            Err(e) if e.kind == ConfigErrorKind::NotFound => {}
            Err(e) => report.brain_errors.push((name.clone(), e.to_string())),
        }
    }
    let all = selected.is_empty();
    let active: Vec<&'static str> = REGISTRY
        .iter()
        .copied()
        .filter(|a| all || selected.contains(a))
        .filter(|a| *a != "copilot" || req.copilot_home.is_some())
        .collect();
    report.agents = active.iter().map(|a| a.to_string()).collect();

    // Render once per brain: every folder gets the same bytes.
    let companion_home = req.home.join(".kluris").join("companions");
    let companion_home = companion_home.to_string_lossy();
    let skills: Vec<(String, String)> = req
        .config
        .brains
        .iter()
        .map(|(name, entry)| {
            let brain_companions = match req.brains.get(name) {
                Some(Ok(cfg)) => companions::normalize(&cfg.companions),
                _ => Vec::new(),
            };
            let input = SkillInput {
                brain_name: name,
                brain_path: &entry.path,
                has_git: Path::new(&entry.path).join(".git").exists(),
                description: &entry.description,
                companions: &brain_companions,
                companion_home: &companion_home,
            };
            (render::skill_name(name), render::render_skill(&input))
        })
        .collect();
    report.skills = skills.iter().map(|(n, _)| n.clone()).collect();

    // Written folders: universal first, then each active agent's.
    let mut written_paths: Vec<PathBuf> = Vec::new();
    let mut targets = vec![(
        req.home.join(registry::UNIVERSAL),
        DestinationRole::Universal,
    )];
    for agent in &active {
        for folder in registry::skill_folders(agent, req.home, req.copilot_home) {
            targets.push((folder, DestinationRole::Agent(agent)));
        }
    }
    let mut universal_ok = false;
    for (path, role) in targets {
        if written_paths.contains(&path) {
            continue;
        }
        let dest = write_folder(path.clone(), role.clone(), &skills);
        if role == DestinationRole::Universal {
            universal_ok = dest.failed.is_empty();
        }
        written_paths.push(path);
        report.destinations.push(dest);
    }

    // Cleared folders: deselected agents', then v2's.
    let mut cleanup: Vec<(PathBuf, DestinationRole, Kind)> = Vec::new();
    for agent in REGISTRY.iter().copied().filter(|a| !active.contains(a)) {
        for folder in registry::skill_folders(agent, req.home, req.copilot_home) {
            cleanup.push((folder, DestinationRole::Deselected(agent), Kind::Skills));
        }
    }
    // The migration waits for the universal copy, so an agent that read a
    // v2 copy always has one to read.
    if universal_ok {
        let (windsurf_skills, windsurf_workflows) = registry::windsurf_folders(req.home);
        for folder in windsurf_skills {
            cleanup.push((folder, DestinationRole::Migration, Kind::Skills));
        }
        for folder in windsurf_workflows {
            cleanup.push((folder, DestinationRole::Migration, Kind::Workflows));
        }
        for rel in registry::V2_ONLY_SKILL_FOLDERS {
            if rel == ".copilot/skills" && req.copilot_home.is_some() {
                continue;
            }
            cleanup.push((req.home.join(rel), DestinationRole::Migration, Kind::Skills));
        }
    }
    for (path, role, kind) in cleanup {
        if written_paths.contains(&path) || !path.is_dir() {
            continue;
        }
        let mut dest = DestinationReport::new(path, role);
        clear_folder(&mut dest, kind, &[]);
        written_paths.push(dest.path.clone());
        report.destinations.push(dest);
    }
    report
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `kluris*/SKILL.md` skill folders.
    Skills,
    /// `kluris*.md` Windsurf workflows.
    Workflows,
}

/// Write `skills` into `path`: create it, stage every skill, remove the
/// Kluris-written skills not in `skills`, then move each staged skill into
/// place.
fn write_folder(
    path: PathBuf,
    role: DestinationRole,
    skills: &[(String, String)],
) -> DestinationReport {
    let mut dest = DestinationReport::new(path, role);
    if let Err(e) = fs::create_dir_all(&dest.path) {
        dest.failed
            .push(format!("cannot create {}: {e}", dest.path.display()));
        return dest;
    }

    let mut staged: Vec<(&str, PathBuf)> = Vec::new();
    for (name, text) in skills {
        let staging = dest.path.join(format!(".{name}.tmp"));
        // A `.old` left by an interrupted run is the copy being replaced.
        let aside = dest.path.join(format!(".{name}.old"));
        let staged_now = match exists(&aside) {
            true => remove_entry(&aside),
            false => Ok(()),
        }
        .and_then(|()| stage(&staging, text));
        if let Err(e) = staged_now {
            dest.failed
                .push(format!("cannot stage {}: {e}", staging.display()));
            for (_, s) in &staged {
                let _ = remove_entry(s);
            }
            let _ = remove_entry(&staging);
            return dest;
        }
        staged.push((name, staging));
    }

    let keep: Vec<&str> = skills.iter().map(|(n, _)| n.as_str()).collect();
    clear_folder(&mut dest, Kind::Skills, &keep);

    for (name, staging) in staged {
        let target = dest.path.join(name);
        let aside = dest.path.join(format!(".{name}.old"));
        match place(name, &staging, &target, &aside) {
            Ok(()) => {
                dest.written.push(name.to_string());
                // The new skill is in place; only the old copy is left over.
                if exists(&aside)
                    && let Err(e) = remove_entry(&aside)
                {
                    dest.failed
                        .push(format!("cannot remove {}: {e}", aside.display()));
                }
            }
            Err(message) => {
                let _ = remove_entry(&staging);
                dest.failed.push(message);
            }
        }
    }
    dest
}

/// A fresh `staging/SKILL.md` holding `text`.
fn stage(staging: &Path, text: &str) -> io::Result<()> {
    if exists(staging) {
        remove_entry(staging)?;
    }
    fs::create_dir(staging)?;
    let file = staging.join("SKILL.md");
    fs::write(&file, text)?;
    if !file.is_file() {
        return Err(io::Error::other(format!(
            "{} was not written",
            file.display()
        )));
    }
    Ok(())
}

/// Move `staging` to `target`. An existing Kluris-written `target` is first
/// moved to `aside` (restored if the move fails; the caller removes it once
/// the new one is in place); any other entry at `target` is left as it is.
fn place(name: &str, staging: &Path, target: &Path, aside: &Path) -> Result<(), String> {
    if !exists(target) {
        return fs::rename(staging, target)
            .map_err(|e| format!("cannot move {} into place: {e}", target.display()));
    }
    if !is_generated_skill_dir(target, name) {
        return Err(format!(
            "{} exists and was not written by Kluris; left as it is",
            target.display()
        ));
    }
    if exists(aside) {
        remove_entry(aside).map_err(|e| format!("cannot remove {}: {e}", aside.display()))?;
    }
    fs::rename(target, aside).map_err(|e| format!("cannot replace {}: {e}", target.display()))?;
    fs::rename(staging, target).map_err(|e| {
        let _ = fs::rename(aside, target);
        format!("cannot move {} into place: {e}", target.display())
    })
}

/// Remove the Kluris-written `kluris*` entries of `dest.path` whose names
/// are not in `keep`, recording each in `dest`.
fn clear_folder(dest: &mut DestinationReport, kind: Kind, keep: &[&str]) {
    let entries = match fs::read_dir(&dest.path) {
        Ok(entries) => entries,
        Err(e) => {
            dest.failed
                .push(format!("cannot read {}: {e}", dest.path.display()));
            return;
        }
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with("kluris") && !keep.contains(&n.as_str()))
        .collect();
    names.sort();
    for name in names {
        let path = dest.path.join(&name);
        let generated = match kind {
            Kind::Skills => is_generated_skill_dir(&path, &name),
            Kind::Workflows => is_generated_workflow_file(&path, &name),
        };
        if !generated {
            continue;
        }
        match remove_entry(&path) {
            Ok(()) => dest.removed.push(name),
            Err(e) => dest
                .failed
                .push(format!("cannot remove {}: {e}", path.display())),
        }
    }
}

/// How much of a SKILL.md or workflow is read to recognise it: the
/// frontmatter and the marker line come first.
const HEAD_BYTES: u64 = 64 * 1024;

/// The first [`HEAD_BYTES`] of a regular file, as text; `None` for anything
/// else (a symlink included).
fn read_head(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// A real folder (not a symlink) whose SKILL.md Kluris wrote.
fn is_generated_skill_dir(path: &Path, name: &str) -> bool {
    let is_dir = fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    is_dir
        && read_head(&path.join("SKILL.md"))
            .is_some_and(|text| render::is_generated_skill(name, &text))
}

/// A regular file holding a Windsurf workflow Kluris wrote.
fn is_generated_workflow_file(path: &Path, name: &str) -> bool {
    read_head(path).is_some_and(|text| render::is_generated_workflow(name, &text))
}

/// Whether anything (a dangling symlink included) is at `path`.
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Remove a folder tree, or a file or symlink.
fn remove_entry(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}
