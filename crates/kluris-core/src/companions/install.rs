//! Installing a companion folder into `~/.kluris/companions/<name>/` (v2
//! `companions.install`): copy the allowlisted entries into a staging
//! folder, verify it, move the old copy (folder, file or link) aside to
//! `.<name>.old`, rename the staging folder in, then drop the old copy.
//! Any failure leaves the old copy: in place, or, when it cannot be moved
//! back, in `.<name>.old`, which the error names and the next install
//! (`kluris doctor` included) restores first (SPEC decision 2026-10-26,
//! T-103: the backup is deleted only once the new copy is in place).

use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use super::{CompanionTree, KNOWN, embedded, normalize};
use crate::config::{ConfigError, GlobalConfig, read_brain_config};
use crate::fileops::{FileOps, RealFs};
use crate::pyrepr::py_path_str;

/// The only top-level file a companion folder may hold.
const RUNTIME_FILES: [&str; 1] = ["SKILL.md"];
/// The only top-level folders a companion folder may hold.
const RUNTIME_DIRS: [&str; 2] = ["references", "assets"];

/// Why a companion could not be installed or an opt-in written.
#[derive(Debug)]
pub enum CompanionError {
    /// Not one of [`KNOWN`].
    Unknown(String),
    /// The source tree of the named companion has no `SKILL.md`.
    MissingSkill(String),
    /// A staged copy that breaks the allowlist (v2 `_verify_runtime_dir`).
    Invalid(String),
    /// A file-system operation on `path` failed.
    Io { path: PathBuf, error: io::Error },
    /// A brain's `kluris.yml` could not be written.
    Config(ConfigError),
    /// The previous copy stays in the backup folder `backup`: it could not
    /// be put back.
    BackupKept { backup: PathBuf, message: String },
}

/// What an install did besides installing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installed {
    /// The backup a failed earlier install left, restored before this one.
    pub restored: Option<PathBuf>,
}

impl fmt::Display for CompanionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompanionError::Unknown(name) => write!(
                f,
                "Unknown companion '{name}'. Allowed: {}",
                KNOWN.join(", ")
            ),
            CompanionError::MissingSkill(name) => {
                write!(f, "Vendored companion missing SKILL.md: {name}")
            }
            CompanionError::Invalid(message) => f.write_str(message),
            CompanionError::Io { path, error } => write!(f, "{}: {error}", path.display()),
            CompanionError::Config(err) => err.fmt(f),
            CompanionError::BackupKept { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for CompanionError {}

impl From<ConfigError> for CompanionError {
    fn from(err: ConfigError) -> Self {
        CompanionError::Config(err)
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> CompanionError + '_ {
    move |error| CompanionError::Io {
        path: path.to_path_buf(),
        error,
    }
}

/// `~/.kluris/companions/<name>` under `home`.
pub fn installed_dir(name: &str, home: &Path) -> PathBuf {
    home.join(".kluris").join("companions").join(name)
}

/// Whether the companion's runtime `SKILL.md` exists (v2 `is_installed`).
pub fn is_installed(name: &str, home: &Path) -> bool {
    installed_dir(name, home).join("SKILL.md").is_file()
}

/// The known companions whose runtime path exists, whatever it is (v2
/// `installed`).
pub fn installed(home: &Path) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|name| installed_dir(name, home).exists())
        .map(|name| name.to_string())
        .collect()
}

/// The known companions the registered brains opt into; a brain whose
/// `kluris.yml` is missing or does not read is skipped (v2 `referenced`).
pub fn referenced(config: &GlobalConfig) -> Vec<String> {
    let mut refs = Vec::new();
    for entry in config.brains.values() {
        let brain = PathBuf::from(py_path_str(&entry.path));
        if !brain.join("kluris.yml").exists() {
            continue;
        }
        if let Ok(cfg) = read_brain_config(&brain) {
            refs.extend(cfg.companions);
        }
    }
    normalize(&refs)
}

/// `~/.kluris/companions/.<name>.old` under `home`: where an install keeps
/// the previous copy until the new one is in place.
fn backup_dir(name: &str, home: &Path) -> PathBuf {
    home.join(".kluris")
        .join("companions")
        .join(format!(".{name}.old"))
}

/// The known companions whose backup a failed install left behind; the
/// next install restores it first.
pub fn pending_backups(home: &Path) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|name| present(&backup_dir(name, home)))
        .map(|name| name.to_string())
        .collect()
}

