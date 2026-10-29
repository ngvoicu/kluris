//! Golden tests of `kluris dream` and `kluris mri` on the fixture brains of
//! `common::dream_brains`.
//!
//! A dream scenario snapshots the work tree before its first step; then each
//! dream its run (the command, exit code, stdout, stderr, `git status
//! --porcelain` of each brain, the files it changed) and, when it changed
//! any, the whole work tree after it. A file dream reports in `skipped`
//! must keep its bytes.
//!
//! The legacy brain, which an earlier Kluris's dream left canonical, is
//! frozen rather than built: one dream must leave every byte and `git
//! status` as they were, and report what the earlier Kluris reported.
//!
//! An MRI scenario snapshots each run, the work tree after the dream
//! preflight with the generated pages left out, and the graph each page
//! embeds (nodes sorted by path, edges as `source path → target path` sorted,
//! `meta` as written). Around its graph every page must be the shell, CSS
//! and JS of `kluris-core/assets/mri/` filled in for the registered brain,
//! with `</` and `<!--` escaped in the graph JSON.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use common::dream_brains::{
    D2, D3, Files, NO_PARENT, Neuron, healthy_brain, legacy_brain, mri_brain, register,
    resilience_brain, rich_brain,
};
use common::golden::python_json;
use common::sandbox::{
    Run, Sandbox, changed_files, fixture, json_dump, run_end, section, write_tree,
};
use common::{
    Entry, Tree, assert_unchanged, files_dump, native_path, native_paths, normalize, tree_dump,
    tree_files, windows_variant,
};
use kluris_core::json;
use regex::Regex;

// --- running ------------------------------------------------------------------

/// A dream or MRI run (`Run(args)`), or a commit of every brain at a date.
enum Step<'a> {
    Run(&'a [&'a str]),
    Commit(&'a str),
}

const DREAM: Step = Step::Run(&["dream", "--json"]);

/// How a scenario runs its steps.
struct Scenario<'a> {
    name: &'a str,
    sb: &'a Sandbox,
    /// The brains, for the porcelain lines and the commit steps.
    brains: Vec<PathBuf>,
    /// Environment changes for every run.
    env: Vec<(&'a str, String)>,
    /// Snapshot `git status --porcelain` of each brain after each run.
    porcelain: bool,
    /// Called on each run's command before it starts.
    prepare: Option<&'a dyn Fn(&mut Command)>,
}

impl<'a> Scenario<'a> {
    fn new(name: &'a str, sb: &'a Sandbox, brains: &[&Path]) -> Scenario<'a> {
        Scenario {
            name,
            sb,
            brains: brains.iter().map(|b| b.to_path_buf()).collect(),
            env: Vec::new(),
            porcelain: true,
            prepare: None,
        }
    }

    fn env(mut self, key: &'a str, value: impl Into<String>) -> Self {
        self.env.push((key, value.into()));
        self
    }

    fn no_porcelain(mut self) -> Self {
        self.porcelain = false;
        self
    }

    #[cfg(unix)]
    fn prepare(mut self, prepare: &'a dyn Fn(&mut Command)) -> Self {
        self.prepare = Some(prepare);
        self
    }

    fn run(&self, args: &[&str]) -> Run {
        let mut cmd = self.sb.command(args);
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        if let Some(prepare) = self.prepare {
            prepare(&mut cmd);
        }
        Run::of(cmd)
    }

    fn commit(&self, date: &str) {
        for brain in &self.brains {
            self.sb.commit(brain, "between dreams", date);
        }
    }

    /// A run's snapshot text: the command, stdout (a JSON document through
    /// [`json_dump`]) and stderr as [`section`]s, the [`run_end`], then
    /// `extra`.
    fn run_text(&self, args: &[&str], out: &Run, extra: &str) -> String {
        let stdout = self.paths(&out.stdout);
        let stdout = if stdout.trim_start().starts_with('{') {
            json_dump(&stdout, &self.sb.root)
        } else {
            self.shown(&self.brain_paths(&stdout))
        };
        format!(
            "$ kluris {}\n{}{}{}{extra}",
            args.join(" "),
            section("stdout", &stdout),
            section("stderr", &self.shown(&out.stderr)),
            run_end(out.code),
        )
    }

    /// `text` with each brain's folder and default page, which Kluris and
    /// the config hold in the OS's own form, shown `/`-joined after
    /// `<ROOT>` (see [`native_path`]); a page before its folder.
    fn paths(&self, text: &str) -> String {
        let mut text = text.to_string();
        for brain in &self.brains {
            let page = brain.join("brain-mri.html");
            let shown = format!(
                "<ROOT>/work/{}",
                slashed(page.strip_prefix(&self.sb.work).unwrap())
            );
            text = native_path(&text, &page, &shown);
        }
        for brain in &self.brains {
            let shown = format!(
                "<ROOT>/work/{}",
                slashed(brain.strip_prefix(&self.sb.work).unwrap())
            );
            text = native_path(&text, brain, &shown);
        }
        text
    }

    /// `text`, a text report, with each folder and file of each brain,
    /// relative to the brain, which the report prints in the OS's own form
    /// (`knowledge\deep` on Windows), checked in that form and shown
    /// `/`-joined ([`native_paths`]).
    fn brain_paths(&self, text: &str) -> String {
        let mut rels = BTreeSet::new();
        for brain in &self.brains {
            for file in tree_files(brain).into_keys() {
                let mut rel = file.as_str();
                rels.insert(rel.to_string());
                while let Some((parent, _)) = rel.rsplit_once('/') {
                    rels.insert(parent.to_string());
                    rel = parent;
                }
            }
        }
        let native: Vec<PathBuf> = rels
            .iter()
            .map(|rel| rel.split('/').collect::<PathBuf>())
            .collect();
        let pairs: Vec<(&Path, &str)> = native
            .iter()
            .zip(&rels)
            .map(|(path, rel)| (path.as_path(), rel.as_str()))
            .collect();
        native_paths(text, &pairs)
    }

    /// [`Self::paths`], then the root and today normalized.
    fn shown(&self, text: &str) -> String {
        normalize(&self.paths(text), &self.sb.root)
    }

    fn porcelain_text(&self) -> String {
        let mut text = String::new();
        if !self.porcelain {
            return text;
        }
        for brain in &self.brains {
            if brain.join(".git").exists() {
                let rel = brain.strip_prefix(&self.sb.work).unwrap();
                text.push_str(&format!(
                    "== git status --porcelain {}\n{}",
                    slashed(rel),
                    self.sb.porcelain(brain)
                ));
            }
        }
        text
    }

    /// Run the dream `steps`; snapshot the tree before them, each run and
    /// each tree a run changed. Returns each run with the tree after it.
    fn dream(&self, steps: &[Step]) -> Vec<(Run, Tree)> {
        snapshot(
            &format!("{}__0_before", self.name),
            &tree_dump(&self.sb.work, |t| self.shown(t)),
        );
        let mut outcomes = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            let args = match step {
                Step::Commit(date) => {
                    self.commit(date);
                    continue;
                }
                Step::Run(args) => args,
            };
            let before = tree_files(&self.sb.work);
            let out = self.run(args);
            let after = tree_files(&self.sb.work);
            assert_skipped_untouched(&out, &before, &after);
            let changed = changed_files(&before, &after);
            let extra = format!("{}== changed\n{changed}", self.porcelain_text());
            snapshot(
                &format!("{}__{}_run", self.name, i + 1),
                &self.run_text(args, &out, &extra),
            );
            if before != after {
                snapshot(
                    &format!("{}__{}_tree", self.name, i + 1),
                    &tree_dump(&self.sb.work, |t| self.shown(t)),
                );
            }
            outcomes.push((out, after));
        }
        outcomes
    }
}

