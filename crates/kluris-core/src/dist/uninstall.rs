//! `kluris self-uninstall`: remove what the installers wrote ([`super`]),
//! read from the receipt, and keep everything else: `~/.kluris/config.yml`,
//! the companions, the brains, the skills. `purge` also removes `~/.kluris`
//! and every Kluris-written skill.
//!
//! An rc file loses each line that is exactly one the installer appends
//! (`. "<env.sh>"`, or `source "<env.fish>"` in the fish conf.d file, the
//! env script named as `$HOME/...` or in full), with the empty line the
//! installer wrote before it; every other byte stays.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use super::preflight::{Blocker, Planned, preflight};
use super::{DistEnv, Receipt, ReceiptError, SyncServices};
use crate::agents::install::{
    DestinationReport, generated_skill_entries, remove_generated_skills_with,
};
use crate::fileops::{FileOps, RealFs};

/// The per-user PATH on Windows (`HKCU\Environment\Path`).
pub trait UserPath {
    /// The value as stored, environment references unexpanded; `None` when
    /// there is none.
    fn read(&self) -> io::Result<Option<String>>;
    /// Store `value` (as `REG_EXPAND_SZ`) and tell running programs.
    fn write(&mut self, value: &str) -> io::Result<()>;
}

/// What [`uninstall`] works from.
#[derive(Debug, Clone, Copy)]
pub struct UninstallRequest<'a> {
    pub env: DistEnv<'a>,
    pub receipt: &'a Receipt,
    /// Where the receipt is; its folder holds the env scripts.
    pub receipt_path: &'a Path,
    /// Also remove `~/.kluris` and every Kluris-written skill.
    pub purge: bool,
    /// `COPILOT_HOME` when set and non-empty (for the skill folders).
    pub copilot_home: Option<&'a Path>,
    /// The registered brains, `(name, path)`: a purge never removes one.
    pub brains: &'a [(String, PathBuf)],
    /// The sync services to stop and remove first.
    pub services: Option<&'a dyn SyncServices>,
}

/// What [`uninstall`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallReport {
    /// Files and folders removed, in the order they went.
    pub removed: Vec<PathBuf>,
    /// rc files a line was taken out of.
    pub edited: Vec<PathBuf>,
    /// What the uninstall leaves that it would otherwise remove: files the
    /// installer's names point at that are not what it wrote, and the
    /// user's links to folders it would have removed ([`Self::kept_links`]).
    pub kept: Vec<PathBuf>,
    /// The entries of `kept` that are the user's links to folders (`~/.kluris`,
    /// its bin folder, the receipt's folder): each link stays, and so does
    /// the folder it points to; what Kluris put in that folder went.
    pub kept_links: Vec<PathBuf>,
    /// What the pre-flight found in the way: nothing was changed.
    pub blocked: Vec<Blocker>,
    /// What is left to remove by hand after a failure once the binary was
    /// gone, one instruction each.
    pub left: Vec<String>,
    /// User PATH entries removed (Windows), as they were written.
    pub path_entries: Vec<String>,
    /// The skill folders a purge cleared something from (or failed in).
    pub skills: Vec<DestinationReport>,
    /// `~/.kluris` was removed.
    pub purged: bool,
    /// The sync services stopped and removed, by name.
    pub services: Vec<String>,
    /// One message per thing that could not be done; the rest went on.
    pub failed: Vec<String>,
}

impl UninstallReport {
    /// Nothing was in the way, nothing failed and nothing is left.
    pub fn ok(&self) -> bool {
        self.blocked.is_empty()
            && self.failed.is_empty()
            && self.left.is_empty()
            && self.skills.iter().all(|d| d.failed.is_empty())
    }
}

/// Why nothing was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallError {
    /// A purge would remove these registered brains (they are in `~/.kluris`).
    BrainsInsideKluris(Vec<String>),
    /// The receipt is not the one dist writes for Kluris
    /// ([`ReceiptError::NotKluris`]): it authorises nothing.
    NotKluris(ReceiptError),
}

