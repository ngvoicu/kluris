//! The pre-flight of `kluris self-uninstall` (SPEC decision 2026-10-26,
//! T-119): every change the uninstall plans is checked before the first one
//! is made, without changing anything.
//!
//! - Only NotFound means "not there": any other error looking at a path
//!   (a folder on the way that cannot be searched, a file that cannot be
//!   read) blocks.
//! - An rc file to edit must be readable and writable, and so must the
//!   folder of the file it really is (a symlink's target): the rewrite is a
//!   temp file renamed over it ([`crate::atomic::write_atomic`]).
//! - An entry to remove needs a writable and searchable folder; a tree to
//!   remove needs every folder in it readable, writable and searchable
//!   (links are not followed, as the removal does not follow them).
//! - macOS: no immutable or append-only flag (`chflags`) on anything removed
//!   or edited, nor on the folder it leaves. Windows: no read-only attribute
//!   on a file removed or edited (a folder's is ignored: Explorer sets it on
//!   customised folders and it does not stop a removal).
//!
//! The access checks are the effective user's (`faccessat` with
//! `AT_EACCESS`); ACLs on Windows are not checked, nor Linux's `chattr`
//! flags: a removal those stop fails later, and the uninstall says what is
//! left.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::atomic::resolve_link_target;

/// One change the uninstall plans, as the pre-flight checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Planned {
    /// A file rewritten in place (an rc file).
    Edit(PathBuf),
    /// A file (or link) removed.
    Remove(PathBuf),
    /// A folder removed with everything in it.
    RemoveTree(PathBuf),
    /// A folder removed once empty.
    RemoveDir(PathBuf),
}

/// Something in the way of a planned change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub path: PathBuf,
    /// What is wrong with it.
    pub problem: String,
    /// What to do about it.
    pub fix: String,
}

impl Blocker {
    /// `path` could not be looked at (`e` is not NotFound).
    pub(super) fn unchecked(path: &Path, e: &io::Error) -> Blocker {
        Blocker {
            path: path.to_path_buf(),
            problem: format!("cannot be checked: {e}"),
            fix: if cfg!(unix) {
                "make every folder on its path searchable to you (chmod u+x) and it readable \
                 (chmod u+r)"
                    .to_string()
            } else {
                "make sure you can read it and the folders it is in".to_string()
            },
        }
    }
}

/// The blockers of `planned`, one per path; none when every change can be
/// made.
pub fn preflight(planned: &[Planned]) -> Vec<Blocker> {
    let mut found = Found::default();
    for change in planned {
        match change {
            Planned::Edit(path) => check_edit(path, &mut found),
            Planned::Remove(path) | Planned::RemoveDir(path) => check_remove(path, &mut found),
            Planned::RemoveTree(path) => check_tree(path, &mut found),
        }
    }
    found.0
}

/// The blockers so far, the first one per path.
#[derive(Default)]
struct Found(Vec<Blocker>);

impl Found {
    fn push(&mut self, blocker: Blocker) {
        if !self.0.iter().any(|b| b.path == blocker.path) {
            self.0.push(blocker);
        }
    }
}

/// What a folder or file must allow.
#[derive(Clone, Copy)]
enum Need {
    /// A file rewritten: read and write.
    ReadWrite,
    /// A folder an entry leaves (or a temp file is renamed into): write
    /// and search.
    Leave,
    /// A folder in a removed tree: also read, to list it.
    Clear,
}

fn check_edit(path: &Path, found: &mut Found) {
    let target = match resolve_link_target(path) {
        Ok(target) => target,
        Err(e) => return found.push(Blocker::unchecked(path, &e)),
    };
    let meta = match fs::metadata(&target) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => return found.push(Blocker::unchecked(path, &e)),
    };
    if !meta.is_file() {
        return;
    }
    let why = format!("it loses the kluris PATH line of {}", path.display());
    need(&target, Need::ReadWrite, &why, found);
    attributes(&target, &meta, found);
    folder_of(&target, &why, found);
}

fn check_remove(path: &Path, found: &mut Found) {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => return found.push(Blocker::unchecked(path, &e)),
    };
    attributes(path, &meta, found);
    folder_of(
        path,
        &format!("{} is removed from it", path.display()),
        found,
    );
}

fn check_tree(root: &Path, found: &mut Found) {
    let meta = match fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => return found.push(Blocker::unchecked(root, &e)),
    };
    folder_of(
        root,
        &format!("{} is removed from it", root.display()),
        found,
    );
    attributes(root, &meta, found);
    if !meta.is_dir() {
        return;
    }
    let why = format!("it is removed with everything in {}", root.display());
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        need(&dir, Need::Clear, &why, found);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                found.push(Blocker::unchecked(&dir, &e));
                continue;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => paths.push(entry.path()),
                Err(e) => found.push(Blocker::unchecked(&dir, &e)),
            }
        }
        paths.sort();
        for path in paths {
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    found.push(Blocker::unchecked(&path, &e));
                    continue;
                }
            };
            attributes(&path, &meta, found);
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
}

