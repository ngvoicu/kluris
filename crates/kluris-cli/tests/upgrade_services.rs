//! The per-user sync services an earlier release installed, each running
//! `kluris sync --watch --brain <b>` from login on (`kluris_core::upgrade::
//! services`). Such a service now starts this kluris, whose hidden `sync
//! --watch` arm removes its own service and log and exits 0; `doctor` and
//! `self-uninstall` remove every one.
//!
//! - The steps of each service manager, in order, through a fake runner at
//!   the `launchctl` / `systemctl --user` / `schtasks` boundary.
//! - The binary: the arm, `doctor`, `self-uninstall` and `remove`, with a
//!   fake `launchctl` (macOS) or `systemctl` (Linux) first on PATH; the
//!   service flags are unknown and the help shows none.
//! - `real_round_trip`, opt-in (`--ignored`): a real service of this
//!   machine's manager, for a brain named for the test, runs the arm and
//!   is gone, with its log; its guard removes whatever is left however the
//!   test ends. On Windows `real_doctor_removes_a_task` too: `doctor`, run
//!   outside the task, removes a real task and its log.

mod common;

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use common::{Run, Sandbox};
use kluris_core::upgrade::services::{self, Env, Manager, Ran, Runner};
use serde_json::Value;
use tempfile::TempDir;

/// The service's definition file an earlier release wrote for `brain` in
/// the sandbox HOME (none on Windows: a task).
fn definition(sb: &Sandbox, brain: &str) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        Some(
            sb.home
                .join("Library/LaunchAgents")
                .join(format!("dev.ngvoicu.kluris.sync.{brain}.plist")),
        )
    } else if cfg!(windows) {
        None
    } else {
        Some(
            sb.home
                .join(".config/systemd/user")
                .join(format!("kluris-sync-{brain}.service")),
        )
    }
}

/// Its log (macOS; systemd's is the journal, a task's is under
/// LOCALAPPDATA, which the sandbox does not set).
fn log(sb: &Sandbox, brain: &str) -> Option<PathBuf> {
    cfg!(target_os = "macos").then(|| {
        sb.home
            .join("Library/Logs/kluris")
            .join(format!("sync-{brain}.log"))
    })
}

/// The definition and log of `brain`'s service, as an earlier release left
/// them (their bytes do not matter here).
fn install_service(sb: &Sandbox, brain: &str) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = definition(sb, brain)
        .into_iter()
        .chain(log(sb, brain))
        .collect();
    for file in &files {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, "an earlier release's\n").unwrap();
    }
    files
}

