//! `search-eval`: the search-quality eval of research-05 §5 with the method
//! of research-06 §5, run through the real `kluris search` process.
//!
//! - Default: the public fixture `parity/fixtures/search-eval/` (its brains
//!   registered in a temporary home, so the developer's config is never
//!   read) and its graded queries. Fails below hit@5 [`MIN_HIT5`] or MRR
//!   [`MIN_MRR`].
//! - `--local`: the 74 tuning queries (`--queries`, default the spec's
//!   `assets/search-eval-queries.jsonl` beside this worktree) against the
//!   brains registered on this machine (`KLURIS_CONFIG` or
//!   `~/.kluris/config.yml`); queries for brains that are not registered are
//!   skipped. Prints the same metrics; no gate.
//!
//! A query is `{"id", "brain", "type", "q", "rel"}`; `rel` maps files to
//! grades 0-3 (0: judged not relevant) or lists relevant files (grade 1).
//! Each query runs `--repeat` times (in an order shuffled by `--seed`) with
//! `--limit 20`; the first run's ranking is scored, every run is timed.
//! Metrics are over the answerable queries (a file graded 1+): hit@1, hit@5,
//! MRR (first relevant file in the top 20), nDCG@10 (gain `2^g - 1`); the
//! no-answer queries are scored on the top hit's `coverage`; the labelled
//! share is the share of top-5 files that have a grade (a lower bound when
//! low). Glossary entries count as `glossary.md`.
//!
//! The report (`--report`, default `target/search-eval/report-<scope>.json`,
//! owner-only on Unix) is text-free: query ids, ranked file ids (FNV-1a of
//! `<brain>/<file>`), the metrics and the seed.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use serde_json::{Value, json};

/// The fixture gate (research-05 §5: v3.0 must not fall below these).
pub const MIN_HIT5: f64 = 0.90;
pub const MIN_MRR: f64 = 0.74;

const DEFAULT_SEED: u64 = 20_261_026;
const LIMIT: &str = "20";

fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

struct Options {
    local: bool,
    queries: Option<PathBuf>,
    bin: Option<PathBuf>,
    seed: u64,
    repeat: usize,
    report: Option<PathBuf>,
}

const USAGE: &str = "usage: search-eval [--local] [--queries FILE] [--bin KLURIS] [--seed N] [--repeat N] [--report FILE]";

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        local: false,
        queries: None,
        bin: None,
        seed: DEFAULT_SEED,
        repeat: 3,
        report: None,
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--local" => o.local = true,
            "--queries" => o.queries = Some(PathBuf::from(value()?)),
            "--bin" => o.bin = Some(PathBuf::from(value()?)),
            "--report" => o.report = Some(PathBuf::from(value()?)),
            "--seed" => o.seed = value()?.parse().map_err(|e| format!("--seed: {e}"))?,
            "--repeat" => {
                o.repeat = value()?.parse().map_err(|e| format!("--repeat: {e}"))?;
                o.repeat = o.repeat.max(1);
            }
            other => return Err(format!("unknown argument '{other}'\n{USAGE}")),
        }
    }
    Ok(o)
}

/// One labelled query.
struct Query {
    id: String,
    brain: String,
    kind: String,
    q: String,
    grades: HashMap<String, u32>,
}

impl Query {
    fn answerable(&self) -> bool {
        self.grades.values().any(|g| *g > 0)
    }
}

fn load_queries(path: &Path) -> Result<Vec<Query>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let v: Value =
            serde_json::from_str(line).map_err(|e| format!("{}:{}: {e}", path.display(), n + 1))?;
        let field = |k: &str| {
            v[k].as_str()
                .map(String::from)
                .ok_or_else(|| format!("{}:{}: no '{k}'", path.display(), n + 1))
        };
        let grades = match &v["rel"] {
            Value::Array(files) => files
                .iter()
                .filter_map(Value::as_str)
                .map(|f| (f.to_string(), 1))
                .collect(),
            Value::Object(map) => map
                .iter()
                .map(|(f, g)| {
                    (
                        f.clone(),
                        u32::try_from(g.as_u64().unwrap_or(0)).unwrap_or(0),
                    )
                })
                .collect(),
            _ => HashMap::new(),
        };
        out.push(Query {
            id: field("id")?,
            brain: field("brain")?,
            kind: v["type"].as_str().unwrap_or("").to_string(),
            q: field("q")?,
            grades,
        });
    }
    Ok(out)
}

