//! BM25F over search units (research-05 §3): one length-normalised,
//! weighted term frequency per unit summed over its fields, then the BM25
//! saturation with `k1` [`K1`] and the inverse document frequency
//! `ln(1 + (N - df + 0.5) / (df + 0.5))`, where `N` is the number of units and
//! `df` the units holding the term in any field.

use super::fxhash::FxMap;

/// A unit's fields, in [`FIELD_WEIGHTS`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title = 0,
    Tags = 1,
    Path = 2,
    Heading = 3,
    Body = 4,
}

/// How many fields a unit has.
pub const FIELDS: usize = 5;

impl Field {
    pub const ALL: [Field; FIELDS] = [
        Field::Title,
        Field::Tags,
        Field::Path,
        Field::Heading,
        Field::Body,
    ];

    /// The name `matched_fields` uses.
    pub fn name(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Tags => "tag",
            Field::Path => "path",
            Field::Heading => "heading",
            Field::Body => "body",
        }
    }
}

/// Field weights: title 3, tags 2, path 1.5, headings 1.5, body 1 (research-05
/// §3; v2's 10/5/3/1 measured the same).
pub const FIELD_WEIGHTS: [f64; FIELDS] = [3.0, 2.0, 1.5, 1.5, 1.0];

/// Term-frequency saturation.
pub const K1: f64 = 1.2;

/// Length normalisation.
pub const B: f64 = 0.75;

/// The most vocabulary terms one prefix expands to (the most frequent win).
pub const MAX_PREFIX_TERMS: usize = 64;

/// One unit's terms, per field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unit {
    pub fields: [Vec<String>; FIELDS],
}

/// A query word: its term, or the vocabulary terms a prefix expands to; a
/// unit scores the best of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryTerm {
    pub terms: Vec<String>,
}

impl QueryTerm {
    pub fn exact(term: &str) -> QueryTerm {
        QueryTerm {
            terms: vec![term.to_string()],
        }
    }

    pub fn any(terms: Vec<String>) -> QueryTerm {
        QueryTerm { terms }
    }
}

/// Scores with what matched where.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub scores: Vec<f64>,
    /// Per unit, the indexes of the query terms that matched, ascending.
    pub terms: Vec<Vec<u32>>,
    /// Per unit, bit `f` set when some query term matched field `f`.
    pub fields: Vec<u8>,
}

type Posting = (u32, [u32; FIELDS]);

/// The inverted index of a set of units.
/// A unit's terms as ids into a list of term names, per field.
pub type IdUnit = [Vec<u32>; FIELDS];

#[derive(Debug, Clone)]
pub struct Index {
    units: usize,
    lengths: Vec<[u32; FIELDS]>,
    average: [f64; FIELDS],
    /// By term id.
    postings: Vec<Vec<Posting>>,
    /// The ids of the terms with postings.
    ids: FxMap<String, u32>,
    names: Vec<String>,
    /// Term ids with postings, sorted by name.
    vocabulary: Vec<u32>,
}

impl Index {
    pub fn new(units: &[Unit]) -> Index {
        let mut names = super::tokenize::Interner::default();
        let id_units: Vec<IdUnit> = units
            .iter()
            .map(|unit| {
                std::array::from_fn(|f| unit.fields[f].iter().map(|t| names.id(t)).collect())
            })
            .collect();
        Index::from_ids(&id_units, names.names())
    }