/// A `launchctl` and a `systemctl` that append each call, as `<program>
/// <args>` and whether `FAKE_DEFINITION` is there (`(there)` or `(gone)`),
/// to `FAKE_CALLS`, and answer 0, or 1 to the call (`<program> <args>`)
/// `FAKE_FAIL` holds.
#[cfg(unix)]
fn fake_tools(sb: &Sandbox) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = sb.root.join("fake-bin");
    fs::create_dir_all(&bin).unwrap();
    for name in ["launchctl", "systemctl"] {
        let path = bin.join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ -e \"$FAKE_DEFINITION\" ]; then at=there; else at=gone; fi\n\
                 echo \"{name} $* ($at)\" >> \"$FAKE_CALLS\"\n\
                 if [ -n \"$FAKE_FAIL\" ] && [ \"{name} $*\" = \"$FAKE_FAIL\" ]; then\n\
                 echo 'Operation not permitted' >&2; exit 1; fi\nexit 0\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// `kluris args` in the sandbox; on Unix with the fake tools first on PATH,
/// `FAKE_DEFINITION` the definition of `brain`'s service.
fn kluris(sb: &Sandbox, brain: &str, args: &[&str]) -> Run {
    kluris_with(sb, brain, args, &[])
}

/// [`kluris`] with `env` set too.
fn kluris_with(sb: &Sandbox, brain: &str, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = sb.command(args);
    cmd.envs(env.iter().copied());
    #[cfg(unix)]
    {
        let path = format!(
            "{}:{}",
            fake_tools(sb).display(),
            std::env::var("PATH").unwrap()
        );
        cmd.env("PATH", path)
            .env("FAKE_CALLS", calls_file(sb))
            .env("FAKE_DEFINITION", definition(sb, brain).unwrap());
    }
    #[cfg(windows)]
    let _ = brain;
    Run::of(cmd)
}

fn calls_file(sb: &Sandbox) -> PathBuf {
    sb.root.join("tool-calls")
}

/// The fake tools' calls so far.
fn calls(sb: &Sandbox) -> Vec<String> {
    fs::read_to_string(calls_file(sb))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// launchd's `gui/<uid>/<label>` for `brain`, as the arm names it.
#[cfg(target_os = "macos")]
fn launchd_target(brain: &str) -> String {
    format!(
        "gui/{}/dev.ngvoicu.kluris.sync.{brain}",
        rustix::process::getuid().as_raw()
    )
}

/// The calls the arm makes for its own brain's service, the definition
/// already gone at each: launchd's job is booted out after the plist is
/// deleted (the bootout ends the arm itself).
#[cfg(target_os = "macos")]
fn arm_calls(brain: &str) -> Vec<String> {
    vec![
        format!("launchctl print {} (gone)", launchd_target(brain)),
        format!("launchctl bootout {} (gone)", launchd_target(brain)),
    ]
}

/// systemd's unit is disabled before its file goes and is never stopped
/// (that would end the arm before the file is gone).
#[cfg(not(target_os = "macos"))]
fn arm_calls(brain: &str) -> Vec<String> {
    vec![
        format!("systemctl --user disable kluris-sync-{brain}.service (there)"),
        "systemctl --user daemon-reload (gone)".to_string(),
    ]
}

/// The call that stops one service's job: launchd's bootout, systemd's
/// `disable --now`.
#[cfg(target_os = "macos")]
fn stop_call(brain: &str) -> String {
    format!("launchctl bootout {}", launchd_target(brain))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn stop_call(brain: &str) -> String {
    format!("systemctl --user disable --now kluris-sync-{brain}.service")
}

/// The calls `doctor` and `self-uninstall` make for one service: the job is
/// booted out before its plist goes; a running unit is stopped (`--now`).
#[cfg(target_os = "macos")]
fn sweep_calls(brain: &str) -> Vec<String> {
    vec![
        format!("launchctl print {} (there)", launchd_target(brain)),
        format!("{} (there)", stop_call(brain)),
    ]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn sweep_calls(brain: &str) -> Vec<String> {
    vec![
        format!("{} (there)", stop_call(brain)),
        "systemctl --user daemon-reload (gone)".to_string(),
    ]
}

fn json(run: &Run) -> Value {
    serde_json::from_str(run.stdout.trim()).unwrap_or_else(|e| panic!("{e}: {run:?}"))
}

#[test]
fn the_service_flags_are_unknown() {
    let sb = Sandbox::new();
    for flag in ["--install-service", "--uninstall-service"] {
        let run = sb.kluris(&["sync", flag, "--brain", "acme"]);
        assert_eq!(run.code, 2, "{flag}: {run:?}");
        assert!(run.stderr.contains(flag), "{flag}: {run:?}");
    }
}

#[test]
fn sync_help_shows_no_watch_or_service_flag() {
    let sb = Sandbox::new();
    for args in [&["help", "sync"][..], &["sync", "--help"]] {
        let run = sb.kluris(args);
        assert_eq!(run.code, 0, "{args:?}: {run:?}");
        for word in [
            "--watch",
            "--interval",
            "--install-service",
            "--uninstall-service",
            "--log-file",
            "--config",
            "service",
            "watch",
        ] {
            assert!(!run.stdout.contains(word), "{args:?} shows {word}: {run:?}");
        }
    }
}

/// What an earlier release's service runs: on Windows a brain named for
/// this run, whose task is not there (the real `schtasks` is asked).
fn arm_brain() -> String {
    if cfg!(windows) {
        format!("kluris-ut-{}", std::process::id())
    } else {
        "acme".to_string()
    }
}

#[test]
fn the_watch_arm_removes_its_own_service_and_exits_0() {
    let sb = Sandbox::new();
    let brain = arm_brain();
    let files = install_service(&sb, &brain);
    let run = kluris(
        &sb,
        &brain,
        &[
            "sync",
            "--watch",
            "--brain",
            &brain,
            "--interval",
            "30",
            "--log-file",
            "sync.log",
            "--config",
            "config.yml",
        ],
    );
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(run.stdout, "", "{run:?}");
    for file in &files {
        assert!(!file.exists(), "{} is still there", file.display());
    }
    if cfg!(unix) {
        assert_eq!(calls(&sb), arm_calls(&brain));
    }
    assert!(!sb.root.join("work/sync.log").exists());
}

/// The arm reads no config: an earlier release's service for a brain that
/// is not registered (or a config that does not read) still exits 0.
#[test]
fn the_watch_arm_needs_no_config() {
    let sb = Sandbox::new();
    fs::create_dir_all(sb.config.parent().unwrap()).unwrap();
    fs::write(&sb.config, "brains: [not, a, mapping\n").unwrap();
    let brain = arm_brain();
    let files = install_service(&sb, &brain);
    let run = kluris(&sb, &brain, &["sync", "--watch", "--brain", &brain]);
    assert_eq!(run.code, 0, "{run:?}");
    for file in &files {
        assert!(!file.exists(), "{} is still there", file.display());
    }
}

/// A brain name no service could carry reaches no file name and no
/// command.
#[test]
fn the_watch_arm_ignores_a_name_no_service_has() {
    let sb = Sandbox::new();
    let run = kluris(&sb, "acme", &["sync", "--watch", "--brain", "../acme"]);
    assert_eq!(run.code, 0, "{run:?}");
    assert_eq!(calls(&sb), Vec::<String>::new());
}

fn doctor_check(doc: &Value) -> Option<&Value> {
    doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "sync-services")
}

#[cfg(unix)]
#[test]
fn doctor_removes_every_service_and_reports_them_in_one_check() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let mut files = install_service(&sb, "acme");
    files.extend(install_service(&sb, "beta"));
    let run = kluris(&sb, "acme", &["doctor", "--json"]);
    let doc = json(&run);
    let checks: Vec<&Value> = doc["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().contains("sync"))
        .collect();
    assert_eq!(checks.len(), 1, "{doc}");
    let check = checks[0];
    assert_eq!(check["name"], "sync-services", "{check}");
    assert_eq!(check["passed"], true, "{check}");
    let detail = check["detail"].as_str().unwrap();
    for brain in ["acme", "beta"] {
        assert!(
            detail.contains(&format!("sync.{brain}")) || detail.contains(&format!("sync-{brain}")),
            "{detail}"
        );
    }
    for file in &files {
        assert!(!file.exists(), "{} is still there", file.display());
    }
    let mut expected = sweep_calls("acme");
    expected.extend(
        sweep_calls("beta")
            .into_iter()
            .map(|c| c.replace("(there)", "(gone)")),
    );
    assert_eq!(calls(&sb), expected);
}

/// A service that will not go is a warning naming it, in the manager's
/// words, never a pass.
#[cfg(unix)]
#[test]
fn doctor_warns_of_a_service_it_could_not_remove() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    install_service(&sb, "acme");
    let stop = stop_call("acme");
    let run = kluris_with(
        &sb,
        "acme",
        &["doctor", "--json"],
        &[("FAKE_FAIL", stop.as_str())],
    );
    let doc = json(&run);
    let check = doctor_check(&doc).unwrap_or_else(|| panic!("no sync-services check: {doc}"));
    assert_eq!(check["warning"], true, "{check}");
    let detail = check["detail"].as_str().unwrap();
    assert!(
        detail.contains("acme") && detail.contains("Operation not permitted"),
        "{detail}"
    );
}

#[cfg(unix)]
#[test]
fn doctor_no_refresh_leaves_the_services() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let files = install_service(&sb, "acme");
    let doc = json(&kluris(&sb, "acme", &["doctor", "--no-refresh", "--json"]));
    assert!(doctor_check(&doc).is_none(), "{doc}");
    for file in &files {
        assert!(file.exists(), "{} was removed", file.display());
    }
    assert_eq!(calls(&sb), Vec::<String>::new());
}

#[test]
fn doctor_without_services_has_no_services_check() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let doc = json(&kluris(&sb, "acme", &["doctor", "--json"]));
    assert!(doctor_check(&doc).is_none(), "{doc}");
    if cfg!(unix) {
        assert_eq!(calls(&sb), Vec::<String>::new());
    }
}

