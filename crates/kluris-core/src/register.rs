//! `kluris register` and `kluris remove` (research-03 §1.2, §1.12; v2
//! `cli.py:677-796`, `cli.py:1875-1897`).
//!
//! `register` takes a brain directory where it is: its identity comes from
//! `brain.md` ([`scaffold::read_brain_identity`], v2's name rule, so a
//! brain v2 made keeps its name), a missing `kluris.yml` is written, and the
//! brain is added to the global config. `remove` takes it out again; the
//! files stay. Both leave the skill install to the caller, as its last step.
//!
//! [`scaffold::read_brain_identity`]: crate::scaffold::read_brain_identity

use std::fmt;
use std::path::{Path, PathBuf};

use crate::brain::sandbox;
use crate::companions::{self, prompt::Aborted};
use crate::config::{
    BrainConfig, BrainEntry, ConfigError, read_brain_config, read_global_config, register_brain,
    unregister_brain, write_brain_config,
};
use crate::pycase;
use crate::pyrepr::py_path_str;
use crate::resolve::ResolveError;
use crate::scaffold::{fallback_name, is_valid_v2_name, read_brain_identity};

/// What `register` works with.
#[derive(Debug, Clone, Copy)]
pub struct RegisterRequest<'a> {
    /// SOURCE as given.
    pub source: &'a str,
    /// The home `~` expands to (Python's `expanduser`: `HOME` on POSIX,
    /// `USERPROFILE` on Windows); `None` leaves `~` as written.
    pub user_home: Option<&'a Path>,
    pub config_path: &'a Path,
    /// The HOME companions install under.
    pub home: &'a Path,
}

/// What `register` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// Registered now, with the companions the picker added.
    New {
        name: String,
        path: String,
        companions: Vec<String>,
    },
    /// Already registered under this name at this path: nothing changed.
    Already { name: String, path: String },
}

/// Why `register` stopped; `Display` is v2's message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterError {
    Refused(String),
    /// v2's `Register failed: <exception>`.
    Failed(String),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegisterError::Refused(message) => f.write_str(message),
            RegisterError::Failed(detail) => write!(f, "Register failed: {detail}"),
        }
    }
}

impl std::error::Error for RegisterError {}

fn failed(err: impl fmt::Display) -> RegisterError {
    RegisterError::Failed(err.to_string())
}

/// Python's `Path.expanduser()` for `~` and `~/...` (`~user` is kept).
fn expand_user(source: &str, user_home: Option<&Path>) -> String {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let Some(rest) = source.strip_prefix('~') else {
        return source.to_string();
    };
    if !(rest.is_empty() || rest.starts_with(separators)) {
        return source.to_string();
    }
    let Some(home) = user_home else {
        return source.to_string();
    };
    let home = home.display().to_string();
    let home = home.trim_end_matches(separators);
    let joined = format!("{home}{rest}");
    if joined.is_empty() {
        "/".to_string()
    } else {
        joined
    }
}

/// `PurePath.suffix`, lowercased: the last part's text from its last `.`,
/// when that is neither its first nor its last character.
fn suffix_lower(path: &str) -> String {
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let name = path.rsplit(separators).next().unwrap_or_default();
    match name.rfind('.') {
        Some(at) if at > 0 && at + 1 < name.len() => pycase::lower(&name[at..]),
        _ => String::new(),
    }
}

/// Whether two resolved paths are the same (case-insensitively on Windows,
/// as Python compares `WindowsPath`s).
fn same_path(a: &Path, b: &Path) -> bool {
    sandbox::is_within(a, b) && sandbox::is_within(b, a)
}

fn resolve(path: &Path) -> Result<PathBuf, RegisterError> {
    sandbox::resolve(path).map_err(|_| failed(format!("Symlink loop from {}", path.display())))
}

