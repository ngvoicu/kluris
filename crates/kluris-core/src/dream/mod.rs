//! `kluris dream` for one brain: the state sync then the validators, with
//! every write in one change set.
//!
//! Steps, in order, each reading what the earlier ones wrote:
//! 1. `updated`/`created` from git ([`sync_dates`]);
//! 2. `parent: ./map.md` where it is missing ([`linker::fix_missing_parents`]);
//! 3. reverse `related` links ([`linker::fix_reverse_links`]);
//! 4. the orphan count before the maps;
//! 5. every directory's `map.md`, twice, deepest first ([`maps`]);
//! 6. `brain.md`.
//!
//! The writes are applied together; the validators (broken and one-way
//! synapses, orphans, frontmatter issues, deprecation) then run on the brain
//! as written. The decided rules: dates are written in the quoted form,
//! YAML anchors as plain copies, each file keeps its line endings and new
//! files are LF, pruned directories get no `map.md` and are not listed,
//! report paths are `/`-joined, and files are taken in path order. A file
//! that cannot be read or written is skipped, reported and makes the brain
//! unhealthy; odd `related` values never crash; link checking skips code,
//! other schemes and titles and reads URL-encoded paths.

mod linker;
mod links;
mod maps;
mod sync_dates;
mod workspace;

use std::collections::{BTreeMap, HashSet};

use crate::brain::change::{Applied, ChangeSet};
use crate::brain::deprecation::{DeprecationIssue, detect_deprecation_issues};
use crate::brain::store::{BrainStore, ReadError};
use crate::git::{FileDates, Git};
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

/// The automatic fixes, counted by kind.
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
    /// Present with the wrong type (`kind: "type"`).
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
    /// Every directory a map was generated for, in the order the maps are
    /// written (deepest first, then by path), `/`-joined.
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
    /// Temp files an interrupted atomic write left, removed before the run
    /// (brain-relative, `/`-joined, sorted).
    pub removed_temp_files: Vec<String>,
}

impl DreamReport {
    /// No broken or one-way synapses, orphans, frontmatter issues or skipped
    /// files; deprecation issues are warnings only.
    pub fn healthy(&self) -> bool {
        self.skipped.is_empty()
            && self.broken.is_empty()
            && self.one_way.is_empty()
            && self.orphans.is_empty()
            && self.frontmatter_issues.is_empty()
    }
}

/// Dream's writes alone, the preflight `kluris mri` runs: dream without its
/// validators. `orphan_references_added` compares the orphans before the
/// maps with the orphans after the writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub fixes: Fixes,
    /// Every directory a map was generated for, in the order the maps are
    /// written (deepest first, then by path), `/`-joined.
    pub maps_regenerated: Vec<String>,
    /// The files written, by path.
    pub written: Vec<String>,
    pub skipped: Vec<SkippedFile>,
}

/// Run the writes of dream on a brain, as one
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
    // Counted before steps 1-3, which change neither the maps nor the set of
    // neurons, so the brain as read gives the count after them.
    let orphans_before = linker::orphans(&*store, skipped).len();
    let planned = plan(&*store, Some(&dates), options);
    for (rel, reason) in planned.skipped {
        skipped.entry(rel).or_insert(reason);
    }
    let (applied, held) = apply_unless_skipped(store, planned.changes, skipped);
    // A fix counts only when its file was written.
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
    let dates_updated = landed(&planned.dates_updated);
    let parents_inferred = landed(&planned.parents_inferred);
    let reverse_synapses_added = landed(&planned.reverse_synapses_added);
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
        order: planned.order,
        written: applied.written,
    }
}

/// What [`plan`] computed, not applied.
struct Planned {
    changes: ChangeSet,
    /// The files that could not be read or rewritten.
    skipped: BTreeMap<String, String>,
    /// The maps' directories, in the order the maps are written.
    order: Vec<String>,
    /// The neurons each of steps 1-3 rewrote.
    dates_updated: Vec<String>,
    parents_inferred: Vec<String>,
    reverse_synapses_added: Vec<String>,
}

/// Dream's writes over `store`, computed and not applied: steps 1-3 with
/// git's `dates` (`None`: those steps do not run), then every map and
/// `brain.md`. Each step reads what the earlier ones would write.
fn plan<S: BrainStore>(store: &S, dates: Option<&FileDates>, options: &DreamOptions) -> Planned {
    let order = maps::map_order(store.dirs());
    let mut ws = workspace::Workspace::new(store);
    let (dates_updated, parents_inferred, reverse_synapses_added) = match dates {
        Some(dates) => {
            let neurons = store.neurons().to_vec();
            (
                sync_dates::sync_dates(&mut ws, &neurons, dates),
                linker::fix_missing_parents(&mut ws, &neurons),
                linker::fix_reverse_links(&mut ws, &neurons, |_| true),
            )
        }
        None => Default::default(),
    };
    maps::generate_maps(&mut ws, &order, options.today);
    maps::generate_brain_md(&mut ws, options.name, options.description, options.today);
    let (changes, skipped) = ws.into_change_set();
    Planned {
        changes,
        skipped,
        order,
        dates_updated,
        parents_inferred,
        reverse_synapses_added,
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

/// What `create` runs on a new brain: every
/// directory's `map.md`, twice, then `brain.md` (steps 5 and 6 alone, no
/// dates or link fixes), as one change set. Every structure file there is
/// the scaffold's own, so one that does not parse is regenerated. Returns
/// the files it could not read or write.
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

/// Steps 5 and 6 alone (every directory's `map.md`, twice, then
/// `brain.md`), computed over the brain as it is and not applied: the change
/// set, and the files that could not be read or rewritten. `kluris commit`
/// applies the part of it its paths call for.
pub(crate) fn map_changes<S: BrainStore>(
    store: &S,
    options: &DreamOptions,
) -> (ChangeSet, BTreeMap<String, String>) {
    let planned = plan(store, None, options);
    (planned.changes, planned.skipped)
}

/// Every step of dream's writes (1-3 with git's `dates`, then the maps and
/// `brain.md`), computed over the brain as it is and not applied: the
/// change set, and the files that could not be read or rewritten. The
/// housekeeping commit of `dream` and `mri` is this over HEAD's files.
pub(crate) fn housekeeping_changes<S: BrainStore>(
    store: &S,
    dates: &FileDates,
    options: &DreamOptions,
) -> (ChangeSet, BTreeMap<String, String>) {
    let planned = plan(store, Some(dates), options);
    (planned.changes, planned.skipped)
}

/// Step 3 alone for the links of the neurons at `sources`: the reverse
/// `related` links they call for in other neurons, computed over the brain
/// as it is and not applied: the change set, and the files that could not
/// be read or rewritten. `kluris commit` takes the ones its paths call for.
pub(crate) fn link_changes<S: BrainStore>(
    store: &S,
    sources: &[String],
) -> (ChangeSet, BTreeMap<String, String>) {
    let neurons = store.neurons().to_vec();
    let mut ws = workspace::Workspace::new(store);
    linker::fix_reverse_links(&mut ws, &neurons, |rel| sources.iter().any(|s| s == rel));
    ws.into_change_set()
}

/// Run dream on a brain: remove the temp files an interrupted write left,
/// compute every write, apply them as one change set, and
/// validate the result.
pub fn dream<S: BrainStore>(store: &mut S, git: &impl Git, options: &DreamOptions) -> DreamReport {
    // First, so `git add -A` after a dream never commits one.
    let removed_temp_files = store.remove_stale_temp_files(crate::atomic::STALE_TEMP_AGE);
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
        removed_temp_files,
    }
}