/// An install with `modify_path` false: the binary and the receipt.
fn installed(sb: &Sandbox) -> (PathBuf, PathBuf) {
    let exe = if cfg!(windows) {
        "kluris.exe"
    } else {
        "kluris"
    };
    let prefix = sb.home.join(".kluris").join("bin");
    let receipt_dir = if cfg!(windows) {
        sb.home.join("lad").join("kluris")
    } else {
        sb.home.join(".config").join("kluris")
    };
    fs::create_dir_all(&prefix).unwrap();
    fs::create_dir_all(&receipt_dir).unwrap();
    fs::write(prefix.join(exe), b"installed binary").unwrap();
    let receipt = receipt_dir.join("kluris-receipt.json");
    fs::write(
        &receipt,
        format!(
            "{{\"binaries\":[\"{exe}\"],\"binary_aliases\":{{}},\"cdylibs\":[],\"cstaticlibs\":[],\
             \"install_layout\":\"flat\",\"install_prefix\":{},\"modify_path\":false,\
             \"provider\":{{\"source\":\"cargo-dist\",\"version\":\"0.33.1\"}},\
             \"source\":{{\"app_name\":\"kluris\",\"name\":\"kluris\",\"owner\":\"ngvoicu\",\
             \"release_type\":\"github\"}},\"version\":\"3.0.0\"}}\n",
            serde_json::to_string(&prefix.display().to_string()).unwrap()
        ),
    )
    .unwrap();
    (prefix.join(exe), receipt)
}

#[cfg(unix)]
#[test]
fn self_uninstall_removes_the_services_and_lists_them_under_removed() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let (binary, receipt) = installed(&sb);
    let files = install_service(&sb, "acme");
    let run = kluris(&sb, "acme", &["self-uninstall", "--json"]);
    assert_eq!(run.code, 0, "{run:?}");
    let doc = json(&run);
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert!(!keys.iter().any(|k| k.contains("service")), "{keys:?}");
    let removed: Vec<&str> = doc["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    for file in files.iter().chain([&binary, &receipt]) {
        assert!(
            removed.contains(&file.to_str().unwrap()),
            "{} not in {removed:?}",
            file.display()
        );
        assert!(!file.exists(), "{} is still there", file.display());
    }
    // The services go before the binary.
    let first = |p: &Path| removed.iter().position(|r| *r == p.to_str().unwrap());
    assert!(first(&files[0]) < first(&binary), "{removed:?}");
    assert_eq!(calls(&sb), sweep_calls("acme"));
}

/// A definition folder that cannot lose an entry blocks the uninstall
/// before anything changes.
#[cfg(unix)]
#[test]
fn self_uninstall_a_service_that_cannot_be_removed_blocks_before_any_change() {
    use std::os::unix::fs::PermissionsExt;
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let (binary, receipt) = installed(&sb);
    let files = install_service(&sb, "acme");
    let folder = files[0].parent().unwrap().to_path_buf();
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
    let run = kluris(&sb, "acme", &["self-uninstall", "--json"]);
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(run.code, 1, "{run:?}");
    let doc = json(&run);
    let blocked = doc["blocked"].as_array().unwrap();
    assert!(
        blocked
            .iter()
            .any(|b| b["path"] == folder.to_str().unwrap()),
        "{doc}"
    );
    for file in files.iter().chain([&binary, &receipt]) {
        assert!(file.exists(), "{} was removed", file.display());
    }
    assert_eq!(calls(&sb), Vec::<String>::new());
}

/// A service that will not stop stops the uninstall with the install
/// whole; once it stops, a rerun finishes.
#[cfg(unix)]
#[test]
fn self_uninstall_a_service_that_will_not_stop_keeps_the_install_and_a_rerun_finishes() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let (binary, receipt) = installed(&sb);
    let files = install_service(&sb, "acme");
    let stop = stop_call("acme");
    let run = kluris_with(
        &sb,
        "acme",
        &["self-uninstall", "--json"],
        &[("FAKE_FAIL", stop.as_str())],
    );
    assert_eq!(run.code, 1, "{run:?}");
    let doc = json(&run);
    let failed = doc["failed"].to_string();
    assert!(
        failed.contains("acme") && failed.contains("Operation not permitted"),
        "{doc}"
    );
    assert!(binary.exists() && receipt.exists());
    // launchd's plist stays while its job will not go.
    if cfg!(target_os = "macos") {
        assert!(files[0].exists());
    }
    let run = kluris(&sb, "acme", &["self-uninstall", "--json"]);
    assert_eq!(run.code, 0, "{run:?}");
    for file in files.iter().chain([&binary, &receipt]) {
        assert!(!file.exists(), "{} is still there", file.display());
    }
}

#[test]
fn self_uninstall_json_has_no_services_key() {
    let sb = Sandbox::new();
    sb.write_config(&[]);
    installed(&sb);
    let mut cmd = sb.command(&["self-uninstall", "--json"]);
    cmd.env("LOCALAPPDATA", sb.home.join("lad"));
    let doc = json(&Run::of(cmd));
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "ok",
            "removed",
            "edited",
            "kept",
            "path_entries",
            "skills",
            "purged",
            "failed",
            "blocked",
            "left"
        ]
    );
}

#[test]
fn remove_json_has_no_service_keys_and_runs_no_service_command() {
    let sb = Sandbox::new();
    let brain = sb.work.join("acme");
    fs::create_dir_all(&brain).unwrap();
    sb.write_config(&[("acme", &brain, "the acme brain")]);
    install_service(&sb, "acme");
    let run = kluris(&sb, "acme", &["remove", "acme", "--json"]);
    assert_eq!(run.code, 0, "{run:?}");
    let doc = json(&run);
    let keys: Vec<&String> = doc.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["ok", "name"], "{doc}");
    assert_eq!(calls(&sb), Vec::<String>::new());

    sb.write_config(&[("acme", &brain, "the acme brain")]);
    let text = kluris(&sb, "acme", &["remove", "acme"]);
    assert_eq!(text.code, 0, "{text:?}");
    assert_eq!(text.stdout, "Unregistered: acme (files preserved)\n");
    assert_eq!(calls(&sb), Vec::<String>::new());
}

// Each manager's steps for one brain, through a fake runner.

/// Records every command as `program args (there|gone)`: whether the
/// watched file was there when it ran. Answers by exact command line, the
/// latest answer first (exit 0 and no output otherwise).
struct FakeRunner {
    watched: PathBuf,
    calls: RefCell<Vec<String>>,
    answers: RefCell<Vec<(String, Ran)>>,
}