/// A small deterministic generator for the run order.
fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    for i in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = usize::try_from(state % (i as u64 + 1)).unwrap_or(0);
        items.swap(i, j);
    }
}

fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The search binary: `--bin`, `KLURIS_V3_BIN`, else a fresh release build.
fn binary(o: &Options) -> Result<PathBuf, String> {
    if let Some(bin) = o
        .bin
        .clone()
        .or_else(|| std::env::var_os("KLURIS_V3_BIN").map(PathBuf::from))
    {
        return Ok(bin);
    }
    let root = workspace_root();
    let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["build", "--release", "-q", "-p", "kluris-cli"])
        .current_dir(&root)
        .status()
        .map_err(|e| format!("cargo build: {e}"))?;
    if !status.success() {
        return Err("cargo build --release -p kluris-cli failed".into());
    }
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from);
    Ok(target
        .join("release")
        .join(format!("kluris{}", std::env::consts::EXE_SUFFIX)))
}

/// One query's ranking and timings.
struct Run {
    files: Vec<String>,
    top_coverage: Option<f64>,
    millis: Vec<f64>,
}

fn run_query(
    bin: &Path,
    env: &[(String, String)],
    q: &Query,
    repeat: usize,
) -> Result<Run, String> {
    let mut first: Option<Value> = None;
    let mut millis = Vec::new();
    for _ in 0..repeat {
        let mut cmd = Command::new(bin);
        cmd.args([
            "search", "--brain", &q.brain, "--json", "--limit", LIMIT, "--", &q.q,
        ]);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let started = Instant::now();
        let out = cmd
            .output()
            .map_err(|e| format!("{}: {e}", bin.display()))?;
        millis.push(started.elapsed().as_secs_f64() * 1000.0);
        if !out.status.success() {
            return Err(format!(
                "query {}: exit {:?}: {}",
                q.id,
                out.status.code(),
                String::from_utf8_lossy(&out.stdout)
            ));
        }
        if first.is_none() {
            first = Some(
                serde_json::from_slice(&out.stdout).map_err(|e| format!("query {}: {e}", q.id))?,
            );
        }
    }
    let doc = first.unwrap_or(Value::Null);
    let results = doc["results"].as_array().cloned().unwrap_or_default();
    let mut files: Vec<String> = Vec::new();
    for r in &results {
        if let Some(f) = r["file"].as_str()
            && !files.iter().any(|x| x == f)
        {
            files.push(f.to_string());
        }
    }
    Ok(Run {
        files,
        top_coverage: results.first().and_then(|r| r["coverage"].as_f64()),
        millis,
    })
}

#[derive(Default, Clone, Copy)]
struct Sums {
    n: usize,
    hit1: f64,
    hit5: f64,
    mrr: f64,
    ndcg: f64,
}