/// The snapshots with a Windows variant, for reasons of the OS's own.
const WINDOWS_VARIANTS: &[&str] = &[
    // Windows lists a folder without regard to case, so the rich brain's
    // regenerated `knowledge/src/map.md` lists `Beta.md` after `_u.md`, and
    // `projects/api/map.md` `endpoints.md` before `Shout.MD`.
    "dream_rich__1_tree",
    "dream_rich__3_tree",
    "dream_broken_only__1_tree",
    "dream_brain_all__1_tree",
    "dream_legacy__1_run",
    "mri_rich__1_tree",
    "mri_rich__1_graph_rich",
    "mri_preflight_skipped__1_tree",
    "mri_brain_all__1_tree",
    "mri_brain_all__1_graph_rich",
    // The test makes no symlinks on Windows, so nothing is pruned for them.
    "mri_pruned__1_tree",
    "mri_pruned__1_run",
    "mri_pruned__1_graph_calm",
];

/// `text` as the snapshot `name` (its Windows variant when
/// [`WINDOWS_VARIANTS`] names it).
fn snapshot(name: &str, text: &str) {
    let check = || insta::assert_snapshot!(name, text);
    if WINDOWS_VARIANTS.contains(&name) {
        windows_variant(check);
    } else {
        check();
    }
}

fn slashed(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// A file a `--json` dream reports in `skipped` keeps its bytes.
fn assert_skipped_untouched(out: &Run, before: &Tree, after: &Tree) {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&out.stdout) else {
        return;
    };
    let entries: Vec<&serde_json::Value> = match doc.get("brains") {
        Some(serde_json::Value::Array(brains)) => brains.iter().collect(),
        _ => vec![&doc],
    };
    for entry in entries {
        let Some(skipped) = entry.get("skipped").and_then(|s| s.as_array()) else {
            continue;
        };
        for item in skipped {
            let key = format!(
                "brains/{}/{}",
                item["brain"].as_str().unwrap(),
                item["file"].as_str().unwrap()
            );
            assert_eq!(after.get(&key), before.get(&key), "changed skipped {key}");
        }
    }
}

// --- dream: the fixture brains ----------------------------------------------------

#[test]
fn dream_on_the_rich_brain_converges_in_one_run() {
    let sb = Sandbox::new();
    let brain = rich_brain(&sb, "rich");
    register(&sb, &[&brain]);
    let steps = [DREAM, Step::Commit(D3), DREAM, DREAM, Step::Run(&["dream"])];
    let outcomes = Scenario::new("dream_rich", &sb, &[&brain]).dream(&steps);
    // Report paths are `/`-joined on every OS.
    let first: serde_json::Value = serde_json::from_str(&outcomes[0].0.stdout).unwrap();
    for item in first["deprecation"]
        .as_array()
        .unwrap()
        .iter()
        .chain(first["broken_synapses_detail"].as_array().unwrap())
    {
        for key in ["file", "source", "target"] {
            if let Some(path) = item.get(key).and_then(|p| p.as_str()) {
                assert!(!path.contains('\\'), "{key} {path}");
            }
        }
    }
}

#[test]
fn dream_on_a_healthy_brain_exits_zero() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    register(&sb, &[&brain]);
    Scenario::new("dream_healthy", &sb, &[&brain]).dream(&[DREAM, Step::Run(&["dream"])]);
}

#[test]
fn dream_on_a_brain_without_git() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", false);
    write_tree(
        &brain,
        [(
            "projects/no-dates.md",
            "---\nparent: ./map.md\n---\n\n# No dates\n",
        )],
    );
    register(&sb, &[&brain]);
    Scenario::new("dream_no_git", &sb, &[&brain]).dream(&[DREAM, DREAM]);
}

/// Neuron dates are the commit dates in the author's time zone; today (the
/// new maps' `updated`) is local, here 14 hours ahead of UTC. On Windows
/// the run keeps UTC.
#[test]
fn dream_dates_near_midnight_in_the_author_time_zone() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    write_tree(
        &brain,
        [(
            "projects/late.md",
            Neuron::new("Late")
                .updated("'2020-01-01'")
                .created("'2020-01-01'")
                .text(),
        )],
    );
    sb.commit(
        &brain,
        "late at night in Chicago",
        "2026-10-24T23:30:00-05:00",
    );
    write_tree(
        &brain,
        [(
            "projects/early.md",
            Neuron::new("Early").updated("'2020-01-01'").text(),
        )],
    );
    sb.commit(
        &brain,
        "early morning in Bucharest",
        "2026-10-25T00:30:00+03:00",
    );
    register(&sb, &[&brain]);
    let zone = "Pacific/Kiritimati";
    let before = tree_files(&sb.work);
    let mut scenario = Scenario::new("dream_near_midnight", &sb, &[&brain]);
    if !cfg!(windows) {
        scenario = scenario.env("TZ", zone);
    }
    snapshot(
        "dream_near_midnight__0_before",
        &tree_dump(&sb.work, |t| scenario.shown(t)),
    );
    let out = scenario.run(&["dream", "--json"]);
    let after = tree_files(&sb.work);
    let local_today = |text: &str| {
        if cfg!(windows) {
            return text.to_string();
        }
        let today = jiff::Timestamp::now()
            .in_tz(zone)
            .unwrap()
            .date()
            .to_string();
        text.replace(&format!("updated: '{today}'"), "updated: '<TODAY>'")
    };
    let extra = format!(
        "{}== changed\n{}",
        scenario.porcelain_text(),
        changed_files(&before, &after)
    );
    snapshot(
        "dream_near_midnight__1_run",
        &scenario.run_text(&["dream", "--json"], &out, &extra),
    );
    snapshot(
        "dream_near_midnight__1_tree",
        &tree_dump(&sb.work, |t| local_today(&scenario.shown(t))),
    );
}

#[test]
fn dream_dates_move_with_later_commits() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    register(&sb, &[&brain]);
    let steps = [DREAM, Step::Commit(D2), DREAM, Step::Commit(D3), DREAM];
    Scenario::new("dream_later_commits", &sb, &[&brain]).dream(&steps);
}

/// A dreamed brain with one map's date set back and an extra key added
/// (body unchanged) and a new description: dream rewrites `brain.md`, whose
/// body changed, and leaves the map alone.
#[test]
fn dream_writes_a_file_only_when_its_body_changed() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    register(&sb, &[&brain]);
    let prep = sb.kluris(&["dream", "--json"]);
    assert_eq!(prep.code, 0, "{}{}", prep.stdout, prep.stderr);
    let map = brain.join("projects").join("map.md");
    let text = fs::read_to_string(&map).unwrap();
    let text = Regex::new(r"updated: '[0-9-]+'")
        .unwrap()
        .replace_all(&text, "updated: '2020-01-01'")
        .replace("parent: ../brain.md", "parent: ../brain.md\nextra: kept");
    fs::write(&map, text).unwrap();
    fs::write(
        brain.join("kluris.yml"),
        "name: calm\ndescription: A new description\n",
    )
    .unwrap();
    sb.commit(&brain, "normalized", D3);
    let outcomes = Scenario::new("dream_body_changed", &sb, &[&brain]).dream(&[DREAM]);
    let tree = &outcomes[0].1;
    assert!(
        String::from_utf8_lossy(tree["brains/calm/projects/map.md"].file()).contains("extra: kept")
    );
}

