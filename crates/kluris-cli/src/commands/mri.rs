//! `kluris mri [--brain NAME|all] [--output FILE] [--json]` (v2
//! `cli.py:1556-1604`): each brain's session brings teammates' commits in
//! first, then the dream preflight, the brain's page, and the brain's own
//! housekeeping commit, built from HEAD, as dream makes it. The files the
//! preflight skipped are reported as dream reports them; each entry
//! carries its `committed` files and its `sync`, each only when there is
//! something in it, and text says them on one stdout and one stderr line.

use std::io::{self, Write};
use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::config::read_brain_config;
use kluris_core::dream::{DreamOptions, sync_brain_state};
use kluris_core::git::SystemGit;
use kluris_core::mri::{generate, page_uri};
use kluris_core::pyrepr::py_path_str;
use kluris_core::resolve::ResolvedBrain;
use kluris_core::sync::session::Intent;
use kluris_core::yaml::Date;

use crate::brains::{self, Opened, Target};
use crate::commands::dream::{committed_line, fixes_json, today};
use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

/// v2's `--output` refusal for more than one brain.
const OUTPUT_WITH_FAN_OUT: &str = "--output cannot be combined with multi-brain fan-out (each brain \
would overwrite the others' output). Pass --output without --brain all, or pick brains one at a time.";

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

pub fn run(brain: Option<&str>, output: Option<&str>, as_json: bool) -> Result<(), CliError> {
    // Click passes `--output ""` on as a falsy value: the default page.
    let output = output.filter(|o| !o.is_empty());
    let brains = resolve_brains(brain, true, as_json)?;
    if output.is_some() && brains.len() > 1 {
        return Err(CliError::new(OUTPUT_WITH_FAN_OUT));
    }
    // Under `--brain all` a skipped line names its brain, as in dream.
    let name_brains = brains.len() > 1 || brain == Some("all");
    let today = today()?;
    let targets: Vec<Target> = brains
        .iter()
        .map(|b| Target::new(&b.entry.path, &b.name, &b.entry.description))
        .collect();
    let mut opened = brains::open(&targets, Intent::Housekeeping, today, false);
    // Brain by brain, up to the first that fails; every session closes.
    let mut results = Vec::new();
    let mut failure = None;
    for (at, brain) in brains.iter().enumerate() {
        match scan(&mut opened, at, brain, output, today, as_json, name_brains) {
            Ok(result) => results.push(result),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    let closed = brains::close(opened);
    if let Some(failure) = failure {
        return Err(failure);
    }
    if as_json {
        let entries = results
            .into_iter()
            .zip(&closed)
            .map(|(result, closed)| brains::with_sync(result, closed.report.as_ref()))
            .collect();
        human::print(&json::to_line(&Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("brains".into(), Json::Array(entries)),
        ])));
    } else {
        brains::print_lines(
            brains
                .iter()
                .zip(&closed)
                .map(|(brain, closed)| (brain.name.as_str(), closed.report.as_ref())),
        );
    }
    Ok(())
}

/// One brain's preflight, page and housekeeping commit (the brain at
/// `at` among the targets `opened` holds): its `brains[]` entry, without
/// its `sync` key, after printing its text.
fn scan(
    opened: &mut Opened,
    at: usize,
    brain: &ResolvedBrain,
    output: Option<&str>,
    today: Date,
    as_json: bool,
    name_brains: bool,
) -> Result<Json, CliError> {
    let git = SystemGit::new();
    let root = py_path_str(&brain.entry.path);
    let config = read_brain_config(Path::new(&root)).map_err(|e| CliError::new(e.to_string()))?;
    let mut store = LocalGitStore::open(Path::new(&root));
    let options = DreamOptions {
        name: &config.name,
        description: &config.description,
        today,
    };
    let sync = sync_brain_state(&mut store, &git, &options);
    // `str(Path(--output))`, or `str(brain_path / "brain-mri.html")`.
    let out = match output {
        Some(o) => py_path_str(o),
        None => py_path_str(&format!("{root}/brain-mri.html")),
    };
    let stats = generate(&store, &brain.name, Path::new(&out))
        .map_err(|e| CliError::new(format!("Cannot write {out}: {e}")))?;
    let committed = opened.housekeeping(at, &options);
    if !as_json {
        let uri = page_uri(Path::new(&out));
        let mut text = format!(
            "MRI complete — {out}\n  Open: {uri}\n  {} nodes, {} edges\n",
            stats.nodes, stats.edges
        );
        if sync.fixes.total > 0 {
            text.push_str(&format!(
                "  MRI preflight applied {} automatic fixes\n",
                sync.fixes.total
            ));
        }
        if !committed.is_empty() {
            text.push_str(&committed_line(&brain.name, committed.len()));
        }
        human::print(&text);
        let who = if name_brains {
            format!("{}: ", brain.name)
        } else {
            String::new()
        };
        let lines: String = sync
            .skipped
            .iter()
            .map(|s| format!("Skipped {who}{}: {}\n", s.rel, s.reason))
            .collect();
        let _ = io::stderr().write_all(lines.as_bytes());
    }
    let mut fields = vec![
        ("name".into(), Json::Str(brain.name.clone())),
        ("output_path".into(), Json::Str(out)),
        ("preflight_fixes".into(), fixes_json(&sync.fixes)),
        ("nodes".into(), count(stats.nodes)),
        ("edges".into(), count(stats.edges)),
    ];
    // After the keys above, only when the preflight skipped a file,
    // with dream's `{brain, file, error}` items; then the files the
    // housekeeping commit holds, when it was made.
    if !sync.skipped.is_empty() {
        let skipped = sync
            .skipped
            .iter()
            .map(|s| {
                Json::Object(vec![
                    ("brain".into(), Json::Str(brain.name.clone())),
                    ("file".into(), Json::Str(s.rel.clone())),
                    ("error".into(), Json::Str(s.reason.clone())),
                ])
            })
            .collect();
        fields.push(("skipped".into(), Json::Array(skipped)));
    }
    if !committed.is_empty() {
        let files = committed.into_iter().map(Json::Str).collect();
        fields.push(("committed".into(), Json::Array(files)));
    }
    Ok(Json::Object(fields))
}
