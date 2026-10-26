//! Step 1 of dream: `updated` and `created` from git (v2 `_sync_brain_state`,
//! `cli.py:156-195`).
//!
//! For each neuron with a commit, `updated` becomes the first 10 characters
//! of its newest commit's author date (the author's own time zone) when
//! `str(updated)` differs; a markdown neuron without `created` gets its oldest
//! commit's. One write per neuron. An uncommitted neuron is left alone.

use super::workspace::{Workspace, converged};
use crate::brain::neuron_index::{Neuron, NeuronKind};
use crate::brain::store::BrainStore;
use crate::git::FileDates;
use crate::yaml::{Value, date_iso};

/// Python's `date[:10]`.
fn day(date: &str) -> String {
    date.chars().take(10).collect()
}

/// v2's `str(meta.get("updated", "")) != day`, for the values whose `str()`
/// can be a date: a string, or a date (read as its ISO text).
fn same_day(updated: Option<&Value>, day: &str) -> bool {
    match updated {
        Some(Value::Str(text)) => text == day,
        Some(Value::Date(date)) => date_iso(date) == day,
        _ => false,
    }
}

/// Apply the git dates; returns the neurons written, one entry each.
pub(crate) fn sync_dates<S: BrainStore>(
    ws: &mut Workspace<'_, S>,
    neurons: &[Neuron],
    dates: &FileDates,
) -> Vec<String> {
    let mut written = Vec::new();
    for neuron in neurons {
        let Some(fm) = ws.read_for_write(&neuron.rel) else {
            continue;
        };
        let mut patch = Vec::new();
        if let Some(latest) = dates.latest.get(&neuron.rel).filter(|d| !d.is_empty()) {
            let day = day(latest);
            if !same_day(fm.meta.get("updated"), &day) {
                patch.push(("updated", day));
            }
        }
        if neuron.kind == NeuronKind::Markdown
            && !fm.meta.contains_key("created")
            && let Some(created) = dates.created.get(&neuron.rel).filter(|d| !d.is_empty())
        {
            patch.push(("created", day(created)));
        }
        if patch.is_empty() {
            continue;
        }
        let mut meta = converged(&fm.meta);
        for (key, value) in patch {
            meta.insert(key.to_string(), Value::Str(value));
        }
        ws.write(&neuron.rel, Some(&fm), &meta, &fm.body);
        written.push(neuron.rel.clone());
    }
    written
}