/// The maps of the legacy brain that list a lower-case name after an
/// upper-case one. Windows sorts a folder's names without regard to case,
/// so dream there lists `Beta.md` after `_u.md` and `endpoints.md` before
/// `Shout.MD`, and rewrites these two maps of the brain made on macOS.
const LEGACY_MAPS_IN_WINDOWS_ORDER: &[&str] = &["knowledge/src/map.md", "projects/api/map.md"];

/// The legacy brain, which an earlier Kluris's dream left canonical: dream
/// changes no byte of it (on Windows none but the maps it lists in Windows
/// order, which must then hold their frozen Windows bytes), leaves `git
/// status` clean but for those maps, and reports what the earlier Kluris's
/// own next dream reported.
#[test]
fn dream_leaves_a_brain_an_earlier_kluris_left_canonical_as_it_is() {
    let sb = Sandbox::new();
    let (brain, earlier) = legacy_brain(&sb, "rich");
    register(&sb, &[&brain]);
    assert_eq!(
        sb.porcelain(&brain),
        "",
        "the legacy brain is all committed"
    );
    let scenario = Scenario::new("dream_legacy", &sb, &[&brain]);
    let args = ["dream", "--json"];
    let before = tree_files(&sb.work);
    let out = scenario.run(&args);
    let after = tree_files(&sb.work);
    assert_legacy_kept(&before, &after, cfg!(windows));
    let porcelain: String = reordered(cfg!(windows))
        .iter()
        .map(|map| format!(" M {map}\n"))
        .collect();
    assert_eq!(sb.porcelain(&brain), porcelain, "git status --porcelain");
    let extra = format!(
        "{}== changed\n{}",
        scenario.porcelain_text(),
        changed_files(&before, &after)
    );
    snapshot(
        "dream_legacy__1_run",
        &scenario.run_text(&args, &out, &extra),
    );
    assert_eq!(out.code, 1, "{}{}", out.stdout, out.stderr);
    assert_same_report(&out.stdout, &earlier);
}

/// The maps dream rewrites in the legacy brain: on Windows
/// [`LEGACY_MAPS_IN_WINDOWS_ORDER`], elsewhere none.
fn reordered(windows: bool) -> &'static [&'static str] {
    if windows {
        LEGACY_MAPS_IN_WINDOWS_ORDER
    } else {
        &[]
    }
}

/// `after`, the work tree after dream on the legacy brain, is `before`, byte
/// for byte; with `windows`, but for the maps dream lists in Windows order,
/// which must be their frozen Windows bytes ([`legacy_windows_map`]) dated
/// today (in UTC or in the machine's own zone).
fn assert_legacy_kept(before: &Tree, after: &Tree, windows: bool) {
    let expected = |today: &str| -> Tree {
        let mut tree = before.clone();
        for map in reordered(windows) {
            let bytes = legacy_windows_map(map, today);
            tree.insert(format!("brains/rich/{map}"), Entry::File(bytes));
        }
        tree
    };
    let todays = [utc_today(), local_today()];
    if !todays.iter().any(|today| expected(today) == *after) {
        assert_unchanged(&expected(&todays[0]), after, "dream");
    }
}

/// The legacy map `map` as dream must write it on Windows, dated `today`:
/// `tests/fixtures/dream-legacy/rich/windows/<map>`, `<TODAY>` its date.
fn legacy_windows_map(map: &str, today: &str) -> Vec<u8> {
    let frozen = fixture("dream-legacy").join("rich/windows").join(map);
    let text = fs::read_to_string(&frozen).unwrap();
    text.replace("'<TODAY>'", &format!("'{today}'"))
        .into_bytes()
}

/// The frozen Windows maps of the legacy brain are its maps as the earlier
/// Kluris wrote them on macOS, but for their date and the order of their
/// Contents: names compared without regard to case, as Windows lists a
/// folder (ties by the name's bytes).
#[test]
fn the_legacy_windows_maps_are_the_maps_in_windows_order() {
    let rich = fixture("dream-legacy").join("rich");
    for map in LEGACY_MAPS_IN_WINDOWS_ORDER {
        let mac = fs::read_to_string(rich.join("commit-3").join(map)).unwrap();
        let windows = fs::read_to_string(rich.join("windows").join(map)).unwrap();
        let mut lines: Vec<String> = mac
            .split('\n')
            .map(|line| match line.strip_prefix("updated: '") {
                Some(_) => "updated: '<TODAY>'".to_string(),
                None => line.to_string(),
            })
            .collect();
        let start = lines.iter().position(|l| l == "## Contents").unwrap() + 2;
        let end = start
            + lines[start..]
                .iter()
                .take_while(|l| l.starts_with("- ["))
                .count();
        let name = |line: &String| line[3..line.find(']').unwrap()].to_string();
        lines[start..end].sort_by_key(|line| (name(line).to_lowercase(), name(line)));
        assert_ne!(
            mac, windows,
            "{map} lists its names in another order on Windows"
        );
        assert_eq!(lines.join("\n"), windows, "{map}");
    }
}

/// The Windows check of the legacy brain refuses each map that is not the
/// map in Windows order: two unrelated texts (one with a NUL and a byte
/// that is not UTF-8), and the Windows-order map with one byte changed;
/// it takes the maps in Windows order.
#[test]
fn the_windows_legacy_check_refuses_any_other_map_bytes() {
    let sb = Sandbox::new();
    legacy_brain(&sb, "rich");
    let before = tree_files(&sb.work);
    let windows_map = |map: &str| legacy_windows_map(map, &utc_today());
    let one_byte_changed = |map: &str| -> Vec<u8> {
        let mut bytes = windows_map(map);
        let at = bytes.len() - 2;
        bytes[at] ^= 0x20;
        bytes
    };
    let cases: Vec<(String, [Vec<u8>; 2])> = vec![
        (
            "a text".into(),
            [
                b"# deliberately wrong map A\n".to_vec(),
                b"# deliberately wrong map A\n".to_vec(),
            ],
        ),
        (
            "a NUL and a byte that is not UTF-8".into(),
            [
                b"# completely different map B\n\0\xff".to_vec(),
                b"# completely different map B\n\0\xff".to_vec(),
            ],
        ),
        (
            format!("one byte of {} changed", LEGACY_MAPS_IN_WINDOWS_ORDER[0]),
            [
                one_byte_changed(LEGACY_MAPS_IN_WINDOWS_ORDER[0]),
                windows_map(LEGACY_MAPS_IN_WINDOWS_ORDER[1]),
            ],
        ),
        (
            format!("one byte of {} changed", LEGACY_MAPS_IN_WINDOWS_ORDER[1]),
            [
                windows_map(LEGACY_MAPS_IN_WINDOWS_ORDER[0]),
                one_byte_changed(LEGACY_MAPS_IN_WINDOWS_ORDER[1]),
            ],
        ),
    ];
    let accepted: Vec<&str> = cases
        .iter()
        .filter(|(_, maps)| {
            let mut after = before.clone();
            for (map, bytes) in LEGACY_MAPS_IN_WINDOWS_ORDER.iter().zip(maps) {
                after.insert(format!("brains/rich/{map}"), Entry::File(bytes.clone()));
            }
            std::panic::catch_unwind(|| assert_legacy_kept(&before, &after, true)).is_ok()
        })
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        accepted.is_empty(),
        "the Windows check accepted maps with {accepted:?}"
    );
    let mut after = before.clone();
    for map in LEGACY_MAPS_IN_WINDOWS_ORDER {
        after.insert(format!("brains/rich/{map}"), Entry::File(windows_map(map)));
    }
    assert_legacy_kept(&before, &after, true);
}