impl FakeRunner {
    fn watching(watched: &Path) -> FakeRunner {
        FakeRunner {
            watched: watched.to_path_buf(),
            calls: RefCell::default(),
            answers: RefCell::default(),
        }
    }

    fn answer(&self, line: &str, code: i32, stderr: &str) {
        self.answers.borrow_mut().push((
            line.to_string(),
            Ran {
                code: Some(code),
                stdout: String::new(),
                stderr: stderr.into(),
            },
        ));
    }

    /// Answer `line` with exit 0 and `stdout`.
    fn answer_out(&self, line: &str, stdout: &str) {
        self.answers.borrow_mut().push((
            line.to_string(),
            Ran {
                code: Some(0),
                stdout: stdout.into(),
                stderr: String::new(),
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
        let at = if self.watched.exists() {
            "there"
        } else {
            "gone"
        };
        self.calls.borrow_mut().push(format!("{line} ({at})"));
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

const PLIST: &str = "Library/LaunchAgents/dev.ngvoicu.kluris.sync.acme.plist";
const TARGET: &str = "gui/501/dev.ngvoicu.kluris.sync.acme";

#[test]
fn launchd_the_plist_and_log_go_then_the_job_is_booted_out() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let plist = touch(&home.join(PLIST));
    let log = touch(&home.join("Library/Logs/kluris/sync-acme.log"));
    let runner = FakeRunner::watching(&plist);
    let done = services::remove_one("acme", &env(home, Manager::Launchd, &runner));
    assert_eq!(
        runner.calls(),
        [
            format!("launchctl print {TARGET} (gone)"),
            format!("launchctl bootout {TARGET} (gone)"),
        ]
    );
    assert_eq!(done.name, "dev.ngvoicu.kluris.sync.acme");
    assert_eq!(done.removed, [plist, log]);
    assert_eq!(done.problem, None);
    assert!(!home.join("Library/Logs/kluris").exists());
}

#[test]
fn launchd_a_job_that_is_not_loaded_is_not_booted_out() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let plist = touch(&home.join(PLIST));
    let runner = FakeRunner::watching(&plist);
    runner.answer(
        &format!("launchctl print {TARGET}"),
        113,
        "Could not find service",
    );
    let done = services::remove_one("acme", &env(home, Manager::Launchd, &runner));
    assert_eq!(runner.calls(), [format!("launchctl print {TARGET} (gone)")]);
    assert_eq!(done.removed, [plist]);
    assert_eq!(done.problem, None);
}

#[test]
fn systemd_the_unit_is_disabled_deleted_and_reloaded_never_stopped() {
    for xdg in [false, true] {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let config = if xdg {
            tmp.path().join("xdg")
        } else {
            home.join(".config")
        };
        let unit = touch(&config.join("systemd/user/kluris-sync-acme.service"));
        let runner = FakeRunner::watching(&unit);
        let mut env = env(&home, Manager::Systemd, &runner);
        if xdg {
            env.xdg_config_home = Some(config.clone());
        }
        let done = services::remove_one("acme", &env);
        assert_eq!(
            runner.calls(),
            [
                "systemctl --user disable kluris-sync-acme.service (there)",
                "systemctl --user daemon-reload (gone)",
            ],
            "XDG_CONFIG_HOME {xdg}"
        );
        assert_eq!(done.removed, [unit]);
        assert_eq!(done.problem, None);
    }
}

#[test]
fn systemd_without_a_unit_file_runs_nothing() {
    let tmp = TempDir::new().unwrap();
    let runner = FakeRunner::watching(&tmp.path().join("none"));
    let done = services::remove_one("acme", &env(tmp.path(), Manager::Systemd, &runner));
    assert_eq!(runner.calls(), Vec::<String>::new());
    assert!(
        done.removed.is_empty() && done.problem.is_none(),
        "{done:?}"
    );
}

/// No systemd user instance to disable it: the unit file goes all the same,
/// with the link `enable` made, and the failure is reported.
#[test]
fn systemd_a_disable_that_fails_still_deletes_the_unit() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let units = home.join(".config/systemd/user");
    let unit = touch(&units.join("kluris-sync-acme.service"));
    let wants = units.join("default.target.wants/kluris-sync-acme.service");
    #[cfg(unix)]
    {
        fs::create_dir_all(wants.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&unit, &wants).unwrap();
    }
    let runner = FakeRunner::watching(&unit);
    runner.answer(
        "systemctl --user disable kluris-sync-acme.service",
        1,
        "Failed to connect to bus: No medium found",
    );
    let done = services::remove_one("acme", &env(home, Manager::Systemd, &runner));
    assert_eq!(
        runner.calls(),
        ["systemctl --user disable kluris-sync-acme.service (there)"]
    );
    assert_eq!(done.removed, std::slice::from_ref(&unit));
    let problem = done.problem.unwrap_or_default();
    assert!(problem.contains("No medium found"), "{problem}");
    assert!(!unit.exists());
    assert!(fs::symlink_metadata(&wants).is_err(), "the link is left");
}

const TASK: &str = r"\Kluris\sync-acme";

#[test]
fn task_scheduler_the_task_is_deleted_never_ended_then_its_log() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let log = touch(&home.join("lad/kluris/logs/sync-acme.log"));
    let runner = FakeRunner::watching(&log);
    let done = services::remove_one("acme", &env(home, Manager::TaskScheduler, &runner));
    assert_eq!(
        runner.calls(),
        [
            format!("schtasks /Query /TN {TASK} (there)"),
            format!("schtasks /Delete /TN {TASK} /F (there)"),
        ]
    );
    assert_eq!(done.removed, [PathBuf::from(TASK), log.clone()]);
    assert_eq!(done.problem, None);
    assert!(!log.exists());
    assert!(!home.join("lad/kluris/logs").exists());
}

#[test]
fn task_scheduler_without_the_task_only_its_log_goes() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let log = touch(&home.join("lad/kluris/logs/sync-acme.log"));
    let runner = FakeRunner::watching(&log);
    runner.answer(
        &format!("schtasks /Query /TN {TASK}"),
        1,
        "ERROR: The system cannot find the file specified.",
    );
    let done = services::remove_one("acme", &env(home, Manager::TaskScheduler, &runner));
    assert_eq!(
        runner.calls(),
        [format!("schtasks /Query /TN {TASK} (there)")]
    );
    assert_eq!(done.removed, [log]);
    assert_eq!(done.problem, None);
}

