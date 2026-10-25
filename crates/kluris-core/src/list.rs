//! `kluris list`: the registered brains with their companion state.

use std::path::Path;

use crate::config::{ConfigError, brain_companions, read_global_config};

/// One row of `kluris list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainListing {
    pub name: String,
    pub path: String,
    pub description: String,
    /// `None` when the brain's `kluris.yml` is missing or invalid.
    pub companions: Option<Vec<String>>,
}

/// List the brains registered in the global config at `config_path`, in
/// config order. Never fails on a stale brain path.
pub fn list_brains(config_path: &Path) -> Result<Vec<BrainListing>, ConfigError> {
    let config = read_global_config(config_path)?;
    Ok(config
        .brains
        .into_iter()
        .map(|(name, entry)| BrainListing {
            companions: brain_companions(Path::new(&entry.path)),
            name,
            path: entry.path,
            description: entry.description,
        })
        .collect())
}
