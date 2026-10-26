//! `kluris dream [--brain NAME|all] [--json] [--broken-only]` (v2
//! `cli.py:1413-1522`).

use std::path::Path;

use kluris_core::brain::store::LocalGitStore;
use kluris_core::config::{ConfigErrorKind, read_brain_config};
use kluris_core::dream::{DreamOptions, DreamReport, Fixes, dream};
use kluris_core::git::SystemGit;
use kluris_core::pyrepr::py_path_str;
use kluris_core::yaml::Date;

use crate::commands::wake_up::issue;
use crate::error::CliError;
use crate::output::human::{self, Painter, Style};
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn str(text: &str) -> Json {
    Json::Str(text.to_string())
}

/// The local date (Python's `date.today()`, which follows `TZ`).
pub(crate) fn today() -> Result<Date, CliError> {
    let date = jiff::Zoned::now().date();
    let (Ok(year), Ok(month), Ok(day)) = (
        u32::try_from(date.year()),
        u32::try_from(date.month()),
        u32::try_from(date.day()),
    ) else {
        return Err(CliError::new(format!("Invalid local date {date}")));
    };
    Date::new(year, month, day).ok_or_else(|| CliError::new(format!("Invalid local date {date}")))
}

fn add(sum: &mut Fixes, f: &Fixes) {
    sum.dates_updated += f.dates_updated;
    sum.parents_inferred += f.parents_inferred;
    sum.reverse_synapses_added += f.reverse_synapses_added;
    sum.orphan_references_added += f.orphan_references_added;
    sum.total += f.total;
}

pub(crate) fn fixes_json(f: &Fixes) -> Json {
    Json::Object(vec![
        ("dates_updated".into(), count(f.dates_updated)),
        ("parents_inferred".into(), count(f.parents_inferred)),
        (
            "reverse_synapses_added".into(),
            count(f.reverse_synapses_added),
        ),
        (
            "orphan_references_added".into(),
            count(f.orphan_references_added),
        ),
        ("total".into(), count(f.total)),
    ])
}

/// `{"brain": name, **entry}`.
fn tagged(brain: &str, entry: Json) -> Json {
    let mut fields = vec![("brain".to_string(), str(brain))];
    if let Json::Object(rest) = entry {
        fields.extend(rest);
    }
    Json::Object(fields)
}

pub fn run(brain: Option<&str>, as_json: bool, broken_only: bool) -> Result<(), CliError> {
    if broken_only && !as_json {
        return Err(CliError::new("--broken-only requires --json"));
    }
    let brains = resolve_brains(brain, true, as_json)?;
    let git = SystemGit::new();
    let painter = Painter::stdout();
    let today = today()?;

    // Every brain's kluris.yml first, so a missing one stops the run before
    // any brain is written (v2 raises midway, after earlier brains, PAR-048).
    let mut configs = Vec::new();
    for brain in &brains {
        let root = py_path_str(&brain.entry.path);
        let config = read_brain_config(Path::new(&root)).map_err(|e| match e.kind {
            ConfigErrorKind::NotFound => CliError::new(format!(
                "Brain {} has no kluris.yml: {}",
                brain.name,
                e.path.display()
            )),
            _ => CliError::new(e.to_string()),
        })?;
        configs.push((root, config));
    }
    // Under `--brain all` a skipped line names its brain.
    let name_brains = brains.len() > 1 || brain == Some("all");

    let mut healthy = true;
    let mut fixes = Fixes::default();
    let (mut broken, mut one_way, mut orphans, mut fm_issues) = (0, 0, 0, 0);
    let mut deprecation = Vec::new();
    let mut broken_detail = Vec::new();
    let mut skipped = Vec::new();
    for (brain, (root, config)) in brains.iter().zip(&configs) {
        let mut store = LocalGitStore::open(Path::new(root));
        let report = dream(
            &mut store,
            &git,
            &DreamOptions {
                name: &config.name,
                description: &config.description,
                today,
            },
        );
        add(&mut fixes, &report.fixes);
        broken += report.broken.len();
        one_way += report.one_way.len();
        orphans += report.orphans.len();
        fm_issues += report.frontmatter_issues.len();
        deprecation.extend(
            report
                .deprecation
                .iter()
                .map(|i| tagged(&brain.name, issue(i))),
        );
        broken_detail.extend(report.broken.iter().map(|b| {
            Json::Object(vec![
                ("brain".into(), str(&brain.name)),
                ("file".into(), str(&b.file)),
                ("target".into(), str(&b.target)),
            ])
        }));
        skipped.extend(report.skipped.iter().map(|s| {
            Json::Object(vec![
                ("brain".into(), str(&brain.name)),
                ("file".into(), str(&s.rel)),
                ("error".into(), str(&s.reason)),
            ])
        }));
        healthy &= report.healthy();
        if !as_json {
            human::print(&text(painter, &brain.name, &report));
            let mut lines = String::new();
            for s in &report.skipped {
                let who = if name_brains {
                    format!("{}: ", brain.name)
                } else {
                    String::new()
                };
                lines.push_str(&format!("Skipped {who}{}: {}\n", s.rel, s.reason));
            }
            let _ = std::io::Write::write_all(&mut std::io::stderr(), lines.as_bytes());
        }
    }

    if as_json {
        let mut doc = if broken_only {
            vec![
                ("ok".into(), Json::Bool(true)),
                ("broken_synapses_count".into(), count(broken_detail.len())),
                ("broken_synapses".into(), Json::Array(broken_detail)),
            ]
        } else {
            vec![
                ("ok".into(), Json::Bool(true)),
                ("healthy".into(), Json::Bool(healthy)),
                ("broken_synapses".into(), count(broken)),
                ("one_way_synapses".into(), count(one_way)),
                ("orphans".into(), count(orphans)),
                ("frontmatter_issues".into(), count(fm_issues)),
                ("dates_updated".into(), count(fixes.dates_updated)),
                ("deprecation_issues".into(), count(deprecation.len())),
                ("deprecation".into(), Json::Array(deprecation)),
                ("broken_synapses_detail".into(), Json::Array(broken_detail)),
                ("fixes".into(), fixes_json(&fixes)),
            ]
        };
        // After v2's keys, only when a file was skipped (PAR-038).
        if !skipped.is_empty() {
            doc.push(("skipped".into(), Json::Array(skipped)));
        }
        human::print(&json::to_line(&Json::Object(doc)));
    } else {
        let mut out = String::new();
        if fixes.total > 0 {
            let line = format!("{} automatic fixes applied across all brains.", fixes.total);
            out.push_str(&format!(
                "\n{}\n",
                painter.paint(Style::Green, &painter.bold(&line))
            ));
        }
        let footer = if healthy {
            painter.paint(Style::Green, &painter.bold("Brain is healthy."))
        } else {
            painter.paint(
                Style::Yellow,
                &painter.bold("Remaining issues need manual attention."),
            )
        };
        out.push_str(&format!("\n{footer}\n"));
        human::print(&out);
    }
    if healthy {
        Ok(())
    } else {
        Err(CliError::exit_only())
    }
}

