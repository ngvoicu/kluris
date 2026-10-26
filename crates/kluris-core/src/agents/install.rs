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
//!    skill (the folders stay). Every folder written or cleared also loses
//!    the staging folders an earlier run left (`.kluris-<brain>.<pid>-<16
//!    hex>.tmp` / `.old`) that hold a Kluris-written skill.
//! 5. Once `~/.agents/skills` is written, the v2 migration removes the
//!    Kluris-written skills and workflows v2 left in folders v3 no longer
//!    uses ([`registry::V2_ONLY_SKILL_FOLDERS`], `~/.codeium/windsurf*`).
//!
//! What may be removed (SPEC decision 2026-10-26, T-103):
//!
//! - Only an entry proven Kluris's: a folder holding nothing but a
//!   `SKILL.md` file that [`render::is_generated_skill`] recognises under
//!   the entry's own name (v3's marker, or a v1/v2 release's full
//!   template), a link to such a folder (the link goes, never what it
//!   points at), or a workflow [`render::is_generated_workflow`]
//!   recognises. Finder's and Explorer's `.DS_Store`, `Thumbs.db` and
//!   `desktop.ini` (exact names, regular files) may sit beside the
//!   `SKILL.md`; they go with the folder.
//! - Everything else is left alone. Near misses are also listed in `kept`:
//!   an entry under the name of a skill being written (which also fails
//!   that folder, with how to fix it), a staging-like name (`.kluris*.tmp`,
//!   `.old`, `.tmp.md`), and a `kluris*` entry whose SKILL.md or workflow
//!   carries a trait of Kluris's ([`render::has_kluris_trait`]). A user's
//!   own `kluris-notes` with none of these is not listed.
//! - Names come from the folder listing and are compared exactly; staging
//!   and quarantine names carry a token no other run made
//!   ([`crate::fileops::unique_token`]).
//! - An entry is first renamed to a quarantine name beside it, checked
//!   again there, and only then deleted; one that changed in between is
//!   put back (or, failing that, left under its quarantine name, which the
//!   report names).
//! - A folder is handled once whatever its names: a symlink or case
//!   variant of a folder already written or cleared is skipped and
//!   reported as its alias ([`crate::fileops::folder_id`]).

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::registry::{self, REGISTRY};
use super::render::{self, SkillInput};
use crate::companions;
use crate::config::{BrainConfig, ConfigError, ConfigErrorKind, GlobalConfig, read_brain_config};
use crate::fileops::{FileOps, FolderId, RealFs, folder_id, is_unique_token, under, unique_token};

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
    /// The folder already handled that this one is another name for (a
    /// symlink, a case variant); nothing was written or removed here.
    pub alias_of: Option<PathBuf>,
    /// Skill names written, in brain order.
    pub written: Vec<String>,
    /// Entry names removed (skill folders, or `kluris-*.md` workflows).
    pub removed: Vec<String>,
    /// Entry names left in place, sorted: `kluris*` entries (and staging
    /// names) Kluris could not prove it wrote.
    pub kept: Vec<String>,
    /// One message per failure; empty when everything worked.
    pub failed: Vec<String>,
}

