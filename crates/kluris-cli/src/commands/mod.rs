//! One module per command: call `kluris-core`, print the result.

pub mod commit;
pub mod companion;
pub mod create;
pub mod doctor;
pub mod dream;
pub mod help;
pub mod list;
pub mod mri;
pub mod register;
pub mod remove;
pub mod resolve;
pub mod search;
pub mod self_uninstall;
pub mod self_update;
pub mod status;
pub mod sync;
pub mod wake_up;
pub mod watch;

use std::path::Path;

/// Bring the brain at `path` up to date from its remote before a read
/// (`kluris_core::sync::fresh`): quiet, bounded, never failing the read.
/// Off when `KLURIS_NO_AUTO_SYNC` is exactly `1`.
pub(crate) fn fresh_read(path: &Path) {
    let off = std::env::var_os("KLURIS_NO_AUTO_SYNC").is_some_and(|v| v == "1");
    let _ = kluris_core::sync::fresh::refresh(path, off);
}