/// A `/`-joined path in native form: the JSON paths are `/`-joined
/// (PAR-037), the text keeps v2's native ones.
fn native(rel: &str) -> String {
    rel.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// v2's per-brain health report (`cli.py:1470-1495`).
fn text(p: Painter, name: &str, r: &DreamReport) -> String {
    let check = |n: usize, style: Style, what: &str| {
        if n == 0 {
            p.paint(Style::Green, "OK")
        } else {
            p.paint(style, &format!("{n} {what}"))
        }
    };
    let maps: Vec<String> = r.maps_regenerated.iter().map(|rel| native(rel)).collect();
    let mut out = format!("\n{} health report:\n", p.bold(name));
    out.push_str(&format!("  Lobes: {}\n", r.lobes.join(", ")));
    out.push_str(&format!(
        "  Maps regenerated: {} ({})\n",
        maps.len(),
        maps.join(", ")
    ));
    out.push_str(&format!(
        "  {} synapses\n",
        check(r.broken.len(), Style::Red, "broken")
    ));
    out.push_str(&format!(
        "  {} bidirectional\n",
        check(r.one_way.len(), Style::Yellow, "one-way")
    ));
    out.push_str(&format!(
        "  {}\n",
        check(r.orphans.len(), Style::Yellow, "orphans")
    ));
    out.push_str(&format!(
        "  {} frontmatter\n",
        check(r.frontmatter_issues.len(), Style::Yellow, "issues")
    ));
    out.push_str(&format!(
        "  {}\n",
        check(r.deprecation.len(), Style::Yellow, "deprecation warnings")
    ));
    use kluris_core::brain::deprecation::DeprecationIssue as D;
    for item in &r.deprecation {
        // v2 never prints `replaced_by_not_active`.
        let line = match item {
            D::ActiveLinksToDeprecated { source, target } => {
                format!("{} links to deprecated {}", native(source), native(target))
            }
            D::DeprecatedWithoutReplacement { file } => {
                format!("{} is deprecated but has no replaced_by", native(file))
            }
            D::ReplacedByMissing { file, target } => {
                format!("{} replaced_by points to missing {target}", native(file))
            }
            D::ReplacedByNotActive { .. } => continue,
        };
        out.push_str(&format!("    - {line}\n"));
    }
    let f = &r.fixes;
    out.push_str(&format!("  {} automatic fixes applied\n", f.total));
    out.push_str(&format!(
        "  {} neuron dates refreshed from git\n",
        f.dates_updated
    ));
    out.push_str(&format!(
        "  {} missing parent frontmatter values inferred\n",
        f.parents_inferred
    ));
    out.push_str(&format!(
        "  {} missing reverse related links added\n",
        f.reverse_synapses_added
    ));
    out.push_str(&format!(
        "  {} missing neuron references added to parent map.md files\n",
        f.orphan_references_added
    ));
    out
}