    /// An index over units whose terms are ids into `names`.
    pub fn from_ids(units: &[IdUnit], names: &[String]) -> Index {
        let mut postings: Vec<Vec<Posting>> = vec![Vec::new(); names.len()];
        let mut lengths = Vec::with_capacity(units.len());
        let mut totals = [0f64; FIELDS];
        let mut mark: Vec<u32> = vec![u32::MAX; names.len()];
        let mut slot: Vec<u32> = vec![0; names.len()];
        let mut local: Vec<(u32, [u32; FIELDS])> = Vec::new();
        for (id, unit) in units.iter().enumerate() {
            let id = u32::try_from(id).unwrap_or(u32::MAX);
            local.clear();
            let mut length = [0u32; FIELDS];
            for (f, terms) in unit.iter().enumerate() {
                length[f] = u32::try_from(terms.len()).unwrap_or(u32::MAX);
                totals[f] += terms.len() as f64;
                for &term in terms {
                    let t = term as usize;
                    if t >= names.len() {
                        continue;
                    }
                    if mark[t] != id {
                        mark[t] = id;
                        slot[t] = u32::try_from(local.len()).unwrap_or(u32::MAX);
                        local.push((term, [0; FIELDS]));
                    }
                    local[slot[t] as usize].1[f] += 1;
                }
            }
            lengths.push(length);
            for (term, tf) in &local {
                postings[*term as usize].push((id, *tf));
            }
        }
        let n = units.len().max(1) as f64;
        let average = totals.map(|total| if total > 0.0 { total / n } else { 1.0 });
        let mut vocabulary: Vec<u32> = (0..names.len())
            .filter(|&t| !postings[t].is_empty())
            .map(|t| u32::try_from(t).unwrap_or(u32::MAX))
            .collect();
        vocabulary.sort_by(|a, b| names[*a as usize].cmp(&names[*b as usize]));
        let ids = vocabulary
            .iter()
            .map(|&t| (names[t as usize].clone(), t))
            .collect();
        Index {
            units: units.len(),
            lengths,
            average,
            postings,
            ids,
            names: names.to_vec(),
            vocabulary,
        }
    }

    pub fn has_term(&self, term: &str) -> bool {
        self.ids.contains_key(term)
    }

    /// The vocabulary terms that start with `prefix` and are longer than it,
    /// at most [`MAX_PREFIX_TERMS`] of them (the most frequent), sorted.
    pub fn prefix_terms(&self, prefix: &str) -> Vec<String> {
        let name = |t: &u32| self.names[*t as usize].as_str();
        let from = self.vocabulary.partition_point(|t| name(t) < prefix);
        let mut found: Vec<u32> = self.vocabulary[from..]
            .iter()
            .take_while(|t| name(t).starts_with(prefix))
            .filter(|t| name(t).len() > prefix.len())
            .copied()
            .collect();
        if found.len() > MAX_PREFIX_TERMS {
            found.sort_by_key(|t| std::cmp::Reverse(self.postings[*t as usize].len()));
            found.truncate(MAX_PREFIX_TERMS);
            found.sort_by(|a, b| name(a).cmp(name(b)));
        }
        found.iter().map(|t| name(t).to_string()).collect()
    }

    fn df(&self, term: &str) -> usize {
        self.ids
            .get(term)
            .map_or(0, |t| self.postings[*t as usize].len())
    }

    /// The inverse document frequency of a term.
    pub fn idf(&self, term: &str) -> f64 {
        let n = self.units as f64;
        let df = self.df(term) as f64;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    /// Each unit's score for the query.
    pub fn score(&self, query: &[QueryTerm]) -> Vec<f64> {
        self.score_detailed(query).scores
    }

    /// Each unit's score, and which terms and fields matched.
    pub fn score_detailed(&self, query: &[QueryTerm]) -> Scored {
        let mut scored = Scored {
            scores: vec![0.0; self.units],
            terms: vec![Vec::new(); self.units],
            fields: vec![0; self.units],
        };
        let mut best = vec![0f64; self.units];
        for (qi, qt) in query.iter().enumerate() {
            let mut touched: Vec<usize> = Vec::new();
            for term in &qt.terms {
                let Some(list) = self.ids.get(term).map(|t| &self.postings[*t as usize]) else {
                    continue;
                };
                let idf = self.idf(term);
                for (id, tf) in list {
                    let id = *id as usize;
                    let mut weighted = 0.0;
                    for f in 0..FIELDS {
                        if tf[f] == 0 {
                            continue;
                        }
                        let norm = 1.0 - B + B * f64::from(self.lengths[id][f]) / self.average[f];
                        weighted += FIELD_WEIGHTS[f] * f64::from(tf[f]) / norm;
                        scored.fields[id] |= 1 << f;
                    }
                    let contribution = idf * weighted / (K1 + weighted);
                    if best[id] == 0.0 {
                        touched.push(id);
                    }
                    if contribution > best[id] {
                        best[id] = contribution;
                    }
                    let qi = u32::try_from(qi).unwrap_or(u32::MAX);
                    if scored.terms[id].last() != Some(&qi) {
                        scored.terms[id].push(qi);
                    }
                }
            }
            for id in touched {
                scored.scores[id] += best[id];
                best[id] = 0.0;
            }
        }
        scored
    }
}