impl DestinationReport {
    fn new(path: PathBuf, role: DestinationRole) -> DestinationReport {
        DestinationReport {
            path,
            role,
            alias_of: None,
            written: Vec::new(),
            removed: Vec::new(),
            kept: Vec::new(),
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
    install_skills_with(req, &RealFs)
}

/// [`install_skills`], moving and deleting through `ops`.
pub fn install_skills_with(req: &InstallRequest, ops: &dyn FileOps) -> InstallReport {
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
    let mut seen = Seen::default();
    let mut targets = vec![(
        under(req.home, registry::UNIVERSAL),
        DestinationRole::Universal,
    )];
    for agent in &active {
        for folder in registry::skill_folders(agent, req.home, req.copilot_home) {
            targets.push((folder, DestinationRole::Agent(agent)));
        }
    }
    let mut universal_ok = false;
    for (path, role) in targets {
        let Some(dest) = write_folder(path, role.clone(), &skills, ops, &mut seen) else {
            continue;
        };
        if role == DestinationRole::Universal {
            universal_ok = dest.failed.is_empty();
        }
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
            cleanup.push((
                under(req.home, rel),
                DestinationRole::Migration,
                Kind::Skills,
            ));
        }
    }
    for (path, role, kind) in cleanup {
        if let Some(dest) = clear_existing(path, role, kind, ops, &mut seen) {
            report.destinations.push(dest);
        }
    }
    report
}

/// Remove every Kluris-written skill (and Windsurf workflow) from every
/// folder Kluris has written to: `~/.agents/skills`, each registry agent's
/// folders, and the folders only v2 used (`kluris self-uninstall --purge`).
/// A user's own `kluris-*` folder is never touched. One report per folder
/// that exists, in that order.
pub fn remove_generated_skills(home: &Path, copilot_home: Option<&Path>) -> Vec<DestinationReport> {
    remove_generated_skills_with(home, copilot_home, &RealFs)
}

/// [`remove_generated_skills`], moving and deleting through `ops`.
pub fn remove_generated_skills_with(
    home: &Path,
    copilot_home: Option<&Path>,
    ops: &dyn FileOps,
) -> Vec<DestinationReport> {
    let mut seen = Seen::default();
    generated_skill_folders(home, copilot_home)
        .into_iter()
        .filter_map(|(path, role, kind)| clear_existing(path, role, kind, ops, &mut seen))
        .collect()
}

/// The entries [`remove_generated_skills`] would remove, as it finds them
/// now, changing nothing (the uninstall's pre-flight); and each path that
/// could not be looked at, with why (anything but NotFound).
pub fn generated_skill_entries(
    home: &Path,
    copilot_home: Option<&Path>,
) -> (Vec<PathBuf>, Vec<(PathBuf, io::Error)>) {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    let mut seen = Seen::default();
    for (folder, _, kind) in generated_skill_folders(home, copilot_home) {
        match fs::metadata(&folder) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                errors.push((folder, e));
                continue;
            }
        }
        if !matches!(seen.visit(&folder), Handled::New) {
            continue;
        }
        let mut names = match listing(&folder) {
            Ok(names) => names,
            Err(e) => {
                errors.push((folder, e));
                continue;
            }
        };
        names.sort();
        for name in names {
            let base = match staged_base(&name, kind) {
                Some(base) => base,
                None if !name.starts_with("kluris") => continue,
                None if kind == Kind::Workflows && !name.ends_with(".md") => continue,
                None => name.as_str(),
            };
            let path = folder.join(&name);
            let owned = match kind {
                Kind::Skills => is_owned_skill_entry(&path, base),
                Kind::Workflows => is_owned_workflow_file(&path, base),
            };
            if owned {
                entries.push(path);
            }
        }
    }
    (entries, errors)
}

/// Every folder Kluris has written skills (or Windsurf workflows) to, in
/// the order [`remove_generated_skills`] clears them.
fn generated_skill_folders(
    home: &Path,
    copilot_home: Option<&Path>,
) -> Vec<(PathBuf, DestinationRole, Kind)> {
    let mut folders = vec![(
        under(home, registry::UNIVERSAL),
        DestinationRole::Universal,
        Kind::Skills,
    )];
    for agent in REGISTRY {
        for folder in registry::skill_folders(agent, home, copilot_home) {
            folders.push((folder, DestinationRole::Agent(agent), Kind::Skills));
        }
    }
    let (windsurf_skills, windsurf_workflows) = registry::windsurf_folders(home);
    for folder in windsurf_skills {
        folders.push((folder, DestinationRole::Migration, Kind::Skills));
    }
    for folder in windsurf_workflows {
        folders.push((folder, DestinationRole::Migration, Kind::Workflows));
    }
    for rel in registry::V2_ONLY_SKILL_FOLDERS {
        folders.push((under(home, rel), DestinationRole::Migration, Kind::Skills));
    }
    folders
}

/// The folders handled so far, by physical identity.
#[derive(Default)]
struct Seen(Vec<(FolderId, PathBuf)>);

enum Handled {
    /// Not handled yet (now it is).
    New,
    /// The same path again: nothing to report.
    Again,
    /// Another name for the folder at this path.
    AliasOf(PathBuf),
}

impl Seen {
    /// Record the folder at `path` (when it is one) and say whether it was
    /// handled before.
    fn visit(&mut self, path: &Path) -> Handled {
        let Some(id) = folder_id(path) else {
            return Handled::New;
        };
        match self.0.iter().find(|(seen, _)| *seen == id) {
            Some((_, first)) if first == path => Handled::Again,
            Some((_, first)) => Handled::AliasOf(first.clone()),
            None => {
                self.0.push((id, path.to_path_buf()));
                Handled::New
            }
        }
    }
}

