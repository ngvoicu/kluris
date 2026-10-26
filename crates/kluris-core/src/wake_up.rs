//! `kluris wake-up`: v2's `kluris_runtime.wake_up.build_payload` without a
//! snapshot (research-03 §1.6, §2.7).
//!
//! - `lobes`: [`BrainStore::lobe_names`], each with its [`lobe_description`]
//!   and its neuron counts walked from the lobe; `total_neurons` and
//!   `total_yaml_neurons` sum them (root-level neurons are in no lobe).
//! - `recent`: every neuron whose `updated` is set, as `str(updated)`,
//!   newest first by `(is ISO date, updated, path)`, the first
//!   [`RECENT_LIMIT`].
//! - `brain_md`: brain.md's body, stripped, cut by [`truncate_brain_md`].
//! - `glossary`: glossary.md's entries; none when it cannot be read.
//! - `deprecation`: the store's deprecation issues, over the neurons that
//!   read (PAR-007), in store order (PAR-008); `parse_errors` counts the
//!   markdown neurons that do not read (PAR-009; v2's CLI always says 0).

use std::path::Path;

use crate::brain::deprecation::{DeprecationIssue, SkippedNeuron, detect_deprecation_issues};
use crate::brain::glossary::GlossaryEntry;
use crate::brain::neuron_index::NeuronKind;
use crate::brain::store::BrainStore;
use crate::frontmatter::py_splitlines;
use crate::pyrepr::{py_path_str, py_str};
use crate::yaml::{Value, date_iso, datetime_iso, is_py_space};

/// brain.md bodies longer than this many UTF-8 bytes are cut.
pub const BRAIN_MD_MAX_BYTES: usize = 4000;

/// How many neurons `recent` lists.
pub const RECENT_LIMIT: usize = 5;

/// One lobe of the payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobeSummary {
    pub name: String,
    pub description: String,
    pub neurons: usize,
    pub yaml_count: usize,
}

/// One entry of `recent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentNeuron {
    /// The `/`-separated path relative to the brain.
    pub path: String,
    /// Python's `str()` of the `updated` value.
    pub updated: String,
    pub kind: NeuronKind,
}

/// The wake-up payload, in v2's field order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeUp {
    pub name: String,
    pub path: String,
    pub description: String,
    pub brain_md: String,
    pub lobes: Vec<LobeSummary>,
    pub total_neurons: usize,
    pub total_yaml_neurons: usize,
    pub recent: Vec<RecentNeuron>,
    pub glossary: Vec<GlossaryEntry>,
    pub deprecation: Vec<DeprecationIssue>,
    pub parse_errors: usize,
    /// The neurons left out of `recent` and `deprecation` because they do
    /// not read (`parse_errors` of them).
    pub skipped: Vec<SkippedNeuron>,
}

fn py_strip(text: &str) -> &str {
    text.trim_matches(is_py_space)
}

/// What Python's text-mode read hands over: `\r\n` and `\r` become `\n`.
fn universal_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Build the payload for the brain in `store`. `name` is the registered
/// name (the path's last part when empty), `path` the stored path (shown as
/// Python's `str(Path(path))`), `description` the config entry's.
pub fn wake_up(store: &impl BrainStore, name: &str, path: &str, description: &str) -> WakeUp {
    let path = py_path_str(path);
    let name = if name.is_empty() {
        Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        name.to_string()
    };

    let lobes: Vec<LobeSummary> = store
        .lobe_names()
        .into_iter()
        .map(|lobe| {
            let neurons = store.lobe_neurons(&lobe);
            LobeSummary {
                description: lobe_description(store, &lobe),
                neurons: neurons.len(),
                yaml_count: neurons
                    .iter()
                    .filter(|n| n.kind == NeuronKind::Yaml)
                    .count(),
                name: lobe,
            }
        })
        .collect();
    let report = detect_deprecation_issues(store);

    WakeUp {
        name,
        path,
        description: description.to_string(),
        brain_md: brain_md(store),
        total_neurons: lobes.iter().map(|l| l.neurons).sum(),
        total_yaml_neurons: lobes.iter().map(|l| l.yaml_count).sum(),
        lobes,
        recent: recent(store),
        glossary: store.glossary().unwrap_or_default(),
        parse_errors: report.skipped.len(),
        deprecation: report.issues,
        skipped: report.skipped,
    }
}

fn recent(store: &impl BrainStore) -> Vec<RecentNeuron> {
    let mut candidates: Vec<(bool, RecentNeuron)> = store
        .neurons()
        .iter()
        .filter_map(|neuron| {
            let fm = store.read_neuron(neuron).ok()?;
            let updated = match fm.meta.get("updated") {
                None | Some(Value::Null) => return None,
                Some(value) => py_str(value),
            };
            Some((
                is_iso_date_prefix(&updated),
                RecentNeuron {
                    path: neuron.rel.clone(),
                    updated,
                    kind: neuron.kind,
                },
            ))
        })
        .collect();
    candidates.sort_by(|(a_iso, a), (b_iso, b)| {
        (b_iso, &b.updated, &b.path).cmp(&(a_iso, &a.updated, &a.path))
    });
    candidates
        .into_iter()
        .take(RECENT_LIMIT)
        .map(|(_, r)| r)
        .collect()
}

