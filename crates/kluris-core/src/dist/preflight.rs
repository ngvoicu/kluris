//! The pre-flight of `kluris self-uninstall`: every planned change checked
//! before the first one is made.

use std::path::PathBuf;

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

/// The blockers of `planned`; none when every change can be made.
pub fn preflight(_planned: &[Planned]) -> Vec<Blocker> {
    Vec::new()
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
