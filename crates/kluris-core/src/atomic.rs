//! The one way Kluris rewrites a file a user owns (brain files, `config.yml`,
//! `kluris.yml`, shell rc files): [`write_atomic`].
//!
//! The new bytes go to a temp file beside the real target, which is synced
//! and then renamed over it, so a failed or partial write (a full disk, a
//! file-size limit) never leaves the original truncated:
//! - a symlink is followed, also a dangling one, and stays a link: its
//!   target gets the new bytes (rc files and neurons symlinked into a
//!   dotfile repo);
//! - an existing target keeps its permissions; one that cannot be opened
//!   for writing (read-only) is refused, as an in-place write is. On Unix
//!   the temp file is created `0600` and only gets the target's permissions
//!   once its bytes are synced, so a crash never leaves a private file's
//!   contents readable by others. On Windows the temp file inherits the
//!   folder's ACL: an explicit ACL on the original is not carried over;
//! - the temp file is `.<name>.kluris-tmp-<pid>-<n>`, a dot file with no
//!   `.md`/`.yml`/`.yaml` suffix, so no walker reads it as a neuron;
//! - on Windows a sharing violation (an editor, an indexer or antivirus
//!   holding the file) is retried a few times, opening the target and
//!   renaming over it alike, then fails: there is no fallback to an
//!   in-place write;
//! - on every failure the temp file is removed and the original is left
//!   byte-identical.
//!
//! Not preserved: hard links (the target gets a new inode, so other names of
//! the old file keep the old bytes), ownership when the writer is not the
//! owner, and extended attributes.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// How many symlinks [`resolve_link_target`] follows (Linux's `MAXSYMLINKS`).
const MAX_SYMLINKS: usize = 40;

/// How many times a retryable probe or rename is tried in all.
const RENAME_ATTEMPTS: u32 = 6;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The file-system steps of [`write_atomic`] that can fail, so tests can
/// make each one fail. The shipped binary uses [`RealFs`] only.
pub(crate) trait AtomicFs {
    /// Open an existing target for writing (nothing is written): refused
    /// where an in-place write would be, a read-only file say.
    fn probe(&self, path: &Path) -> io::Result<()> {
        OpenOptions::new().write(true).open(path).map(drop)
    }
    fn write(&self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    }
    /// Give the synced temp file the target's permissions.
    fn set_permissions(&self, path: &Path, perms: fs::Permissions) -> io::Result<()> {
        fs::set_permissions(path, perms)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }
    /// A probe or rename error worth retrying: on Windows a sharing
    /// violation (32) or the access denied (5) a file held open by another
    /// process gives.
    fn retryable(&self, e: &io::Error) -> bool {
        cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32))
    }
    fn pause(&self, attempt: u32) {
        std::thread::sleep(Duration::from_millis(25 << attempt.min(4)));
    }
}

/// The real file system.
pub(crate) struct RealFs;

impl AtomicFs for RealFs {}

/// Replace the file at `path` with `bytes` atomically (see the module
/// documentation). A missing file is created.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, bytes, &RealFs)
}

/// The file a write to `path` lands on, as an in-place write sees it: each
/// symlink is followed (relative targets against the link's directory), also
/// when the last target does not exist yet; a path that is not a link is
/// itself.
pub fn resolve_link_target(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINKS {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let to = fs::read_link(&current)?;
                current = match current.parent() {
                    Some(dir) if to.is_relative() => dir.join(to),
                    _ => to,
                };
            }
            Ok(_) => return Ok(current),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "too many levels of symbolic links: {}",
        path.display()
    )))
}

/// `step`, tried again while it fails with a retryable error, at most
/// [`RENAME_ATTEMPTS`] times in all.
fn retrying(fs_ops: &dyn AtomicFs, mut step: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match step() {
            Err(e) if attempt < RENAME_ATTEMPTS && fs_ops.retryable(&e) => {
                fs_ops.pause(attempt);
                attempt += 1;
            }
            result => return result,
        }
    }
}