/// The folder `path` is in must let an entry go (or a temp file be
/// renamed in): writable and searchable, no immutable flag.
fn folder_of(path: &Path, why: &str, found: &mut Found) {
    let Some(folder) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };
    need(folder, Need::Leave, why, found);
    match fs::metadata(folder) {
        Ok(meta) => attributes(folder, &meta, found),
        Err(e) => found.push(Blocker::unchecked(folder, &e)),
    }
}

/// Whether the effective user has `what` on `path`.
#[cfg(unix)]
fn need(path: &Path, what: Need, why: &str, found: &mut Found) {
    use rustix::fs::{Access, AtFlags, CWD, accessat};
    let (access, problem, mode) = match what {
        Need::ReadWrite => (
            Access::READ_OK | Access::WRITE_OK,
            "is not readable and writable",
            "u+rw",
        ),
        Need::Leave => (
            Access::WRITE_OK | Access::EXEC_OK,
            "folder is not writable and searchable",
            "u+wx",
        ),
        Need::Clear => (
            Access::READ_OK | Access::WRITE_OK | Access::EXEC_OK,
            "folder is not readable, writable and searchable",
            "u+rwx",
        ),
    };
    match accessat(CWD, path, access, AtFlags::EACCESS) {
        Ok(()) => {}
        Err(e) if e == rustix::io::Errno::NOENT => {}
        Err(e)
            if e == rustix::io::Errno::ACCESS
                || e == rustix::io::Errno::ROFS
                || e == rustix::io::Errno::PERM =>
        {
            found.push(Blocker {
                path: path.to_path_buf(),
                problem: format!("{problem} ({why}): {}", io::Error::from(e)),
                fix: format!("chmod {mode} {}", sh_quote(path)),
            })
        }
        Err(e) => found.push(Blocker::unchecked(path, &io::Error::from(e))),
    }
}

/// Windows: the access an ACL gives is not checked.
#[cfg(not(unix))]
fn need(_path: &Path, _what: Need, _why: &str, _found: &mut Found) {}

/// macOS: the immutable and append-only flags, each of which stops a
/// removal, a rename over the file and a removal from the folder.
#[cfg(target_os = "macos")]
fn attributes(path: &Path, meta: &fs::Metadata, found: &mut Found) {
    use std::os::macos::fs::MetadataExt;
    const UF_IMMUTABLE: u32 = 0x0000_0002;
    const UF_APPEND: u32 = 0x0000_0004;
    const SF_IMMUTABLE: u32 = 0x0002_0000;
    const SF_APPEND: u32 = 0x0004_0000;
    let flags = meta.st_flags();
    let mut clear = Vec::new();
    let mut names = Vec::new();
    for (bit, name, off) in [
        (UF_IMMUTABLE, "uchg", "chflags nouchg"),
        (UF_APPEND, "uappnd", "chflags nouappnd"),
        (SF_IMMUTABLE, "schg", "sudo chflags noschg"),
        (SF_APPEND, "sappnd", "sudo chflags nosappnd"),
    ] {
        if flags & bit != 0 {
            names.push(name);
            clear.push(format!("{off} {}", sh_quote(path)));
        }
    }
    if !names.is_empty() {
        found.push(Blocker {
            path: path.to_path_buf(),
            problem: format!("has the {} flag set", names.join(" and ")),
            fix: clear.join("; "),
        });
    }
}

