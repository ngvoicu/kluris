//! The pending writes of one dream, over the store.
//!
//! v2 writes each fix to disk and the next step reads it back. Kluris 3 keeps
//! every write here, as the bytes it would write, and reads through it with
//! the same parser it uses for files on disk, so each step sees exactly what
//! v2's re-read would see. The writes become one change set at the end.

use std::collections::BTreeMap;
use std::path::Path;

use crate::brain::change::{ChangeSet, FileWrite};
use crate::brain::store::BrainStore;
use crate::frontmatter::{
    self, Frontmatter, LineEnding, has_yaml_suffix, write_markdown, write_yaml_neuron_with,
};
use crate::yaml::{Map, Value, date_iso, datetime_iso};

/// How a file is read: as v2's `read_frontmatter` reads it (a YAML neuron
/// whose block does not parse reads as no metadata), or strictly, as dream
/// reads a file it may rewrite (that block is an error, and the file is
/// skipped rather than given a second block).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    Lenient,
    Strict,
}

pub(crate) struct Workspace<'a, S: BrainStore> {
    store: &'a S,
    /// New contents by the real brain-relative path (a symlink's target).
    pending: BTreeMap<String, String>,
    /// Files left alone, with why.
    pub(crate) skipped: BTreeMap<String, String>,
}

fn is_yaml(rel: &str) -> bool {
    has_yaml_suffix(Path::new(rel))
}

/// Parse a file's bytes the way [`Read`] says, by the suffix of the path it
/// was reached by.
pub(crate) fn parse(rel: &str, bytes: &[u8], how: Read) -> Result<Frontmatter, String> {
    if is_yaml(rel) {
        let text = String::from_utf8_lossy(bytes);
        return match how {
            Read::Lenient => Ok(frontmatter::parse_yaml_neuron_lenient(&text)),
            Read::Strict => frontmatter::parse_yaml_neuron(&text)
                .map_err(|e| format!("invalid frontmatter: {e}")),
        };
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not valid UTF-8".to_string())?;
    frontmatter::parse(text).map_err(|e| format!("invalid frontmatter: {e}"))
}

/// The metadata as v2's dream path writes it back: every top-level date and
/// datetime as its ISO text (v2's `_normalize_metadata`), so it is written
/// quoted (interview-01 #2). Nested values keep their types.
pub(crate) fn converged(meta: &Map) -> Map {
    meta.iter()
        .map(|(key, value)| {
            let value = match value {
                Value::Date(date) => Value::Str(date_iso(date)),
                Value::DateTime(dt) => Value::Str(datetime_iso(dt, 'T')),
                other => other.clone(),
            };
            (key.clone(), value)
        })
        .collect()
}

impl<'a, S: BrainStore> Workspace<'a, S> {
    pub(crate) fn new(store: &'a S) -> Workspace<'a, S> {
        Workspace {
            store,
            pending: BTreeMap::new(),
            skipped: BTreeMap::new(),
        }
    }

    pub(crate) fn store(&self) -> &'a S {
        self.store
    }

    fn real(&self, rel: &str) -> Option<String> {
        self.store.real_rel(rel)
    }

    /// The bytes a read of `rel` sees now.
    fn bytes(&self, rel: &str) -> Option<Vec<u8>> {
        match self.real(rel).and_then(|real| self.pending.get(&real)) {
            Some(text) => Some(text.as_bytes().to_vec()),
            None => self.store.bytes(rel),
        }
    }

    /// The file's text now; `None` when it cannot be read or is not UTF-8.
    pub(crate) fn text(&self, rel: &str) -> Option<String> {
        String::from_utf8(self.bytes(rel)?).ok()
    }

    /// `Path.exists()`, pending files included.
    pub(crate) fn exists(&self, rel: &str) -> bool {
        self.real(rel)
            .is_some_and(|real| self.pending.contains_key(&real))
            || self.store.exists(rel)
    }

    /// Read `rel` as it is now.
    pub(crate) fn read(&self, rel: &str, how: Read) -> Result<Frontmatter, String> {
        let bytes = self
            .bytes(rel)
            .ok_or_else(|| "cannot read the file".to_string())?;
        parse(rel, &bytes, how)
    }

    /// Read a file dream may rewrite; a file that does not read is skipped
    /// and reported.
    pub(crate) fn read_for_write(&mut self, rel: &str) -> Option<Frontmatter> {
        match self.read(rel, Read::Strict) {
            Ok(fm) => Some(fm),
            Err(error) => {
                self.skip(rel, error);
                None
            }
        }
    }

    pub(crate) fn skip(&mut self, rel: &str, error: String) {
        self.skipped.entry(rel.to_string()).or_insert(error);
    }

    /// Write a file read as `old` (`None` for a new file) with new metadata:
    /// markdown in python-frontmatter's framing, a YAML neuron in v2's `#---`
    /// block, each keeping the old file's line endings and BOM (new files are
    /// LF, interview-01 #3).
    pub(crate) fn write(&mut self, rel: &str, old: Option<&Frontmatter>, meta: &Map, body: &str) {
        let line_ending = old.map_or(LineEnding::Lf, |fm| fm.line_ending);
        let mut text = if is_yaml(rel) {
            write_yaml_neuron_with(meta, body, line_ending)
        } else {
            write_markdown(meta, body, line_ending)
        };
        if old.is_some_and(|fm| fm.had_bom) {
            text.insert(0, '\u{feff}');
        }
        match self.real(rel) {
            Some(real) => {
                self.pending.insert(real, text);
            }
            None => self.skip(rel, "it links outside the brain".into()),
        }
    }

    /// The pending writes whose bytes differ from the files on disk, in path
    /// order.
    pub(crate) fn into_change_set(self) -> (ChangeSet, BTreeMap<String, String>) {
        let store = self.store;
        let writes = self
            .pending
            .into_iter()
            .filter(|(rel, text)| store.bytes(rel).as_deref() != Some(text.as_bytes()))
            .map(|(rel, text)| FileWrite {
                rel,
                contents: text.into_bytes(),
            })
            .collect();
        (
            ChangeSet {
                writes,
                ..ChangeSet::default()
            },
            self.skipped,
        )
    }
}
