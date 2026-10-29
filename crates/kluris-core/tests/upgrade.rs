//! What earlier releases left on a user's machine, through
//! `kluris_core::upgrade` directly: the skill copies they wrote and the
//! folders those copies are in (`upgrade::skills`), and the sync services
//! they installed, found by name and swept through a fake runner
//! (`upgrade::services`; each manager's steps for one brain are in the CLI's
//! `upgrade_services.rs`, beside the binary that runs them).

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use kluris_core::upgrade::services::{self, Env, Found, Manager, Ran, Runner};
use kluris_core::upgrade::skills::{
    self, Folder, has_earlier_trait, is_earlier_skill, is_earlier_workflow,
};
use tempfile::TempDir;

fn legacy_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills/legacy")
}

/// Every `fixtures/skills/legacy/` file, an earlier release's own rendering:
/// `(file, the name it is installed under, its text)`. `-bare` and the
/// `v1.*` files are the single-brain `kluris`, the others `kluris-acme`.
fn renderings() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(legacy_fixtures()).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap().to_string();
        let stem = file.trim_end_matches(".md").trim_end_matches(".workflow");
        let skill = if stem.ends_with("-bare") || stem.starts_with("v1.") {
            "kluris"
        } else {
            "kluris-acme"
        };
        let name = if file.ends_with(".workflow.md") {
            format!("{skill}.md")
        } else {
            skill.to_string()
        };
        out.push((file, name, fs::read_to_string(&path).unwrap()));
    }
    out.sort();
    assert!(out.len() >= 30, "{}", out.len());
    out
}

#[test]
fn every_earlier_rendering_is_recognised_lf_and_crlf() {
    for (file, name, text) in renderings() {
        for text in [text.clone(), text.replace('\n', "\r\n")] {
            let ok = if file.ends_with(".workflow.md") {
                is_earlier_workflow(&name, &text)
            } else {
                is_earlier_skill(&name, &text)
            };
            assert!(ok, "{file} as {name}");
            assert!(has_earlier_trait(&text), "{file}");
        }
    }
}

#[test]
fn an_earlier_rendering_is_recognised_only_under_its_own_name() {
    for (file, name, text) in renderings() {
        if file.ends_with(".workflow.md") {
            assert!(!is_earlier_workflow("kluris-other.md", &text), "{file}");
            assert!(!is_earlier_workflow(&name.replace(".md", ""), &text));
        } else {
            assert!(!is_earlier_skill("kluris-other", &text), "{file}");
        }
    }
}

/// A user's own skill whose description merely opens like an earlier
/// release's carries the trait (a near miss) but is not an earlier skill.
#[test]
fn a_users_own_skill_is_not_an_earlier_skill() {
    let own = "---\nname: kluris-notes\ndescription: \"You are the team's subject matter expert on notes\"\n---\n\n# Notes\n";
    assert!(has_earlier_trait(own));
    assert!(!is_earlier_skill("kluris-notes", own));
    assert!(!has_earlier_trait("---\nname: notes\n---\n"));
}

fn dirs(home: &Path, rels: &[&str]) {
    for rel in rels {
        fs::create_dir_all(home.join(rel)).unwrap();
    }
}

#[test]
fn the_folders_earlier_releases_wrote_skills_to_in_order() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    dirs(
        home,
        &[
            ".codeium/windsurf-next",
            ".codeium/windsurf",
            ".codeium/other",
        ],
    );
    let windsurf = home.join(".codeium").join("windsurf");
    let next = home.join(".codeium").join("windsurf-next");
    let mut expected = vec![
        Folder::Skills(windsurf.join("skills")),
        Folder::Skills(next.join("skills")),
        Folder::Workflows(windsurf.join("global_workflows")),
        Folder::Workflows(next.join("global_workflows")),
    ];
    let homed = |rel: &str| rel.split('/').fold(home.to_path_buf(), |p, s| p.join(s));
    for rel in [
        ".cursor/skills",
        ".codex/skills",
        ".copilot/skills",
        ".gemini/skills",
        ".kilo/skills",
        ".kilocode/skills",
        ".junie/skills",
    ] {
        expected.push(Folder::Skills(homed(rel)));
    }
    assert_eq!(skills::folders(home, None), expected);

    // With COPILOT_HOME set, ~/.copilot/skills may be the folder Copilot
    // reads: it is not an earlier copy's.
    let copilot = home.join("copilot-home");
    expected.retain(|f| f != &Folder::Skills(homed(".copilot/skills")));
    assert_eq!(skills::folders(home, Some(&copilot)), expected);
}

#[test]
fn without_windsurf_only_the_fixed_folders_are_listed() {
    let tmp = TempDir::new().unwrap();
    let listed = skills::folders(tmp.path(), None);
    assert_eq!(listed.len(), 7, "{listed:?}");
    assert!(listed.iter().all(|f| matches!(f, Folder::Skills(_))));
}