/// Windows: the read-only attribute, which stops a file's removal or
/// rewrite.
#[cfg(windows)]
fn attributes(path: &Path, meta: &fs::Metadata, found: &mut Found) {
    if !meta.is_dir() && meta.permissions().readonly() {
        found.push(Blocker {
            path: path.to_path_buf(),
            problem: "has the read-only attribute".to_string(),
            fix: format!("attrib -r \"{}\"", path.display()),
        });
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn attributes(_path: &Path, _meta: &fs::Metadata, _found: &mut Found) {}

/// `path` quoted for a POSIX shell.
#[cfg(unix)]
fn sh_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn names(blockers: &[Blocker]) -> Vec<&Path> {
        blockers.iter().map(|b| b.path.as_path()).collect()
    }

    #[cfg(unix)]
    fn chmod(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// A folder with a file to edit, a file to remove, a tree and an empty
    /// folder, all of which the uninstall may change.
    fn clear() -> (tempfile::TempDir, Vec<Planned>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("profile"), "x\n").unwrap();
        fs::write(root.join("env.sh"), "y\n").unwrap();
        fs::create_dir_all(root.join("tree/sub")).unwrap();
        fs::write(root.join("tree/sub/file"), "z").unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        let planned = vec![
            Planned::Edit(root.join("profile")),
            Planned::Remove(root.join("env.sh")),
            Planned::RemoveTree(root.join("tree")),
            Planned::RemoveDir(root.join("empty")),
            // Not there: nothing to change, nothing in the way.
            Planned::Remove(root.join("absent")),
            Planned::Edit(root.join("absent.rc")),
        ];
        (dir, planned)
    }

    #[test]
    fn an_all_clear_plan_has_no_blocker() {
        let (_dir, planned) = clear();
        assert_eq!(preflight(&planned), Vec::new());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_checked_blocks() {
        // conf.d without search permission: the file in it cannot be looked
        // at, which is not the same as not being there.
        let (dir, _) = clear();
        let conf = dir.path().join("conf.d");
        fs::create_dir(&conf).unwrap();
        fs::write(conf.join("kluris.env.fish"), "source x\n").unwrap();
        chmod(&conf, 0o400);
        let blockers = preflight(&[Planned::Edit(conf.join("kluris.env.fish"))]);
        chmod(&conf, 0o700);
        assert_eq!(
            names(&blockers),
            [conf.join("kluris.env.fish").as_path()],
            "{blockers:?}"
        );
        assert!(
            blockers[0].problem.contains("cannot be checked"),
            "{blockers:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_to_edit_that_is_not_writable_blocks() {
        let (dir, _) = clear();
        let profile = dir.path().join("profile");
        chmod(&profile, 0o444);
        let blockers = preflight(&[Planned::Edit(profile.clone())]);
        chmod(&profile, 0o644);
        assert_eq!(names(&blockers), [profile.as_path()], "{blockers:?}");
        assert!(blockers[0].fix.contains("chmod"), "{blockers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_an_entry_must_leave_that_is_not_writable_blocks() {
        let (dir, _) = clear();
        let folder = dir.path().join("receipt-folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("kluris-receipt.json"), "{}").unwrap();
        chmod(&folder, 0o500);
        let blockers = preflight(&[
            Planned::Remove(folder.join("kluris-receipt.json")),
            Planned::Edit(dir.path().join("profile")),
        ]);
        chmod(&folder, 0o700);
        assert_eq!(names(&blockers), [folder.as_path()], "{blockers:?}");
        assert!(blockers[0].problem.contains("writable"), "{blockers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn every_folder_of_a_tree_to_remove_must_be_writable_and_searchable() {
        let (dir, _) = clear();
        let sub = dir.path().join("tree/sub");
        chmod(&sub, 0o500);
        let blockers = preflight(&[Planned::RemoveTree(dir.path().join("tree"))]);
        chmod(&sub, 0o700);
        assert_eq!(names(&blockers), [sub.as_path()], "{blockers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn an_edited_file_s_folder_must_take_the_atomic_rewrite() {
        // The rewrite is a temp file renamed over the file: its folder (the
        // link's target's, for a linked profile) must be writable.
        let (dir, _) = clear();
        let dotfiles = dir.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        fs::write(dotfiles.join("profile"), "x\n").unwrap();
        std::os::unix::fs::symlink(dotfiles.join("profile"), dir.path().join("linked")).unwrap();
        chmod(&dotfiles, 0o500);
        let blockers = preflight(&[Planned::Edit(dir.path().join("linked"))]);
        chmod(&dotfiles, 0o700);
        assert_eq!(names(&blockers), [dotfiles.as_path()], "{blockers:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_immutable_flag_blocks() {
        let (dir, _) = clear();
        let env_sh = dir.path().join("env.sh");
        let flags = |on: &str| {
            assert!(
                std::process::Command::new("chflags")
                    .arg(on)
                    .arg(&env_sh)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        flags("uchg");
        let blockers = preflight(&[Planned::Remove(env_sh.clone())]);
        flags("nouchg");
        assert_eq!(names(&blockers), [env_sh.as_path()], "{blockers:?}");
        assert!(blockers[0].fix.contains("chflags nouchg"), "{blockers:?}");
    }

    #[cfg(windows)]
    #[test]
    fn a_read_only_attribute_blocks() {
        let (dir, _) = clear();
        let env_sh = dir.path().join("env.sh");
        let mut perms = fs::metadata(&env_sh).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&env_sh, perms.clone()).unwrap();
        let blockers = preflight(&[Planned::Remove(env_sh.clone())]);
        perms.set_readonly(false);
        fs::set_permissions(&env_sh, perms).unwrap();
        assert_eq!(names(&blockers), [env_sh.as_path()], "{blockers:?}");
        assert!(blockers[0].fix.contains("attrib -r"), "{blockers:?}");
    }
}
