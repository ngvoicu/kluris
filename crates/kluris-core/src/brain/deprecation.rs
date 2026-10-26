//! Deprecation diagnostics: v2's `deprecation.detect_deprecation_issues`.
//!
//! A neuron's status is v2's `str(meta.get("status", "active")).lower()`: no
//! `status` key is active, a string compares lowercased, and any other value
//! (`~`, `true`, a number, a list) is neither active nor deprecated. Then:
//! 1. for each deprecated neuron, in neuron order, its `replaced_by`:
//!    missing, `~` or `""` gives `deprecated_without_replacement`; a value
//!    that is not a string (after v2 turns a top-level date into its ISO
//!    text) gives nothing; a target that does not exist or resolves outside
//!    the brain gives `replaced_by_missing`; a target that is not an active
//!    neuron gives `replaced_by_not_active`.
//! 2. for each neuron that is not deprecated, each string `related` entry
//!    that resolves to a deprecated neuron gives `active_links_to_deprecated`
//!    (map.md is not a neuron, so its links count for nothing).
//!
//! Statuses are keyed by resolved path, so a symlink and its target are one
//! neuron, while each walked path still reports its own issues. `file` and
//! `source` are the walked relative paths, the `active_links_to_deprecated`
//! target is the resolved target's relative path, all in native form (v2's
//! `str(Path)`); `replaced_by` targets are kept as written.
//!
//! A markdown neuron whose frontmatter cannot be read is skipped and reported
//! in [`DeprecationReport::skipped`]; v2 raises instead (PAR-007).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::neuron_index::Neuron;
use super::sandbox;
use super::store::{BrainStore, ReadError};
use crate::yaml::{Map, Value, date_iso, datetime_iso};

/// One deprecation-frontmatter inconsistency, with v2's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeprecationIssue {
    ActiveLinksToDeprecated { source: PathBuf, target: PathBuf },
    DeprecatedWithoutReplacement { file: PathBuf },
    ReplacedByMissing { file: PathBuf, target: String },
    ReplacedByNotActive { file: PathBuf, target: String },
}

impl DeprecationIssue {
    /// v2's `kind` value.
    pub fn kind(&self) -> &'static str {
        match self {
            DeprecationIssue::ActiveLinksToDeprecated { .. } => "active_links_to_deprecated",
            DeprecationIssue::DeprecatedWithoutReplacement { .. } => {
                "deprecated_without_replacement"
            }
            DeprecationIssue::ReplacedByMissing { .. } => "replaced_by_missing",
            DeprecationIssue::ReplacedByNotActive { .. } => "replaced_by_not_active",
        }
    }
}

/// A neuron left out because it cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedNeuron {
    pub rel: String,
    pub error: ReadError,
}

/// The issues, and the neurons the check could not read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeprecationReport {
    pub issues: Vec<DeprecationIssue>,
    pub skipped: Vec<SkippedNeuron>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Active,
    Deprecated,
    Other,
}

fn status(meta: &Map) -> Status {
    match meta.get("status") {
        None => Status::Active,
        Some(Value::Str(text)) => match text.to_lowercase().as_str() {
            "active" => Status::Active,
            "deprecated" => Status::Deprecated,
            _ => Status::Other,
        },
        Some(_) => Status::Other,
    }
}

/// What `replaced_by` asks for.
enum Replacement {
    None,
    Path(String),
    Ignored,
}

/// v2 reads `replaced_by` after `_normalize_metadata`, which turns a
/// top-level date or datetime into its `isoformat()` text.
fn replacement(meta: &Map) -> Replacement {
    match meta.get("replaced_by") {
        None | Some(Value::Null) => Replacement::None,
        Some(Value::Str(text)) if text.is_empty() => Replacement::None,
        Some(Value::Str(text)) => Replacement::Path(text.clone()),
        Some(Value::Date(date)) => Replacement::Path(date_iso(date)),
        Some(Value::DateTime(dt)) => Replacement::Path(datetime_iso(dt, 'T')),
        Some(_) => Replacement::Ignored,
    }
}

/// A brain-relative `/`-separated path in native form.
fn native(rel: &str) -> PathBuf {
    rel.split('/').collect()
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new(""))
}

/// The deprecation issues of the store's neurons, in v2's order over the
/// store's neuron order.
pub fn detect_deprecation_issues(store: &impl BrainStore) -> DeprecationReport {
    let root = store.root();
    let resolved_root = sandbox::resolve(root).ok();

    let mut report = DeprecationReport::default();
    let mut neurons: Vec<(&Neuron, PathBuf)> = Vec::new();
    let mut status_by_path: HashMap<PathBuf, Status> = HashMap::new();
    let mut meta_by_path: HashMap<PathBuf, Map> = HashMap::new();
    for neuron in store.neurons() {
        let meta = match store.read_neuron(neuron) {
            Ok(fm) => fm.meta,
            Err(error) => {
                report.skipped.push(SkippedNeuron {
                    rel: neuron.rel.clone(),
                    error,
                });
                continue;
            }
        };
        // The walker drops links that loop, so a neuron always resolves.
        let resolved = sandbox::resolve(&neuron.path).unwrap_or_else(|_| neuron.path.clone());
        status_by_path.insert(resolved.clone(), status(&meta));
        meta_by_path.insert(resolved.clone(), meta);
        neurons.push((neuron, resolved));
    }
    let within = |path: &Path| {
        resolved_root
            .as_deref()
            .is_some_and(|root| sandbox::is_within(path, root))
    };

    for (neuron, resolved) in &neurons {
        let Some(meta) = meta_by_path.get(resolved) else {
            continue;
        };
        if status(meta) != Status::Deprecated {
            continue;
        }
        let file = native(&neuron.rel);
        let target_text = match replacement(meta) {
            Replacement::None => {
                report
                    .issues
                    .push(DeprecationIssue::DeprecatedWithoutReplacement { file });
                continue;
            }
            Replacement::Ignored => continue,
            Replacement::Path(text) => text,
        };
        let target = sandbox::resolve(&parent(&neuron.path).join(&target_text)).ok();
        let Some(target) = target.filter(|t| t.exists() && within(t)) else {
            report.issues.push(DeprecationIssue::ReplacedByMissing {
                file,
                target: target_text,
            });
            continue;
        };
        if status_by_path.get(&target) != Some(&Status::Active) {
            report.issues.push(DeprecationIssue::ReplacedByNotActive {
                file,
                target: target_text,
            });
        }
    }

    for (neuron, resolved) in &neurons {
        let Some(meta) = meta_by_path.get(resolved) else {
            continue;
        };
        if status(meta) == Status::Deprecated {
            continue;
        }
        let Some(related) = meta.get("related").and_then(Value::as_seq) else {
            continue;
        };
        for link in related.iter().filter_map(Value::as_str) {
            let Ok(target) = sandbox::resolve(&parent(&neuron.path).join(link)) else {
                continue;
            };
            if status_by_path.get(&target) != Some(&Status::Deprecated) {
                continue;
            }
            let rel = resolved_root
                .as_deref()
                .and_then(|root| target.strip_prefix(root).ok())
                .or_else(|| target.strip_prefix(root).ok());
            if let Some(rel) = rel {
                report
                    .issues
                    .push(DeprecationIssue::ActiveLinksToDeprecated {
                        source: native(&neuron.rel),
                        target: rel.to_path_buf(),
                    });
            }
        }
    }
    report
}