// The sync services.

/// Records every command as one line (`program arg arg`) and answers by
/// exact line, the latest answer first (exit 0 and no output otherwise).
#[derive(Default)]
struct FakeRunner {
    calls: RefCell<Vec<String>>,
    answers: RefCell<Vec<(String, Ran)>>,
}

impl FakeRunner {
    fn answer(&self, line: &str, code: i32, stdout: &str, stderr: &str) {
        self.answers.borrow_mut().push((
            line.to_string(),
            Ran {
                code: Some(code),
                stdout: stdout.into(),
                stderr: stderr.into(),
            },
        ));
    }

    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Runner for FakeRunner {
    fn run(&self, program: &str, args: &[&str]) -> io::Result<Ran> {
        let line = std::iter::once(program)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        self.calls.borrow_mut().push(line.clone());
        let answer = self
            .answers
            .borrow()
            .iter()
            .rev()
            .find(|(l, _)| *l == line)
            .map(|(_, ran)| ran.clone());
        Ok(answer.unwrap_or(Ran {
            code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }))
    }
}

fn env<'a>(home: &Path, manager: Manager, runner: &'a FakeRunner) -> Env<'a> {
    Env {
        manager,
        home: home.to_path_buf(),
        xdg_config_home: None,
        local_app_data: Some(home.join("lad")),
        uid: 501,
        runner,
    }
}

fn touch(path: &Path) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "an earlier release's\n").unwrap();
    path.to_path_buf()
}

#[test]
fn services_are_found_by_their_names_alone() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let agents = home.join("Library/LaunchAgents");
    let beta = touch(&agents.join("dev.ngvoicu.kluris.sync.beta.plist"));
    let acme = touch(&agents.join("dev.ngvoicu.kluris.sync.acme.plist"));
    for other in [
        "com.other.agent.plist",
        "dev.ngvoicu.kluris.sync.Zeta.plist",
        "dev.ngvoicu.kluris.sync..plist",
        "dev.ngvoicu.kluris.sync.acme.txt",
    ] {
        touch(&agents.join(other));
    }
    let log = touch(&home.join("Library/Logs/kluris/sync-acme.log"));
    let runner = FakeRunner::default();
    assert_eq!(
        services::find(&env(home, Manager::Launchd, &runner)),
        [
            Found {
                brain: "acme".into(),
                name: "dev.ngvoicu.kluris.sync.acme".into(),
                definition: Some(acme),
                log: Some(log),
            },
            Found {
                brain: "beta".into(),
                name: "dev.ngvoicu.kluris.sync.beta".into(),
                definition: Some(beta),
                log: None,
            },
        ]
    );
    assert_eq!(runner.calls(), Vec::<String>::new());
}

#[test]
fn units_are_found_under_xdg_config_home_when_set() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let xdg = tmp.path().join("xdg");
    touch(&home.join(".config/systemd/user/kluris-sync-home.service"));
    let unit = touch(&xdg.join("systemd/user/kluris-sync-acme.service"));
    let runner = FakeRunner::default();
    let mut env = env(&home, Manager::Systemd, &runner);
    env.xdg_config_home = Some(xdg);
    let found = services::find(&env);
    assert_eq!(
        found,
        [Found {
            brain: "acme".into(),
            name: "kluris-sync-acme.service".into(),
            definition: Some(unit),
            log: None,
        }]
    );
}

const TASKS: &str = "schtasks /Query /FO CSV /NH";

#[test]
fn tasks_are_found_by_the_name_schtasks_lists() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let log = touch(&home.join("lad/kluris/logs/sync-beta.log"));
    let runner = FakeRunner::default();
    runner.answer(
        TASKS,
        0,
        "\r\n\"\\Kluris\\sync-beta\",\"N/A\",\"Bereit\"\r\n\
         \"\\Kluris\\sync-acme\",\"28.10.2026 08:00:00\",\"Wird ausgef\u{fc}hrt\"\r\n\
         \"\\Kluris\\sync-acme\",\"N/A\",\"Bereit\"\r\n\
         \"\\Kluris\\other\",\"N/A\",\"Bereit\"\r\n\
         \"\\Other\\sync-x\",\"N/A\",\"Bereit\"\r\n\
         \"\\Kluris\\sync-Bad\",\"N/A\",\"Bereit\"\r\n",
        "",
    );
    let found = services::find(&env(home, Manager::TaskScheduler, &runner));
    assert_eq!(
        found,
        [
            Found {
                brain: "acme".into(),
                name: r"\Kluris\sync-acme".into(),
                definition: None,
                log: None,
            },
            Found {
                brain: "beta".into(),
                name: r"\Kluris\sync-beta".into(),
                definition: None,
                log: Some(log),
            },
        ]
    );
    assert_eq!(runner.calls(), [TASKS]);
}

