//! A brain's companion opt-ins: the `companions` list of its `kluris.yml`,
//! changed by `kluris companion add|remove`.
//!
//! Every resolved brain's `kluris.yml` is read before anything is written,
//! so a brain whose file is missing or does not read stops the command
//! with an error and no change to any brain.

use std::path::{Path, PathBuf};

use super::{CompanionError, install, normalize};
use crate::config::{
    BrainConfig, ConfigError, brain_companions, read_brain_config, write_brain_config,
};
use crate::pyrepr::py_path_str;
use crate::resolve::ResolvedBrain;

/// A resolved brain with its `kluris.yml` read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptInBrain {
    pub name: String,
    pub path: PathBuf,
    pub config: BrainConfig,
}

/// Read every brain's `kluris.yml`; the first that does not read is the
/// error.
pub fn load_brains(brains: &[ResolvedBrain]) -> Result<Vec<OptInBrain>, ConfigError> {
    brains
        .iter()
        .map(|brain| {
            let path = PathBuf::from(py_path_str(&brain.entry.path));
            let config = read_brain_config(&path)?;
            Ok(OptInBrain {
                name: brain.name.clone(),
                path,
                config,
            })
        })
        .collect()
}

/// The companions enabled on any of `brains`, known ones only, in canonical
/// order.
pub fn enabled(brains: &[OptInBrain]) -> Vec<String> {
    let all: Vec<&str> = brains
        .iter()
        .flat_map(|b| b.config.companions.iter().map(String::as_str))
        .collect();
    normalize(&all)
}

fn write(brain: &OptInBrain, companions: Vec<String>) -> Result<(), CompanionError> {
    let mut config = brain.config.clone();
    config.companions = companions;
    write_brain_config(Path::new(&brain.path), &config)?;
    Ok(())
}

/// Install each of `selected` into `home`, then opt every brain into them
/// (`normalize(old + selected)`); returns the brains written: all of them.
pub fn add(
    brains: &[OptInBrain],
    selected: &[String],
    home: &Path,
) -> Result<Vec<String>, CompanionError> {
    for name in selected {
        install(name, home)?;
    }
    let mut changed = Vec::new();
    for brain in brains {
        let mut companions = brain.config.companions.clone();
        companions.extend(selected.iter().cloned());
        write(brain, normalize(&companions))?;
        changed.push(brain.name.clone());
    }
    Ok(changed)
}

/// Opt the brains that have any of `selected` out of them (the rest of the
/// list normalized); a brain without any of them is not touched: its
/// `kluris.yml` is rewritten only when the opt-out changes it. Returns
/// the brains written. The runtime folders are kept.
pub fn remove(brains: &[OptInBrain], selected: &[String]) -> Result<Vec<String>, CompanionError> {
    let mut changed = Vec::new();
    for brain in brains {
        let had = brain.config.companions.iter().any(|c| selected.contains(c));
        if !had {
            continue;
        }
        let kept: Vec<&String> = brain
            .config
            .companions
            .iter()
            .filter(|c| !selected.contains(c))
            .collect();
        write(brain, normalize(&kept))?;
        changed.push(brain.name.clone());
    }
    Ok(changed)
}

/// One brain's row in `kluris companion list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainOptIns {
    pub name: String,
    /// As stored in `config.yml`.
    pub path: String,
    /// The known companions it opts into; `None` when its `kluris.yml` is
    /// missing or does not read.
    pub companions: Option<Vec<String>>,
}

/// The rows of `kluris companion list` for `brains`, in their order.
pub fn listing(brains: &[ResolvedBrain]) -> Vec<BrainOptIns> {
    brains
        .iter()
        .map(|brain| BrainOptIns {
            name: brain.name.clone(),
            path: brain.entry.path.clone(),
            companions: brain_companions(Path::new(&py_path_str(&brain.entry.path))),
        })
        .collect()
}