#[test]
fn task_scheduler_a_delete_that_fails_is_reported() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let runner = FakeRunner::watching(&home.join("none"));
    runner.answer(
        &format!("schtasks /Delete /TN {TASK} /F"),
        1,
        "ERROR: Access is denied.",
    );
    let done = services::remove_one("acme", &env(home, Manager::TaskScheduler, &runner));
    assert!(done.removed.is_empty(), "{done:?}");
    let problem = done.problem.unwrap_or_default();
    assert!(problem.contains("Access is denied"), "{problem}");
}

/// A task that will not go keeps its log, which gains one line saying why:
/// inside the task nobody reads stderr, and the next sweep removes both.
#[test]
fn task_scheduler_a_task_that_stays_keeps_its_log_with_the_failure() {
    for stop in [false, true] {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path();
        let log = touch(&home.join("lad/kluris/logs/sync-acme.log"));
        let runner = FakeRunner::watching(&log);
        runner.answer(
            &format!("schtasks /Delete /TN {TASK} /F"),
            1,
            "ERROR: Access is denied.\r\r\n",
        );
        let env = env(home, Manager::TaskScheduler, &runner);
        let done = if stop {
            let tasks = format!("\"{TASK}\",\"N/A\",\"Running\"\r\n");
            runner.answer_out("schtasks /Query /FO CSV /NH", &tasks);
            services::sweep(&env).remove(0)
        } else {
            services::remove_one("acme", &env)
        };
        assert!(done.removed.is_empty(), "stop {stop}: {done:?}");
        let problem = done.problem.clone().unwrap_or_default();
        assert_eq!(
            problem,
            format!("`schtasks /Delete /TN {TASK} /F`: ERROR: Access is denied."),
            "stop {stop}"
        );
        assert_eq!(
            fs::read_to_string(&log).unwrap(),
            format!(
                "an earlier release's\nkluris: could not remove the task {TASK}, \
                 which keeps this log: {problem}\n"
            ),
            "stop {stop}"
        );
    }
}

/// Without its log, a task that stays leaves no file behind.
#[test]
fn task_scheduler_a_task_that_stays_without_a_log_writes_none() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path();
    let log = home.join("lad/kluris/logs/sync-acme.log");
    let runner = FakeRunner::watching(&log);
    runner.answer(
        &format!("schtasks /Delete /TN {TASK} /F"),
        1,
        "ERROR: Access is denied.",
    );
    let done = services::remove_one("acme", &env(home, Manager::TaskScheduler, &runner));
    assert!(done.problem.is_some(), "{done:?}");
    assert!(!home.join("lad/kluris").exists());
}

#[test]
fn a_name_no_service_has_reaches_no_command() {
    let tmp = TempDir::new().unwrap();
    for manager in [Manager::Launchd, Manager::Systemd, Manager::TaskScheduler] {
        for brain in ["../acme", "Acme", "", "a/b"] {
            let runner = FakeRunner::watching(&tmp.path().join("none"));
            let done = services::remove_one(brain, &env(tmp.path(), manager, &runner));
            assert_eq!(
                runner.calls(),
                Vec::<String>::new(),
                "{manager:?} {brain:?}"
            );
            assert!(done.removed.is_empty(), "{done:?}");
        }
    }
}

// The opt-in real round trip.

/// A command's exit code (-1 when it has none or could not start), stdout
/// and stderr. It never panics, so a guard can run it while unwinding.
fn ran(program: &str, args: &[&str]) -> (i32, String, String) {
    match StdCommand::new(program).args(args).output() {
        Ok(out) => (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ),
        Err(e) => (-1, String::new(), e.to_string()),
    }
}

/// A real service of this machine's manager, in the shape an earlier
/// release installed, for a brain named for this run alone. Dropped, so on
/// every unwind path too, it removes whatever of it is left (that brain's
/// definition, job, unit or task, and log, never another's) and says what
/// it could not. No guard runs when the test process itself is killed:
/// what is left is then named `kluris-rt-*`, removed by hand.
struct RealService {
    brain: String,
    /// The definition file (none for a task).
    definition: Option<PathBuf>,
    log: Option<PathBuf>,
}

/// The services run the very binary under test.
fn kluris_exe() -> PathBuf {
    fs::canonicalize(assert_cmd::cargo::cargo_bin!("kluris")).unwrap()
}

fn round_trip_brain() -> String {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("kluris-rt-{}-{nonce}", std::process::id())
}

#[cfg(target_os = "macos")]
impl RealService {
    fn label(&self) -> String {
        format!("dev.ngvoicu.kluris.sync.{}", self.brain)
    }

    fn target(&self) -> String {
        format!(
            "gui/{}/{}",
            rustix::process::getuid().as_raw(),
            self.label()
        )
    }