impl Sums {
    fn add(&mut self, q: &Query, files: &[String]) {
        let first = files
            .iter()
            .take(20)
            .position(|f| q.grades.get(f).is_some_and(|g| *g > 0));
        self.n += 1;
        if let Some(rank) = first {
            self.hit1 += f64::from(u8::from(rank < 1));
            self.hit5 += f64::from(u8::from(rank < 5));
            self.mrr += 1.0 / (rank as f64 + 1.0);
        }
        let gain = |g: u32| 2f64.powi(i32::try_from(g).unwrap_or(0)) - 1.0;
        let dcg: f64 = files
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, f)| gain(q.grades.get(f).copied().unwrap_or(0)) / (i as f64 + 2.0).log2())
            .sum();
        let mut ideal: Vec<u32> = q.grades.values().copied().filter(|g| *g > 0).collect();
        ideal.sort_unstable_by(|a, b| b.cmp(a));
        let idcg: f64 = ideal
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, g)| gain(*g) / (i as f64 + 2.0).log2())
            .sum();
        if idcg > 0.0 {
            self.ndcg += dcg / idcg;
        }
    }

    fn mean(&self, x: f64) -> f64 {
        if self.n == 0 { 0.0 } else { x / self.n as f64 }
    }

    fn json(&self) -> Value {
        let r = |x: f64| (x * 1000.0).round() / 1000.0;
        json!({
            "n": self.n,
            "hit@1": r(self.mean(self.hit1)),
            "hit@5": r(self.mean(self.hit5)),
            "mrr": r(self.mean(self.mrr)),
            "ndcg@10": r(self.mean(self.ndcg)),
        })
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn write_report(path: &Path, report: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let text = serde_json::to_string_pretty(report).map_err(|e| e.to_string())?;
    file.write_all(format!("{text}\n").as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn yaml_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The fixture's brains registered in a temporary home.
fn fixture_env(home: &Path, brains: &Path) -> Result<Vec<(String, String)>, String> {
    let mut config = String::from("brains:\n");
    let mut entries: Vec<PathBuf> = fs::read_dir(brains)
        .map_err(|e| format!("{}: {e}", brains.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        config.push_str(&format!(
            "  {}:\n    path: {}\n    description: ''\n",
            yaml_quote(&name),
            yaml_quote(&path.display().to_string())
        ));
    }
    let config_path = home.join(".kluris").join("config.yml");
    fs::create_dir_all(home.join(".kluris")).map_err(|e| e.to_string())?;
    fs::write(&config_path, config).map_err(|e| e.to_string())?;
    let home = home.display().to_string();
    Ok(vec![
        ("HOME".into(), home.clone()),
        ("USERPROFILE".into(), home),
        ("KLURIS_CONFIG".into(), config_path.display().to_string()),
        ("KLURIS_NO_PROMPT".into(), "1".into()),
        ("NO_COLOR".into(), "1".into()),
    ])
}

fn registered_brains() -> Vec<String> {
    let path = kluris_core::config::global_config_path(
        std::env::var_os("KLURIS_CONFIG").as_deref(),
        std::env::var_os("HOME").as_deref(),
        std::env::home_dir(),
    );
    path.and_then(|p| kluris_core::config::read_global_config(&p).ok())
        .map(|c| {
            c.brains
                .into_iter()
                .filter(|(_, e)| Path::new(&e.path).is_dir())
                .map(|(n, _)| n)
                .collect()
        })
        .unwrap_or_default()
}

pub fn main(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let root = workspace_root();
    let fixture = root.join("parity").join("fixtures").join("search-eval");
    let scope = if o.local { "local" } else { "fixture" };
    let queries_path = o.queries.clone().unwrap_or_else(|| {
        if o.local {
            root.parent()
                .unwrap_or(&root)
                .join("kluris/.specs/kluris-v3-rust/assets/search-eval-queries.jsonl")
        } else {
            fixture.join("queries.jsonl")
        }
    });
    let mut queries = load_queries(&queries_path)?;
    let home = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let env = if o.local {
        let registered = registered_brains();
        let before = queries.len();
        queries.retain(|q| registered.contains(&q.brain));
        println!(
            "search-eval --local: {} of {before} queries name a registered brain",
            queries.len()
        );
        vec![("KLURIS_NO_PROMPT".to_string(), "1".to_string())]
    } else {
        fixture_env(home.path(), &fixture.join("brains"))?
    };
    if queries.is_empty() {
        return Err(format!("no queries to run from {}", queries_path.display()));
    }
    let bin = binary(&o)?;

    let mut order: Vec<usize> = (0..queries.len()).collect();
    shuffle(&mut order, o.seed);
    let mut runs: HashMap<usize, Run> = HashMap::new();
    for i in order {
        runs.insert(i, run_query(&bin, &env, &queries[i], o.repeat)?);
    }

    let mut all = Sums::default();
    let mut by_type: BTreeMap<String, Sums> = BTreeMap::new();
    let mut by_brain: BTreeMap<String, Sums> = BTreeMap::new();
    let (mut labelled, mut shown) = (0usize, 0usize);
    let mut no_answer: Vec<Value> = Vec::new();
    let mut latencies: Vec<f64> = Vec::new();
    let mut ranked: Vec<Value> = Vec::new();
    let mut misses: Vec<String> = Vec::new();
    for (i, q) in queries.iter().enumerate() {
        let Some(run) = runs.get(&i) else { continue };
        latencies.extend(&run.millis);
        for f in run.files.iter().take(5) {
            shown += 1;
            labelled += usize::from(q.grades.contains_key(f));
        }
        if q.answerable() {
            all.add(q, &run.files);
            by_type
                .entry(q.kind.clone())
                .or_default()
                .add(q, &run.files);
            by_brain
                .entry(q.brain.clone())
                .or_default()
                .add(q, &run.files);
            let rank = run
                .files
                .iter()
                .position(|f| q.grades.get(f).is_some_and(|g| *g > 0));
            if rank.is_none_or(|r| r >= 5) {
                misses.push(format!(
                    "{} (first relevant at {})",
                    q.id,
                    rank.map_or("none".into(), |r| (r + 1).to_string())
                ));
            }
        } else {
            no_answer.push(json!({"id": q.id, "top_coverage": run.top_coverage}));
        }
        ranked.push(json!({
            "id": q.id,
            "ranked": run.files.iter().map(|f| fnv1a(&format!("{}/{f}", q.brain))).collect::<Vec<_>>(),
        }));
    }
    latencies.sort_by(f64::total_cmp);
    let summary = all.json();
    let hit5 = all.mean(all.hit5);
    let mrr = all.mean(all.mrr);
    let report = json!({
        "scope": scope,
        "engine": "bm25",
        "seed": o.seed,
        "repeat": o.repeat,
        "queries_run": queries.len(),
        "metrics": summary,
        "by_type": by_type.iter().map(|(k, s)| (k.clone(), s.json())).collect::<serde_json::Map<_, _>>(),
        "by_brain": by_brain.iter().map(|(k, s)| (k.clone(), s.json())).collect::<serde_json::Map<_, _>>(),
        "no_answer": no_answer,
        "labelled_share_top5": if shown == 0 { 0.0 } else { (labelled as f64 / shown as f64 * 1000.0).round() / 1000.0 },
        "latency_ms": {
            "p50": (percentile(&latencies, 50.0) * 10.0).round() / 10.0,
            "p95": (percentile(&latencies, 95.0) * 10.0).round() / 10.0,
            "runs": latencies.len(),
        },
        "gate": if o.local { Value::Null } else { json!({"hit@5": MIN_HIT5, "mrr": MIN_MRR}) },
        "results": ranked,
    });
    let report_path = o.report.clone().unwrap_or_else(|| {
        root.join("target")
            .join("search-eval")
            .join(format!("report-{scope}.json"))
    });
    write_report(&report_path, &report)?;

    println!(
        "search-eval ({scope}, seed {}, {} runs per query)",
        o.seed, o.repeat
    );
    println!("  answerable {}: {}", all.n, summary);
    for (kind, s) in &by_type {
        println!("    {kind:<16} {}", s.json());
    }
    for (brain, s) in &by_brain {
        println!("    {brain:<16} {}", s.json());
    }
    println!("  no-answer top-hit coverage: {}", report["no_answer"]);
    println!(
        "  labelled share of the top 5: {}",
        report["labelled_share_top5"]
    );
    println!("  latency ms: {}", report["latency_ms"]);
    if !misses.is_empty() {
        println!("  not in the top 5: {}", misses.join(", "));
    }
    println!("  report: {}", report_path.display());
    if !o.local && (hit5 < MIN_HIT5 || mrr < MIN_MRR) {
        return Err(format!(
            "search-eval gate failed: hit@5 {hit5:.3} (min {MIN_HIT5}), MRR {mrr:.3} (min {MIN_MRR})"
        ));
    }
    Ok(())
}