/// Today's date in UTC.
fn utc_today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

/// Today's date in the machine's own time zone.
fn local_today() -> String {
    jiff::Zoned::now().date().to_string()
}

/// `stdout`, a dream's JSON report, is `earlier`, another dream's: the same
/// keys in the same order with the same values, but for `deprecation` and
/// `broken_synapses_detail`, which hold the same entries in any order (each
/// report lists them in the order its walk found them).
fn assert_same_report(stdout: &str, earlier: &str) {
    let read = |text: &str| -> String {
        let mut doc: serde_json::Value = serde_json::from_str(text).unwrap();
        for key in ["deprecation", "broken_synapses_detail"] {
            doc[key]
                .as_array_mut()
                .unwrap()
                .sort_by_cached_key(|entry| entry.to_string());
        }
        serde_json::to_string_pretty(&doc).unwrap()
    };
    assert_eq!(read(stdout), read(earlier), "the report");
}

#[test]
fn dream_broken_only() {
    let sb = Sandbox::new();
    let brain = rich_brain(&sb, "rich");
    register(&sb, &[&brain]);
    Scenario::new("dream_broken_only", &sb, &[&brain]).dream(&[
        Step::Run(&["dream", "--json", "--broken-only"]),
        Step::Run(&["dream", "--broken-only"]),
    ]);
}

#[test]
fn dream_brain_all() {
    let sb = Sandbox::new();
    let rich = rich_brain(&sb, "rich");
    let calm = healthy_brain(&sb, "calm", true);
    register(&sb, &[&rich, &calm]);
    Scenario::new("dream_brain_all", &sb, &[&rich, &calm]).dream(&[
        Step::Run(&["dream", "--json", "--brain", "all"]),
        Step::Run(&["dream", "--brain", "all"]),
    ]);
}

#[test]
fn dream_one_brain_of_two_by_name() {
    let sb = Sandbox::new();
    let rich = rich_brain(&sb, "rich");
    let calm = healthy_brain(&sb, "calm", true);
    register(&sb, &[&rich, &calm]);
    Scenario::new("dream_one_of_two", &sb, &[&rich, &calm])
        .dream(&[Step::Run(&["dream", "--json", "--brain", "calm"])]);
}

/// Dream writes no `map.md` into a dot directory, `node_modules` or
/// `__pycache__`, leaves the ones there alone and lists none of them.
#[test]
fn dream_prunes_directories() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    let old = Neuron::new("Old archived").text();
    write_tree(
        &brain,
        [
            (
                ".archive/map.md",
                "---\nauto_generated: true\nparent: ../brain.md\nsiblings: []\nupdated: '2026-01-01'\n---\n\n# .Archive\n\nup [brain.md](../brain.md)\n\n## Contents\n\n(empty)\n",
            ),
            (".archive/old.md", old.as_str()),
            (".github/workflows/ci.yml", "on: push\n"),
            ("node_modules/pkg/index.md", "# pkg\n"),
            (
                "projects/.hidden/map.md",
                "---\nauto_generated: true\n---\n\n# Hidden\n",
            ),
            ("projects/__pycache__/x.md", "# cache\n"),
        ],
    );
    sb.commit(&brain, "pruned dirs", D2);
    register(&sb, &[&brain]);
    Scenario::new("dream_pruned", &sb, &[&brain])
        .no_porcelain()
        .dream(&[DREAM, DREAM]);
}

#[test]
fn dream_help() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let out = sb.kluris(&["dream", "--help"]);
    let scenario = Scenario::new("dream_help", &sb, &[]);
    snapshot(
        "dream_help",
        &scenario.run_text(&["dream", "--help"], &out, ""),
    );
}

// --- dream: files it cannot read or must not damage -------------------------------

fn files(items: &[(&str, &[u8])]) -> Files {
    items
        .iter()
        .map(|(rel, bytes)| (rel.to_string(), bytes.to_vec()))
        .collect()
}

/// The healthy brain plus `extra`, committed, registered and dreamed once.
fn dream_once(name: &str, extra: Files) {
    let sb = Sandbox::new();
    let brain = resilience_brain(&sb, extra);
    register(&sb, &[&brain]);
    Scenario::new(name, &sb, &[&brain]).dream(&[DREAM]);
}

/// A neuron whose frontmatter cannot be read is left alone and reported in
/// `skipped`; the rest is dreamed, exit 1.
#[test]
fn dream_skips_a_malformed_neuron() {
    let cases: [(&str, &[u8]); 3] = [
        ("invalid_yaml", b"---\ntitle: Foo: bar\n---\n\n# Invalid\n"),
        ("non_string_key", b"---\n1: x\n---\n\n# Key\n"),
        (
            "not_utf8",
            b"---\ncreated: '2026-01-01'\n---\n\n# Caf\xe9\n",
        ),
    ];
    for (case, bad) in cases {
        dream_once(
            &format!("dream_malformed_{case}"),
            files(&[
                ("projects/bad.md", bad),
                ("projects/fix-me.md", NO_PARENT.as_bytes()),
            ]),
        );
    }
}

/// A YAML neuron whose `#---` block does not parse is left alone and
/// reported.
#[test]
fn dream_skips_a_yaml_neuron_whose_block_does_not_parse() {
    dream_once(
        "dream_yaml_block_unparsed",
        files(&[("projects/bad.yml", b"#---\n# title: Foo: bar\n#---\nk: 1\n")]),
    );
}

/// A neuron behind a BOM keeps the BOM and one frontmatter block.
#[test]
fn dream_keeps_a_bom_and_one_block() {
    let bom = format!("\u{feff}{NO_PARENT}");
    dream_once("dream_bom", files(&[("projects/bom.md", bom.as_bytes())]));
}

#[test]
fn dream_keeps_the_first_body_line_indented() {
    let text = "---\ncreated: '2026-01-01'\nupdated: '2026-01-01'\n---\n\n    let x = 1;\n    let y = 2;\n\nAfter.\n";
    dream_once(
        "dream_first_body_line",
        files(&[("projects/code.md", text.as_bytes())]),
    );
}

/// A CRLF neuron stays CRLF; the files dream creates are LF.
#[test]
fn dream_keeps_a_crlf_file_crlf() {
    let crlf = NO_PARENT.replace('\n', "\r\n");
    dream_once(
        "dream_crlf",
        files(&[("projects/crlf.md", crlf.as_bytes())]),
    );
}

/// A `related` entry that is not a string is a frontmatter issue.
#[test]
fn dream_reports_a_related_entry_that_is_not_a_string() {
    let odd = Neuron::new("Odd")
        .extra("related:\n- 1\n- ./alpha.md\n")
        .text();
    dream_once(
        "dream_related_not_a_string",
        files(&[("projects/odd.md", odd.as_bytes())]),
    );
}

/// `related` links that leave the brain are broken, not one-way.
#[test]
fn dream_counts_related_links_outside_the_brain_as_broken() {
    let sb = Sandbox::new();
    let out = Neuron::new("Out")
        .extra("related:\n- ../../../outside.md\n- /etc/hosts.md\n")
        .text();
    let brain = resilience_brain(&sb, files(&[("projects/out.md", out.as_bytes())]));
    write_tree(&sb.work, [("outside.md", Neuron::new("Outside").text())]);
    register(&sb, &[&brain]);
    Scenario::new("dream_related_outside", &sb, &[&brain]).dream(&[DREAM]);
}