impl std::fmt::Display for UninstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UninstallError::BrainsInsideKluris(names) => write!(
                f,
                "--purge would remove ~/.kluris, which holds the registered brain(s) {}. \
                 Move them out of ~/.kluris (or run without --purge); nothing was removed.",
                names.join(", ")
            ),
            UninstallError::NotKluris(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for UninstallError {}

/// Remove what the installers wrote, in an order that keeps what a retry
/// needs (T-111), once every change is checked (T-119):
///
/// 0. Nothing is done unless the receipt is Kluris's
///    ([`Receipt::check_kluris`]); a purge that would remove a registered
///    brain is refused too.
///
/// The pre-flight then builds the whole plan from reads alone and checks
/// it ([`preflight`]): anything in the way (a path that cannot be looked
/// at, a folder that cannot lose an entry, an immutable flag, a read-only
/// attribute, sync services that cannot be listed because no service
/// manager answers) is listed in `blocked`, each with its fix, and nothing
/// is changed. Whether a service manager will agree to stop a service
/// cannot be asked without stopping it.
///
/// 0. The sync services ([`SyncServices`]), stopped and removed, as they
///    run the binary about to go. A failure stops here, with the install
///    whole: a rerun finishes.
/// 1. With `modify_path` (the installer set PATH up), the rc files lose the
///    line sourcing env.sh, unless env.sh is there and is not dist's (then it
///    and its lines are kept and reported). A failure stops here: env
///    scripts, receipt and binary all stay.
/// 2. The fish conf.d file loses its line (the same rule for env.fish); a
///    failure stops, as it still sources env.fish.
/// 3. The env scripts dist wrote, and on Windows the user PATH entry.
/// 4. With a purge: the Kluris-written skills, then everything in
///    `~/.kluris` but the folder the binary is in.
///
/// Then the binary and the receipt, the last two things removed, once
/// every step before has succeeded (a failure anywhere stops and keeps
/// both, so a rerun picks up where this one stopped): 5. the binary,
/// through `remove_binary` (the running one may need the platform's
/// self-delete; one that will not go keeps the receipt), its emptied folder
/// and under a purge what is left of `~/.kluris`; 6. the receipt. A failure
/// once the binary is gone (a change since the pre-flight) is in `failed`,
/// and `left` says what to remove by hand: by then no shell file sources a
/// removed script and the PATH entry is gone.
///
/// Without `modify_path` (KLURIS_NO_MODIFY_PATH=1) no rc file, env script,
/// fish file or PATH entry is touched: the user manages PATH.
pub fn uninstall(
    req: &UninstallRequest,
    user_path: Option<&mut dyn UserPath>,
    remove_binary: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> Result<UninstallReport, UninstallError> {
    uninstall_with(req, user_path, remove_binary, &RealFs)
}

/// [`uninstall`], removing through `ops` (tests make one fail).
pub fn uninstall_with(
    req: &UninstallRequest,
    user_path: Option<&mut dyn UserPath>,
    remove_binary: &mut dyn FnMut(&Path) -> io::Result<()>,
    ops: &dyn FileOps,
) -> Result<UninstallReport, UninstallError> {
    req.receipt.check_kluris(req.env.windows).map_err(|why| {
        UninstallError::NotKluris(ReceiptError::NotKluris(req.receipt_path.to_path_buf(), why))
    })?;
    let home = req.env.home;
    let kluris_dir = home.join(".kluris");
    if req.purge {
        let inside: Vec<String> = req
            .brains
            .iter()
            .filter(|(_, path)| is_within(path, &kluris_dir))
            .map(|(name, _)| name.clone())
            .collect();
        if !inside.is_empty() {
            return Err(UninstallError::BrainsInsideKluris(inside));
        }
    }

    let mut report = UninstallReport::default();
    let plan = Plan::new(req, &mut report);
    report.blocked = plan.blocked;
    report.blocked.extend(preflight(&plan.planned));
    if !report.blocked.is_empty() {
        return Ok(report);
    }
    if let Some(services) = req.services {
        for service in &plan.services {
            match services.remove(service) {
                Ok(()) => report.services.push(service.name.clone()),
                Err(e) => {
                    report.failed.push(format!(
                        "cannot stop and remove the sync service {}: {e}",
                        service.name
                    ));
                    return Ok(report);
                }
            }
        }
    }
    let prefix = req.receipt.install_prefix.as_path();
    let receipt_dir = req.receipt_path.parent().unwrap_or(Path::new(""));

    if req.receipt.modify_path {
        // 1. The rc files.
        for (rc, remove_if_empty) in &plan.rc_files {
            let lines = if *remove_if_empty {
                &plan.fish_lines
            } else {
                &plan.sh_lines
            };
            edit_rc(rc, lines, *remove_if_empty, ops, &mut report);
            // 2. A failure stops here: the files still source the scripts.
            if !report.failed.is_empty() {
                return Ok(report);
            }
        }

        // 3. The env scripts and the Windows user PATH entry.
        for script in &plan.scripts {
            remove_file(script, ops, &mut report);
        }
        if let Some(user_path) = user_path {
            match user_path.read() {
                Ok(Some(value)) => {
                    if let Some((new_value, removed)) = remove_path_entry(&value, prefix) {
                        match user_path.write(&new_value) {
                            Ok(()) => report.path_entries.extend(removed),
                            Err(e) => report
                                .failed
                                .push(format!("cannot update the user PATH: {e}")),
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => report
                    .failed
                    .push(format!("cannot read the user PATH: {e}")),
            }
        }
    }
    if !report.ok() {
        return Ok(report);
    }

    // 4. A purge: the skills, then ~/.kluris but the folder the binary is in.
    let binary = prefix.join(req.env.binary_name());
    if req.purge {
        report.skills = remove_generated_skills_with(home, req.copilot_home, ops)
            .into_iter()
            .filter(|d| !d.removed.is_empty() || !d.kept.is_empty() || !d.failed.is_empty())
            .collect();
        if !report.ok() {
            return Ok(report);
        }
        clear_kluris_dir(&kluris_dir, prefix, &binary, ops, &mut report);
        if !report.ok() {
            return Ok(report);
        }
    }

    // 5. The binary, by its own name in the receipt's prefix. One that will
    // not go keeps the receipt.
    match fs::symlink_metadata(&binary) {
        Ok(_) => match remove_binary(&binary) {
            Ok(()) => report.removed.push(binary),
            Err(e) => {
                report
                    .failed
                    .push(format!("cannot remove {}: {e}", binary.display()));
                return Ok(report);
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            report
                .failed
                .push(format!("cannot check {}: {e}", binary.display()));
            return Ok(report);
        }
    }

    // From here on the binary is gone: what fails is left for the user to
    // remove by hand, and the rest still goes.
    remove_dir_if_empty(prefix, ops, &mut report);
    if req.purge {
        match fs::symlink_metadata(&kluris_dir) {
            // A link of the user's stays, and so does its folder: only what
            // is in it goes.
            Ok(meta) if meta.file_type().is_symlink() => {
                match remove_entries(&kluris_dir, &|_| false, ops, &mut report) {
                    Ok(()) => report.purged = true,
                    Err((path, e)) => left(
                        &mut report,
                        format!("cannot remove {}: {e}", path.display()),
                        format!("remove {} and everything in it", path.display()),
                    ),
                }
                keep_link(&mut report, kluris_dir);
            }
            Ok(_) => match ops.remove_dir_all(&kluris_dir) {
                Ok(()) => {
                    // Links in ~/.kluris went with it (never what they
                    // point to).
                    let gone = |link: &PathBuf| link.starts_with(&kluris_dir);
                    report.kept.retain(|path| !gone(path));
                    report.kept_links.retain(|path| !gone(path));
                    report.removed.push(kluris_dir);
                    report.purged = true;
                }
                Err(e) => left(
                    &mut report,
                    format!("cannot remove {}: {e}", kluris_dir.display()),
                    format!(
                        "remove the folder {} and everything in it",
                        kluris_dir.display()
                    ),
                ),
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => report.purged = true,
            Err(e) => left(
                &mut report,
                format!("cannot check {}: {e}", kluris_dir.display()),
                format!(
                    "remove the folder {} and everything in it",
                    kluris_dir.display()
                ),
            ),
        }
    }

    // 6. The receipt.
    if let Err(e) = remove_if_there(req.receipt_path, ops, &mut report) {
        left(
            &mut report,
            format!("cannot remove {}: {e}", req.receipt_path.display()),
            format!("remove the file {}", req.receipt_path.display()),
        );
    }
    remove_dir_if_empty(receipt_dir, ops, &mut report);
    Ok(report)
}

/// The user's link to a folder, kept ([`UninstallReport::kept_links`]).
fn keep_link(report: &mut UninstallReport, link: PathBuf) {
    report.kept.push(link.clone());
    report.kept_links.push(link);
}

/// A failure once the binary is gone: `message` in `failed`, and the
/// instruction `todo` in `left`.
fn left(report: &mut UninstallReport, message: String, todo: String) {
    report.failed.push(message);
    report.left.push(todo);
}

/// What the uninstall will change, worked out from reads alone.
struct Plan {
    /// The rc files that lose a line, each with whether it is the fish
    /// conf.d file (which goes once only whitespace is left).
    rc_files: Vec<(PathBuf, bool)>,
    sh_lines: Vec<Vec<u8>>,
    fish_lines: Vec<Vec<u8>>,
    /// The env scripts that go: dist's own.
    scripts: Vec<PathBuf>,
    /// Every change, for [`preflight`].
    planned: Vec<Planned>,
    /// What the reads themselves found in the way.
    blocked: Vec<Blocker>,
    /// The sync services to stop and remove.
    services: Vec<crate::service::Installed>,
}

impl Plan {
    fn new(req: &UninstallRequest, report: &mut UninstallReport) -> Plan {
        let home = req.env.home;
        let prefix = req.receipt.install_prefix.as_path();
        let receipt_dir = req.receipt_path.parent().unwrap_or(Path::new(""));
        let env_sh = receipt_dir.join("env.sh");
        let env_fish = receipt_dir.join("env.fish");
        let mut plan = Plan {
            rc_files: Vec::new(),
            sh_lines: source_lines(".", &[&env_sh], home),
            fish_lines: source_lines("source", &[&env_fish], home),
            scripts: Vec::new(),
            planned: Vec::new(),
            blocked: Vec::new(),
            services: Vec::new(),
        };
        if let Some(services) = req.services {
            match services.installed() {
                Ok(list) => {
                    for service in &list {
                        if let Some(definition) = &service.definition {
                            plan.planned.push(Planned::Remove(definition.clone()));
                        }
                    }
                    plan.services = list;
                }
                Err(e) => plan.blocked.push(Blocker {
                    path: services.location(),
                    problem: format!("the Kluris sync services here cannot be listed: {e}"),
                    fix: "remove each with `kluris sync --uninstall-service --brain <name>` \
                          where its service manager runs, or by hand"
                        .to_string(),
                }),
            }
        }

        if req.receipt.modify_path {
            let exprs = install_dir_exprs(prefix, home);
            let sh = dists_script(&env_sh, &exprs, env_sh_text, report);
            let fish = dists_script(&env_fish, &exprs, env_fish_text, report);
            let mut rc_files = Vec::new();
            match &sh {
                Script::Ours { .. } => {
                    let zdir = req.env.zdotdir.unwrap_or(home);
                    for rc in [
                        home.join(".profile"),
                        home.join(".bashrc"),
                        home.join(".bash_profile"),
                        home.join(".bash_login"),
                        zdir.join(".zshrc"),
                        zdir.join(".zshenv"),
                    ] {
                        rc_files.push((rc, false));
                    }
                }
                Script::Unreadable(e) => plan.blocked.push(Blocker::unchecked(&env_sh, e)),
                Script::Theirs => {}
            }
            match &fish {
                Script::Ours { .. } => rc_files.push((
                    home.join(".config")
                        .join("fish")
                        .join("conf.d")
                        .join(format!("{}.env.fish", super::APP_NAME)),
                    true,
                )),
                Script::Unreadable(e) => plan.blocked.push(Blocker::unchecked(&env_fish, e)),
                Script::Theirs => {}
            }
            for (rc, remove_if_empty) in rc_files {
                let lines = if remove_if_empty {
                    &plan.fish_lines
                } else {
                    &plan.sh_lines
                };
                match rc_change(&rc, lines, remove_if_empty) {
                    Ok(None) => {}
                    Ok(Some(change)) => {
                        plan.planned.push(change);
                        plan.rc_files.push((rc, remove_if_empty));
                    }
                    Err(e) => plan.blocked.push(Blocker::unchecked(&rc, &e)),
                }
            }
            for (script, state) in [(&env_fish, &fish), (&env_sh, &sh)] {
                if let Script::Ours { present: true } = state {
                    plan.planned.push(Planned::Remove(script.clone()));
                    plan.scripts.push(script.clone());
                }
            }
        }

        let binary = prefix.join(req.env.binary_name());
        if req.purge {
            let (entries, errors) = generated_skill_entries(home, req.copilot_home);
            for entry in entries {
                let link = fs::symlink_metadata(&entry).is_ok_and(|m| m.file_type().is_symlink());
                plan.planned.push(if link {
                    Planned::Remove(entry)
                } else {
                    Planned::RemoveTree(entry)
                });
            }
            for (path, e) in errors {
                plan.blocked.push(Blocker::unchecked(&path, &e));
            }
            let kluris_dir = home.join(".kluris");
            match fs::symlink_metadata(&kluris_dir) {
                // A link stays: the purge goes through it and removes each
                // entry of its folder, so each is checked as it is removed.
                Ok(meta) if meta.file_type().is_symlink() => match fs::read_dir(&kluris_dir) {
                    Ok(entries) => {
                        for entry in entries {
                            match entry {
                                Ok(entry) => plan.planned.push(Planned::RemoveTree(entry.path())),
                                Err(e) => plan.blocked.push(Blocker::unchecked(&kluris_dir, &e)),
                            }
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => plan.blocked.push(Blocker::unchecked(&kluris_dir, &e)),
                },
                _ => plan.planned.push(Planned::RemoveTree(kluris_dir)),
            }
        }
        plan.planned.push(Planned::Remove(binary.clone()));
        plan.planned
            .push(Planned::Remove(req.receipt_path.to_path_buf()));

        // The folders that go once empty: the prefix (the binary in it) and
        // the receipt's (the receipt and the scripts).
        let mut going = vec![binary, req.receipt_path.to_path_buf()];
        going.extend(plan.scripts.iter().cloned());
        for folder in [prefix, receipt_dir] {
            // A link of the user's is never removed.
            if fs::symlink_metadata(folder).is_ok_and(|m| m.file_type().is_symlink()) {
                continue;
            }
            match fs::read_dir(folder) {
                Ok(entries) => {
                    let empties = entries
                        .map(|e| e.map(|e| going.contains(&e.path())))
                        .collect::<io::Result<Vec<bool>>>();
                    match empties {
                        Ok(all) if all.iter().all(|going| *going) => {
                            plan.planned.push(Planned::RemoveDir(folder.to_path_buf()))
                        }
                        Ok(_) => {}
                        Err(e) => plan.blocked.push(Blocker::unchecked(folder, &e)),
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => plan.blocked.push(Blocker::unchecked(folder, &e)),
            }
        }
        plan
    }
}

/// The change the uninstall makes to the rc file `path`: an edit, the
/// removal of a fish conf.d file left empty, or none (not there, not a
/// file, no line of the installer's).
fn rc_change(path: &Path, lines: &[Vec<u8>], remove_if_empty: bool) -> io::Result<Option<Planned>> {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let Some(new_text) = without_lines(&fs::read(path)?, lines) else {
        return Ok(None);
    };
    Ok(Some(
        if remove_if_empty && new_text.iter().all(u8::is_ascii_whitespace) {
            Planned::Remove(path.to_path_buf())
        } else {
            Planned::Edit(path.to_path_buf())
        },
    ))
}

/// A purge's ~/.kluris before the binary goes: every entry but the one that
/// holds the install prefix (`~/.kluris/bin`) or is the binary itself.
fn clear_kluris_dir(
    kluris_dir: &Path,
    prefix: &Path,
    binary: &Path,
    ops: &dyn FileOps,
    report: &mut UninstallReport,
) {
    let keep = |path: &Path| prefix.starts_with(path) || path == binary;
    if let Err((path, e)) = remove_entries(kluris_dir, &keep, ops, report) {
        report
            .failed
            .push(format!("cannot remove {}: {e}", path.display()));
    }
}

/// Remove every entry of the folder `dir` (through it when it is a link)
/// but those `keep` holds for: a folder with everything in it, a file or a
/// link (only the link). Stops at the first failure, with its path.
fn remove_entries(
    dir: &Path,
    keep: &dyn Fn(&Path) -> bool,
    ops: &dyn FileOps,
    report: &mut UninstallReport,
) -> Result<(), (PathBuf, io::Error)> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err((dir.to_path_buf(), e)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| (dir.to_path_buf(), e))?.path());
    }
    paths.sort();
    for path in paths {
        if keep(&path) {
            continue;
        }
        let removed = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => ops.remove_dir_all(&path),
            Ok(_) => ops.remove_file(&path),
            Err(e) => Err(e),
        };
        match removed {
            Ok(()) => report.removed.push(path),
            Err(e) => return Err((path, e)),
        }
    }
    Ok(())
}

/// How the installer names the install folder inside its env scripts:
/// `$HOME/<rest>` under HOME (its `replace_home`), else the folder itself.
fn install_dir_exprs(prefix: &Path, home: &Path) -> Vec<String> {
    let mut exprs = Vec::new();
    if let Ok(rel) = prefix.strip_prefix(home) {
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        exprs.push(format!("$HOME/{}", rel.join("/")));
    }
    exprs.push(prefix.to_string_lossy().into_owned());
    exprs
}

/// dist 0.33.1's env.sh for the install folder `expr`
/// (`write_env_script_sh`).
fn env_sh_text(expr: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # add binaries to PATH if they aren't added yet\n\
         # affix colons on either side of $PATH to simplify matching\n\
         case \":${{PATH}}:\" in\n    \
         *:\"{expr}\":*)\n        \
         ;;\n    \
         *)\n        \
         # Prepending path in case a system-installed binary needs to be overridden\n        \
         export PATH=\"{expr}:$PATH\"\n        \
         ;;\n\
         esac\n"
    )
}

/// dist 0.33.1's env.fish for the install folder `expr`
/// (`write_env_script_fish`).
fn env_fish_text(expr: &str) -> String {
    format!(
        "if not contains \"{expr}\" $PATH\n    \
         # Prepending path in case a system-installed binary needs to be overridden\n    \
         set -x PATH \"{expr}\" $PATH\n\
         end\n"
    )
}

/// What the env script at `path` is.
enum Script {
    /// Not there, or exactly what dist writes: it may go with the lines
    /// sourcing it.
    Ours { present: bool },
    /// Anything else is the user's: kept with its lines, and said in
    /// `report.kept`.
    Theirs,
    /// It could not be read.
    Unreadable(io::Error),
}

/// Whether the env script at `path` is dist's for one of `exprs`.
fn dists_script(
    path: &Path,
    exprs: &[String],
    text_for: fn(&str) -> String,
    report: &mut UninstallReport,
) -> Script {
    let text = match fs::read(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Script::Ours { present: false },
        Err(e) => return Script::Unreadable(e),
    };
    if exprs.iter().any(|expr| text_for(expr).as_bytes() == text) {
        Script::Ours { present: true }
    } else {
        report.kept.push(path.to_path_buf());
        Script::Theirs
    }
}

/// `value` (a `;`-separated PATH) without the entries naming `prefix`, and
/// those entries as written; `None` when there is none. Entries match as
/// Windows compares paths: without case, `/` as `\`, a trailing `\` ignored.
/// Every other entry, empty ones included, stays as it was.
pub fn remove_path_entry(value: &str, prefix: &Path) -> Option<(String, Vec<String>)> {
    let want = windows_key(&prefix.to_string_lossy());
    let (removed, kept): (Vec<&str>, Vec<&str>) = value
        .split(';')
        .partition(|entry| !entry.is_empty() && windows_key(entry) == want);
    if removed.is_empty() {
        return None;
    }
    Some((
        kept.join(";"),
        removed.into_iter().map(str::to_string).collect(),
    ))
}

fn windows_key(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Whether `path` is `dir` or inside it (lexically, and once more with both
/// resolved when they exist).
fn is_within(path: &Path, dir: &Path) -> bool {
    if path.starts_with(dir) {
        return true;
    }
    match (fs::canonicalize(path), fs::canonicalize(dir)) {
        (Ok(p), Ok(d)) => p.starts_with(d),
        _ => false,
    }
}

/// The lines that source `scripts`: `<command> "<script>"`, the script named
/// with `$HOME` in front when under HOME (as the installer writes it) and
/// in full.
fn source_lines(command: &str, scripts: &[&Path], home: &Path) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    for script in scripts {
        let mut exprs = Vec::new();
        if let Ok(rel) = script.strip_prefix(home) {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            exprs.push(format!("$HOME/{}", rel.join("/")));
        }
        exprs.push(script.to_string_lossy().into_owned());
        for expr in exprs {
            let line = format!("{command} \"{expr}\"").into_bytes();
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    lines
}

/// Take `lines` out of the rc file `path` ([`without_lines`]). A file left
/// with only whitespace is removed when `remove_if_empty` (the installer's
/// own fish conf.d file).
fn edit_rc(
    path: &Path,
    lines: &[Vec<u8>],
    remove_if_empty: bool,
    ops: &dyn FileOps,
    report: &mut UninstallReport,
) {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            report
                .failed
                .push(format!("cannot check {}: {e}", path.display()));
            return;
        }
    }
    let text = match fs::read(path) {
        Ok(text) => text,
        Err(e) => {
            report
                .failed
                .push(format!("cannot read {}: {e}", path.display()));
            return;
        }
    };
    let Some(new_text) = without_lines(&text, lines) else {
        return;
    };
    if remove_if_empty && new_text.iter().all(u8::is_ascii_whitespace) {
        remove_file(path, ops, report);
        return;
    }
    match crate::atomic::write_atomic(path, &new_text) {
        Ok(()) => report.edited.push(path.to_path_buf()),
        Err(e) => report
            .failed
            .push(format!("cannot write {}: {e}", path.display())),
    }
}

/// `text` without each line (ignoring its `\n` / `\r\n`) that equals one of
/// `lines`, nor the empty line right before it; `None` when there is none.
fn without_lines(text: &[u8], lines: &[Vec<u8>]) -> Option<Vec<u8>> {
    fn content(line: &[u8]) -> &[u8] {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        line.strip_suffix(b"\r").unwrap_or(line)
    }
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut changed = false;
    for line in text.split_inclusive(|b| *b == b'\n') {
        if lines.iter().any(|l| l.as_slice() == content(line)) {
            changed = true;
            if kept.last().is_some_and(|prev| content(prev).is_empty()) {
                kept.pop();
            }
            continue;
        }
        kept.push(line);
    }
    changed.then(|| kept.concat())
}

/// Remove the file (or symlink) at `path` when there is one; a failure is
/// in `report.failed`.
fn remove_file(path: &Path, ops: &dyn FileOps, report: &mut UninstallReport) {
    if let Err(e) = remove_if_there(path, ops, report) {
        report
            .failed
            .push(format!("cannot remove {}: {e}", path.display()));
    }
}

/// Remove the file (or symlink) at `path` when there is one (not a folder).
fn remove_if_there(path: &Path, ops: &dyn FileOps, report: &mut UninstallReport) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => {
            ops.remove_file(path)?;
            report.removed.push(path.to_path_buf());
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove the folder `path` when it is there and empty; it goes once the
/// binary is gone, so a failure is [`left`] to the user. A link of the
/// user's to an emptied folder stays, and so does the folder: it is in
/// `kept_links`.
fn remove_dir_if_empty(path: &Path, ops: &dyn FileOps, report: &mut UninstallReport) {
    let link = match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_symlink(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            return left(
                report,
                format!("cannot check {}: {e}", path.display()),
                format!("remove the folder {} if it is empty", path.display()),
            );
        }
    };
    let empty = match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            return left(
                report,
                format!("cannot read {}: {e}", path.display()),
                format!("remove the folder {} if it is empty", path.display()),
            );
        }
    };
    if !empty {
        return;
    }
    if link {
        keep_link(report, path.to_path_buf());
        return;
    }
    match ops.remove_dir(path) {
        Ok(()) => report.removed.push(path.to_path_buf()),
        Err(e) => left(
            report,
            format!("cannot remove {}: {e}", path.display()),
            format!("remove the empty folder {}", path.display()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|l| l.as_bytes().to_vec()).collect()
    }

    /// An rc file is rewritten atomically (a new file renamed over it, so a
    /// failed write cannot truncate it), and a symlinked rc file stays a
    /// link to the file that gets the edit.
    #[cfg(unix)]
    #[test]
    fn edit_rc_replaces_the_file_and_keeps_a_symlink() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("dotfiles-zshrc");
        fs::write(
            &real,
            "alias ll='ls -l'\n\n. \"$HOME/.config/kluris/env.sh\"\n",
        )
        .unwrap();
        let rc = dir.path().join(".zshrc");
        symlink(&real, &rc).unwrap();
        let inode = fs::metadata(&real).unwrap().ino();
        let mut report = UninstallReport::default();
        edit_rc(
            &rc,
            &lines(&[". \"$HOME/.config/kluris/env.sh\""]),
            false,
            &RealFs,
            &mut report,
        );
        assert_eq!(report.failed, Vec::<String>::new());
        assert_eq!(report.edited, std::slice::from_ref(&rc));
        assert!(fs::symlink_metadata(&rc).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "alias ll='ls -l'\n");
        assert_ne!(fs::metadata(&real).unwrap().ino(), inode);
    }

    #[test]
    fn without_lines_takes_the_line_and_the_empty_line_before_it() {
        let ours = lines(&["X"]);
        assert_eq!(without_lines(b"a\n\nX\n", &ours), Some(b"a\n".to_vec()));
        assert_eq!(without_lines(b"a\nX\n", &ours), Some(b"a\n".to_vec()));
        assert_eq!(without_lines(b"\nX\n", &ours), Some(Vec::new()));
        assert_eq!(without_lines(b"a\n\nX", &ours), Some(b"a\n".to_vec()));
        assert_eq!(without_lines(b"X \nX#\n X\n", &ours), None);
        assert_eq!(
            without_lines(b"a\r\n\r\nX\r\nb\r\n", &ours),
            Some(b"a\r\nb\r\n".to_vec())
        );
    }

    #[test]
    fn source_lines_name_the_script_both_ways() {
        let home = Path::new("/h");
        assert_eq!(
            source_lines(".", &[Path::new("/h/.config/kluris/env.sh")], home),
            lines(&[
                ". \"$HOME/.config/kluris/env.sh\"",
                ". \"/h/.config/kluris/env.sh\""
            ])
        );
        assert_eq!(
            source_lines("source", &[Path::new("/x/env.fish")], home),
            lines(&["source \"/x/env.fish\""])
        );
    }
}
