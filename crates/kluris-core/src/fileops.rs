//! The file operations the skill and companion installs move and delete
//! with, behind a trait so tests can make one fail or race ([`FileOps`]);
//! the binary always uses [`RealFs`]. Also the helpers those installs
//! share: a run-unique name token ([`unique_token`]), a folder's
//! physical identity ([`folder_id`]), and [`under`] for joining a
//! `/`-relative path under a base.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};

/// Renames and removals, one call each. Every method defaults to `std::fs`.
pub trait FileOps {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }
    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir_all(path)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }
    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir(path)
    }
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        fs::write(path, bytes)
    }
}

/// `std::fs` itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

impl FileOps for RealFs {}

/// `<pid>-<16 lowercase hex digits>`, new on every call: the part of a
/// staging or quarantine name no other run (and no user) can have made.
pub fn unique_token() -> String {
    // Each RandomState is keyed afresh, so each call hashes differently.
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    format!("{}-{:016x}", std::process::id(), hasher.finish())
}

/// Whether `token` is what [`unique_token`] makes.
pub fn is_unique_token(token: &str) -> bool {
    let Some((pid, hex)) = token.split_once('-') else {
        return false;
    };
    !pid.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && hex.len() == 16
        && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `rel` (a `/`-separated relative path) joined under `base` component by
/// component, so the stored path's separators are the OS's own: a plain
/// `base.join(rel)` keeps the `/` inside one component (`C:\u\.agents/skills`
/// on Windows), which `display()` and reports then show.
pub fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|part| !part.is_empty())
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

/// The physical identity of a folder: two paths with the same identity are
/// one folder (a symlink, a case variant on a case-insensitive file
/// system, `..` spellings). `None` when `path` is not a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderId(Id);

#[cfg(unix)]
type Id = (u64, u64);
#[cfg(not(unix))]
type Id = String;

/// See [`FolderId`]: device and inode on Unix; elsewhere the canonical
/// path, compared without case (NTFS).
pub fn folder_id(path: &Path) -> Option<FolderId> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_dir() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(FolderId((meta.dev(), meta.ino())))
    }
    #[cfg(not(unix))]
    {
        let canonical = fs::canonicalize(path).ok()?;
        Some(FolderId(canonical.to_string_lossy().to_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_recognised() {
        let a = unique_token();
        let b = unique_token();
        assert_ne!(a, b);
        assert!(is_unique_token(&a), "{a}");
        assert!(a.starts_with(&format!("{}-", std::process::id())));
        for bad in [
            "",
            "-0123456789abcdef",
            "12-0123456789abcde",
            "12-0123456789ABCDEF",
            "x2-0123456789abcdef",
            "12_0123456789abcdef",
        ] {
            assert!(!is_unique_token(bad), "{bad}");
        }
    }

    /// `under` joins a `/`-string component by component, so the stored
    /// path's separators are the OS's own: a plain `join` keeps the `/`
    /// inside one component (`C:\u\.agents/skills` on Windows), which
    /// `display()` then shows (SPEC 2026-10-27: absolute paths are native;
    /// only `~/` labels and JSON `path` values are `/`-joined).
    #[test]
    fn under_joins_a_slash_path_component_by_component() {
        let base = Path::new(if cfg!(windows) { r"C:\u" } else { "/u" });
        let joined = under(base, ".agents/skills");
        // The same components as a plain `join`; the stored separators are
        // the difference.
        assert_eq!(joined, base.join(".agents/skills"));
        assert_eq!(
            joined.as_os_str(),
            base.join(".agents").join("skills").as_os_str()
        );
        assert_eq!(under(base, "").as_os_str(), base.as_os_str());
        assert_eq!(
            under(base, "a//b/").as_os_str(),
            base.join("a").join("b").as_os_str()
        );
        #[cfg(windows)]
        assert!(!joined.to_string_lossy().contains('/'));
    }

    #[test]
    fn folder_ids_see_through_links() {
        let tmp = tempfile::TempDir::new().unwrap();
        let a = tmp.path().join("a");
        fs::create_dir(&a).unwrap();
        let b = tmp.path().join("b");
        fs::create_dir(&b).unwrap();
        assert_eq!(folder_id(&a), folder_id(&tmp.path().join("b/../a")));
        assert_ne!(folder_id(&a), folder_id(&b));
        assert_eq!(folder_id(&tmp.path().join("missing")), None);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&a, tmp.path().join("link")).unwrap();
            assert_eq!(folder_id(&a), folder_id(&tmp.path().join("link")));
        }
    }
}