/// A brain without `kluris.yml` is an error naming the file; nothing is
/// written.
#[test]
fn dream_on_a_brain_without_kluris_yml() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    fs::remove_file(brain.join("kluris.yml")).unwrap();
    register(&sb, &[&brain]);
    let scenario = Scenario::new("dream_no_kluris_yml", &sb, &[&brain]);
    snapshot(
        "dream_no_kluris_yml__0_before",
        &tree_dump(&sb.work, |t| scenario.shown(t)),
    );
    let before = tree_files(&sb.work);
    for (i, args) in [&["dream"][..], &["dream", "--json"]].iter().enumerate() {
        let mut out = scenario.run(args);
        let file = brain.join("kluris.yml");
        let message = format!("Brain calm has no kluris.yml: {}", file.display());
        if args.contains(&"--json") {
            let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
            assert_eq!(doc["error"], message.as_str());
        } else {
            assert_eq!(out.stderr, format!("Error: {message}\n"));
        }
        let shown = "<ROOT>/work/brains/calm/kluris.yml";
        out.stdout = native_path(&out.stdout, &file, shown);
        out.stderr = native_path(&out.stderr, &file, shown);
        let after = tree_files(&sb.work);
        let extra = format!(
            "{}== changed\n{}",
            scenario.porcelain_text(),
            changed_files(&before, &after)
        );
        snapshot(
            &format!("dream_no_kluris_yml__{}_run", i + 1),
            &scenario.run_text(args, &out, &extra),
        );
    }
}

/// Only links whose file is missing are broken: not links in code, other
/// schemes, a link with a title, or a `%20` path to a file that exists.
#[test]
fn dream_checks_links() {
    let body = "# Links\n\n```\n[fenced](./nope1.md)\n```\n\n`[inline](./nope2.md)`\n\n\
                [mail](mailto:a@b.c) [ftp](ftp://x/y)\n\n[titled](./alpha.md \"Alpha\")\n\n\
                [encoded](./my%20note.md)\n\n![img](./pic.png) ![gone](./nopic.png)\n\n[missing](./missing.md)\n";
    let links = Neuron::new("Links").body(body).text();
    let note = Neuron::new("Note").text();
    dream_once(
        "dream_links",
        files(&[
            ("projects/links.md", links.as_bytes()),
            ("projects/my note.md", note.as_bytes()),
            ("projects/pic.png", b"png"),
        ]),
    );
}

const MALFORMED_MAP: &str = "---\nrelated: [\n---\n\n# Projects\n\nKeep this.\n";

/// A `map.md` or `brain.md` dream cannot read keeps its bytes and is
/// reported; the rest is written.
#[test]
fn dream_keeps_a_structure_file_it_cannot_read() {
    let cases: [(&str, &str, &[u8]); 3] = [
        (
            "map_invalid_yaml",
            "projects/map.md",
            MALFORMED_MAP.as_bytes(),
        ),
        (
            "brain_md_invalid_yaml",
            "brain.md",
            b"---\nupdated: [\n---\n\n# calm\n\nMy own words.\n",
        ),
        (
            "map_not_utf8",
            "projects/map.md",
            b"---\nupdated: '2026-01-01'\n---\n\n# Caf\xe9\n",
        ),
    ];
    for (case, rel, bad) in cases {
        dream_once(
            &format!("dream_unreadable_{case}"),
            files(&[(rel, bad), ("projects/fix-me.md", NO_PARENT.as_bytes())]),
        );
    }
}

/// A YAML neuron that is not UTF-8 keeps its bytes and is reported.
#[test]
fn dream_keeps_a_yaml_neuron_that_is_not_utf8() {
    dream_once(
        "dream_yaml_not_utf8",
        files(&[(
            "projects/spec.yml",
            b"#---\n# parent: ./map.md\n# title: Spec\n# updated: '2026-01-01'\n#---\nopenapi: 3.0.0\ninfo: invalid-\xff-byte\n",
        )]),
    );
}

/// Under a 1,024-byte file-size limit the large neuron's rewrite fails: it
/// keeps every byte, is reported, and no temp file is left.
#[cfg(unix)]
#[test]
fn dream_keeps_a_file_whose_write_is_cut_short() {
    use std::os::unix::process::CommandExt;

    let sb = Sandbox::new();
    let large = format!("{NO_PARENT}{}", "Lorem ipsum dolor sit amet.\n".repeat(800));
    let brain = resilience_brain(&sb, files(&[("projects/large.md", large.as_bytes())]));
    register(&sb, &[&brain]);
    let limit = |cmd: &mut Command| {
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
                let limit = libc::rlimit {
                    rlim_cur: 1024,
                    rlim_max: 1024,
                };
                if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    };
    let outcomes = Scenario::new("dream_cut_short", &sb, &[&brain])
        .prepare(&limit)
        .dream(&[DREAM]);
    let tree = &outcomes[0].1;
    assert_eq!(
        tree["brains/calm/projects/large.md"].file(),
        large.as_bytes()
    );
    assert!(!tree.keys().any(|rel| rel.contains("kluris-tmp")));
}

/// Dream removes the temp files an interrupted write left (old, regular,
/// the exact name) and keeps a fresh one and any other name.
#[test]
fn dream_removes_old_temp_files() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    write_tree(
        &brain,
        [
            ("projects/.alpha.md.kluris-tmp-41-0", "half a write"),
            ("projects/.beta.md.kluris-tmp-41-1", "fresh"),
            ("projects/alpha.md.kluris-tmp-41-2", "no leading dot"),
        ],
    );
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2 * 3600);
    for rel in [
        "projects/.alpha.md.kluris-tmp-41-0",
        "projects/alpha.md.kluris-tmp-41-2",
    ] {
        fs::File::options()
            .write(true)
            .open(brain.join(rel))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    register(&sb, &[&brain]);
    Scenario::new("dream_old_temp_files", &sb, &[&brain]).dream(&[DREAM]);
}

// --- mri: the page -----------------------------------------------------------------

const SHELL: &str = include_str!("../../kluris-core/assets/mri/shell.html");
const CSS: &str = include_str!("../../kluris-core/assets/mri/mri.css");
const JS: &str = include_str!("../../kluris-core/assets/mri/mri.js");

/// The page's assets, by their SHA-256: the shell, CSS and JS every page
/// is checked against.
#[test]
fn mri_page_assets() {
    use sha2::{Digest, Sha256};
    let text: String = [("shell.html", SHELL), ("mri.css", CSS), ("mri.js", JS)]
        .iter()
        .map(|(name, bytes)| {
            let digest = Sha256::digest(bytes.as_bytes());
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            format!("{name} {hex} {} bytes\n", bytes.len())
        })
        .collect();
    snapshot("mri_page_assets", &text);
}

/// `</` and `<!--` in JSON text escaped so that it cannot end the script.
fn script_safe(text: &str) -> String {
    text.replace("</", "<\\/").replace("<!--", "\\u003c!--")
}