pub(crate) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    fs_ops: &dyn AtomicFs,
) -> io::Result<()> {
    let target = resolve_link_target(path)?;
    let existing = match fs::metadata(&target) {
        Ok(meta) => {
            retrying(fs_ops, || fs_ops.probe(&target))?;
            Some(meta.permissions())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let file_name = target.file_name().unwrap_or(OsStr::new("file"));
    let (tmp, mut file) = loop {
        let mut name = OsStr::new(".").to_os_string();
        name.push(file_name);
        name.push(format!(
            ".kluris-tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let tmp = dir.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        // Private before the first byte: the old file's contents may be
        // private, and a crash leaves the temp file behind. A new file
        // keeps the default mode, which is what it ends up with anyway.
        #[cfg(unix)]
        if existing.is_some() {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        fs_ops.write(&mut file, bytes)?;
        drop(file);
        if let Some(perms) = existing {
            fs_ops.set_permissions(&tmp, perms)?;
        }
        retrying(fs_ops, || fs_ops.rename(&tmp, &target))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    /// Every entry of `dir` by name.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Writes only the first half, then fails (a full disk, a size limit).
    struct ShortWrite;
    impl AtomicFs for ShortWrite {
        fn write(&self, file: &mut File, bytes: &[u8]) -> io::Result<()> {
            file.write_all(&bytes[..bytes.len() / 2])?;
            Err(io::Error::other("File too large"))
        }
    }

    /// Every rename fails; records the temp path it was given.
    #[derive(Default)]
    struct FailingRename {
        from: RefCell<Vec<PathBuf>>,
    }
    impl AtomicFs for FailingRename {
        fn rename(&self, from: &Path, _: &Path) -> io::Result<()> {
            self.from.borrow_mut().push(from.to_path_buf());
            Err(io::Error::other("rename refused"))
        }
    }

    /// The first `fails` renames fail with a retryable error.
    struct Busy {
        fails: u32,
        calls: Cell<u32>,
    }
    impl AtomicFs for Busy {
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() <= self.fails {
                return Err(io::Error::other("sharing violation"));
            }
            fs::rename(from, to)
        }
        fn retryable(&self, _: &io::Error) -> bool {
            true
        }
        fn pause(&self, _: u32) {}
    }

    #[test]
    fn creates_a_missing_file_and_replaces_an_existing_one() {
        let d = dir();
        let path = d.path().join("a.md");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        write_atomic(&path, b"two, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two, longer");
        assert_eq!(names(d.path()), ["a.md"]);
    }

    #[test]
    fn a_short_write_leaves_the_original_and_no_temp_file() {
        let d = dir();
        let path = d.path().join("large.md");
        let original = "x".repeat(22_009);
        fs::write(&path, &original).unwrap();
        let err = write_atomic_with(&path, "y".repeat(30_000).as_bytes(), &ShortWrite).unwrap_err();
        assert_eq!(err.to_string(), "File too large");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(names(d.path()), ["large.md"]);
    }

    #[test]
    fn a_failing_rename_leaves_the_original_and_no_temp_file() {
        let d = dir();
        let path = d.path().join("n.yml");
        fs::write(&path, "old").unwrap();
        let fs_ops = FailingRename::default();
        assert!(write_atomic_with(&path, b"new", &fs_ops).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(names(d.path()), ["n.yml"]);
        // The temp file sat beside the target, named so no walker takes it
        // for a neuron.
        let from = fs_ops.from.borrow();
        let tmp = from[0].file_name().unwrap().to_str().unwrap();
        assert_eq!(from[0].parent(), Some(d.path()));
        assert!(tmp.starts_with(".n.yml.kluris-tmp-"), "{tmp}");
        for suffix in [".md", ".yml", ".yaml"] {
            assert!(!tmp.to_lowercase().ends_with(suffix), "{tmp}");
        }
    }

    #[test]
    fn a_retryable_rename_is_retried_then_given_up() {
        let d = dir();
        let path = d.path().join("c.yml");
        fs::write(&path, "old").unwrap();
        let busy = Busy {
            fails: 2,
            calls: Cell::new(0),
        };
        write_atomic_with(&path, b"new", &busy).unwrap();
        assert_eq!(busy.calls.get(), 3);
        assert_eq!(fs::read(&path).unwrap(), b"new");

        let stuck = Busy {
            fails: u32::MAX,
            calls: Cell::new(0),
        };
        assert!(write_atomic_with(&path, b"newer", &stuck).is_err());
        assert_eq!(stuck.calls.get(), RENAME_ATTEMPTS);
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(names(d.path()), ["c.yml"]);
    }

    /// The first `fails` writability probes fail with a retryable error
    /// (on Windows a sharing violation opening the target).
    struct BusyProbe {
        fails: u32,
        calls: Cell<u32>,
    }
    impl AtomicFs for BusyProbe {
        fn probe(&self, path: &Path) -> io::Result<()> {
            self.calls.set(self.calls.get() + 1);
            if self.calls.get() <= self.fails {
                return Err(io::Error::other("sharing violation"));
            }
            RealFs.probe(path)
        }
        fn retryable(&self, _: &io::Error) -> bool {
            true
        }
        fn pause(&self, _: u32) {}
    }

    #[test]
    fn a_retryable_probe_is_retried_then_given_up() {
        let d = dir();
        let path = d.path().join("p.md");
        fs::write(&path, "old").unwrap();
        let busy = BusyProbe {
            fails: 2,
            calls: Cell::new(0),
        };
        write_atomic_with(&path, b"new", &busy).unwrap();
        assert_eq!(busy.calls.get(), 3);
        assert_eq!(fs::read(&path).unwrap(), b"new");

        let stuck = BusyProbe {
            fails: u32::MAX,
            calls: Cell::new(0),
        };
        assert!(write_atomic_with(&path, b"newer", &stuck).is_err());
        assert_eq!(stuck.calls.get(), RENAME_ATTEMPTS);
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(names(d.path()), ["p.md"]);
    }

    #[test]
    fn a_rename_error_is_not_retried_by_default_off_windows() {
        let e = io::Error::from_raw_os_error(32);
        assert_eq!(RealFs.retryable(&e), cfg!(windows));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_stays_a_link_and_its_target_gets_the_bytes() {
        use std::os::unix::fs::symlink;
        let d = dir();
        let dotfiles = d.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        fs::write(dotfiles.join("zshrc"), "old\n").unwrap();
        let link = d.path().join(".zshrc");
        symlink("dotfiles/zshrc", &link).unwrap();
        write_atomic(&link, b"new\n").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("dotfiles/zshrc"));
        assert_eq!(fs::read(dotfiles.join("zshrc")).unwrap(), b"new\n");
        assert_eq!(names(&dotfiles), ["zshrc"]);

        // A chain of links, the last one dangling: the file is created
        // where the chain ends.
        let chain = d.path().join("chain");
        symlink(".zshrc2", &chain).unwrap();
        symlink("dotfiles/new-rc", d.path().join(".zshrc2")).unwrap();
        write_atomic(&chain, b"made\n").unwrap();
        assert!(
            fs::symlink_metadata(&chain)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(dotfiles.join("new-rc")).unwrap(), b"made\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_loop_is_an_error_and_writes_nothing() {
        use std::os::unix::fs::symlink;
        let d = dir();
        symlink("b", d.path().join("a")).unwrap();
        symlink("a", d.path().join("b")).unwrap();
        assert!(write_atomic(&d.path().join("a"), b"x").is_err());
        assert_eq!(names(d.path()), ["a", "b"]);
    }

    /// Stops the write where a crash could: after the bytes are synced,
    /// before the target's permissions are set; records the temp file's
    /// mode at that moment.
    #[cfg(unix)]
    #[derive(Default)]
    struct CrashBeforePermissions {
        mode: Cell<Option<u32>>,
    }
    #[cfg(unix)]
    impl AtomicFs for CrashBeforePermissions {
        fn set_permissions(&self, path: &Path, _: fs::Permissions) -> io::Result<()> {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path)?.permissions().mode() & 0o7777;
            self.mode.set(Some(mode));
            Err(io::Error::other("interrupted"))
        }
    }

    /// A private file's new bytes are never readable by others, not even in
    /// the temp file a crash leaves behind (review T-106).
    #[cfg(unix)]
    #[test]
    fn the_temp_file_is_private_until_it_has_the_targets_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir();
        let path = d.path().join("private.md");
        fs::write(&path, "old secret").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let crash = CrashBeforePermissions::default();
        assert!(write_atomic_with(&path, b"new secret", &crash).is_err());
        let mode = crash
            .mode
            .get()
            .expect("the write reached the permissions step");
        assert_eq!(mode & 0o077, 0, "temp file mode {mode:o}");
        assert_eq!(fs::read(&path).unwrap(), b"old secret");
        assert_eq!(names(d.path()), ["private.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn permissions_are_kept() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir();
        let path = d.path().join("secret.yml");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_file_is_refused_and_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir();
        let path = d.path().join("locked.md");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        if OpenOptions::new().write(true).open(&path).is_ok() {
            return; // running as root: nothing is read-only
        }
        let err = write_atomic(&path, b"new").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(names(d.path()), ["locked.md"]);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_replaced_not_rewritten_in_place() {
        use std::os::unix::fs::MetadataExt;
        let d = dir();
        let path = d.path().join("a.md");
        fs::write(&path, "old").unwrap();
        let before = fs::metadata(&path).unwrap().ino();
        write_atomic(&path, b"new").unwrap();
        assert_ne!(fs::metadata(&path).unwrap().ino(), before);
    }
}
