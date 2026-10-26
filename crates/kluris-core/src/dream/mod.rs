//! `kluris dream` for one brain: v2's `_sync_brain_state` then its
//! validators (research-03 §2.5), with every write in one change set.
//!
//! Steps, in v2's order, each reading what the earlier ones wrote:
//! 1. `updated`/`created` from git ([`sync_dates`]);
//! 2. `parent: ./map.md` where it is missing ([`linker::fix_missing_parents`]);
//! 3. reverse `related` links ([`linker::fix_reverse_links`]);
//! 4. the orphan count before the maps;
//! 5. every directory's `map.md`, twice, deepest first ([`maps`]);
//! 6. `brain.md`.
//!
//! The writes are applied together; the validators (broken and one-way
//! synapses, orphans, frontmatter issues, deprecation) then run on the brain
//! as written. Differences from v2: dates are written in the quoted form
//! v2's dream converges to (PAR-033), YAML anchors as plain copies
//! (PAR-034), each file keeps its line endings and new files are LF
//! (PAR-035), pruned directories get no `map.md` and are not listed
//! (PAR-036), report paths are `/`-joined (PAR-037), and files are taken in
//! path order (PAR-008). A file that cannot be read or written is skipped,
//! reported and makes the brain unhealthy (PAR-038); odd `related` values
//! never crash (PAR-047); link checking skips code, other schemes and titles
//! and reads URL-encoded paths (PAR-049).

mod linker;
mod links;
mod maps;
mod sync_dates;
mod workspace;

use std::collections::{BTreeMap, HashSet};

use crate::brain::change::{Applied, ChangeSet};
use crate::brain::deprecation::{DeprecationIssue, detect_deprecation_issues};
use crate::brain::store::{BrainStore, ReadError};
use crate::git::Git;
use crate::yaml::Date;

/// What dream needs besides the brain.
#[derive(Debug, Clone, Copy)]
pub struct DreamOptions<'a> {
    /// `name` and `description` from the brain's `kluris.yml` (for brain.md).
    pub name: &'a str,
    pub description: &'a str,
    /// The local date, written as `updated` into regenerated maps.
    pub today: Date,
}

/// The automatic fixes, v2's `fixes`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fixes {
    pub dates_updated: usize,
    pub parents_inferred: usize,
    pub reverse_synapses_added: usize,
    pub orphan_references_added: usize,
    pub total: usize,
}

/// A link whose target does not exist or is outside the brain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenSynapse {
    pub file: String,
    /// As written in the link.
    pub target: String,
}

/// A `related` link the target does not return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneWaySynapse {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldProblem {
    Missing,
    /// Present with the wrong type (v2's `kind: "type"`).
    Type,
}

/// A required frontmatter field that is missing, or a field of the wrong
/// type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterIssue {
    pub file: String,
    pub field: &'static str,
    pub problem: FieldProblem,
}

/// A file dream left alone because it could not read or write it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedFile {
    pub rel: String,
    pub reason: String,
}

/// One brain's dream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DreamReport {
    pub fixes: Fixes,
    /// Every directory a map was generated for, in v2's order, `/`-joined.
    pub maps_regenerated: Vec<String>,
    /// The top-level lobes after the run.
    pub lobes: Vec<String>,
    pub broken: Vec<BrokenSynapse>,
    pub one_way: Vec<OneWaySynapse>,
    pub orphans: Vec<String>,
    pub frontmatter_issues: Vec<FrontmatterIssue>,
    pub deprecation: Vec<DeprecationIssue>,
    /// The files written, by path.
    pub written: Vec<String>,
    pub skipped: Vec<SkippedFile>,
}

impl DreamReport {
    /// No broken or one-way synapses, orphans, frontmatter issues or skipped
    /// files (v2 exits 1 on every file Kluris 3 skips, PAR-038); deprecation
    /// issues are warnings only.
    pub fn healthy(&self) -> bool {
        self.skipped.is_empty()
            && self.broken.is_empty()
            && self.one_way.is_empty()
            && self.orphans.is_empty()
            && self.frontmatter_issues.is_empty()
    }
}

/// v2's `_sync_brain_state` alone, the preflight `kluris mri` runs: dream's
/// writes without its validators. `orphan_references_added` compares the
/// orphans before the maps with the orphans after the writes, as v2 does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub fixes: Fixes,
    /// Every directory a map was generated for, in v2's order, `/`-joined.
    pub maps_regenerated: Vec<String>,
    /// The files written, by path.
    pub written: Vec<String>,
    pub skipped: Vec<SkippedFile>,
}

/// Run the writes of dream on a brain (v2's `_sync_brain_state`), as one
/// change set, without validating the result.
pub fn sync_brain_state<S: BrainStore>(
    store: &mut S,
    git: &impl Git,
    options: &DreamOptions,
) -> SyncReport {
    let mut skipped = BTreeMap::new();
    let sync = sync(store, git, options, &mut skipped);
    let orphans_after = linker::orphans(&*store, &mut skipped).len();
    SyncReport {
        fixes: sync.fixes(orphans_after),
        maps_regenerated: sync.order,
        written: sync.written,
        skipped: skipped_files(skipped),
    }
}

/// Steps 1-6, applied.
struct Sync {
    dates_updated: usize,
    parents_inferred: usize,
    reverse_synapses_added: usize,
    orphans_before: usize,
    order: Vec<String>,
    written: Vec<String>,
}

impl Sync {
    fn fixes(&self, orphans_after: usize) -> Fixes {
        let orphan_references_added = self.orphans_before.saturating_sub(orphans_after);
        Fixes {
            dates_updated: self.dates_updated,
            parents_inferred: self.parents_inferred,
            reverse_synapses_added: self.reverse_synapses_added,
            orphan_references_added,
            total: self.dates_updated
                + self.parents_inferred
                + self.reverse_synapses_added
                + orphan_references_added,
        }
    }
}