/// The page around its graph: the shell with the brain's name and counts,
/// the CSS and JS, and an empty graph.
fn expected_shell(name: &str, graph: &serde_json::Value) -> String {
    static FIELD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{(\w+)\}\}").unwrap());
    let nodes = graph["nodes"].as_array().unwrap();
    let lobes: std::collections::BTreeSet<&str> = nodes
        .iter()
        .filter_map(|n| n["lobe"].as_str())
        .filter(|l| !l.is_empty() && *l != "root")
        .collect();
    let neurons = nodes.iter().filter(|n| n["type"] == "neuron").count();
    let edges = graph["edges"].as_array().unwrap().len();
    let name_json = script_safe(&json::to_string(&json::Json::Str(name.to_string())));
    FIELD
        .replace_all(SHELL, |caps: &regex::Captures| match &caps[1] {
            "brain_name" => name.to_string(),
            "lobe_count" => lobes.len().to_string(),
            "neuron_count" => neurons.to_string(),
            "edge_count" => edges.to_string(),
            "css" => CSS.to_string(),
            "graph_json" => String::new(),
            "brain_name_json" => name_json.clone(),
            "js" => JS.to_string(),
            other => panic!("unknown shell field {other}"),
        })
        .into_owned()
}

/// Check a page for the brain registered as `name` and return its graph:
/// LF, one `</script>`, the graph JSON `json.dumps(indent=2)` of its parse
/// with `</` and `<!--` escaped, and the expected shell around it.
fn page_graph(page: &Path, name: &str) -> serde_json::Value {
    let text = fs::read_to_string(page).unwrap();
    assert!(!text.contains('\r'), "{} is not LF", page.display());
    assert_eq!(text.matches("</script>").count(), 1, "{}", page.display());
    let start = text.find("const graph = ").unwrap() + "const graph = ".len();
    let end = text.find(";\nconst BRAIN_NAME = ").unwrap();
    let raw = &text[start..end];
    let graph: serde_json::Value = serde_json::from_str(raw).unwrap();
    assert_eq!(
        raw,
        script_safe(&json::to_string_indented(&python_json(&graph), 2)),
        "the graph JSON is not json.dumps(indent=2) of its parse"
    );
    assert!(!raw.contains("<!--"));
    let rest = format!("{}{}", &text[..start], &text[end..]);
    assert!(
        rest == expected_shell(name, &graph),
        "{}: the page around the graph is not the shell for {name}",
        page.display()
    );
    graph
}

/// The graph as the snapshot shows it: each node without its `id` (its
/// index), sorted by path; each edge, whose keys must be `source`, `target`
/// and `type` in that order, as its source's and target's paths and its
/// type, sorted; `meta` as written.
fn sorted_graph(graph: &serde_json::Value) -> String {
    use serde_json::Value;
    let nodes = graph["nodes"].as_array().unwrap();
    let mut paths = Vec::new();
    let mut sorted = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        assert_eq!(node["id"], i, "ids are list indexes");
        let path = node["path"].as_str().unwrap().to_string();
        let mut node = node.as_object().unwrap().clone();
        node.shift_remove("id");
        paths.push(path.clone());
        sorted.push((path, Value::Object(node)));
    }
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for pair in sorted.windows(2) {
        assert_ne!(pair[0].0, pair[1].0, "one node per path");
    }
    let mut edges: Vec<(String, String, String)> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let keys: Vec<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
            assert_eq!(keys, ["source", "target", "type"], "the keys of edge {i}");
            let end = |key: &str| paths[e[key].as_u64().unwrap() as usize].clone();
            (
                end("source"),
                end("target"),
                e["type"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    edges.sort();
    let doc = serde_json::json!({
        "nodes": sorted.into_iter().map(|(_, n)| n).collect::<Vec<_>>(),
        "edges": edges
            .into_iter()
            .map(|(s, t, kind)| serde_json::json!({"source": s, "target": t, "type": kind}))
            .collect::<Vec<_>>(),
        "meta": graph["meta"],
    });
    json::to_string_indented(&python_json(&doc), 2) + "\n"
}

/// A graph of the nodes `a.md` and `b.md` and the one edge `edge`.
fn graph_with_edge(edge: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "nodes": [{"id": 0, "path": "a.md"}, {"id": 1, "path": "b.md"}],
        "edges": [edge],
        "meta": {},
    })
}

#[test]
#[should_panic(expected = "the keys of edge 0")]
fn the_graph_check_refuses_an_edge_with_another_key() {
    sorted_graph(&graph_with_edge(serde_json::json!({
        "source": 0, "target": 1, "type": "related", "weight": 1,
    })));
}

#[test]
#[should_panic(expected = "the keys of edge 0")]
fn the_graph_check_refuses_an_edge_with_its_keys_in_another_order() {
    sorted_graph(&graph_with_edge(serde_json::json!({
        "target": 1, "source": 0, "type": "related",
    })));
}

// --- mri: running ------------------------------------------------------------------

/// An `mri` run and the pages it must write: `(registered name, page path
/// relative to work/)`.
struct MriRun<'a> {
    args: &'a [&'a str],
    pages: &'a [(&'a str, &'a str)],
}

fn without_pages(tree: Tree) -> Tree {
    tree.into_iter()
        .filter(|(rel, _)| !rel.ends_with(".html"))
        .collect()
}

