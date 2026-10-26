//! A neuron's frontmatter merged field by field, never line by line (SPEC
//! Phase 7 conflict rules):
//! - `created`: the earlier of the two sides, `updated`: the later, as the
//!   quoted `'YYYY-MM-DD'` text dream writes;
//! - `related` and `tags`: the union of both lists, ours first, then the
//!   items only theirs has, in their order;
//! - every other key three-way against the base: a side that left it as
//!   the base had it takes the other side's value (a removal included); the
//!   same change on both sides is that change; different changes are a
//!   conflict, [`ConflictKind::Parent`] for `parent`,
//!   [`ConflictKind::Frontmatter`] for any other key.
//!
//! Keys keep ours' order, then theirs' new keys in their order (the writer
//! sorts them anyway). Top-level dates become their ISO text, as dream's
//! writes converge them.

use super::ConflictKind;
use crate::yaml::{Map, Value, date_iso, datetime_iso};

/// A date-like value as its ISO text; `None` for anything else.
fn date_text(value: &Value) -> Option<String> {
    match value {
        Value::Date(date) => Some(date_iso(date)),
        Value::DateTime(dt) => Some(datetime_iso(dt, 'T')),
        Value::Str(text) => Some(text.clone()),
        _ => None,
    }
}

/// A top-level value as dream writes it back: dates as ISO text.
fn converged(value: &Value) -> Value {
    match value {
        Value::Date(_) | Value::DateTime(_) => Value::Str(date_text(value).unwrap_or_default()),
        other => other.clone(),
    }
}

/// The three-way choice: `None` when both sides changed it differently.
fn three_way(
    base: Option<&Value>,
    ours: Option<&Value>,
    theirs: Option<&Value>,
) -> Option<Option<Value>> {
    if ours == theirs || theirs == base {
        Some(ours.cloned())
    } else if ours == base {
        Some(theirs.cloned())
    } else {
        None
    }
}

/// The earlier (`later == false`) or later of the dates both sides have.
/// `None` when a side's value is not a date (the caller then goes three-way).
fn pick_date(ours: Option<&Value>, theirs: Option<&Value>, later: bool) -> Option<Option<Value>> {
    let mut texts = Vec::new();
    for value in [ours, theirs].into_iter().flatten() {
        texts.push(date_text(value)?);
    }
    let chosen = if later {
        texts.into_iter().max()
    } else {
        texts.into_iter().min()
    };
    Some(chosen.map(Value::Str))
}

/// Ours' list, then the items only theirs has. `None` unless each side is a
/// list or absent (and one is a list).
fn union(ours: Option<&Value>, theirs: Option<&Value>) -> Option<Value> {
    fn list(value: Option<&Value>) -> Option<&[Value]> {
        match value {
            None => Some(&[]),
            Some(Value::Seq(items)) => Some(items.as_slice()),
            Some(_) => None,
        }
    }
    let (o, t) = (list(ours)?, list(theirs)?);
    if ours.is_none() && theirs.is_none() {
        return None;
    }
    let mut items: Vec<Value> = o.to_vec();
    for item in t {
        if !items.contains(item) {
            items.push(item.clone());
        }
    }
    Some(Value::Seq(items))
}

/// Merge three frontmatters (see the module docs); a person must decide
/// when a key other than the merged ones changed differently on both sides.
pub fn merge_meta(base: &Map, ours: &Map, theirs: &Map) -> Result<Map, ConflictKind> {
    let mut keys: Vec<&String> = ours.keys().collect();
    keys.extend(theirs.keys().filter(|k| !ours.contains_key(*k)));
    keys.extend(
        base.keys()
            .filter(|k| !ours.contains_key(*k) && !theirs.contains_key(*k)),
    );
    let mut merged = Map::new();
    for key in keys {
        let (b, o, t) = (base.get(key), ours.get(key), theirs.get(key));
        let decided = match key.as_str() {
            "created" => pick_date(o, t, false),
            "updated" => pick_date(o, t, true),
            "related" | "tags" => union(o, t).map(Some),
            _ => None,
        };
        let value = match decided {
            Some(value) => value,
            None => three_way(b, o, t).ok_or(if key == "parent" {
                ConflictKind::Parent
            } else {
                ConflictKind::Frontmatter
            })?,
        };
        if let Some(value) = value {
            merged.insert(key.clone(), converged(&value));
        }
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yaml::Date;

    fn s(text: &str) -> Value {
        Value::Str(text.into())
    }

    fn map(entries: &[(&str, Value)]) -> Map {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn seq(items: &[&str]) -> Value {
        Value::Seq(items.iter().map(|i| s(i)).collect())
    }

    #[test]
    fn dates_take_the_earlier_created_and_the_later_updated() {
        let base = map(&[("created", s("2026-01-01")), ("updated", s("2026-01-01"))]);
        let ours = map(&[
            ("created", Value::Date(Date::new(2025, 12, 1).unwrap())),
            ("updated", s("2026-02-01")),
        ]);
        let theirs = map(&[("created", s("2026-01-01")), ("updated", s("2026-03-01"))]);
        let merged = merge_meta(&base, &ours, &theirs).unwrap();
        assert_eq!(merged["created"], s("2025-12-01"));
        assert_eq!(merged["updated"], s("2026-03-01"));
    }

    #[test]
    fn lists_are_unions_ours_first() {
        let base = map(&[("tags", seq(&["a"]))]);
        let ours = map(&[("tags", seq(&["a", "o"]))]);
        let theirs = map(&[("tags", seq(&["t", "a"])), ("related", seq(&["x.md"]))]);
        let merged = merge_meta(&base, &ours, &theirs).unwrap();
        assert_eq!(merged["tags"], seq(&["a", "o", "t"]));
        assert_eq!(merged["related"], seq(&["x.md"]));
    }

    #[test]
    fn other_keys_go_three_way() {
        let base = map(&[
            ("parent", s("./map.md")),
            ("owner", s("a")),
            ("gone", s("x")),
        ]);
        let ours = map(&[("parent", s("./map.md")), ("owner", s("b"))]);
        let theirs = map(&[
            ("parent", s("../map.md")),
            ("owner", s("a")),
            ("gone", s("x")),
        ]);
        let merged = merge_meta(&base, &ours, &theirs).unwrap();
        assert_eq!(merged["parent"], s("../map.md"));
        assert_eq!(merged["owner"], s("b"));
        assert!(!merged.contains_key("gone"), "a removal on one side wins");
        let theirs = map(&[("parent", s("../x.md")), ("owner", s("a"))]);
        let ours = map(&[("parent", s("../y.md")), ("owner", s("a"))]);
        assert_eq!(merge_meta(&base, &ours, &theirs), Err(ConflictKind::Parent));
        let ours = map(&[("parent", s("./map.md")), ("owner", s("c"))]);
        let theirs = map(&[("parent", s("./map.md")), ("owner", s("d"))]);
        assert_eq!(
            merge_meta(&base, &ours, &theirs),
            Err(ConflictKind::Frontmatter)
        );
    }
}
