//! `kluris search QUERY [--brain NAME] [--lobe L] [--tag T] [--limit N]
//! [--json]` (research-03 §1.5; the engine is `kluris_core::search`).

use std::path::Path;

use kluris_core::brain::deprecation::SkippedNeuron;
use kluris_core::brain::store::{LocalGitStore, ReadError};
use kluris_core::config::read_brain_config;
use kluris_core::pyrepr::py_path_str;
use kluris_core::search::{Hit, Language, SearchRequest, search};

use crate::error::CliError;
use crate::output::human::{self, Painter, Style};
use crate::output::json::{self, Json};
use crate::resolve::resolve_brains;

/// The parsed options.
pub struct Options<'a> {
    pub query: &'a str,
    pub brain: Option<&'a str>,
    pub lobe: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub limit: i64,
    pub json: bool,
}

/// `--limit` as Click's `int` reads it.
pub fn parse_limit(text: &str) -> Result<i64, String> {
    kluris_core::search::parse_limit(text).ok_or_else(|| "is not a valid integer".to_string())
}

pub fn run(options: &Options) -> Result<(), CliError> {
    if options.query.is_empty() {
        return Err(CliError::new("Query cannot be empty."));
    }
    let brains = resolve_brains(options.brain, false, options.json)?;
    let Some(brain) = brains.first() else {
        return Ok(());
    };
    let root = py_path_str(&brain.entry.path);
    super::fresh_read(&brain.name, Path::new(&root));
    let language = read_brain_config(Path::new(&root))
        .ok()
        .and_then(|cfg| cfg.search.language)
        .and_then(|code| Language::from_code(&code));
    let store = LocalGitStore::open(Path::new(&root));
    let request = SearchRequest {
        query: options.query.to_string(),
        lobe: options.lobe.map(String::from),
        tag: options.tag.map(String::from),
        limit: options.limit,
        language,
    };
    let outcome = search(&store, &request);
    if options.json {
        human::print(&json::to_line(&to_json(
            &brain.name,
            options.query,
            &outcome.hits,
            &outcome.skipped,
        )));
    } else {
        human::print(&text(
            Painter::stdout(),
            &brain.name,
            options.query,
            &outcome.hits,
        ));
        let mut stderr = String::new();
        for s in &outcome.skipped {
            stderr.push_str(&format!("Skipped {}: {}\n", s.rel, reason(&s.error)));
        }
        let _ = std::io::Write::write_all(&mut std::io::stderr(), stderr.as_bytes());
    }
    Ok(())
}

/// Why a file was skipped, without its absolute path.
fn reason(error: &ReadError) -> String {
    match error {
        ReadError::File(e) => e.problem.to_string(),
        ReadError::Sandbox(e) => e.to_string(),
    }
}

fn str(text: &str) -> Json {
    Json::Str(text.to_string())
}

fn opt(text: Option<&str>) -> Json {
    text.map_or(Json::Null, str)
}

fn count(n: usize) -> Json {
    Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

fn strings(items: &[String]) -> Json {
    Json::Array(items.iter().map(|s| str(s)).collect())
}

/// v2's keys (`cli.py:1101-1107`, `rt/search.py:296-304`), then Kluris 3's.
/// `skipped` (PAR-024) is added only when files were left out.
fn to_json(brain: &str, query: &str, hits: &[Hit], skipped: &[SkippedNeuron]) -> Json {
    let results = hits
        .iter()
        .map(|h| {
            let mut fields = vec![
                ("file".to_string(), str(&h.file)),
                ("file_type".into(), str(h.file_type)),
                ("title".into(), str(&h.title)),
                (
                    "matched_fields".into(),
                    Json::Array(h.matched_fields.iter().map(|f| str(f)).collect()),
                ),
                ("snippet".into(), str(&h.snippet)),
                ("score".into(), Json::Int(h.score)),
                ("deprecated".into(), Json::Bool(h.deprecated)),
                ("heading".into(), opt(h.heading.as_deref())),
                ("line".into(), h.line.map_or(Json::Null, count)),
                ("matched_terms".into(), strings(&h.matched_terms)),
                ("coverage".into(), Json::Float(h.coverage)),
                ("related".into(), strings(&h.related)),
                ("updated".into(), opt(h.updated.as_deref())),
            ];
            if h.deprecated {
                fields.push(("replaced_by".into(), opt(h.replaced_by.as_deref())));
            }
            Json::Object(fields)
        })
        .collect();
    let mut doc = vec![
        ("ok".into(), Json::Bool(true)),
        ("brain".into(), str(brain)),
        ("query".into(), str(query)),
        ("total".into(), count(hits.len())),
        ("results".into(), Json::Array(results)),
        ("engine".into(), str("bm25")),
    ];
    if !skipped.is_empty() {
        let files = skipped
            .iter()
            .map(|s| {
                Json::Object(vec![
                    ("file".into(), str(&s.rel)),
                    ("error".into(), str(&reason(&s.error))),
                ])
            })
            .collect();
        doc.push(("skipped".into(), Json::Array(files)));
    }
    Json::Object(doc)
}

/// v2's wording (`cli.py:1112-1127`).
fn text(painter: Painter, brain: &str, query: &str, hits: &[Hit]) -> String {
    if hits.is_empty() {
        return format!(
            "{}\n",
            painter.paint(Style::Dim, &format!("No results for '{query}' in {brain}."))
        );
    }
    let mut out = format!(
        "\n{}\n\n",
        painter.bold(&format!(
            "{} result(s) for '{query}' in {brain}",
            hits.len()
        ))
    );
    for hit in hits {
        let marker = if hit.deprecated {
            format!(" {}", painter.paint(Style::Yellow, "(deprecated)"))
        } else {
            String::new()
        };
        out.push_str(&format!(
            "  {}  {}{marker}\n",
            painter.bold(&format!("{:>3}", hit.score)),
            hit.title
        ));
        out.push_str(&format!(
            "        {}\n",
            painter.paint(Style::Dim, &hit.file)
        ));
        if !hit.snippet.is_empty() {
            for line in hit.snippet.lines() {
                out.push_str(&format!("        {line}\n"));
            }
        }
        out.push('\n');
    }
    out
}