    /// The launchd agent, bootstrapped in `gui/<uid>`: launchd starts it at
    /// once (`RunAtLoad`) and restarts it after a failing exit.
    fn install(brain: &str) -> Option<RealService> {
        let home = std::env::home_dir().unwrap();
        let mut service = RealService {
            brain: brain.to_string(),
            definition: None,
            log: None,
        };
        let plist = home
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", service.label()));
        let log = home
            .join("Library/Logs/kluris")
            .join(format!("sync-{brain}.log"));
        service.definition = Some(plist.clone());
        service.log = Some(log.clone());
        fs::create_dir_all(plist.parent().unwrap()).unwrap();
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        fs::write(
            &plist,
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\">\n<dict>\n\
                 \t<key>Label</key>\n\t<string>{label}</string>\n\
                 \t<key>ProgramArguments</key>\n\t<array>\n\
                 \t\t<string>{exe}</string>\n\t\t<string>sync</string>\n\t\t<string>--watch</string>\n\
                 \t\t<string>--brain</string>\n\t\t<string>{brain}</string>\n\t</array>\n\
                 \t<key>EnvironmentVariables</key>\n\t<dict>\n\t\t<key>PATH</key>\n\t\t<string>{path}</string>\n\t</dict>\n\
                 \t<key>RunAtLoad</key>\n\t<true/>\n\
                 \t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\
                 \t<key>ThrottleInterval</key>\n\t<integer>30</integer>\n\
                 \t<key>ExitTimeOut</key>\n\t<integer>300</integer>\n\
                 \t<key>ProcessType</key>\n\t<string>Background</string>\n\
                 \t<key>StandardOutPath</key>\n\t<string>{log}</string>\n\
                 \t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n\
                 </dict>\n</plist>\n",
                label = service.label(),
                exe = kluris_exe().display(),
                path = path.replace('&', "&amp;").replace('<', "&lt;"),
                log = log.display(),
            ),
        )
        .unwrap();
        let domain = format!("gui/{}", rustix::process::getuid().as_raw());
        let plist_text = plist.display().to_string();
        let (code, out, err) = ran("launchctl", &["bootstrap", &domain, &plist_text]);
        assert_eq!(code, 0, "launchctl bootstrap: {out}{err}");
        Some(service)
    }

    /// What of it is there: the plist, the loaded job, the log.
    fn left(&self) -> Vec<String> {
        let mut left = self.files_left();
        if ran("launchctl", &["print", &self.target()]).0 == 0 {
            left.push(format!("the job {}", self.target()));
        }
        left
    }

    /// What launchd says of the job.
    fn report(&self) -> String {
        let (_, out, err) = ran("launchctl", &["print", &self.target()]);
        format!("launchctl print: {out}{err}")
    }

    fn clean(&self) {
        let _ = ran("launchctl", &["bootout", &self.target()]);
        self.remove_files();
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl RealService {
    fn unit(&self) -> String {
        format!("kluris-sync-{}.service", self.brain)
    }

    /// The unit folder of the systemd user manager: its own
    /// XDG_CONFIG_HOME or HOME, which the unit's process inherits.
    fn unit_dir(manager_env: &str) -> PathBuf {
        let var = |key: &str| {
            manager_env
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{key}=")))
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".config")))
            .unwrap_or_else(|| std::env::home_dir().unwrap().join(".config"))
            .join("systemd/user")
    }

    /// The unit, enabled and started; none without a systemd user
    /// instance to run it.
    fn install(brain: &str) -> Option<RealService> {
        let (code, manager_env, err) = ran("systemctl", &["--user", "show-environment"]);
        if code != 0 {
            eprintln!("skipped: `systemctl --user` does not answer here: {err}");
            return None;
        }
        let mut service = RealService {
            brain: brain.to_string(),
            definition: None,
            log: None,
        };
        let unit = Self::unit_dir(&manager_env).join(service.unit());
        service.definition = Some(unit.clone());
        fs::create_dir_all(unit.parent().unwrap()).unwrap();
        let path = std::env::var("PATH").unwrap_or_default();
        fs::write(
            &unit,
            format!(
                "[Unit]\nDescription=Kluris sync for the brain {brain}\n\n[Service]\nType=simple\n\
                 ExecStart=\"{exe}\" sync --watch --brain {brain}\n\
                 Environment=\"PATH={path}\"\n\
                 Restart=on-failure\nRestartSec=30\nTimeoutStopSec=300\n\n\
                 [Install]\nWantedBy=default.target\n",
                exe = kluris_exe().display(),
            ),
        )
        .unwrap();
        let unit_name = service.unit();
        for args in [
            &["--user", "daemon-reload"][..],
            &["--user", "enable", "--now", &unit_name],
        ] {
            let (code, out, err) = ran("systemctl", args);
            assert_eq!(code, 0, "systemctl {args:?}: {out}{err}");
        }
        Some(service)
    }

    /// What of it is there: the unit file, its `enable` link, the unit
    /// loaded or running.
    fn left(&self) -> Vec<String> {
        let mut left = self.files_left();
        let unit = self.unit();
        if let Some(wants) = self.wants()
            && fs::symlink_metadata(&wants).is_ok()
        {
            left.push(format!("the link {}", wants.display()));
        }
        let (_, out, _) = ran(
            "systemctl",
            &[
                "--user",
                "show",
                "-p",
                "LoadState",
                "-p",
                "ActiveState",
                &unit,
            ],
        );
        if !out.contains("LoadState=not-found") || out.contains("ActiveState=active") {
            left.push(format!(
                "the unit {unit}: {}",
                out.trim().replace('\n', ", ")
            ));
        }
        left
    }

    fn wants(&self) -> Option<PathBuf> {
        let dir = self.definition.as_ref()?.parent()?;
        Some(dir.join("default.target.wants").join(self.unit()))
    }

    /// What systemd says of the unit.
    fn report(&self) -> String {
        let (_, out, err) = ran(
            "systemctl",
            &["--user", "status", "--no-pager", &self.unit()],
        );
        format!("systemctl --user status: {out}{err}")
    }

    fn clean(&self) {
        let unit = self.unit();
        let _ = ran("systemctl", &["--user", "disable", "--now", &unit]);
        self.remove_files();
        if let Some(wants) = self.wants() {
            let _ = fs::remove_file(wants);
        }
        let _ = ran("systemctl", &["--user", "daemon-reload"]);
        let _ = ran("systemctl", &["--user", "reset-failed", &unit]);
    }
}

/// Whether this process runs elevated: its token carries the High (or
/// System) mandatory label, as an administrator's ssh session does.
#[cfg(windows)]
fn elevated() -> bool {
    let (_, out, _) = ran("whoami", &["/groups", "/fo", "csv", "/nh"]);
    out.contains("S-1-16-12288") || out.contains("S-1-16-16384")
}

/// `text` in an XML element's text.
#[cfg(windows)]
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The account's SID, as `whoami /user` says it.
#[cfg(windows)]
fn sid() -> String {
    let (code, out, err) = ran("whoami", &["/user", "/fo", "csv", "/nh"]);
    assert_eq!(code, 0, "whoami: {out}{err}");
    out.lines()
        .next()
        .and_then(|l| l.rsplit(',').next())
        .map(|s| s.trim().trim_matches('"').to_string())
        .expect("a SID")
}

/// The task XML an earlier release registered: at this account's logon,
/// with its interactive token at its least privilege, `command
/// arguments`, as UTF-16 with its BOM.
#[cfg(windows)]
fn task_xml(task: &str, description: &str, command: &str, arguments: &str) -> Vec<u8> {
    let sid = sid();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>{description}</Description>
    <URI>{task}</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{sid}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
    </Exec>
  </Actions>
</Task>
"#,
        description = xml_text(description),
        task = xml_text(task),
        command = xml_text(command),
        arguments = xml_text(arguments),
    );
    let mut bytes = vec![0xFF, 0xFE];
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// A folder under LOCALAPPDATA, which the person's own token can write.
#[cfg(windows)]
fn person_dir() -> TempDir {
    let lad = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"));
    tempfile::Builder::new()
        .prefix("kluris-rt-")
        .tempdir_in(lad)
        .unwrap()
}

