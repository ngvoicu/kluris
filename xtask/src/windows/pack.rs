//! The tarball ([`crate::ssh_ci::pack`]) with the PowerShell run script in it.

use std::path::Path;

use super::script::RUN_FILE;
pub use crate::ssh_ci::pack::channel;

/// Writes the tarball of the worktree at `root` to `tarball`, with `run_script`
/// in it, and answers the commit it was made from.
pub fn pack(root: &Path, tarball: &Path, run_script: &str) -> Result<String, String> {
    crate::ssh_ci::pack::pack(root, tarball, RUN_FILE, run_script)
}