fn brain_md(store: &impl BrainStore) -> String {
    match store.structure_file("brain.md") {
        Some(Ok(fm)) => truncate_brain_md(py_strip(&universal_newlines(&fm.body))),
        _ => String::new(),
    }
}

/// v2's cap: over [`BRAIN_MD_MAX_BYTES`] UTF-8 bytes, the text is cut there
/// (a character the cut splits is dropped) and `\n\n[... truncated]` added.
pub fn truncate_brain_md(body: &str) -> String {
    if body.len() <= BRAIN_MD_MAX_BYTES {
        return body.to_string();
    }
    let mut end = BRAIN_MD_MAX_BYTES;
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[... truncated]", &body[..end])
}

/// A lobe's description, v2's `_lobe_description`: map.md's frontmatter
/// `description` when it is a string (a top-level date counts, as its ISO
/// text) that is not blank, stripped; else the first line after a `# `
/// title that is not blank and does not start with `up `, `sideways `,
/// `## ` or `- [`, stripped, in the body (in the whole text when the
/// frontmatter does not parse). `""` without map.md, or when it is not
/// UTF-8 (v2 raises there, PAR-011).
pub fn lobe_description(store: &impl BrainStore, lobe: &str) -> String {
    let rel = format!("{lobe}/map.md");
    let content = match store.structure_file(&rel) {
        None => return String::new(),
        Some(Ok(fm)) => {
            let description = match fm.meta.get("description") {
                Some(Value::Str(s)) => Some(s.clone()),
                Some(Value::Date(d)) => Some(date_iso(d)),
                Some(Value::DateTime(dt)) => Some(datetime_iso(dt, 'T')),
                _ => None,
            };
            if let Some(description) = description {
                let stripped = py_strip(&description);
                if !stripped.is_empty() {
                    return stripped.to_string();
                }
            }
            fm.body
        }
        Some(Err(_)) => match store.structure_text(&rel) {
            Some(text) => text,
            None => return String::new(),
        },
    };
    let content = universal_newlines(&content);
    let mut title_seen = false;
    for raw in py_splitlines(&content) {
        let line = py_strip(raw);
        if line.is_empty() {
            continue;
        }
        if line.starts_with("# ") {
            title_seen = true;
            continue;
        }
        if !title_seen {
            continue;
        }
        if ["up ", "sideways ", "## ", "- ["]
            .iter()
            .any(|prefix| line.starts_with(prefix))
        {
            continue;
        }
        return line.to_string();
    }
    String::new()
}

/// Whether Python 3.11's `datetime.date.fromisoformat(text[:10])` succeeds:
/// the first 10 characters, 7, 8 or 10 UTF-8 bytes long, read as
/// `YYYY[-]MM[-]DD` or `YYYY[-]Www[[-]D]` (trailing bytes are not checked),
/// with a real calendar date in years 1..=9999.
pub fn is_iso_date_prefix(text: &str) -> bool {
    let prefix: String = text.chars().take(10).collect();
    iso_date(prefix.as_bytes()).is_some()
}

fn digits(bytes: &[u8], at: usize, count: usize) -> Option<i64> {
    let slice = bytes.get(at..at + count)?;
    slice.iter().try_fold(0i64, |acc, b| {
        b.is_ascii_digit().then(|| acc * 10 + i64::from(b - b'0'))
    })
}

fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year of a day count from [`days_from_civil`].
fn year_from_days(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}

/// Monday = 0, as Python's `weekday()`.
fn weekday(days: i64) -> i64 {
    // 1970-01-01 was a Thursday.
    (days + 3).rem_euclid(7)
}

/// CPython 3.11's `parse_isoformat_date` and the date checks after it; the
/// resulting year when it is a valid date. Translated from CPython 3.11
/// `Modules/_datetimemodule.c` (`parse_isoformat_date`); see LICENSE-PSF.
fn iso_date(bytes: &[u8]) -> Option<i64> {
    if ![7, 8, 10].contains(&bytes.len()) {
        return None;
    }
    let year = digits(bytes, 0, 4)?;
    let mut p = 4;
    let separated = bytes.get(p) == Some(&b'-');
    if separated {
        p += 1;
    }
    if bytes.get(p) == Some(&b'W') {
        p += 1;
        let week = digits(bytes, p, 2)?;
        p += 2;
        let day = if p < bytes.len() {
            if separated {
                if bytes.get(p) != Some(&b'-') {
                    return None;
                }
                p += 1;
            }
            digits(bytes, p, 1)?
        } else {
            1
        };
        let jan1 = days_from_civil(year, 1, 1);
        let first_weekday = weekday(jan1);
        if !(1..53).contains(&week) {
            let long_year =
                week == 53 && (first_weekday == 3 || (first_weekday == 2 && is_leap(year)));
            if !long_year {
                return None;
            }
        }
        if !(1..=7).contains(&day) {
            return None;
        }
        let mut monday = jan1 - first_weekday;
        if first_weekday > 3 {
            monday += 7;
        }
        let year = year_from_days(monday + (week - 1) * 7 + day - 1);
        return (1..=9999).contains(&year).then_some(year);
    }
    let month = digits(bytes, p, 2)?;
    p += 2;
    if separated {
        if bytes.get(p) != Some(&b'-') {
            return None;
        }
        p += 1;
    }
    let day = digits(bytes, p, 2)?;
    let valid = (1..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day);
    valid.then_some(year)
}
