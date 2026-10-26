//! The path sandbox: Python's `Path.resolve()` (non-strict), v2's
//! `is_within_brain` and `resolve_in_brain`.
//!
//! v2 decides with these whether a symlinked neuron escapes the brain
//! (`neuron_index._escapes_brain`), whether a `replaced_by` target is in the
//! brain, and which neuron a `related` entry points at. Kluris 3 resolves
//! paths the way Python 3.11 does, so the same links mean the same thing:
//! on POSIX, `..` is applied after the symlinks before it are followed (not
//! lexically), a missing tail is kept as written, a broken link resolves to
//! its target's path, and a symlink loop is an error.
//!
//! Translated from CPython 3.11 `Lib/posixpath.py` (`realpath`,
//! `_joinrealpath`) and `Lib/ntpath.py` (`realpath`, `normpath`,
//! `_readlink_deep`); see LICENSE-PSF.

use std::path::{Path, PathBuf};

/// `Path.resolve()` met a symlink loop (Python raises `RuntimeError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymlinkLoop;

/// Why [`resolve_in_brain`] refused a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxError {
    /// The path is empty.
    Empty,
    /// It resolves outside the brain (or through a symlink loop).
    Outside,
    /// It resolves inside the brain but nothing is there.
    NotFound,
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SandboxError::Empty => "the path is empty",
            SandboxError::Outside => "the path is outside the brain",
            SandboxError::NotFound => "the path does not exist",
        })
    }
}

impl std::error::Error for SandboxError {}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir().map_or_else(|_| path.to_path_buf(), |cwd| cwd.join(path))
}

/// Python 3.11's `Path.resolve()` (non-strict): symlinks are followed where
/// they exist, the rest of the path is kept, and a symlink loop is an error.
pub fn resolve(path: &Path) -> Result<PathBuf, SymlinkLoop> {
    let path = absolute(path);
    #[cfg(windows)]
    {
        windows::resolve(&path)
    }
    #[cfg(not(windows))]
    {
        posix::resolve(&path)
    }
}

#[cfg(not(windows))]
mod posix {
    //! `posixpath.realpath(strict=False)` as Python 3.11 has it.

    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Component, Path, PathBuf};

    use super::SymlinkLoop;

    /// One step of a path still to resolve.
    enum Part {
        Parent,
        Name(OsString),
    }

    /// The steps of `path` after its root: `.` and empty components dropped.
    fn parts(path: &Path) -> Vec<Part> {
        path.components()
            .filter_map(|c| match c {
                Component::ParentDir => Some(Part::Parent),
                Component::Normal(name) => Some(Part::Name(name.to_os_string())),
                Component::CurDir | Component::RootDir | Component::Prefix(_) => None,
            })
            .collect()
    }

    /// `posixpath.realpath(strict=False)` plus pathlib's loop check. Python's
    /// `_joinrealpath` recurses into each link target; here each frame holds the
    /// steps still to take and the link whose target they spell, so a deep chain
    /// of links needs no deep call stack.
    pub(super) fn resolve(path: &Path) -> Result<PathBuf, SymlinkLoop> {
        struct Frame {
            rest: std::vec::IntoIter<Part>,
            link: Option<PathBuf>,
        }
        // A link's resolved path once known; `None` while its target is being
        // resolved (meeting it again then is a loop).
        let mut seen: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();
        let mut resolved = PathBuf::from("/");
        let mut stack = vec![Frame {
            rest: parts(path).into_iter(),
            link: None,
        }];
        while let Some(frame) = stack.last_mut() {
            let Some(part) = frame.rest.next() else {
                if let Some(link) = stack.pop().and_then(|f| f.link) {
                    seen.insert(link, Some(resolved.clone()));
                }
                continue;
            };
            let name = match part {
                Part::Parent => {
                    resolved.pop();
                    continue;
                }
                Part::Name(name) => name,
            };
            let candidate = resolved.join(&name);
            let is_link =
                fs::symlink_metadata(&candidate).is_ok_and(|m| m.file_type().is_symlink());
            if !is_link {
                resolved = candidate;
                continue;
            }
            match seen.get(&candidate) {
                Some(Some(done)) => {
                    resolved = done.clone();
                    continue;
                }
                Some(None) => return Err(SymlinkLoop),
                None => {}
            }
            let Ok(target) = fs::read_link(&candidate) else {
                resolved = candidate;
                continue;
            };
            seen.insert(candidate.clone(), None);
            if target.is_absolute() {
                resolved = PathBuf::from("/");
            }
            stack.push(Frame {
                rest: parts(&target).into_iter(),
                link: Some(candidate),
            });
        }
        Ok(resolved)
    }
}