/// Clear the existing folder `path` (see [`clear_folder`]); `None` when
/// it is not a folder or was handled under this very path.
fn clear_existing(
    path: PathBuf,
    role: DestinationRole,
    kind: Kind,
    ops: &dyn FileOps,
    seen: &mut Seen,
) -> Option<DestinationReport> {
    if !path.is_dir() {
        return None;
    }
    let mut dest = DestinationReport::new(path, role);
    match seen.visit(&dest.path) {
        Handled::Again => return None,
        Handled::AliasOf(first) => dest.alias_of = Some(first),
        Handled::New => clear_folder(&mut dest, kind, &[], &[], ops),
    }
    Some(dest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `kluris*/SKILL.md` skill folders.
    Skills,
    /// `kluris*.md` Windsurf workflows.
    Workflows,
}

/// Write `skills` into `path`: create it, stage every skill, remove the
/// Kluris-written entries not in `skills`, then move each staged skill into
/// place. `None` when `path` was already written under this very path.
fn write_folder(
    path: PathBuf,
    role: DestinationRole,
    skills: &[(String, String)],
    ops: &dyn FileOps,
    seen: &mut Seen,
) -> Option<DestinationReport> {
    let mut dest = DestinationReport::new(path, role);
    if let Err(e) = fs::create_dir_all(&dest.path) {
        dest.failed
            .push(format!("cannot create {}: {e}", dest.path.display()));
        return Some(dest);
    }
    match seen.visit(&dest.path) {
        Handled::Again => return None,
        Handled::AliasOf(first) => {
            dest.alias_of = Some(first);
            return Some(dest);
        }
        Handled::New => {}
    }

    let mut staged: Vec<(&str, PathBuf)> = Vec::new();
    for (name, text) in skills {
        let staging = dest.path.join(format!(".{name}.{}.tmp", unique_token()));
        if let Err(e) = stage(ops, &staging, text) {
            dest.failed
                .push(format!("cannot stage {}: {e}", staging.display()));
            // Only this run's own, freshly named staging folders.
            for (_, s) in &staged {
                let _ = ops.remove_dir_all(s);
            }
            let _ = ops.remove_dir_all(&staging);
            return Some(dest);
        }
        staged.push((name, staging));
    }

    let keep: Vec<&str> = skills.iter().map(|(n, _)| n.as_str()).collect();
    let own: Vec<String> = staged
        .iter()
        .filter_map(|(_, s)| s.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    clear_folder(&mut dest, Kind::Skills, &keep, &own, ops);

    let listed = listing(&dest.path).unwrap_or_default();
    for (name, staging) in staged {
        match place(ops, &dest.path, &listed, name, &staging) {
            Placed::Written(leftover) => {
                dest.written.push(name.to_string());
                dest.failed.extend(leftover);
            }
            Placed::InTheWay { listed, message } => {
                let _ = ops.remove_dir_all(&staging);
                dest.kept.extend(listed);
                dest.failed.push(message);
            }
            Placed::Failed(message) => {
                let _ = ops.remove_dir_all(&staging);
                dest.failed.push(message);
            }
        }
    }
    dest.kept.sort();
    Some(dest)
}

/// A fresh `staging/SKILL.md` holding `text`; `staging` must not exist.
fn stage(ops: &dyn FileOps, staging: &Path, text: &str) -> io::Result<()> {
    ops.create_dir(staging)?;
    let file = staging.join("SKILL.md");
    ops.write(&file, text.as_bytes())?;
    if !file.is_file() {
        return Err(io::Error::other(format!(
            "{} was not written",
            file.display()
        )));
    }
    Ok(())
}

/// What [`place`] did.
enum Placed {
    /// The skill is in place; a message when the old copy could not be
    /// deleted afterwards.
    Written(Option<String>),
    /// An entry Kluris cannot prove it wrote holds the name (`listed`, as
    /// the listing gives it): left as it is, the skill not written.
    InTheWay {
        listed: Option<String>,
        message: String,
    },
    Failed(String),
}

/// How a user clears an entry in a skill's way.
const IN_THE_WAY_FIX: &str = "move or delete it, then run `kluris doctor`";

/// Move `staging` to `dir/name`. A Kluris-written entry there is
/// quarantined first, checked again, and deleted once the new copy is in
/// place (put back if the move fails); anything else there, a name that
/// differs only in case included, is left as it is.
fn place(ops: &dyn FileOps, dir: &Path, listed: &[String], name: &str, staging: &Path) -> Placed {
    let target = dir.join(name);
    let in_the_way = |listed: Option<String>, why: &str| Placed::InTheWay {
        listed,
        message: format!(
            "{} {why}, so the skill was not written there: {IN_THE_WAY_FIX}",
            target.display()
        ),
    };
    if !listed.iter().any(|n| n == name) {
        if exists(&target) {
            let variant = listed
                .iter()
                .find(|n| n.eq_ignore_ascii_case(name))
                .cloned();
            return in_the_way(
                variant,
                "is taken by an entry whose name differs only in case",
            );
        }
        return match ops.rename(staging, &target) {
            Ok(()) => Placed::Written(None),
            Err(e) => Placed::Failed(format!("cannot move {} into place: {e}", target.display())),
        };
    }
    let owned = |p: &Path| is_owned_skill_entry(p, name);
    if !owned(&target) {
        return in_the_way(
            Some(name.to_string()),
            "exists and was not written by Kluris",
        );
    }
    let quarantine = quarantine_path(dir, name);
    if let Err(e) = ops.rename(&target, &quarantine) {
        return Placed::Failed(format!("cannot replace {}: {e}", target.display()));
    }
    if !owned(&quarantine) {
        return match put_back(ops, &quarantine, &target) {
            None => in_the_way(
                Some(name.to_string()),
                "changed while it was being replaced",
            ),
            Some(message) => Placed::Failed(message),
        };
    }
    if let Err(e) = ops.rename(staging, &target) {
        let mut message = format!("cannot move {} into place: {e}", target.display());
        if let Some(kept) = put_back(ops, &quarantine, &target) {
            message.push_str(&format!("; {kept}"));
        }
        return Placed::Failed(message);
    }
    Placed::Written(delete_entry(ops, &quarantine).err().map(|e| {
        format!(
            "cannot remove the old copy of {} at {}: {e}",
            target.display(),
            quarantine.display()
        )
    }))
}

/// Move the quarantined `quarantine` back to `original`; `None` when it is
/// back, else why it stays under its quarantine name.
fn put_back(ops: &dyn FileOps, quarantine: &Path, original: &Path) -> Option<String> {
    let why = if exists(original) {
        "its name is taken again".to_string()
    } else {
        match ops.rename(quarantine, original) {
            Ok(()) => return None,
            Err(e) => format!("it could not be put back ({e})"),
        }
    };
    Some(format!(
        "{} changed while it was being removed and {why}; it is kept at {}",
        original.display(),
        quarantine.display()
    ))
}

/// Remove the Kluris-written `kluris*` entries (and earlier runs' staging
/// entries) of `dest.path` whose names are not in `keep`; `own` are this
/// run's staging names. Every candidate not proven Kluris's goes to
/// `dest.kept`.
fn clear_folder(
    dest: &mut DestinationReport,
    kind: Kind,
    keep: &[&str],
    own: &[String],
    ops: &dyn FileOps,
) {
    let mut names = match listing(&dest.path) {
        Ok(names) => names,
        Err(e) => {
            dest.failed
                .push(format!("cannot read {}: {e}", dest.path.display()));
            return;
        }
    };
    names.sort();
    for name in names {
        if own.contains(&name) {
            continue;
        }
        // The skill (or workflow) the entry would be: its own name, or the
        // one an earlier run's staging or quarantine name carries.
        let (base, staged) = match staged_base(&name, kind) {
            Some(base) => (base, true),
            None if is_staging_like(&name) => {
                dest.kept.push(name);
                continue;
            }
            None if !name.starts_with("kluris") => continue,
            None if kind == Kind::Skills && keep.contains(&name.as_str()) => continue,
            None if kind == Kind::Workflows && !name.ends_with(".md") => continue,
            None => (name.as_str(), false),
        };
        let owned = |path: &Path| match kind {
            Kind::Skills => is_owned_skill_entry(path, base),
            Kind::Workflows => is_owned_workflow_file(path, base),
        };
        match remove_owned(ops, &dest.path, &name, base, owned) {
            Removal::Removed => dest.removed.push(name),
            Removal::NotOwned if staged || is_near_miss(&dest.path.join(&name), kind) => {
                dest.kept.push(name)
            }
            Removal::NotOwned => {}
            Removal::PutBack => dest.kept.push(name),
            Removal::Failed(message) => dest.failed.push(message),
        }
    }
    dest.kept.sort();
}

enum Removal {
    Removed,
    /// Not Kluris's: left where it was.
    NotOwned,
    /// Kluris's when checked, then not: moved back where it was.
    PutBack,
    Failed(String),
}

/// Remove `dir/name` when `owned` holds for it, before and after it is
/// moved to a quarantine name for `base`; see the module docs.
fn remove_owned(
    ops: &dyn FileOps,
    dir: &Path,
    name: &str,
    base: &str,
    owned: impl Fn(&Path) -> bool,
) -> Removal {
    let path = dir.join(name);
    if !owned(&path) {
        return Removal::NotOwned;
    }
    let quarantine = quarantine_path(dir, base);
    if let Err(e) = ops.rename(&path, &quarantine) {
        return Removal::Failed(format!("cannot remove {}: {e}", path.display()));
    }
    if !owned(&quarantine) {
        return match put_back(ops, &quarantine, &path) {
            None => Removal::PutBack,
            Some(message) => Removal::Failed(message),
        };
    }
    match delete_entry(ops, &quarantine) {
        Ok(()) => Removal::Removed,
        Err(e) => Removal::Failed(format!(
            "cannot remove {} (moved to {}): {e}",
            path.display(),
            quarantine.display()
        )),
    }
}

/// `.<base>.<token>.old` in `dir`: a name only this call makes.
fn quarantine_path(dir: &Path, base: &str) -> PathBuf {
    dir.join(format!(".{base}.{}.old", unique_token()))
}

/// The skill (or, among workflows, the `kluris*.md` file) a staging or
/// quarantine name `.<base>.<pid>-<16 hex>.tmp|.old` belongs to.
fn staged_base(name: &str, kind: Kind) -> Option<&str> {
    let rest = name.strip_prefix('.')?;
    let rest = rest
        .strip_suffix(".tmp")
        .or_else(|| rest.strip_suffix(".old"))?;
    let (base, token) = rest.rsplit_once('.')?;
    let base_ok = base.starts_with("kluris")
        && match kind {
            Kind::Skills => !base.contains('.'),
            Kind::Workflows => base.ends_with(".md"),
        };
    (base_ok && is_unique_token(token)).then_some(base)
}

/// A name like a staging folder's, of any run or version:
/// `.kluris*.tmp`, `.kluris*.old` or v2's workflow `.kluris*.tmp.md`.
fn is_staging_like(name: &str) -> bool {
    name.starts_with(".kluris")
        && [".tmp", ".old", ".tmp.md"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

/// A `kluris*` entry Kluris did not write but whose SKILL.md (read through
/// a link) or workflow carries a trait of Kluris's.
fn is_near_miss(path: &Path, kind: Kind) -> bool {
    let file = match kind {
        Kind::Skills => path.join("SKILL.md"),
        Kind::Workflows => path.to_path_buf(),
    };
    read_text(&file).is_some_and(|text| render::has_kluris_trait(&text))
}

/// The names in `dir` as its listing gives them (names that are not UTF-8
/// are never Kluris's and are left out).
fn listing(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        if let Ok(name) = entry?.file_name().into_string() {
            names.push(name);
        }
    }
    Ok(names)
}

/// The largest SKILL.md or workflow Kluris has written is a few tens of
/// KiB; anything over this is not Kluris's.
const MAX_SKILL_BYTES: u64 = 1024 * 1024;

/// The whole regular file at `path` as UTF-8 text, at most
/// [`MAX_SKILL_BYTES`]; `None` for anything else (a symlink included).
fn read_text(path: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_SKILL_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_SKILL_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Files the OS file managers write into folders people open (exact
/// names); a Kluris skill folder may hold them beside its `SKILL.md`.
const OS_METADATA_FILES: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];

/// A folder holding a `SKILL.md` file that Kluris wrote for `name` and
/// nothing else but [`OS_METADATA_FILES`] (regular files), or a link to
/// such a folder.
fn is_owned_skill_entry(path: &Path, name: &str) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    let is_dir = if meta.file_type().is_symlink() {
        fs::metadata(path).is_ok_and(|m| m.is_dir())
    } else {
        meta.is_dir()
    };
    if !is_dir {
        return false;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    let mut skill_md = false;
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            return false;
        }
        let file = entry.file_name();
        if file == "SKILL.md" {
            skill_md = true;
        } else if !OS_METADATA_FILES.iter().any(|m| file == *m) {
            return false;
        }
    }
    skill_md
        && read_text(&path.join("SKILL.md"))
            .is_some_and(|text| render::is_generated_skill(name, &text))
}

/// A regular file holding a Windsurf workflow Kluris wrote as `name`.
fn is_owned_workflow_file(path: &Path, name: &str) -> bool {
    read_text(path).is_some_and(|text| render::is_generated_workflow(name, &text))
}

/// Whether anything (a dangling symlink included) is at `path`.
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Delete a quarantined entry: a folder tree, a file, or a link (only the
/// link: a Windows folder link is removed as a folder).
fn delete_entry(ops: &dyn FileOps, path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        ops.remove_file(path).or_else(|_| ops.remove_dir(path))
    } else if meta.is_dir() {
        ops.remove_dir_all(path)
    } else {
        ops.remove_file(path)
    }
}
