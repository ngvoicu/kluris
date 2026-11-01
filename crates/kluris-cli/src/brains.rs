//! The brains a command opens through the sync session
//! (`kluris_core::sync::session`): [`open`] brings teammates' commits in
//! before the command reads, [`close`] ends every session it opened (one
//! background push at most), and what each sync did is said the one way
//! every command says it: the `sync` object ([`with_sync`]) and one stderr
//! line per brain ([`print_lines`]).
//!
//! A brain registered under two names is opened once, and each name gets
//! its report: a second session of the same brain would wait for the
//! first one's own tree claim.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use kluris_core::config::read_brain_config;
use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::json::Json;
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::session::{self, Alone, Brain, Intent, Session, SessionEnv};
use kluris_core::sync::{LeftReason, SyncReport};
use kluris_core::yaml::Date;

use crate::commands::dream::today;
use crate::error::CliError;

/// A brain a command opens: its root, and the name and description the
/// maps a merge rebuilds carry (its `kluris.yml`'s, else what the caller
/// knows of it).
#[derive(Debug, Clone)]
pub struct Target {
    pub root: PathBuf,
    name: String,
    description: String,
}

impl Target {
    /// The brain at `path` (as the global config writes it), named `name`
    /// and described `description` where its `kluris.yml` does not read.
    pub fn new(path: &str, name: &str, description: &str) -> Target {
        let root = PathBuf::from(py_path_str(path));
        let (name, description) = match read_brain_config(&root) {
            Ok(config) => (config.name, config.description),
            Err(_) => (name.to_string(), description.to_string()),
        };
        Target {
            root,
            name,
            description,
        }
    }
}

/// The sessions of a command's brains, until [`close`].
pub struct Opened<'a> {
    sessions: Vec<Session<'a>>,
    /// Each target's session, by its place in `sessions`.
    of_target: Vec<usize>,
}

/// What [`close`] says of one target.
#[derive(Debug, Clone, Default)]
pub struct Closed {
    /// What its sync did; `None` when there is nothing to say.
    pub report: Option<SyncReport>,
    /// Why it was left alone; `None` for a brain it syncs.
    pub alone: Option<Alone>,
}

/// Open each target for `intent`: one brain alone, several at once
/// (`session::open_all`); a root two targets share once.
pub fn open<'a>(targets: &'a [Target], intent: Intent, today: Date) -> Opened<'a> {
    let mut roots: Vec<(PathBuf, &'a Target)> = Vec::new();
    let mut of_target = Vec::new();
    for target in targets {
        let key = same_brain(&target.root);
        let at = match roots.iter().position(|(root, _)| *root == key) {
            Some(at) => at,
            None => {
                roots.push((key, target));
                roots.len() - 1
            }
        };
        of_target.push(at);
    }
    let brains: Vec<Brain<'a>> = roots
        .iter()
        .map(|(_, target)| Brain {
            root: &target.root,
            maps: DreamOptions {
                name: &target.name,
                description: &target.description,
                today,
            },
        })
        .collect();
    let env = SessionEnv {
        git: SystemGit::new(),
        no_verify: false,
        now: SystemTime::now(),
        program: std::env::current_exe().ok(),
    };
    let sessions = match brains.as_slice() {
        [brain] => vec![session::open(*brain, intent, &env)],
        brains => session::open_all(brains, intent, &env),
    };
    Opened {
        sessions,
        of_target,
    }
}

/// [`open`] with today's date.
pub fn open_today(targets: &[Target], intent: Intent) -> Result<Opened<'_>, CliError> {
    Ok(open(targets, intent, today()?))
}

/// The folder two paths of one brain resolve to.
fn same_brain(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// End every session `opened` holds (`session::close_all`): what each
/// target's sync did, in the targets' order.
pub fn close(opened: Opened) -> Vec<Closed> {
    let Opened {
        sessions,
        of_target,
    } = opened;
    let alone: Vec<Option<Alone>> = sessions.iter().map(|s| s.alone().cloned()).collect();
    let reports = session::close_all(sessions);
    of_target
        .into_iter()
        .map(|at| Closed {
            report: reports.get(at).cloned().flatten(),
            alone: alone.get(at).cloned().flatten(),
        })
        .collect()
}

/// The `sync` key and its object when there is something to say.
pub fn sync_field(report: Option<&SyncReport>) -> Option<(String, Json)> {
    report.map(|report| ("sync".to_string(), report.to_json()))
}

/// `doc` (a JSON object) with the `sync` object after its own keys when
/// there is something to say ([`sync_field`]).
pub fn with_sync(doc: Json, report: Option<&SyncReport>) -> Json {
    match doc {
        Json::Object(mut fields) => {
            fields.extend(sync_field(report));
            Json::Object(fields)
        }
        doc => doc,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What a brain's sync did, in a few words: `pulled 2 commits; 1 conflict
/// (notes/c.md)`; `None` when it has nothing to say.
pub fn summary(report: &SyncReport) -> Option<String> {
    let mut parts = Vec::new();
    if report.pulled > 0 {
        parts.push(format!(
            "pulled {}",
            plural(report.pulled, "commit", "commits")
        ));
    }
    if !report.auto_merged.is_empty() {
        parts.push(format!("auto-merged {}", report.auto_merged.join(", ")));
    }
    if !report.conflicts.is_empty() {
        let paths: Vec<&str> = report.conflicts.iter().map(|c| c.path.as_str()).collect();
        parts.push(format!(
            "{} ({})",
            plural(report.conflicts.len(), "conflict", "conflicts"),
            paths.join(", ")
        ));
    }
    if report.behind_conflict > 0 {
        parts.push(format!(
            "{} behind the conflict",
            plural(report.behind_conflict, "newer commit", "newer commits")
        ));
    }
    if let Some(left) = &report.left_as_is {
        parts.push(match (left.reason, &left.detail) {
            (LeftReason::Uncommitted, _) => {
                format!("left as is (uncommitted): {}", left.paths.join(", "))
            }
            (reason, Some(detail)) => format!("left as is ({}): {detail}", reason.as_str()),
            (reason, None) => format!("left as is ({})", reason.as_str()),
        });
    }
    if report.pushed {
        parts.push("pushed".into());
    }
    if report.to_push > 0 {
        parts.push(format!(
            "{} to push",
            plural(report.to_push, "commit", "commits")
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// One brain's stderr line, `kluris: NAME: ...`; `None` when it has
/// nothing to say.
pub fn line(name: &str, report: &SyncReport) -> Option<String> {
    summary(report).map(|said| format!("kluris: {name}: {said}\n"))
}

/// Print each named brain's line on stderr.
pub fn print_lines<'r>(brains: impl IntoIterator<Item = (&'r str, Option<&'r SyncReport>)>) {
    let lines: String = brains
        .into_iter()
        .filter_map(|(name, report)| line(name, report?))
        .collect();
    eprint!("{lines}");
}