fn skipped_files(skipped: BTreeMap<String, String>) -> Vec<SkippedFile> {
    skipped
        .into_iter()
        .map(|(rel, reason)| SkippedFile { rel, reason })
        .collect()
}

fn sync<S: BrainStore>(
    store: &mut S,
    git: &impl Git,
    options: &DreamOptions,
    skipped: &mut BTreeMap<String, String>,
) -> Sync {
    let dates = git
        .repo(store.root())
        .map(|repo| git.file_dates(&repo))
        .unwrap_or_default();
    let neurons = store.neurons().to_vec();
    // v2 counts these after steps 1-3, which change neither the maps nor the
    // set of neurons, so the brain as read gives the same count.
    let orphans_before = linker::orphans(&*store, skipped).len();

    let order = maps::map_order(store.dirs());
    let mut ws = workspace::Workspace::new(&*store);
    let dates_updated = sync_dates::sync_dates(&mut ws, &neurons, &dates);
    let parents_inferred = linker::fix_missing_parents(&mut ws, &neurons);
    let reverse_synapses_added = linker::fix_reverse_links(&mut ws, &neurons);
    maps::generate_maps(&mut ws, &order, options.today);
    maps::generate_brain_md(&mut ws, options.name, options.description, options.today);
    let (changes, ws_skipped) = ws.into_change_set();
    for (rel, reason) in ws_skipped {
        skipped.entry(rel).or_insert(reason);
    }
    let (applied, held) = apply_unless_skipped(store, changes, skipped);
    // A fix counts only when its file was written (v2's date pass counts
    // after the write).
    let failed: HashSet<&str> = applied
        .failed
        .iter()
        .map(|f| f.rel.as_str())
        .chain(held.iter().map(String::as_str))
        .collect();
    let landed = |rels: &[String]| {
        rels.iter()
            .filter(|rel| {
                !store
                    .real_rel(rel)
                    .is_some_and(|real| failed.contains(real.as_str()))
            })
            .count()
    };
    let dates_updated = landed(&dates_updated);
    let parents_inferred = landed(&parents_inferred);
    let reverse_synapses_added = landed(&reverse_synapses_added);
    for failed in &applied.failed {
        skipped
            .entry(failed.rel.clone())
            .or_insert(failed.error.clone());
    }
    Sync {
        dates_updated,
        parents_inferred,
        reverse_synapses_added,
        orphans_before,
        order,
        written: applied.written,
    }
}

/// Apply `changes` without any write to a path in `skipped`: a file reported
/// as skipped is never written, whichever step skipped it and by whichever
/// path (a symlink and its target are one file). Returns what was applied
/// and the paths held back.
fn apply_unless_skipped<S: BrainStore>(
    store: &mut S,
    mut changes: ChangeSet,
    skipped: &BTreeMap<String, String>,
) -> (Applied, Vec<String>) {
    let blocked: HashSet<String> = skipped
        .keys()
        .flat_map(|rel| [Some(rel.clone()), store.real_rel(rel)])
        .flatten()
        .collect();
    let mut held = Vec::new();
    changes.writes.retain(|write| {
        let keep = !blocked.contains(&write.rel)
            && !store
                .real_rel(&write.rel)
                .is_some_and(|real| blocked.contains(&real));
        if !keep {
            held.push(write.rel.clone());
        }
        keep
    });
    (store.apply(&changes), held)
}

/// v2's `_run_dream_on_brain`, what `create` runs on a new brain: every
/// directory's `map.md`, twice, then `brain.md` (steps 5 and 6 alone, no
/// dates or link fixes), as one change set. Every structure file there is
/// the scaffold's own, so one that does not parse is regenerated as in v2. Returns the files it could not
/// read or write.
pub fn regenerate_maps<S: BrainStore>(store: &mut S, options: &DreamOptions) -> Vec<SkippedFile> {
    let order = maps::map_order(store.dirs());
    let mut ws = workspace::Workspace::for_new_brain(&*store);
    maps::generate_maps(&mut ws, &order, options.today);
    maps::generate_brain_md(&mut ws, options.name, options.description, options.today);
    let (changes, mut skipped) = ws.into_change_set();
    let (applied, _) = apply_unless_skipped(store, changes, &skipped);
    for failed in applied.failed {
        skipped.entry(failed.rel).or_insert(failed.error);
    }
    skipped_files(skipped)
}

/// Run dream on a brain: compute every write, apply them as one change set,
/// and validate the result.
pub fn dream<S: BrainStore>(store: &mut S, git: &impl Git, options: &DreamOptions) -> DreamReport {
    let mut skipped = BTreeMap::new();
    let sync = sync(store, git, options, &mut skipped);

    let validation = linker::validate(&*store, &mut skipped);
    let deprecation = detect_deprecation_issues(&*store);
    for neuron in deprecation.skipped {
        // The reason alone, with no absolute path.
        let reason = match neuron.error {
            ReadError::File(e) => e.problem.to_string(),
            ReadError::Sandbox(e) => e.to_string(),
        };
        skipped.entry(neuron.rel).or_insert(reason);
    }
    DreamReport {
        fixes: sync.fixes(validation.orphans.len()),
        maps_regenerated: sync.order,
        lobes: store.lobe_names(),
        broken: validation.broken,
        one_way: validation.one_way,
        orphans: validation.orphans,
        frontmatter_issues: validation.frontmatter,
        deprecation: deprecation.issues,
        written: sync.written,
        skipped: skipped_files(skipped),
    }
}