/// The helper task that runs a script as the person, removed when dropped.
#[cfg(windows)]
struct Helper(String);

#[cfg(windows)]
impl Drop for Helper {
    fn drop(&mut self) {
        let _ = ran("schtasks", &["/Delete", "/TN", &self.0, "/F"]);
        if ran("schtasks", &["/Query", "/TN", &self.0]).0 == 0 {
            eprintln!("the guard could not remove the helper task {}", self.0);
        } else {
            eprintln!("the helper task {} is gone", self.0);
        }
    }
}

/// `command` (a `cmd` line) with `vars` set, as the person's own terminal
/// runs it: its exit code, stdout and stderr. An elevated process (an
/// administrator's ssh session) has a token that terminal never has, so it
/// runs the script in a helper task, `\kluris-rt-person-<brain>`, with the
/// person's interactive token at its least privilege: the terminal's
/// token (Medium, Administrators for deny only), in their logon session.
/// `None`, said on stderr, when that helper never ran (nobody logged on).
#[cfg(windows)]
fn as_person(brain: &str, vars: &[(&str, String)], command: &str) -> Option<(i32, String, String)> {
    let dir = person_dir();
    let (out, err, code) = (
        dir.path().join("out"),
        dir.path().join("err"),
        dir.path().join("code"),
    );
    let mut script = String::from("@echo off\r\n");
    for (key, value) in vars {
        script.push_str(&format!("set \"{key}={value}\"\r\n"));
    }
    script.push_str(&format!(
        "{command} > \"{}\" 2> \"{}\"\r\n> \"{}\" echo %ERRORLEVEL%\r\n",
        out.display(),
        err.display(),
        code.display()
    ));
    let script_file = dir.path().join("run.cmd");
    fs::write(&script_file, script).unwrap();
    let _helper = if elevated() {
        let name = format!(r"\kluris-rt-person-{brain}");
        let xml = dir.path().join("helper.xml");
        fs::write(
            &xml,
            task_xml(
                &name,
                "Runs a Kluris test's script as the person",
                r"%SystemRoot%\System32\cmd.exe",
                &format!("/c \"{}\"", script_file.display()),
            ),
        )
        .unwrap();
        let xml_text = xml.display().to_string();
        let created = ran(
            "schtasks",
            &["/Create", "/TN", &name, "/XML", &xml_text, "/F"],
        );
        assert_eq!(created.0, 0, "schtasks /Create: {}{}", created.1, created.2);
        let helper = Helper(name);
        let started = ran("schtasks", &["/Run", "/TN", &helper.0]);
        if started.0 != 0 {
            eprintln!(
                "skipped: the helper task that runs as the person did not start: {}{}",
                started.1, started.2
            );
            return None;
        }
        Some(helper)
    } else {
        let (code, out, err) = ran("cmd", &["/c", &script_file.display().to_string()]);
        assert_eq!(code, 0, "{}: {out}{err}", script_file.display());
        None
    };
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = fs::read_to_string(&code)
            .ok()
            .and_then(|c| c.trim().parse::<i32>().ok())
        {
            let read = |p: &Path| fs::read_to_string(p).unwrap_or_default();
            return Some((status, read(&out), read(&err)));
        }
        if Instant::now() > until {
            eprintln!(
                "skipped: the helper task that runs as the person did not finish in 120 s \
                 (is the person logged on?)"
            );
            return None;
        }
        thread::sleep(Duration::from_millis(250));
    }
}

/// Every task named as a sync service's, `\Kluris\sync-*`, on this
/// machine; `None` when `schtasks` cannot list them.
#[cfg(windows)]
fn sync_tasks() -> Option<Vec<String>> {
    let (code, out, _) = ran("schtasks", &["/Query", "/FO", "CSV", "/NH"]);
    (code == 0).then(|| {
        let mut tasks: Vec<String> = out
            .lines()
            .filter_map(|row| row.split(',').next())
            .map(|name| name.trim().trim_matches('"').to_string())
            .filter(|name| name.starts_with(r"\Kluris\sync-"))
            .collect();
        tasks.sort();
        tasks.dedup();
        tasks
    })
}

#[cfg(windows)]
impl RealService {
    fn task(&self) -> String {
        format!(r"\Kluris\sync-{}", self.brain)
    }

    /// The task, created from XML as the person's own terminal created it
    /// ([`as_person`]), with the log its watch wrote; none when it could
    /// not be created as the person.
    fn create(brain: &str) -> Option<RealService> {
        let lad = PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA"));
        let log = lad
            .join("kluris")
            .join("logs")
            .join(format!("sync-{brain}.log"));
        let service = RealService {
            brain: brain.to_string(),
            definition: None,
            log: Some(log.clone()),
        };
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(&log, "an earlier release's watch wrote this\r\n").unwrap();
        let arguments = format!(
            "--headless \"{}\" sync --watch --brain {brain} --log-file \"{}\"",
            kluris_exe().display(),
            log.display()
        );
        let task = service.task();
        let dir = person_dir();
        let xml = dir.path().join("task.xml");
        fs::write(
            &xml,
            task_xml(
                &task,
                &format!("Kluris sync for the brain {brain}"),
                r"%SystemRoot%\System32\conhost.exe",
                &arguments,
            ),
        )
        .unwrap();
        let command = format!("schtasks /Create /TN {task} /XML \"{}\" /F", xml.display());
        let (code, out, err) = as_person(brain, &[], &command)?;
        assert_eq!(code, 0, "{command}: {out}{err}");
        Some(service)
    }

    /// [`RealService::create`], then run at once.
    fn install(brain: &str) -> Option<RealService> {
        let service = Self::create(brain)?;
        let (code, out, err) = ran("schtasks", &["/Run", "/TN", &service.task()]);
        assert_eq!(code, 0, "schtasks /Run: {out}{err}");
        Some(service)
    }

    /// What of it is there: the task, the log.
    fn left(&self) -> Vec<String> {
        let mut left = self.files_left();
        if ran("schtasks", &["/Query", "/TN", &self.task()]).0 == 0 {
            left.push(format!("the task {}", self.task()));
        }
        left
    }