/// Whether `path` is there at all: a file, a folder or a link (dangling
/// included).
fn present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Best-effort removal of whatever is at `path`; a link is removed, never
/// what it points at.
fn remove_any(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_symlink() {
        // A Windows folder link is removed as a folder.
        if fs::remove_file(path).is_err() {
            let _ = fs::remove_dir(path);
        }
    } else if meta.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

/// Whether `rel` (POSIX, relative to the companion folder) is part of what
/// is installed: `SKILL.md`, or an entry inside `references/` or `assets/`
/// none of whose names is hidden or `__pycache__` (v2 `_stage_copy`).
fn allowlisted(rel: &str, is_dir: bool) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    match parts.as_slice() {
        [single] => {
            (!is_dir && RUNTIME_FILES.contains(single)) || (is_dir && RUNTIME_DIRS.contains(single))
        }
        [top, rest @ ..] => {
            RUNTIME_DIRS.contains(top)
                && rest
                    .iter()
                    .all(|p| !p.is_empty() && !p.starts_with('.') && *p != "__pycache__")
        }
        [] => false,
    }
}

/// Write the allowlisted part of `tree` into a new `staging` folder.
fn stage(tree: &CompanionTree, staging: &Path) -> Result<(), CompanionError> {
    fs::create_dir_all(staging).map_err(io_error(staging))?;
    for rel in tree.dirs().filter(|rel| allowlisted(rel, true)) {
        let path = staging.join(rel);
        fs::create_dir_all(&path).map_err(io_error(&path))?;
    }
    for (rel, bytes) in tree.files().filter(|(rel, _)| allowlisted(rel, false)) {
        let path = staging.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_error(parent))?;
        }
        fs::write(&path, bytes).map_err(io_error(&path))?;
    }
    Ok(())
}

/// Check a companion folder (v2 `_verify_runtime_dir`): `SKILL.md` is a
/// file, the only other top-level entries are the `references/` and
/// `assets/` folders, and nothing inside is a link or a hidden name.
pub fn verify_runtime_dir(path: &Path) -> Result<(), CompanionError> {
    let skill = path.join("SKILL.md");
    if !skill.is_file() {
        return Err(CompanionError::Invalid(format!(
            "Companion copy is missing {}",
            skill.display()
        )));
    }
    let mut extras = Vec::new();
    for entry in fs::read_dir(path).map_err(io_error(path))? {
        let entry = entry.map_err(io_error(path))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let entry_path = entry.path();
        let allowed = (RUNTIME_FILES.contains(&name.as_str()) && entry_path.is_file())
            || (RUNTIME_DIRS.contains(&name.as_str()) && entry_path.is_dir());
        if !allowed {
            extras.push(name);
        }
    }
    if !extras.is_empty() {
        extras.sort();
        let allowed: Vec<String> = RUNTIME_FILES
            .iter()
            .map(|f| f.to_string())
            .chain(RUNTIME_DIRS.iter().map(|d| format!("{d}/")))
            .collect();
        return Err(CompanionError::Invalid(format!(
            "Companion runtime dir may contain only {}; found: {}",
            allowed.join(", "),
            extras.join(", ")
        )));
    }
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(io_error(&dir))? {
            let entry = entry.map_err(io_error(&dir))?;
            let entry_path = entry.path();
            let kind = entry.file_type().map_err(io_error(&entry_path))?;
            if kind.is_symlink() || entry.file_name().to_string_lossy().starts_with('.') {
                return Err(CompanionError::Invalid(format!(
                    "Companion runtime dir contains a link or hidden file: {}",
                    entry_path.display()
                )));
            }
            if kind.is_dir() {
                stack.push(entry_path);
            }
        }
    }
    Ok(())
}

/// Install companion `name` from `tree` into `home` (see the module docs).
pub fn install_tree(name: &str, tree: &CompanionTree, home: &Path) -> Result<(), CompanionError> {
    install_tree_with(name, tree, home, &RealFs).map(|_| ())
}

/// [`install_tree`], renaming through `ops`.
pub fn install_tree_with(
    name: &str,
    tree: &CompanionTree,
    home: &Path,
    ops: &dyn FileOps,
) -> Result<Installed, CompanionError> {
    install_with(name, tree, home, verify_runtime_dir, ops)
}

