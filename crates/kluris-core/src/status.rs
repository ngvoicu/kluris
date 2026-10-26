//! `kluris status`: per brain, the lobe and neuron counts and, when the brain
//! is in a git work tree, its recent commits and uncommitted changes
//! (research-03 §1.4). The git calls are [`Git`]'s, scoped to the brain when
//! it is nested in a larger repo.

use crate::brain::store::BrainStore;
use crate::git::{Commit, Git, GitError};

/// How many commits `status` reads (the JSON lists them all, the text the
/// first five).
pub const RECENT_COMMITS: usize = 10;

/// One brain's status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainStatus {
    /// The top-level directories the walker does not prune.
    pub lobes: usize,
    /// Every neuron in the brain, root-level ones included.
    pub neurons: usize,
    pub git_enabled: bool,
    /// The newest commits first, at most [`RECENT_COMMITS`]; none without git.
    pub recent: Vec<Commit>,
    /// `git status --short`, stripped; empty when clean or without git.
    pub uncommitted: String,
}

/// The status of the brain in `store`.
pub fn brain_status(store: &impl BrainStore, git: &impl Git) -> Result<BrainStatus, GitError> {
    let lobes = store.lobe_names().len();
    let neurons = store.neurons().len();
    let (recent, uncommitted) = match git.repo(store.root()) {
        Some(repo) => (
            git.log(&repo, RECENT_COMMITS)?,
            Some(git.status_short(&repo)?),
        ),
        None => (Vec::new(), None),
    };
    Ok(BrainStatus {
        lobes,
        neurons,
        git_enabled: uncommitted.is_some(),
        recent,
        uncommitted: uncommitted.unwrap_or_default(),
    })
}
