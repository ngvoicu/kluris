//! The tarball ([`crate::ssh_ci::pack`]) with the PowerShell run script in it.

use std::path::Path;

use super::script::RUN_FILE;
pub use crate::ssh_ci::pack::channel;

/// Writes the tarball of the worktree at `root` to `tarball`, with `run_script`
/// in it, and answers the commit it was made from.
#[cfg(test)]
pub fn pack(root: &Path, tarball: &Path, run_script: &str) -> Result<String, String> {
    crate::ssh_ci::pack::pack(root, tarball, RUN_FILE, run_script)
}

/// [`pack`], with `files` (path in the tarball, text) beside the tree: the
/// site wrapper of `--wrapper`.
pub fn pack_with_files(
    root: &Path,
    tarball: &Path,
    run_script: &str,
    files: &[(String, String)],
) -> Result<String, String> {
    crate::ssh_ci::pack::pack_with_files(root, tarball, RUN_FILE, run_script, files)
}