fn install_with(
    name: &str,
    tree: &CompanionTree,
    home: &Path,
    verify: impl Fn(&Path) -> Result<(), CompanionError>,
    ops: &dyn FileOps,
) -> Result<Installed, CompanionError> {
    if !KNOWN.contains(&name) {
        return Err(CompanionError::Unknown(name.to_string()));
    }
    if !tree.files().any(|(rel, _)| rel == "SKILL.md") {
        return Err(CompanionError::MissingSkill(name.to_string()));
    }
    let dest = installed_dir(name, home);
    let parent = dest.parent().unwrap_or(home).to_path_buf();
    fs::create_dir_all(&parent).map_err(io_error(&parent))?;
    let staging = parent.join(format!(".{name}.tmp"));
    let backup = backup_dir(name, home);

    // A backup an earlier install kept: the copy to restore when there is
    // none in place; superseded by the one in place otherwise.
    let mut done = Installed::default();
    if present(&backup) {
        if present(&dest) {
            remove_any(&backup);
        } else {
            ops.rename(&backup, &dest)
                .map_err(|e| CompanionError::BackupKept {
                    backup: backup.clone(),
                    message: format!(
                        "cannot restore the previous copy kept at {}: {e}",
                        backup.display()
                    ),
                })?;
            done.restored = Some(backup.clone());
        }
    }
    remove_any(&staging);

    let staged = stage(tree, &staging).and_then(|()| verify(&staging));
    if let Err(e) = staged {
        remove_any(&staging);
        return Err(e);
    }
    if present(&dest)
        && let Err(e) = ops.rename(&dest, &backup)
    {
        remove_any(&staging);
        return Err(CompanionError::Io {
            path: dest,
            error: e,
        });
    }
    if let Err(e) = ops.rename(&staging, &dest) {
        remove_any(&staging);
        if present(&backup)
            && !present(&dest)
            && let Err(restore) = ops.rename(&backup, &dest)
        {
            return Err(CompanionError::BackupKept {
                message: format!(
                    "{}: {e}; the previous copy could not be put back ({restore}) and is kept at {} \
                     (the next install or `kluris doctor` restores it)",
                    dest.display(),
                    backup.display()
                ),
                backup,
            });
        }
        return Err(CompanionError::Io {
            path: dest,
            error: e,
        });
    }
    // The new copy is in place: only now does the old one go.
    if dest.join("SKILL.md").is_file() {
        remove_any(&backup);
    }
    Ok(done)
}

/// Install the embedded copy of companion `name` into `home`.
pub fn install(name: &str, home: &Path) -> Result<(), CompanionError> {
    let tree = embedded(name).ok_or_else(|| CompanionError::Unknown(name.to_string()))?;
    install_tree(name, &tree, home)
}

/// Refresh a companion: an unconditional re-copy of the embedded folder
/// (restoring a backup an earlier failure kept first, [`Installed`]).
pub fn refresh(name: &str, home: &Path) -> Result<Installed, CompanionError> {
    let tree = embedded(name).ok_or_else(|| CompanionError::Unknown(name.to_string()))?;
    install_tree_with(name, &tree, home, &RealFs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> CompanionTree {
        CompanionTree::new()
            .file("SKILL.md", b"new".to_vec())
            .file("references/r.md", b"r".to_vec())
    }

    #[test]
    fn a_failed_verify_keeps_the_existing_folder() {
        let home = tempfile::TempDir::new().unwrap();
        install_tree("specmint-core", &tree(), home.path()).unwrap();
        let dest = installed_dir("specmint-core", home.path());
        fs::write(dest.join("references").join("r.md"), "old").unwrap();

        let err = install_with(
            "specmint-core",
            &tree(),
            home.path(),
            |_| Err(CompanionError::Invalid("verify failed".into())),
            &RealFs,
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "verify failed");
        assert_eq!(
            fs::read_to_string(dest.join("references").join("r.md")).unwrap(),
            "old"
        );
        let names: Vec<_> = fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["specmint-core"]);
    }

    #[test]
    fn allowlist() {
        assert!(allowlisted("SKILL.md", false));
        assert!(!allowlisted("SKILL.md", true));
        assert!(allowlisted("references", true));
        assert!(!allowlisted("references", false));
        assert!(allowlisted("assets/a/b.js", false));
        assert!(!allowlisted("README.md", false));
        assert!(!allowlisted("evals/x", false));
        assert!(!allowlisted("references/.x", false));
        assert!(!allowlisted("references/a/.x/b", false));
        assert!(!allowlisted("assets/__pycache__/x", false));
        assert!(!allowlisted("references//x", false));
    }
}