/// Register the brain at `req.source`. `pick` is the companion picker, run
/// once the brain checks out (the caller passes one that picks nothing when
/// v2 would not ask).
pub fn register(
    req: &RegisterRequest,
    pick: impl FnOnce() -> Result<Vec<String>, Aborted>,
) -> Result<Registered, RegisterError> {
    let source = py_path_str(&expand_user(req.source, req.user_home));
    if suffix_lower(&source) == ".zip" {
        return Err(RegisterError::Refused(
            "kluris register no longer accepts .zip files. Unzip first, then run \
             'kluris register <directory>'. For git-hosted brains, use 'git clone <url> <path>' \
             then 'kluris register <path>'."
                .into(),
        ));
    }
    let source_path = Path::new(&source);
    if !source_path.exists() {
        return Err(RegisterError::Refused(format!("Path not found: {source}")));
    }
    if !source_path.is_dir() {
        return Err(RegisterError::Refused(format!(
            "{source} is not a directory."
        )));
    }
    let root = resolve(source_path)?;
    if !root.join("brain.md").exists() {
        return Err(RegisterError::Refused(format!(
            "{} is not a Kluris brain (missing brain.md).",
            root.display()
        )));
    }

    let dir_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (name, description) = read_brain_identity(&root, &fallback_name(&dir_name));
    if !is_valid_v2_name(&name) {
        return Err(RegisterError::Refused(format!(
            "Brain name '{name}' (from brain.md) is not a valid kluris brain name. Edit brain.md \
             so the H1 heading is lowercase alphanumeric + hyphens (max 48 chars, not 'all')."
        )));
    }

    let path = root.display().to_string();
    let config = read_global_config(req.config_path).map_err(failed)?;
    if let Some(entry) = config.brains.get(&name) {
        let existing = resolve(Path::new(&py_path_str(&entry.path)))?;
        if same_path(&existing, &root) {
            return Ok(Registered::Already { name, path });
        }
        return Err(RegisterError::Refused(format!(
            "A brain named '{name}' is already registered at {}. Run 'kluris remove {name}' \
             first if you want to re-register it from {path}.",
            existing.display()
        )));
    }
    for (other, entry) in &config.brains {
        let existing = resolve(Path::new(&py_path_str(&entry.path)))?;
        if same_path(&existing, &root) {
            return Err(RegisterError::Refused(format!(
                "The directory {path} is already registered as brain '{other}'. Run 'kluris \
                 remove {other}' first to re-register it under a different name."
            )));
        }
    }

    if !root.join("kluris.yml").exists() {
        let mut local = BrainConfig::new(name.as_str());
        local.description = description.clone();
        write_brain_config(&root, &local).map_err(failed)?;
    }
    // v2 catches Click's `Abort` here too: `Register failed: ` and nothing more.
    let selected = companions::normalize(&pick().map_err(|Aborted| failed(""))?);
    if !selected.is_empty() {
        let mut config = read_brain_config(&root).map_err(failed)?;
        config.companions = selected.clone();
        write_brain_config(&root, &config).map_err(failed)?;
        for companion in &selected {
            companions::install(companion, req.home).map_err(failed)?;
        }
    }
    let brain_config = read_brain_config(&root).map_err(failed)?;
    let description = if brain_config.description.is_empty() {
        description
    } else {
        brain_config.description
    };
    let entry = BrainEntry {
        path: path.clone(),
        description,
    };
    register_brain(req.config_path, &name, entry).map_err(failed)?;
    Ok(Registered::New {
        name,
        path,
        companions: selected,
    })
}

/// Why `remove` stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoveError {
    NotRegistered(String),
    Config(ConfigError),
}

impl fmt::Display for RemoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoveError::NotRegistered(name) => {
                write!(f, "{}", ResolveError::UnknownBrain(name.clone()))
            }
            RemoveError::Config(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RemoveError {}

/// Take `name` out of the global config; its files stay where they are.
pub fn remove(config_path: &Path, name: &str) -> Result<(), RemoveError> {
    let config = read_global_config(config_path).map_err(RemoveError::Config)?;
    if !config.brains.contains_key(name) {
        return Err(RemoveError::NotRegistered(name.to_string()));
    }
    unregister_brain(config_path, name).map_err(RemoveError::Config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_user_like_python() {
        let home = Path::new("/home/u/");
        assert_eq!(expand_user("~", Some(home)), "/home/u");
        assert_eq!(expand_user("~/b", Some(home)), "/home/u/b");
        assert_eq!(expand_user("~other/b", Some(home)), "~other/b");
        assert_eq!(expand_user("a/~", Some(home)), "a/~");
        assert_eq!(expand_user("~/b", None), "~/b");
        assert_eq!(expand_user("~", Some(Path::new("/"))), "/");
    }

    #[test]
    fn suffix_like_pathlib() {
        assert_eq!(suffix_lower("x.zip"), ".zip");
        assert_eq!(suffix_lower("a/X.ZIP"), ".zip");
        assert_eq!(suffix_lower(".zip"), "");
        assert_eq!(suffix_lower("a."), "");
        assert_eq!(suffix_lower("a.tar.zip"), ".zip");
        assert_eq!(suffix_lower("x.zip/b"), "");
    }
}