/// The file URL forms of the root a run can print, longest first: the
/// `file://` link of the root (percent-encoded as Kluris encodes it) and,
/// on a WSL host, the `file://wsl.localhost/<distro>` link `wslpath -w`
/// gives.
fn root_links(root: &Path) -> Vec<String> {
    let quote = |bytes: &[u8]| -> String {
        bytes
            .iter()
            .map(|&b| {
                if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    };
    let text = root.display().to_string();
    let mut links = vec![if cfg!(windows) {
        let text = text.replace('\\', "/");
        format!("file:///{}/{}", &text[..2], quote(&text.as_bytes()[3..]))
    } else {
        format!("file://{}", quote(text.as_bytes()))
    }];
    if let Some(distro) = wsl_distro(root) {
        links.push(format!("file://wsl.localhost/{distro}{text}"));
    }
    links.sort_by_key(|l| std::cmp::Reverse(l.len()));
    links
}

/// Whether the host is WSL, as Kluris asks: `/proc/version` mentions
/// Microsoft (the sandbox passes no `WSL_DISTRO_NAME`).
#[cfg(target_os = "linux")]
fn on_wsl() -> bool {
    fs::read_to_string("/proc/version").is_ok_and(|v| v.to_lowercase().contains("microsoft"))
}

#[cfg(not(target_os = "linux"))]
fn on_wsl() -> bool {
    false
}

/// On a WSL host, the distro `wslpath -w` names for `path`.
fn wsl_distro(path: &Path) -> Option<String> {
    if !on_wsl() {
        return None;
    }
    let out = Command::new("wslpath").arg("-w").arg(path).output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let rest = text.trim().strip_prefix(r"\\wsl.localhost\")?;
    Some(rest.split('\\').next()?.to_string())
}

/// Text output with the root's file links as `<ROOT_URL>`, then the root.
fn mri_text(text: &str, sb: &Sandbox) -> String {
    let mut text = text.to_string();
    for link in root_links(&sb.root) {
        text = text.replace(&link, "<ROOT_URL>");
    }
    text
}

impl Scenario<'_> {
    /// Run the MRI `runs`; snapshot each run, each tree a preflight changed
    /// (pages left out) and each page's graph when it differs from the
    /// page's graph before.
    fn mri(&self, runs: &[MriRun]) -> Vec<Run> {
        let mut tree = without_pages(tree_files(&self.sb.work));
        let mut graphs: BTreeMap<String, (String, usize)> = BTreeMap::new();
        let mut outs = Vec::new();
        for (i, step) in runs.iter().enumerate() {
            let n = i + 1;
            let mut out = self.run(step.args);
            self.assert_printed_pages(step.args, &out);
            out.stdout = mri_text(&out.stdout, self.sb);
            let after = without_pages(tree_files(&self.sb.work));
            let mut extra = format!(
                "== changed (pages left out)\n{}",
                changed_files(&tree, &after)
            );
            for (name, rel) in step.pages {
                let graph = sorted_graph(&page_graph(&self.sb.work.join(rel), name));
                let graph = normalize(&graph, &self.sb.root);
                match graphs.get(*rel) {
                    Some((before, run)) if *before == graph => {
                        extra.push_str(&format!(
                            "== page {rel} ({name}): graph as after run {run}\n"
                        ));
                    }
                    _ => {
                        let snap = format!("{}__{n}_graph_{name}", self.name);
                        extra.push_str(&format!("== page {rel} ({name}): graph {snap}\n"));
                        snapshot(&snap, &graph);
                        graphs.insert(rel.to_string(), (graph, n));
                    }
                }
            }
            snapshot(
                &format!("{}__{n}_run", self.name),
                &self.run_text(step.args, &out, &extra),
            );
            if after != tree {
                snapshot(
                    &format!("{}__{n}_tree", self.name),
                    &files_dump(&after, |t| self.shown(t)),
                );
            }
            tree = after;
            outs.push(out);
        }
        outs
    }
}

impl Scenario<'_> {
    /// The page `brain`'s run writes, as Kluris prints it: the `--output`
    /// given (its `.` and empty parts dropped), else the brain's
    /// `brain-mri.html`, each in the OS's own form.
    fn printed_page(&self, args: &[&str], brain: &Path) -> String {
        match args.iter().position(|a| *a == "--output") {
            Some(i) => Path::new(args[i + 1])
                .components()
                .collect::<PathBuf>()
                .display()
                .to_string(),
            None => brain.join("brain-mri.html").display().to_string(),
        }
    }

    /// Each page a run prints is the expected one, built with `std::path`:
    /// the JSON's `output_path` of each brain, and in text each
    /// `MRI complete — <page>` line.
    fn assert_printed_pages(&self, args: &[&str], out: &Run) {
        if out.stdout.trim_start().starts_with('{') {
            let doc: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
            for entry in doc["brains"].as_array().into_iter().flatten() {
                let name = entry["name"].as_str().unwrap();
                let brain = self.brain_named(name);
                assert_eq!(
                    entry["output_path"],
                    self.printed_page(args, brain).as_str(),
                    "{}",
                    out.stdout
                );
            }
            return;
        }
        let pages: Vec<String> = self
            .brains
            .iter()
            .map(|b| self.printed_page(args, b))
            .collect();
        for line in out.stdout.lines() {
            if let Some(page) = line.strip_prefix("MRI complete — ") {
                assert!(
                    pages.iter().any(|p| p == page),
                    "{page:?} is not one of {pages:?}"
                );
            }
        }
    }

    /// The brain registered as `name`: the one in a folder of that name,
    /// else the scenario's only brain.
    fn brain_named(&self, name: &str) -> &Path {
        self.brains
            .iter()
            .find(|b| b.file_name().is_some_and(|f| f == name))
            .or(match self.brains.as_slice() {
                [only] => Some(only),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no brain registered as {name}"))
    }
}

// --- mri: scenarios ----------------------------------------------------------------

const RICH_PAGE: &[(&str, &str)] = &[("rich", "brains/rich/brain-mri.html")];
const CALM_PAGE: &[(&str, &str)] = &[("calm", "brains/calm/brain-mri.html")];

#[test]
fn mri_on_the_rich_brain() {
    let sb = Sandbox::new();
    let brain = mri_brain(&sb, "rich");
    register(&sb, &[&brain]);
    Scenario::new("mri_rich", &sb, &[&brain]).mri(&[
        MriRun {
            args: &["mri", "--json"],
            pages: RICH_PAGE,
        },
        MriRun {
            args: &["mri", "--json"],
            pages: RICH_PAGE,
        },
        MriRun {
            args: &["mri"],
            pages: RICH_PAGE,
        },
    ]);
}

#[test]
fn mri_on_a_healthy_brain() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    register(&sb, &[&brain]);
    Scenario::new("mri_healthy", &sb, &[&brain]).mri(&[MriRun {
        args: &["mri", "--json"],
        pages: CALM_PAGE,
    }]);
}

/// The page is named after the registered brain, not its folder.
#[test]
fn mri_page_is_named_after_the_registered_brain() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "Some Folder", true);
    sb.write_config(&[("calm-brain", &brain, "fixture")]);
    Scenario::new("mri_registered_name", &sb, &[&brain]).mri(&[MriRun {
        args: &["mri", "--json"],
        pages: &[("calm-brain", "brains/Some Folder/brain-mri.html")],
    }]);
}

/// `--output` relative to the working directory, its missing folders made;
/// the JSON names it as given, normalized, in the OS's form. The link the
/// text run prints is percent-encoded, or on a WSL host the
/// `file://wsl.localhost/<distro>/...` link of `wslpath -w`, unencoded.
#[test]
fn mri_output_relative_to_the_working_directory() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    register(&sb, &[&brain]);
    let target = "out dir/./deep//é page.html";
    let page: &[(&str, &str)] = &[("calm", "out dir/deep/é page.html")];
    let scenario = Scenario::new("mri_output_relative", &sb, &[&brain]);
    let native: PathBuf = ["out dir", "deep", "é page.html"].iter().collect();

    let mut tree = without_pages(tree_files(&sb.work));
    let json_args = ["mri", "--json", "--output", target];
    let mut out = scenario.run(&json_args);
    scenario.assert_printed_pages(&json_args, &out);
    out.stdout = native_path(&out.stdout, &native, "out dir/deep/é page.html");
    let graph = sorted_graph(&page_graph(&sb.work.join(page[0].1), "calm"));
    snapshot(
        "mri_output_relative__1_graph_calm",
        &normalize(&graph, &sb.root),
    );
    let after = without_pages(tree_files(&sb.work));
    let extra = format!(
        "== changed (pages left out)\n{}",
        changed_files(&tree, &after)
    );
    snapshot(
        "mri_output_relative__1_run",
        &scenario.run_text(&json_args, &out, &extra),
    );
    snapshot(
        "mri_output_relative__1_tree",
        &files_dump(&after, |t| scenario.shown(t)),
    );
    assert!(!sb.work.join("brains/calm/brain-mri.html").exists());
    tree = after;

    let text_args = ["mri", "--output", target];
    let mut out = scenario.run(&text_args);
    scenario.assert_printed_pages(&text_args, &out);
    let wsl = wsl_distro(&sb.root);
    if let Some(distro) = &wsl {
        eprintln!("WSL host: distro {distro}, link {:?}", out.stdout);
        assert!(
            out.stdout.contains("Open: file://wsl.localhost/"),
            "{}",
            out.stdout
        );
        out.stdout = out.stdout.replace(
            &format!("file://wsl.localhost/{distro}/"),
            "file://wsl.localhost/<DISTRO>/",
        );
    }
    out.stdout = mri_text(&out.stdout, &sb);
    out.stdout = native_path(&out.stdout, &native, "out dir/deep/é page.html");
    assert_eq!(
        sorted_graph(&page_graph(&sb.work.join(page[0].1), "calm")),
        graph
    );
    let after = without_pages(tree_files(&sb.work));
    let extra = format!(
        "== changed (pages left out)\n{}== page {} (calm): graph as after run 1\n",
        changed_files(&tree, &after),
        page[0].1
    );
    let text = scenario.run_text(&text_args, &out, &extra);
    if wsl.is_some() {
        insta::with_settings!({snapshot_suffix => "wsl"}, {
            snapshot("mri_output_relative__2_run", &text);
        });
    } else {
        snapshot("mri_output_relative__2_run", &text);
    }
}

