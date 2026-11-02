//! `kluris resolve [--brain NAME] PATH --ours|--theirs|--file FILE --merge ID
//! [--no-verify] [--json]`: settle a waiting merge path by path, through an
//! explicit session (`kluris_core::sync::resolve`). Its open pulls first;
//! with the last choice the merge is finished, and its close pulls anything
//! newer and pushes. `--merge` names the waiting merge the choice answers;
//! an id that is not the open conflict's is refused, with the current
//! conflict's id, sides and the command to run.
//!
//! The JSON is the call's own keys, then the `sync` object. Every failure
//! after the session opened closes it as usual and carries `sync` too.
//! Exit 1 on a refusal, and, as `kluris sync`, when the sync was refused,
//! blocked or met a new conflict once the merge landed; 0 offline.

use std::path::Path;

use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::pyrepr::py_path_str;
use kluris_core::sync::resolve::{Choice, ResolveError, ResolveReport, ResolveRequest, resolve};
use kluris_core::sync::session::Intent;
use kluris_core::sync::{ConflictText, LeftReason, SyncReport};

use crate::brains::{self, Target};
use crate::commands::dream::today;
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

pub struct Options<'a> {
    pub brain: Option<&'a str>,
    pub path: Option<&'a str>,
    pub ours: bool,
    pub theirs: bool,
    pub file: Option<&'a str>,
    pub merge: Option<&'a str>,
    pub no_verify: bool,
    pub json: bool,
}

fn conflicts(n: usize) -> &'static str {
    if n == 1 { "conflict" } else { "conflicts" }
}

/// `path` as a shell word: quoted when it holds anything but letters,
/// digits and `._/-`.
pub fn shell_word(path: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "._/-".contains(c);
    if !path.is_empty() && path.chars().all(plain) {
        path.to_string()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

/// The command that settles the conflict at `path` of waiting merge
/// `merge` in `brain`.
pub fn command(brain: &str, path: &str, merge: &str) -> String {
    format!(
        "kluris resolve --brain {brain} {} --ours|--theirs|--file FILE --merge {merge}",
        shell_word(path)
    )
}

/// The error for an id that is not the open conflict's: the reason,
/// the current conflict's sides and the command; in JSON, the current id
/// and conflict entry.
fn wrong_merge(
    brain: &str,
    error: &ResolveError,
    current: &str,
    conflict: Option<&ConflictText>,
) -> (String, Vec<(String, Json)>) {
    let mut message = format!("Brain {brain}: {error}");
    if let Some(c) = conflict {
        let side = |text: &Option<String>| match text {
            Some(text) => text.clone(),
            None if c.binary => "(binary)\n".into(),
            None => "(no file)\n".into(),
        };
        message.push_str(&format!(
            "\nIts sides now ({}):\n--- ours\n{}--- theirs\n{}--- base\n{}Then run: {}",
            c.kind.as_str(),
            side(&c.ours),
            side(&c.theirs),
            side(&c.base),
            command(brain, &c.path, current)
        ));
    }
    let fields = vec![
        ("merge".into(), Json::Str(current.into())),
        (
            "conflict".into(),
            conflict.map_or(Json::Null, ConflictText::to_json),
        ),
    ];
    (message, fields)
}

pub fn run(options: &Options) -> Result<(), CliError> {
    let picked = [options.ours, options.theirs, options.file.is_some()]
        .iter()
        .filter(|on| **on)
        .count();
    let brains = resolve_brains(options.brain, false, options.json)?;
    let Some(brain) = brains.first() else {
        return Err(CliError::new("No brain to resolve in"));
    };
    let root = py_path_str(&brain.entry.path);
    match picked {
        0 => return Err(CliError::new("Give one of --ours, --theirs or --file")),
        1 => {}
        _ => return Err(CliError::new("Give only one of --ours, --theirs or --file")),
    }
    let Some(path) = options.path else {
        return Err(CliError::new("Give the conflicted PATH to resolve"));
    };
    let Some(merge) = options.merge else {
        return Err(CliError::new(
            "Give --merge, the id of the waiting merge the choice answers",
        ));
    };
    let choice = match options.file {
        Some(file) => Choice::File(
            std::fs::read(file).map_err(|e| CliError::new(format!("Cannot read {file}: {e}")))?,
        ),
        None if options.ours => Choice::Ours,
        None => Choice::Theirs,
    };
    let config = read_brain_config(Path::new(&root)).map_err(|e| match e.kind {
        ConfigErrorKind::NotFound => CliError::new(format!(
            "Brain {} has no kluris.yml: {}",
            brain.name,
            e.path.display()
        )),
        _ => CliError::new(e.to_string()),
    })?;
    let request = ResolveRequest {
        path,
        merge,
        choice,
    };
    let targets = [Target::new(
        &brain.entry.path,
        &config.name,
        &config.description,
    )];
    let mut opened = brains::open(&targets, Intent::Explicit, today()?, options.no_verify);
    let resolved = match opened.first() {
        Some(session) => resolve(session, &request),
        None => Err(ResolveError::NoOpenConflict),
    };
    let closed = brains::close(opened);
    let sync = closed.into_iter().next().and_then(|c| c.report);
    let name = brain.name.as_str();
    if !options.json {
        brains::print_lines([(name, sync.as_ref())]);
    }
    let report = match resolved {
        Ok(report) => report,
        Err(e) => {
            let (message, mut fields) = match &e {
                ResolveError::WrongMerge {
                    wrong, conflict, ..
                } => wrong_merge(name, &e, &wrong.current, conflict.as_deref()),
                _ => (format!("Brain {name}: {e}"), Vec::new()),
            };
            fields.extend(brains::sync_field(sync.as_ref()));
            return Err(CliError::new(message).with_fields(fields));
        }
    };
    let done = Done::of(&report, sync.as_ref());
    if options.json {
        let doc = Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brain".into(), Json::Str(name.to_string())),
            ("path".into(), Json::Str(report.path.clone())),
            ("choice".into(), Json::Str(report.choice.into())),
            (
                "remaining".into(),
                Json::Array(done.remaining.iter().cloned().map(Json::Str).collect()),
            ),
            ("committed".into(), Json::Bool(report.committed)),
            (
                "commit".into(),
                report.commit.clone().map_or(Json::Null, Json::Str),
            ),
            ("pushed".into(), Json::Bool(done.pushed)),
        ]);
        let doc = match (doc, &report.warning) {
            (Json::Object(mut fields), Some(warning)) => {
                fields.push(("warning".into(), Json::Str(warning.clone())));
                Json::Object(fields)
            }
            (doc, _) => doc,
        };
        human::print(&json::to_line(&brains::with_sync(doc, sync.as_ref())));
    } else {
        human::print(&text(&report, &done));
        if let Some(warning) = &report.warning {
            eprintln!("Warning: {warning}");
        }
    }
    if failed(&report, sync.as_ref()) {
        Err(CliError::exit_only())
    } else {
        Ok(())
    }
}