#[cfg(windows)]
mod windows {
    //! `ntpath.realpath(strict=False)` as Python 3.11 has it: the path is
    //! normalized lexically first, the longest prefix the OS can resolve is
    //! resolved, a link it cannot resolve is followed by hand
    //! (`_readlink_deep`) and the rest is appended as written.

    use std::collections::HashSet;
    use std::fs;
    use std::io;
    use std::path::{Component, Path, PathBuf};

    use super::SymlinkLoop;

    const ERROR_CANT_RESOLVE_FILENAME: i32 = 1921;

    fn is_loop(e: &io::Error) -> bool {
        e.raw_os_error() == Some(ERROR_CANT_RESOLVE_FILENAME)
    }

    /// `\\?\C:\x` -> `C:\x` (`\\?\UNC\...` is kept).
    pub(super) fn strip_verbatim(path: PathBuf) -> PathBuf {
        let text = path.to_string_lossy();
        match text.strip_prefix(r"\\?\") {
            Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
            _ => path,
        }
    }

    /// `ntpath.normpath`: `.` dropped, `..` applied lexically.
    fn normpath(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for c in path.components() {
            match c {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    /// `_readlink_deep`: follow links by hand until a non-link or a repeat.
    fn readlink_deep(path: &Path) -> PathBuf {
        let mut seen = HashSet::new();
        let mut path = path.to_path_buf();
        while seen.insert(path.to_string_lossy().to_lowercase()) {
            let Ok(target) = fs::read_link(&path) else {
                break;
            };
            path = if target.is_absolute() {
                target
            } else {
                normpath(&path.parent().unwrap_or(Path::new("")).join(target))
            };
        }
        path
    }

    pub(super) fn resolve(path: &Path) -> Result<PathBuf, SymlinkLoop> {
        let normal = normpath(path);
        let mut head = normal.clone();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        let mut out = loop {
            match fs::canonicalize(&head) {
                Ok(real) => break strip_verbatim(real),
                Err(_) => {
                    let deep = readlink_deep(&head);
                    if deep != head {
                        break deep;
                    }
                    match (head.file_name(), head.parent()) {
                        (Some(name), Some(parent)) => {
                            tail.push(name.to_os_string());
                            head = parent.to_path_buf();
                        }
                        _ => return Ok(normal),
                    }
                }
            }
        };
        for name in tail.iter().rev() {
            out.push(name);
        }
        // pathlib's own check: a stat that fails with a loop error raises.
        match fs::metadata(&out) {
            Err(e) if is_loop(&e) => Err(SymlinkLoop),
            _ => Ok(out),
        }
    }
}

/// The comparable components of a resolved path: case-insensitive on Windows,
/// as `PureWindowsPath` compares them.
fn key(path: &Path) -> Vec<String> {
    path.components()
        .map(|c| {
            let text = c.as_os_str().to_string_lossy();
            if cfg!(windows) {
                text.to_lowercase()
            } else {
                text.into_owned()
            }
        })
        .collect()
}

/// Whether `resolved` is `resolved_root` or below it (both already resolved).
pub fn is_within(resolved: &Path, resolved_root: &Path) -> bool {
    let (path, root) = (key(resolved), key(resolved_root));
    path.len() >= root.len() && path[..root.len()] == root[..]
}

/// v2's `is_within_brain`: `resolved` is inside `brain_root` (inclusive),
/// after resolving `brain_root`; false when the root cannot be resolved.
pub fn is_within_brain(resolved: &Path, brain_root: &Path) -> bool {
    resolve(brain_root).is_ok_and(|root| is_within(resolved, &root))
}

/// v2's `resolve_in_brain`: `raw` (backslashes read as slashes, leading
/// slashes dropped) joined to the brain root and resolved, refused when it is
/// empty, resolves outside the brain (a symlink loop counts as outside, as in
/// v2's walk) or does not exist. Returns the resolved path.
pub fn resolve_in_brain(brain_root: &Path, raw: &str) -> Result<PathBuf, SandboxError> {
    if raw.is_empty() {
        return Err(SandboxError::Empty);
    }
    let rel = raw.replace('\\', "/");
    let rel = rel.trim_start_matches('/');
    let joined = rel
        .split('/')
        .fold(brain_root.to_path_buf(), |path, part| path.join(part));
    let candidate = resolve(&joined).map_err(|SymlinkLoop| SandboxError::Outside)?;
    if !is_within_brain(&candidate, brain_root) {
        return Err(SandboxError::Outside);
    }
    if !candidate.exists() {
        return Err(SandboxError::NotFound);
    }
    Ok(candidate)
}