#[test]
fn mri_brain_all() {
    let sb = Sandbox::new();
    let rich = mri_brain(&sb, "rich");
    let calm = healthy_brain(&sb, "calm", true);
    register(&sb, &[&rich, &calm]);
    let pages: &[(&str, &str)] = &[
        ("rich", "brains/rich/brain-mri.html"),
        ("calm", "brains/calm/brain-mri.html"),
    ];
    Scenario::new("mri_brain_all", &sb, &[&rich, &calm]).mri(&[
        MriRun {
            args: &["mri", "--json", "--brain", "all"],
            pages,
        },
        MriRun {
            args: &["mri", "--brain", "all"],
            pages,
        },
    ]);
}

#[test]
fn mri_one_brain_of_two_by_name() {
    let sb = Sandbox::new();
    let rich = mri_brain(&sb, "rich");
    let calm = healthy_brain(&sb, "calm", true);
    register(&sb, &[&rich, &calm]);
    Scenario::new("mri_one_of_two", &sb, &[&rich, &calm]).mri(&[MriRun {
        args: &["mri", "--json", "--brain", "calm"],
        pages: CALM_PAGE,
    }]);
    assert!(!rich.join("brain-mri.html").exists());
}

/// `--output` with `--brain all` over two brains is refused; nothing is
/// written.
#[test]
fn mri_output_with_brain_all_is_refused() {
    for (case, args) in [
        (
            "json",
            &["mri", "--brain", "all", "--output", "x.html", "--json"][..],
        ),
        ("text", &["mri", "--brain", "all", "--output", "x.html"][..]),
    ] {
        let sb = Sandbox::new();
        let one = healthy_brain(&sb, "one", true);
        let two = healthy_brain(&sb, "two", true);
        register(&sb, &[&one, &two]);
        Scenario::new(
            &format!("mri_output_brain_all_refused_{case}"),
            &sb,
            &[&one, &two],
        )
        .mri(&[MriRun { args, pages: &[] }]);
    }
}

/// `--output` with `--brain all` over one brain writes that page.
#[test]
fn mri_output_with_brain_all_and_one_brain() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    register(&sb, &[&brain]);
    Scenario::new("mri_output_brain_all_one", &sb, &[&brain]).mri(&[MriRun {
        args: &["mri", "--json", "--brain", "all", "--output", "one.html"],
        pages: &[("calm", "one.html")],
    }]);
}

#[test]
fn mri_with_no_brains() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    Scenario::new("mri_no_brains", &sb, &[]).mri(&[
        MriRun {
            args: &["mri", "--json"],
            pages: &[],
        },
        MriRun {
            args: &["mri"],
            pages: &[],
        },
    ]);
}

/// The graph leaves out dot directories, `node_modules` and (Unix) a
/// symlink that leaves the brain; a symlink inside the brain is a node.
#[test]
fn mri_leaves_out_pruned_directories_and_escaping_symlinks() {
    let sb = Sandbox::new();
    let brain = healthy_brain(&sb, "calm", true);
    let old = Neuron::new("Old archived")
        .body("[gamma](../knowledge/gamma.md)")
        .text();
    let linked = Neuron::new("Linked")
        .body("[archived](../.archive/old.md)")
        .text();
    write_tree(
        &brain,
        [
            (".archive/old.md", old.as_str()),
            ("node_modules/pkg/readme-like.md", "# pkg\n"),
            ("projects/.hidden/note.md", "# Hidden\n"),
            ("projects/linked.md", linked.as_str()),
        ],
    );
    #[cfg(unix)]
    {
        let outside = sb.root.join("outside");
        write_tree(&outside, [("secret.md", "# Secret\n")]);
        std::os::unix::fs::symlink(outside.join("secret.md"), brain.join("projects/leak.md"))
            .unwrap();
        std::os::unix::fs::symlink("linked.md", brain.join("projects/alias.md")).unwrap();
    }
    sb.commit(&brain, "pruned", D2);
    register(&sb, &[&brain]);
    Scenario::new("mri_pruned", &sb, &[&brain]).mri(&[MriRun {
        args: &["mri", "--json"],
        pages: CALM_PAGE,
    }]);
}

/// Under WSL (`WSL_DISTRO_NAME`) the link is the `file://wsl.localhost/...`
/// form of what `wslpath -w` prints; when it fails, the `file:///` link.
#[cfg(unix)]
#[test]
fn mri_links_the_windows_path_under_wsl() {
    use std::os::unix::fs::PermissionsExt;
    for (case, fail) in [("wslpath", false), ("wslpath_fails", true)] {
        let sb = Sandbox::new();
        let brain = healthy_brain(&sb, "calm", true);
        register(&sb, &[&brain]);
        let bin = sb.root.join("fakebin");
        let fake = bin.join("wslpath");
        write_tree(
            &bin,
            [(
                "wslpath",
                r#"#!/bin/sh
[ "$1" = "-w" ] || exit 2
[ -n "$FAIL_WSLPATH" ] && exit 1
printf '%s\n' '\\wsl.localhost\Ubuntu\home\me\brain-mri.html'
"#,
            )],
        );
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let name = format!("mri_wsl_{case}");
        let mut scenario = Scenario::new(&name, &sb, &[&brain])
            .env("WSL_DISTRO_NAME", "Ubuntu")
            .env("PATH", path);
        if fail {
            scenario = scenario.env("FAIL_WSLPATH", "1");
        }
        scenario.mri(&[MriRun {
            args: &["mri"],
            pages: CALM_PAGE,
        }]);
    }
}

#[test]
fn mri_help() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let out = sb.kluris(&["mri", "--help"]);
    let scenario = Scenario::new("mri_help", &sb, &[]);
    snapshot("mri_help", &scenario.run_text(&["mri", "--help"], &out, ""));
}

/// `kluris --help` lists `mri` with its one-line summary.
#[test]
fn mri_in_the_root_help() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let out = sb.kluris(&["--help"]);
    assert_eq!(out.code, 0);
    let line = out
        .stdout
        .lines()
        .find(|l| l.trim_start().starts_with("mri "))
        .unwrap();
    snapshot("mri_root_help_line", line.trim_end());
}

/// A neuron the preflight cannot read is left alone and reported in its
/// brain's entry (and on stderr without `--json`); the page is written and
/// a brain with nothing skipped keeps its entry's shape.
#[test]
fn mri_reports_the_files_its_preflight_skipped() {
    let sb = Sandbox::new();
    let calm = healthy_brain(&sb, "calm", true);
    write_tree(
        &calm,
        [("projects/bad.md", "---\ntitle: Foo: bar\n---\n\n# Bad\n")],
    );
    sb.commit(&calm, "bad", D2);
    let rich = mri_brain(&sb, "rich");
    register(&sb, &[&calm, &rich]);
    let bad = fs::read(calm.join("projects/bad.md")).unwrap();
    Scenario::new("mri_preflight_skipped", &sb, &[&calm, &rich]).mri(&[
        MriRun {
            args: &["mri", "--json", "--brain", "all"],
            pages: &[],
        },
        MriRun {
            args: &["mri", "--brain", "all"],
            pages: &[],
        },
    ]);
    assert_eq!(fs::read(calm.join("projects/bad.md")).unwrap(), bad);
}