/// What the call left once the session closed.
struct Done {
    /// The paths still without a choice; once the merge landed, those of a
    /// new conflict the close's pull met.
    remaining: Vec<String>,
    pushed: bool,
    /// Why the close did not push, unless the network was missing.
    not_pushed: Option<String>,
}

impl Done {
    fn of(report: &ResolveReport, sync: Option<&SyncReport>) -> Done {
        if !report.committed {
            return Done {
                remaining: report.remaining.clone(),
                pushed: false,
                not_pushed: None,
            };
        }
        let sync = sync.cloned().unwrap_or_default();
        Done {
            remaining: sync.conflicts.into_iter().map(|c| c.path).collect(),
            pushed: sync.pushed,
            not_pushed: sync
                .left_as_is
                .filter(|left| left.reason != LeftReason::Offline)
                .map(|left| left.to_string()),
        }
    }
}

/// Whether the call exits 1 though its choice was recorded: the sync was
/// refused or blocked, or, once the merge landed, met a new conflict (the
/// conflicts of the merge still being settled are its own).
fn failed(report: &ResolveReport, sync: Option<&SyncReport>) -> bool {
    sync.is_some_and(|sync| {
        sync.left_as_is
            .as_ref()
            .is_some_and(|left| left.reason.fails())
            || (report.committed && !sync.conflicts.is_empty())
    })
}

/// One line per call.
fn text(r: &ResolveReport, done: &Done) -> String {
    let mut line = format!("Resolved {} ({})", r.path, r.choice);
    match &r.commit {
        None => line.push_str(&format!(
            "; {} {} left: {}",
            done.remaining.len(),
            conflicts(done.remaining.len()),
            done.remaining.join(", ")
        )),
        Some(commit) => {
            let short: String = commit.chars().take(7).collect();
            line.push_str(&format!("; committed {short}"));
            if done.pushed {
                line.push_str(" and pushed");
            } else if !done.remaining.is_empty() {
                line.push_str(&format!(
                    ", not pushed: the remote moved and a new {} is open: {}; run kluris resolve again",
                    conflicts(done.remaining.len()),
                    done.remaining.join(", ")
                ));
            } else {
                line.push_str(&format!(
                    ", not pushed: {}",
                    done.not_pushed.as_deref().unwrap_or("offline")
                ));
            }
        }
    }
    line.push('\n');
    line
}