    /// What the task's manager and log say, and this process's token.
    fn report(&self) -> String {
        let (_, task, task_err) = ran(
            "schtasks",
            &["/Query", "/TN", &self.task(), "/V", "/FO", "LIST"],
        );
        let task: Vec<&str> = task
            .lines()
            .filter(|l| {
                ["Status:", "Last Result:", "Run As User:", "Logon Mode:"]
                    .iter()
                    .any(|k| l.starts_with(k))
            })
            .collect();
        let log = self
            .log
            .as_ref()
            .and_then(|l| fs::read_to_string(l).ok())
            .unwrap_or_else(|| "(no log)".to_string());
        format!(
            "the task: {task:?}{}\nits log: {log}\nthis test elevated: {}",
            task_err.trim(),
            elevated()
        )
    }

    fn clean(&self) {
        let task = self.task();
        let _ = ran("schtasks", &["/End", "/TN", &task]);
        let _ = ran("schtasks", &["/Delete", "/TN", &task, "/F"]);
        self.remove_files();
    }
}

impl RealService {
    fn files_left(&self) -> Vec<String> {
        self.definition
            .iter()
            .chain(&self.log)
            .filter(|p| fs::symlink_metadata(p).is_ok())
            .map(|p| p.display().to_string())
            .collect()
    }

    /// The definition and the log, then the log's folder once empty.
    fn remove_files(&self) {
        for file in self.definition.iter().chain(&self.log) {
            let _ = fs::remove_file(file);
        }
        if let Some(dir) = self.log.as_ref().and_then(|l| l.parent()) {
            let _ = fs::remove_dir(dir);
        }
    }
}

impl Drop for RealService {
    fn drop(&mut self) {
        if self.left().is_empty() {
            eprintln!("the guard found nothing of {}'s service left", self.brain);
            return;
        }
        self.clean();
        let left = self.left();
        if left.is_empty() {
            eprintln!(
                "the guard removed what was left of {}'s service",
                self.brain
            );
        } else {
            eprintln!(
                "the guard could not remove what is left of {}'s service: {left:?}",
                self.brain
            );
        }
    }
}

/// The real tests one at a time: `doctor` removes every sync service it
/// finds, so it must not meet another test's.
static REAL: Mutex<()> = Mutex::new(());

fn one_real_test() -> MutexGuard<'static, ()> {
    REAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A real service in an earlier release's shape, for a brain named for
/// this run, starts this kluris with `sync --watch`; within 90 s its
/// definition, its job (unit, task) and its log are gone, removed by the
/// arm from inside the service. On Windows the task is created as the
/// person's own terminal creates it ([`as_person`]): a task an elevated
/// session created is one its own least-privilege token may not delete.
/// On a Linux without a systemd user instance, or a Windows nobody is
/// logged on to, it is skipped, and says so.
/// `cargo test -p kluris --test upgrade_services -- --ignored`.
#[test]
#[ignore = "installs a real per-user service on this machine"]
fn real_round_trip() {
    let _one = one_real_test();
    let brain = round_trip_brain();
    let Some(service) = RealService::install(&brain) else {
        return;
    };
    let until = Instant::now() + Duration::from_secs(90);
    let mut left = service.left();
    while !left.is_empty() && Instant::now() < until {
        thread::sleep(Duration::from_millis(250));
        left = service.left();
    }
    assert!(
        left.is_empty(),
        "after 90 s, still there: {left:?}\n{}",
        service.report()
    );
    eprintln!("the service of {brain} removed itself: its definition, job and log are gone");
}

/// The path a Windows user takes: `kluris doctor`, run outside the task
/// as the person runs it ([`as_person`], a sandbox HOME and config, the
/// real LOCALAPPDATA), removes a real task in an earlier release's shape,
/// and its log, and says so in a passing `sync-services` check. `doctor`
/// removes every `\Kluris\sync-*` task, so while any other is on the
/// machine the test touches nothing and says why.
#[cfg(windows)]
#[test]
#[ignore = "installs a real per-user service on this machine"]
fn real_doctor_removes_a_task() {
    let _one = one_real_test();
    let Some(others) = sync_tasks() else {
        eprintln!("skipped: `schtasks /Query` cannot list this machine's tasks");
        return;
    };
    if !others.is_empty() {
        eprintln!(
            "skipped: `kluris doctor` would remove these sync tasks too, which are not \
             this test's: {others:?}"
        );
        return;
    }
    let brain = round_trip_brain();
    let Some(service) = RealService::create(&brain) else {
        return;
    };
    let task = service.task();
    assert_eq!(sync_tasks(), Some(vec![task.clone()]));
    let sb = Sandbox::new();
    sb.write_config(&[]);
    let tmp = sb.root.join("tmp");
    let vars = [
        ("HOME", sb.home.display().to_string()),
        ("USERPROFILE", sb.home.display().to_string()),
        ("KLURIS_CONFIG", sb.config.display().to_string()),
        ("GIT_CONFIG_GLOBAL", sb.gitconfig().display().to_string()),
        ("GIT_CONFIG_NOSYSTEM", "1".to_string()),
        ("KLURIS_NO_PROMPT", "1".to_string()),
        ("COPILOT_HOME", String::new()),
        ("XDG_CONFIG_HOME", String::new()),
        ("TEMP", tmp.display().to_string()),
        ("TMP", tmp.display().to_string()),
    ];
    // `cmd` starts no `\\?\` path, which `kluris_exe` is on Windows.
    let exe = assert_cmd::cargo::cargo_bin!("kluris");
    let command = format!("\"{}\" doctor --json", exe.display());
    let Some((code, stdout, stderr)) = as_person(&brain, &vars, &command) else {
        return;
    };
    let doc: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("{e}: exit {code}\n{stdout}\n{stderr}"));
    let check = doctor_check(&doc).unwrap_or_else(|| panic!("no sync-services check: {doc}"));
    assert_eq!(check["passed"], true, "{check}\n{}", service.report());
    assert!(
        check.get("warning").is_none(),
        "{check}\n{}",
        service.report()
    );
    let detail = check["detail"].as_str().unwrap();
    assert!(detail.contains(&task), "{detail}");
    let left = service.left();
    assert!(
        left.is_empty(),
        "after doctor, still there: {left:?}\n{}",
        service.report()
    );
    eprintln!("kluris doctor removed {task} and its log");
}