#[test]
fn schtasks_that_fails_finds_no_task() {
    let tmp = TempDir::new().unwrap();
    let runner = FakeRunner::default();
    runner.answer(TASKS, 1, "", "ERROR: Access is denied.");
    let env = env(tmp.path(), Manager::TaskScheduler, &runner);
    assert_eq!(services::find(&env), []);
    assert_eq!(services::sweep(&env), []);
    assert_eq!(runner.calls(), [TASKS, TASKS]);
}

#[test]
fn a_sweep_stops_and_removes_every_unit() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let units = home.join(".config/systemd/user");
    let acme = touch(&units.join("kluris-sync-acme.service"));
    let beta = touch(&units.join("kluris-sync-beta.service"));
    let runner = FakeRunner::default();
    let removed = services::sweep(&env(home, Manager::Systemd, &runner));
    assert_eq!(
        runner.calls(),
        [
            "systemctl --user disable --now kluris-sync-acme.service",
            "systemctl --user daemon-reload",
            "systemctl --user disable --now kluris-sync-beta.service",
            "systemctl --user daemon-reload",
        ]
    );
    let names: Vec<(&str, &[PathBuf], Option<&str>)> = removed
        .iter()
        .map(|r| (r.name.as_str(), r.removed.as_slice(), r.problem.as_deref()))
        .collect();
    assert_eq!(
        names,
        [
            ("kluris-sync-acme.service", &[acme.clone()][..], None),
            ("kluris-sync-beta.service", &[beta.clone()][..], None),
        ]
    );
    assert!(!acme.exists() && !beta.exists());
}

#[test]
fn a_sweep_ends_and_deletes_every_task_and_its_log() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let log = touch(&home.join("lad/kluris/logs/sync-acme.log"));
    let runner = FakeRunner::default();
    runner.answer(
        TASKS,
        0,
        "\"\\Kluris\\sync-acme\",\"N/A\",\"Ready\"\r\n",
        "",
    );
    let removed = services::sweep(&env(home, Manager::TaskScheduler, &runner));
    assert_eq!(
        runner.calls(),
        [
            TASKS,
            r"schtasks /Query /TN \Kluris\sync-acme",
            r"schtasks /End /TN \Kluris\sync-acme",
            r"schtasks /Delete /TN \Kluris\sync-acme /F",
        ]
    );
    assert_eq!(removed.len(), 1);
    assert_eq!(
        removed[0].removed,
        [PathBuf::from(r"\Kluris\sync-acme"), log.clone()]
    );
    assert_eq!(removed[0].problem, None);
    assert!(!log.exists());
    assert!(!home.join("lad/kluris/logs").exists());
    assert!(home.join("lad/kluris").exists());
}

#[test]
fn a_step_that_fails_is_reported_and_the_sweep_goes_on() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let agents = home.join("Library/LaunchAgents");
    let acme = touch(&agents.join("dev.ngvoicu.kluris.sync.acme.plist"));
    let beta = touch(&agents.join("dev.ngvoicu.kluris.sync.beta.plist"));
    let runner = FakeRunner::default();
    runner.answer(
        "launchctl bootout gui/501/dev.ngvoicu.kluris.sync.acme",
        5,
        "",
        "Boot-out failed: 5: Input/output error",
    );
    let removed = services::sweep(&env(home, Manager::Launchd, &runner));
    assert_eq!(removed.len(), 2);
    let problem = removed[0].problem.as_deref().unwrap_or_default();
    assert!(problem.contains("Input/output error"), "{problem}");
    // A plist whose job will not go stays, for the next sweep.
    assert_eq!(removed[0].removed, Vec::<PathBuf>::new());
    assert!(acme.exists());
    assert_eq!(removed[1].problem, None);
    assert_eq!(removed[1].removed, std::slice::from_ref(&beta));
    assert!(!beta.exists());
    assert_eq!(
        runner.calls(),
        [
            "launchctl print gui/501/dev.ngvoicu.kluris.sync.acme",
            "launchctl bootout gui/501/dev.ngvoicu.kluris.sync.acme",
            "launchctl print gui/501/dev.ngvoicu.kluris.sync.beta",
            "launchctl bootout gui/501/dev.ngvoicu.kluris.sync.beta",
        ]
    );
}

#[test]
fn without_services_a_sweep_on_launchd_or_systemd_runs_nothing() {
    let tmp = TempDir::new().unwrap();
    for manager in [Manager::Launchd, Manager::Systemd] {
        let runner = FakeRunner::default();
        assert_eq!(services::sweep(&env(tmp.path(), manager, &runner)), []);
        assert_eq!(runner.calls(), Vec::<String>::new(), "{manager:?}");
    }
}
