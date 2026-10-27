//! Unit tests of `cargo xtask linux`: the words, the bash zeelin is sent (read
//! as text, checked by `bash -n`, and on Unix run against a home made for the
//! case, with `flock` and `timeout` stood in for), and a whole run against a
//! fake ssh. Nothing here reaches the network.

use std::fs;
use std::path::Path;
use std::time::Duration;

use super::options::{self, Options, Parsed};
use super::run::{self, Ctx};
use super::script::{self, Layout, Spec};
use crate::ssh_ci::fake::{Answer, Clock, Fake, NOW, said, tickets, timed_out};
use crate::ssh_ci::queue::{Queues, Who};
use crate::ssh_ci::runner::{Captured, Stream, Streamed};

const ID: &str = "kluris-0000abcd-12345678";

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

fn spec<'a>(parity: bool, parity_files: &'a [String], cargo_args: &'a [String]) -> Spec<'a> {
    Spec {
        channel: "1.99.0",
        jobs: 8,
        tasks: script::TASK_LIMIT,
        parity,
        parity_files,
        cargo_args,
        installer_smoke: false,
        wrapper: false,
        clippy: false,
        published: None,
        unpinned: false,
    }
}

/// An installer smoke run, with or without the site wrapper sent.
fn smoke_spec(wrapper: bool) -> Spec<'static> {
    Spec {
        installer_smoke: true,
        wrapper,
        ..spec(false, &[], &[])
    }
}

/// A published-release smoke run: the live site wrapper installing `version`.
fn published_spec(version: &'static str) -> Spec<'static> {
    Spec {
        installer_smoke: true,
        published: Some(version),
        ..spec(false, &[], &[])
    }
}

/// The same without `KLURIS_VERSION`: the wrapper installs GitHub's latest,
/// which must be `version`.
fn unpinned_spec(version: &'static str) -> Spec<'static> {
    Spec {
        unpinned: true,
        ..published_spec(version)
    }
}

// --- the words --------------------------------------------------------------

mod words_of_the_command {
    use super::*;

    fn run_of(line: &str) -> Options {
        match options::parse(&words(line)) {
            Ok(Parsed::Run(options)) => options,
            other => panic!("{line:?} parsed as {other:?}"),
        }
    }

    #[test]
    fn no_words_is_a_run_without_parity() {
        assert_eq!(
            run_of(""),
            Options {
                parity: false,
                parity_files: vec![],
                cargo_args: vec![],
                installer_smoke: false,
                wrapper: None,
                clippy: false,
                published: None,
                unpinned: false,
                wait: 60,
            }
        );
    }

    #[test]
    fn the_installer_smoke_and_its_wrapper() {
        let options = run_of("--installer-smoke --wrapper ../site/install.sh");
        assert!(options.installer_smoke && !options.parity, "{options:?}");
        assert_eq!(options.wrapper.as_deref(), Some("../site/install.sh"));
        assert_eq!(run_of("--installer-smoke").wrapper, None);
        for (line, says) in [
            ("--wrapper x.sh", "--wrapper goes with --installer-smoke"),
            ("--installer-smoke --parity", "not with --parity"),
            ("--installer-smoke -- -p kluris-core", "runs no cargo test"),
            ("--installer-smoke --wrapper", "--wrapper takes"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn unpinned_installs_githubs_latest_and_expects_the_named_release() {
        let options = run_of("--installer-smoke --published 3.0.0 --unpinned");
        assert!(options.unpinned, "{options:?}");
        assert_eq!(options.published.as_deref(), Some("3.0.0"));
        assert!(!run_of("--installer-smoke --published 3.0.0").unpinned);
        for (line, says) in [
            (
                "--installer-smoke --unpinned",
                "--unpinned goes with --published",
            ),
            ("--unpinned", "--unpinned goes with --published"),
            (
                "--installer-smoke --published 3.0.0 --unpinned=yes",
                "--unpinned takes no value",
            ),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn the_published_smoke_names_a_release_and_takes_no_wrapper_file() {
        let options = run_of("--installer-smoke --published 3.0.0-rc.1");
        assert!(options.installer_smoke && !options.parity, "{options:?}");
        assert_eq!(options.published.as_deref(), Some("3.0.0-rc.1"));
        assert_eq!(options.wrapper, None);
        assert_eq!(
            run_of("--published=v3.0.0 --installer-smoke")
                .published
                .as_deref(),
            Some("3.0.0"),
            "the = spelling, a leading v dropped, in any order"
        );
        assert_eq!(run_of("--installer-smoke").published, None);
        for (line, says) in [
            (
                "--published 3.0.0",
                "--published goes with --installer-smoke",
            ),
            (
                "--installer-smoke --published 3.0.0 --wrapper x.sh",
                "the live site's install",
            ),
            ("--installer-smoke --published", "--published takes"),
            ("--installer-smoke --published 3.0", "a release number"),
            (
                "--installer-smoke --published 3.0.0/../x",
                "a release number",
            ),
            ("--installer-smoke --published 3.0.0;x", "a release number"),
            ("--installer-smoke --published 3.0.0-", "a release number"),
            (
                "--installer-smoke --published 3.0.0 --published 3.0.1",
                "--published is given once",
            ),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn the_clippy_run() {
        let options = run_of("--clippy");
        assert!(options.clippy && !options.parity && !options.installer_smoke);
        for (line, says) in [
            ("--clippy --parity", "--clippy"),
            ("--clippy --installer-smoke", "--clippy"),
            ("--clippy -- -p kluris-core", "--clippy"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn the_wrapper_is_a_sh_file_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("install.sh");
        fs::write(&file, "#!/bin/sh\nmain\n").unwrap();
        assert_eq!(
            options::wrapper_file(&file.to_string_lossy()),
            Ok((
                ".kluris-site/install.sh".to_owned(),
                "#!/bin/sh\nmain\n".to_owned()
            ))
        );
        let refused = options::wrapper_file(&dir.path().join("install.ps1").to_string_lossy())
            .expect_err("a .ps1 is Windows'");
        assert!(refused.contains("a .sh file"), "{refused}");
    }

    #[test]
    fn parity_parity_files_and_cargo_args() {
        let options = run_of(
            "--parity --parity-files parity/test_smoke.py,parity/test_dream.py -- -p kluris-core -- --nocapture",
        );
        assert!(options.parity);
        assert_eq!(
            options.parity_files,
            words("parity/test_smoke.py parity/test_dream.py")
        );
        assert_eq!(options.cargo_args, words("-p kluris-core -- --nocapture"));
    }

    #[test]
    fn wait_is_the_minutes_of_a_turn_an_hour_unless_given_at_most_a_day() {
        assert_eq!(run_of("").wait, 60);
        assert_eq!(run_of("--wait 0").wait, 0);
        assert_eq!(run_of("--wait=15 --clippy").wait, 15);
        assert_eq!(run_of("--wait 1440").wait, 1440);
        for (line, says) in [
            (
                "--wait 1441",
                "--wait is the minutes to wait for a turn, 0 to 1440: 1441",
            ),
            (
                "--wait=-1",
                "--wait is the minutes to wait for a turn, 0 to 1440: -1",
            ),
            (
                "--wait soon",
                "--wait is the minutes to wait for a turn, 0 to 1440: soon",
            ),
            (
                "--wait",
                "--wait takes the minutes to wait for a turn, 0 to 1440",
            ),
            ("--wait 1 --wait 2", "--wait is given once"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert_eq!(refused, says, "{line:?}");
        }
    }

    #[test]
    fn help() {
        assert_eq!(options::parse(&words("--help")), Ok(Parsed::Help));
        assert_eq!(options::parse(&words("-h")), Ok(Parsed::Help));
    }

    #[test]
    fn refusals() {
        for (line, says) in [
            ("--machine zeelin", "unknown option: --machine"),
            ("--bogus", "unknown option: --bogus"),
            ("--parity=yes", "--parity takes no value"),
            ("--parity-files parity/test_smoke.py", "goes with --parity"),
            ("test", "after --"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }
}

// --- the scripts, as text -------------------------------------------------------

mod scripts {
    use super::*;

    fn every_script() -> Vec<(&'static str, String)> {
        let files = words("parity/test_smoke.py");
        let extra = words("-p kluris-core");
        vec![
            ("probe", script::probe(&Layout::zeelin())),
            ("bootstrap", script::bootstrap(ID, &Layout::zeelin())),
            ("run", script::run_script(&spec(false, &[], &[]))),
            (
                "run --parity",
                script::run_script(&spec(true, &files, &extra)),
            ),
            (
                "run --installer-smoke",
                script::run_script(&smoke_spec(true)),
            ),
            (
                "run --installer-smoke --published",
                script::run_script(&published_spec("3.0.0-rc.1")),
            ),
            ("stop", script::stop_script(ID)),
            ("cleanup", script::cleanup_script(ID)),
        ]
    }

    #[test]
    fn zeelin_and_its_neighbours() {
        assert_eq!(script::HOST, "zeelin");
        let layout = Layout::zeelin();
        assert_eq!(layout.poker_lab, ".compile-lock/owner");
        assert_eq!(
            layout.windows_owner,
            "/mnt/c/Users/AGV/consensflow-build/.windows-run.id"
        );
        assert_eq!(layout.windows_home, "/mnt/c/Users/AGV");
    }

    #[test]
    fn nothing_outside_klurises_folder_is_changed() {
        for (name, text) in every_script() {
            for never in [
                "sudo",
                ".bashrc",
                ".profile",
                ".cargo/env",
                "apt-get",
                "apt install",
                "pip install",
                "rustup default",
                "rustup self",
                "/etc/",
            ] {
                assert!(!text.contains(never), "{name} has {never:?}");
            }
            // Removals name their folder in full (`${HOME:?}/...`, `${run:?}`),
            // and never poker-lab's or Windows' files.
            for line in text.lines().filter(|line| line.contains("rm ")) {
                assert!(
                    !line.contains("compile-lock") && !line.contains("/mnt/c"),
                    "{name}: {line}"
                );
            }
        }
    }

    #[test]
    fn every_script_is_strict_bash() {
        for (name, text) in every_script() {
            if name.starts_with("run") {
                continue; // sourced by the bootstrap, under its settings
            }
            assert!(text.contains("set -euo pipefail"), "{name}");
        }
    }

    #[test]
    fn the_probe_looks_at_the_three_locks_and_takes_none() {
        let text = script::probe(&Layout::zeelin());
        assert!(text.contains(".compile-lock/owner"));
        assert!(text.contains("/mnt/c/Users/AGV/consensflow-build/.windows-run.id"));
        assert!(text.contains(".linux-run.lock"));
        assert!(text.contains("flock -n"));
        assert!(text.contains("exit_run 75"));
        assert!(!text.contains("exec 9"), "the probe holds no lock");
        assert!(!text.contains("tar "), "the probe runs nothing");
    }

    #[test]
    fn the_bootstrap_takes_its_own_lock_first_and_cleans_up_always() {
        let text = script::bootstrap(ID, &Layout::zeelin());
        let at = |what: &str| {
            text.find(what)
                .unwrap_or_else(|| panic!("the bootstrap has no {what:?}"))
        };
        assert!(at("trap cleanup EXIT") < at("flock -n 9"));
        assert!(at("flock -n 9") < at(".compile-lock/owner"));
        assert!(at(".compile-lock/owner") < at("> \"$owner\""));
        assert!(at("/mnt/c/Users/AGV/consensflow-build/.windows-run.id") < at("> \"$owner\""));
        assert!(at("> \"$owner\"") < at("tar -xzmf"));
        assert!(text.contains("trap 'exit 129' HUP"));
        assert!(text.contains("trap 'exit 130' INT"));
        assert!(text.contains("trap 'exit 143' TERM"));
        assert!(text.contains(&format!("incoming/{ID}.tgz")));
        assert!(
            text.contains("pgrep -s \"$$\""),
            "the run's session ends with it"
        );
    }

    #[test]
    fn the_run_is_bounded_in_tasks_and_time() {
        let text = script::run_script(&spec(true, &[], &[]));
        assert!(text.contains(&format!("ulimit -u {}", script::TASK_LIMIT)));
        const { assert!(script::TASK_LIMIT >= 512 && script::TASK_LIMIT <= 4096) };
        for step in crate::ssh_ci::plan::steps(true) {
            assert!(
                text.contains(&format!("step {} {} ", step.name, step.minutes)),
                "{}",
                step.name
            );
        }
        assert!(text.contains("timeout --kill-after=30s \"${minutes}m\""));
        let bound = crate::ssh_ci::plan::bound(true);
        assert!(bound < Duration::from_secs(8 * 3600));
    }

    #[test]
    fn tools_and_caches_stay_in_klurises_own_folder() {
        let text = script::run_script(&spec(true, &[], &[]));
        for line in [
            "export RUSTUP_HOME=\"$tools/rustup\"",
            "export CARGO_HOME=\"$tools/cargo\"",
            "export CARGO_TARGET_DIR=\"$cache/target\"",
            "export CARGO_BUILD_JOBS='8'",
            "export TMPDIR=\"$tmp\"",
            "export XDG_CACHE_HOME=\"$cache/xdg\"",
            "export UV_CACHE_DIR=\"$cache/uv\"",
            "export UV_PYTHON_INSTALL_DIR=\"$tools/python\"",
            "export UV_PYTHON_BIN_DIR=\"$tools/python-bin\"",
            "export UV_PYTHON_PREFERENCE=only-managed",
            "export UV_PROJECT_ENVIRONMENT=\"$cache/parity-venv\"",
            "export KLURIS_PARITY_ORACLE_DIR=\"$cache/oracle\"",
            "export KLURIS_V3_BIN=\"$CARGO_TARGET_DIR/release/kluris\"",
        ] {
            assert!(text.contains(line), "{line}");
        }
        assert!(text.contains("ci=\"${HOME:?}/.kluris-ci\""));
        assert!(text.contains(script::UV_SHA256) && text.contains(script::UV_URL));
        assert!(script::UV_URL.ends_with("/0.12.8/uv-x86_64-unknown-linux-gnu.tar.gz"));
        assert_eq!(
            script::UV_SHA256,
            "2e2b37e9811e17675a9e70bed5e1a58fc8c0388be63d751d72cc735188c149ff"
        );
        assert!(text.contains("\"${HOME:?}/.cargo/bin/rustup\""));
    }

    #[test]
    fn a_clippy_run_lints_every_target_and_runs_no_tests() {
        let spec = Spec {
            clippy: true,
            ..super::spec(false, &[], &[])
        };
        let steps: Vec<_> = script::steps(&spec).iter().map(|s| s.name).collect();
        assert_eq!(steps, ["toolchain", "clippy"]);
        let text = script::run_script(&spec);
        assert!(
            text.contains(
                "step clippy 60 \"$cargo\" 'clippy' '--workspace' '--all-targets' '--' '-D' 'warnings'"
            ),
            "{text}"
        );
        assert!(
            !text.contains("step cargo-test") && !text.contains("pytest"),
            "{text}"
        );
        assert!(script::bound(&spec) < script::bound(&super::spec(true, &[], &[])));
    }

    #[test]
    fn an_installer_smoke_builds_the_release_and_runs_the_smoke_script() {
        let spec = smoke_spec(false);
        let steps: Vec<_> = script::steps(&spec).iter().map(|step| step.name).collect();
        assert_eq!(
            steps,
            [
                "toolchain",
                "python",
                "release-dist",
                "installer-smoke",
                "release-versions",
                "lifecycle"
            ]
        );
        let text = script::run_script(&spec);
        for step in script::steps(&spec) {
            assert!(
                text.contains(&format!("step {} {} ", step.name, step.minutes)),
                "{} is not run with its bound:\n{text}",
                step.name
            );
        }
        assert!(script::bound(&spec) < script::bound(&super::spec(true, &[], &[])));
        for line in [
            "export KLURIS_TOOLS_DIR=\"$tools\"",
            "export UV_PYTHON_INSTALL_DIR=\"$tools/python\"",
            "step python 10 \"$uv\" 'python' 'install' '3.11'",
            "python=$(\"$uv\" python find 3.11)",
            "step release-dist 60 bash 'scripts/release/build-local.sh'",
            "step installer-smoke 20 env KLURIS_SMOKE_PYTHON=\"$python\" sh 'scripts/release/installer-smoke.sh' 'target/distrib'",
            "step release-versions 60 bash 'scripts/release/build-versions.sh' 'target/releases' '3.0.0' '3.0.1'",
            "step lifecycle 20 env KLURIS_SMOKE_PYTHON=\"$python\" sh 'scripts/release/lifecycle-smoke.sh' 'target/releases' '3.0.0' '3.0.1'",
        ] {
            assert!(text.contains(line), "no {line:?} in\n{text}");
        }
        assert!(
            !text.contains("step cargo-test") && !text.contains("pytest"),
            "an installer smoke runs no tests: {text}"
        );
        let with = script::run_script(&smoke_spec(true));
        assert!(
            with.contains("'scripts/release/installer-smoke.sh' '--wrapper' '.kluris-site/install.sh' 'target/distrib'"),
            "{with}"
        );
    }

    #[test]
    fn an_unpinned_published_smoke_tells_the_script_to_leave_the_version_out() {
        let pinned = script::run_script(&published_spec("3.0.0"));
        assert!(pinned.contains("step published-smoke 20 env KLURIS_SMOKE_PYTHON=\"$python\" sh 'scripts/release/published-smoke.sh' '3.0.0'"), "{pinned}");
        assert!(!pinned.contains("npinned"), "{pinned}");
        let spec = unpinned_spec("3.0.0");
        let text = script::run_script(&spec);
        assert!(text.contains("step published-smoke 20 env KLURIS_SMOKE_PYTHON=\"$python\" sh 'scripts/release/published-smoke.sh' '--unpinned' '3.0.0'"), "{text}");
        assert_eq!(
            script::steps(&spec),
            script::steps(&published_spec("3.0.0"))
        );
        assert_eq!(
            script::bound(&spec),
            script::bound(&published_spec("3.0.0"))
        );
    }

    #[test]
    fn a_published_smoke_installs_the_release_through_the_live_wrapper_and_builds_nothing() {
        let spec = published_spec("3.0.0-rc.1");
        let steps: Vec<_> = script::steps(&spec).iter().map(|step| step.name).collect();
        assert_eq!(steps, ["toolchain", "python", "published-smoke"]);
        let text = script::run_script(&spec);
        for step in script::steps(&spec) {
            assert!(
                text.contains(&format!("step {} {} ", step.name, step.minutes)),
                "{} is not run with its bound:\n{text}",
                step.name
            );
        }
        for line in [
            "step python 10 \"$uv\" 'python' 'install' '3.11'",
            "step published-smoke 20 env KLURIS_SMOKE_PYTHON=\"$python\" sh 'scripts/release/published-smoke.sh' '3.0.0-rc.1'",
        ] {
            assert!(text.contains(line), "no {line:?} in\n{text}");
        }
        for absent in [
            "build-local",
            "build-versions",
            "installer-smoke.sh",
            "lifecycle",
            "step cargo-test",
            "pytest",
        ] {
            assert!(
                !text.contains(absent),
                "a published smoke builds and tests nothing ({absent}):\n{text}"
            );
        }
        assert!(script::bound(&spec) < script::bound(&smoke_spec(false)));
    }

    #[test]
    fn the_commands_of_a_run() {
        let files = words("parity/test_smoke.py parity/test_dream.py");
        let extra = vec!["-p".to_owned(), "it's here".to_owned()];
        let text = script::run_script(&spec(true, &files, &extra));
        assert!(text.contains(
            "step toolchain 30 \"$rustup\" toolchain install '1.99.0' '--profile' 'minimal' '--component' 'clippy' '--component' 'rustfmt' '--no-self-update'"
        ));
        assert!(text.contains(
            "step cargo-test 90 \"$cargo\" 'test' '--workspace' '--no-fail-fast' '-p' 'it'\\''s here'"
        ));
        assert!(text.contains(
            "step release-build 60 \"$cargo\" 'build' '--release' '--package' 'kluris'\n"
        ));
        assert!(text.contains(
            "step parity 90 \"$uv\" 'run' '--project' 'parity' '--locked' 'pytest' 'parity/test_smoke.py' 'parity/test_dream.py' '-q'"
        ));
        let plain = script::run_script(&spec(false, &[], &[]));
        assert!(!plain.contains("release-build") && !plain.contains("uv"));
    }

    #[test]
    fn each_step_end_line_carries_the_run_id() {
        let text = script::run_script(&spec(true, &[], &[]));
        for line in [
            "printf '== step %s exit %s %ss %s\\n' \"$name\" \"$code\" \"$((SECONDS - started))\" \"$id\"",
            "printf '== step uv exit %s %ss %s\\n' \"$code\" \"$((SECONDS - started))\" \"$id\"",
        ] {
            assert!(text.contains(line), "no {line:?} in\n{text}");
        }
    }

    #[test]
    fn old_runs_and_tarballs_are_swept_after_a_day() {
        let text = script::run_script(&spec(false, &[], &[]));
        assert!(text.contains("-mmin +1440"));
        assert!(text.contains("! -name \"$id\""));
    }

    #[test]
    fn the_stop_ends_only_its_own_run() {
        let text = script::stop_script(ID);
        assert!(text.contains(&format!("= '{ID}'")));
        assert!(text.contains("pkill -KILL -s"));
        assert!(
            text.contains("/proc/"),
            "the session is checked to be the run's"
        );
        assert!(text.contains(&format!("incoming/{ID}.tgz")));
        assert!(text.trim_end().ends_with("exit 0"));
    }

    #[test]
    fn the_upload_cleanup_removes_only_its_own_tarball() {
        let text = script::cleanup_script(ID);
        assert!(text.contains(&format!("incoming/{ID}.tgz")));
        assert!(text.contains("absent") && text.contains("removed") && text.contains("failed: "));
        assert_eq!(text.matches("rm ").count(), 1, "{text}");
    }

    #[test]
    fn the_lines_ssh_sends() {
        let probe = script::probe_line(&Layout::zeelin());
        assert!(probe.starts_with("bash -c '"), "{probe}");
        let line = script::bootstrap_line(ID, &Layout::zeelin());
        assert!(line.starts_with("exec setsid -w bash -c '"), "{line}");
        assert!(line.ends_with(&format!("' {ID}")), "{line}");
        assert_eq!(script::quote("it's"), "'it'\\''s'");
        assert_eq!(script::quote(""), "''");
        assert_eq!(
            script::remote_tarball(ID),
            format!(".kluris-ci/incoming/{ID}.tgz")
        );
    }

    #[cfg(unix)]
    #[test]
    fn bash_reads_every_script() {
        for (name, text) in every_script() {
            let out = std::process::Command::new("bash")
                .args(["-n", "-c", &text])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

// --- the scripts, run -------------------------------------------------------------

/// The scripts run by bash against a home made for the case. `flock` and
/// `timeout` are stood in for (the Mac has neither): the fake `flock` says the
/// lock is held while `$HOME/.flock-held` exists, the fake `timeout` runs its
/// command.
#[cfg(unix)]
mod scripts_run {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Command;

    struct Home {
        dir: tempfile::TempDir,
    }

    struct Ran {
        code: Option<i32>,
        /// stdout, then stderr.
        out: String,
        stdout: String,
    }

    impl Ran {
        /// stdout's last line, where the exit line is read from.
        fn last_line(&self) -> &str {
            self.stdout
                .lines()
                .rev()
                .find(|l| !l.is_empty())
                .unwrap_or("")
        }
    }

    fn executable(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    impl Home {
        fn new() -> Self {
            let home = Self {
                dir: tempfile::tempdir().unwrap(),
            };
            executable(
                &home.path("bin/flock"),
                "#!/bin/sh\nif [ -e \"$HOME/.flock-held\" ]; then exit 1; fi\n[ \"$#\" -gt 2 ] && { shift 2; exec \"$@\"; }\nexit 0\n",
            );
            executable(
                &home.path("bin/timeout"),
                "#!/bin/sh\nwhile case \"$1\" in -*) true;; *) false;; esac; do shift; done\nshift\nexec \"$@\"\n",
            );
            fs::create_dir_all(home.path("windows")).unwrap();
            home
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.dir.path().join(rel)
        }

        fn layout(&self) -> Layout {
            Layout {
                poker_lab: ".compile-lock/owner".into(),
                windows_home: self.path("windows").to_string_lossy().into_owned(),
                windows_owner: self
                    .path("windows/consensflow-build/.windows-run.id")
                    .to_string_lossy()
                    .into_owned(),
            }
        }

        fn write(&self, rel: &str, text: &str) {
            let path = self.path(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }

        fn run(&self, text: &str) -> Ran {
            let path = format!(
                "{}:/usr/bin:/bin:/usr/sbin:/sbin",
                self.path("bin").display()
            );
            let out = Command::new("bash")
                .args(["-c", text])
                .env_clear()
                .env("HOME", self.dir.path())
                .env("PATH", path)
                .current_dir(self.dir.path())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let mut said = stdout.clone();
            said.push_str(&String::from_utf8_lossy(&out.stderr));
            Ran {
                code: out.status.code(),
                out: said,
                stdout,
            }
        }

        /// The run's tarball in the incoming folder, holding `run_script`.
        fn upload(&self, run_script: &str) {
            let tree = self.path("tree");
            fs::create_dir_all(&tree).unwrap();
            fs::write(tree.join(".kluris-tree"), "5f54983aaaaa\n").unwrap();
            fs::write(tree.join(script::RUN_FILE), run_script).unwrap();
            fs::write(tree.join("Cargo.toml"), "[workspace]\n").unwrap();
            let incoming = self.path(".kluris-ci/incoming");
            fs::create_dir_all(&incoming).unwrap();
            let status = Command::new("tar")
                .arg("-czf")
                .arg(incoming.join(format!("{ID}.tgz")))
                .arg("-C")
                .arg(&tree)
                .arg(".")
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    // The probe.

    #[test]
    fn a_free_zeelin_says_0_and_makes_the_incoming_folder() {
        let home = Home::new();
        let ran = home.run(&script::probe(&home.layout()));
        assert_eq!(ran.last_line(), "== exit 0", "{}", ran.out);
        assert_eq!(ran.code, Some(0));
        assert!(home.path(".kluris-ci/incoming").is_dir());
    }

    #[test]
    fn poker_labs_lock_refuses_with_75_naming_it() {
        let home = Home::new();
        home.write(".compile-lock/owner", "poker-lab job 7\n");
        let ran = home.run(&script::probe(&home.layout()));
        assert_eq!(ran.code, Some(75), "{}", ran.out);
        assert_eq!(ran.last_line(), "== exit 75");
        assert!(ran.out.contains("~/.compile-lock/owner"), "{}", ran.out);
        assert!(ran.out.contains("poker-lab"), "{}", ran.out);
        assert_eq!(
            fs::read_to_string(home.path(".compile-lock/owner")).unwrap(),
            "poker-lab job 7\n",
            "poker-lab's lock is only looked at"
        );
    }

    #[test]
    fn a_run_on_zeewin2_refuses_with_75_naming_it() {
        for (record, names) in [
            (
                "kluris-0000cf2a-be059375\r\n62120\r\n",
                "a Kluris run (kluris-0000cf2a-be059375, process 62120)",
            ),
            (
                "0000cf2a-be059375\r\n4242\r\n",
                "a ConsensFlow run (0000cf2a-be059375, process 4242)",
            ),
        ] {
            let home = Home::new();
            home.write("windows/consensflow-build/.windows-run.id", record);
            let ran = home.run(&script::probe(&home.layout()));
            assert_eq!(ran.code, Some(75), "{}", ran.out);
            assert!(ran.out.contains(names), "{}", ran.out);
            assert!(ran.out.contains("zeewin2"), "{}", ran.out);
            assert_eq!(
                fs::read_to_string(home.path("windows/consensflow-build/.windows-run.id")).unwrap(),
                record
            );
        }
    }

    #[test]
    fn a_windows_side_out_of_sight_refuses_with_75() {
        let home = Home::new();
        let mut layout = home.layout();
        layout.windows_home = home.path("no-such-drive").to_string_lossy().into_owned();
        let ran = home.run(&script::probe(&layout));
        assert_eq!(ran.code, Some(75), "{}", ran.out);
        assert!(ran.out.contains("cannot see"), "{}", ran.out);
    }

    #[test]
    fn its_own_lock_held_refuses_with_75_naming_the_run() {
        let home = Home::new();
        home.write(".kluris-ci/.linux-run.lock", "");
        home.write(
            ".kluris-ci/.linux-run.id",
            "kluris-00000001-00000002\n4711\n",
        );
        home.write(".flock-held", "");
        let ran = home.run(&script::probe(&home.layout()));
        assert_eq!(ran.code, Some(75), "{}", ran.out);
        assert!(
            ran.out
                .contains("a Kluris run (kluris-00000001-00000002, process 4711)"),
            "{}",
            ran.out
        );
    }

    // The bootstrap.

    #[test]
    fn the_bootstrap_unpacks_says_the_tree_runs_and_cleans_up() {
        let home = Home::new();
        home.upload("echo \"ran in $PWD\"\nexit_run 0\n");
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(0), "{}", ran.out);
        assert!(ran.out.contains("== tree 5f54983aaaaa"), "{}", ran.out);
        assert!(ran.out.contains("ran in "), "{}", ran.out);
        assert!(
            ran.out.contains(&format!(".kluris-ci/runs/{ID}")),
            "{}",
            ran.out
        );
        assert_eq!(ran.last_line(), "== exit 0");
        assert!(!home.path(&format!(".kluris-ci/incoming/{ID}.tgz")).exists());
        assert!(!home.path(".kluris-ci/.linux-run.id").exists());
        assert!(
            home.path(&format!(".kluris-ci/runs/{ID}/Cargo.toml"))
                .is_file()
        );
    }

    #[test]
    fn the_bootstrap_writes_its_record_while_it_runs() {
        let home = Home::new();
        home.upload("cat \"$HOME/.kluris-ci/.linux-run.id\"\nexit_run 0\n");
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(0), "{}", ran.out);
        let mut lines = ran.out.lines().skip_while(|l| !l.starts_with("== tree"));
        lines.next();
        assert_eq!(lines.next(), Some(ID), "{}", ran.out);
        assert!(lines.next().unwrap().parse::<u32>().is_ok(), "{}", ran.out);
    }

    #[test]
    fn a_refused_bootstrap_writes_nothing_and_removes_its_tarball() {
        for held in [
            ".compile-lock/owner",
            "windows/consensflow-build/.windows-run.id",
        ] {
            let home = Home::new();
            home.upload("echo ran\nexit_run 0\n");
            home.write(held, "x\n");
            let ran = home.run(&script::bootstrap(ID, &home.layout()));
            assert_eq!(ran.code, Some(75), "{held}: {}", ran.out);
            assert!(!ran.out.contains("ran"), "{}", ran.out);
            assert!(!home.path(".kluris-ci/.linux-run.id").exists());
            assert!(!home.path(&format!(".kluris-ci/runs/{ID}")).exists());
            assert!(!home.path(&format!(".kluris-ci/incoming/{ID}.tgz")).exists());
        }
    }

    #[test]
    fn a_held_own_lock_leaves_the_holders_record() {
        let home = Home::new();
        home.upload("echo ran\nexit_run 0\n");
        home.write(
            ".kluris-ci/.linux-run.id",
            "kluris-00000001-00000002\n4711\n",
        );
        home.write(".flock-held", "");
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(75), "{}", ran.out);
        assert!(ran.out.contains("kluris-00000001-00000002"), "{}", ran.out);
        assert_eq!(
            fs::read_to_string(home.path(".kluris-ci/.linux-run.id")).unwrap(),
            "kluris-00000001-00000002\n4711\n"
        );
    }

    #[test]
    fn a_failed_unpack_ends_the_run_with_its_status() {
        let home = Home::new();
        home.write(&format!(".kluris-ci/incoming/{ID}.tgz"), "not a tarball");
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_ne!(ran.code, Some(0), "{}", ran.out);
        assert!(ran.last_line().starts_with("== exit "), "{}", ran.out);
        assert!(!home.path(".kluris-ci/.linux-run.id").exists());
    }

    // The run script, with a rustup, cargo and uv that only say what they were asked.

    fn tools(home: &Home, cargo_test_status: u8) {
        executable(
            &home.path(".cargo/bin/rustup"),
            "#!/bin/sh\necho \"rustup $*\"\necho \"RUSTUP_HOME=$RUSTUP_HOME\"\n",
        );
        executable(
            &home.path(".cargo/bin/cargo"),
            &format!(
                "#!/bin/bash\necho \"cargo $*\"\necho \"tasks $(ulimit -u)\"\ncase \"$1\" in test) exit {cargo_test_status};; esac\nexit 0\n"
            ),
        );
        executable(
            &home.path(".kluris-ci/tools/uv-0.12.8/uv"),
            "#!/bin/sh\necho \"uv $*\"\necho \"bin $KLURIS_V3_BIN\"\n",
        );
    }

    /// The task limit this test process has now (macOS reports no more than
    /// it, whatever the hard limit says), so the run's cap is one it can set.
    fn task_limit_here() -> u32 {
        // bash, as the scripts: dash (Ubuntu's sh) has no `ulimit -u`.
        let out = Command::new("bash")
            .args(["-c", "ulimit -u"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(100_000)
    }

    #[test]
    fn a_run_says_each_step_and_ends_with_the_first_failure() {
        let home = Home::new();
        tools(&home, 101);
        let files = words("parity/test_smoke.py");
        let extra = vec!["it's".to_owned()];
        let mut spec = spec(true, &files, &extra);
        spec.tasks = task_limit_here();
        home.upload(&script::run_script(&spec));
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(101), "{}", ran.out);
        assert_eq!(ran.last_line(), "== exit 101");
        for line in [
            "rustup toolchain install 1.99.0 --profile minimal",
            "== step toolchain exit 0 ",
            "cargo test --workspace --no-fail-fast it's",
            "== step cargo-test exit 101 ",
            "cargo build --release --package kluris\n",
            "== step release-build exit 0 ",
            "uv run --project parity --locked pytest parity/test_smoke.py -q",
            "== step parity exit 0 ",
            &format!("tasks {}", spec.tasks),
        ] {
            assert!(ran.out.contains(line), "{line:?} in\n{}", ran.out);
        }
        let ends: Vec<&str> = ran
            .out
            .lines()
            .filter(|line| line.starts_with("== step ") && line.contains(" exit "))
            .collect();
        assert_eq!(ends.len(), 4, "{}", ran.out);
        for line in ends {
            assert!(line.ends_with(&format!("s {ID}")), "{line}");
        }
        let kluris_ci = home.path(".kluris-ci");
        assert!(
            ran.out.contains(&format!(
                "RUSTUP_HOME={}",
                kluris_ci.join("tools/rustup").display()
            )),
            "{}",
            ran.out
        );
        assert!(
            ran.out.contains(&format!(
                "bin {}",
                kluris_ci.join("cache/target/release/kluris").display()
            )),
            "{}",
            ran.out
        );
    }

    #[test]
    fn a_failed_toolchain_stops_the_run() {
        let home = Home::new();
        tools(&home, 0);
        executable(&home.path(".cargo/bin/rustup"), "#!/bin/sh\nexit 3\n");
        let mut spec = spec(false, &[], &[]);
        spec.tasks = task_limit_here();
        home.upload(&script::run_script(&spec));
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(3), "{}", ran.out);
        assert!(!ran.out.contains("cargo test"), "{}", ran.out);
    }

    #[test]
    fn a_machine_without_rustup_is_told_about() {
        let home = Home::new();
        let mut spec = spec(false, &[], &[]);
        spec.tasks = task_limit_here();
        home.upload(&script::run_script(&spec));
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(1), "{}", ran.out);
        assert!(ran.out.contains("install rustup"), "{}", ran.out);
    }

    #[test]
    fn old_runs_and_stray_tarballs_are_swept() {
        let home = Home::new();
        tools(&home, 0);
        home.write(".kluris-ci/runs/kluris-old/x", "x");
        home.write(".kluris-ci/runs/kluris-new/x", "x");
        home.write(".kluris-ci/incoming/kluris-old.tgz", "x");
        for old in [
            ".kluris-ci/runs/kluris-old",
            ".kluris-ci/incoming/kluris-old.tgz",
        ] {
            let status = Command::new("touch")
                .args(["-t", "202001010000"])
                .arg(home.path(old))
                .status()
                .unwrap();
            assert!(status.success());
        }
        let mut spec = spec(false, &[], &[]);
        spec.tasks = task_limit_here();
        home.upload(&script::run_script(&spec));
        let ran = home.run(&script::bootstrap(ID, &home.layout()));
        assert_eq!(ran.code, Some(0), "{}", ran.out);
        assert!(
            !home.path(".kluris-ci/runs/kluris-old").exists(),
            "{}",
            ran.out
        );
        assert!(!home.path(".kluris-ci/incoming/kluris-old.tgz").exists());
        assert!(home.path(".kluris-ci/runs/kluris-new").exists());
        assert!(
            ran.out.contains("== sweep: removed runs/kluris-old"),
            "{}",
            ran.out
        );
    }

    #[test]
    fn the_upload_cleanup_says_what_it_did() {
        let home = Home::new();
        let ran = home.run(&script::cleanup_script(ID));
        assert_eq!(ran.out.trim(), "absent");
        home.write(&format!(".kluris-ci/incoming/{ID}.tgz"), "partial");
        let ran = home.run(&script::cleanup_script(ID));
        assert_eq!(ran.out.trim(), "removed");
        assert!(!home.path(&format!(".kluris-ci/incoming/{ID}.tgz")).exists());
    }
}

// --- a whole run ----------------------------------------------------------------------

mod runs {
    use super::*;

    struct Ran {
        code: i32,
        out: String,
        err: String,
        fake: Fake,
        scripts: Vec<String>,
        /// The run's queues, a folder of the test's own.
        queues: tempfile::TempDir,
        pauses: Vec<u64>,
        /// zeewin2's queue each time the tree was packed: after the wait,
        /// while the run holds its place.
        held: Vec<Vec<String>>,
    }

    fn ran_in(root: &Path, options: Options, answers: Vec<Answer>) -> Ran {
        ran_queued(
            root,
            options,
            answers,
            tempfile::tempdir().unwrap(),
            Clock::default(),
        )
    }

    /// A run whose queues are `queues` and whose clock is `clock`.
    fn ran_queued(
        root: &Path,
        options: Options,
        answers: Vec<Answer>,
        queues: tempfile::TempDir,
        mut clock: Clock,
    ) -> Ran {
        let mut fake = Fake::new(answers);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut scripts = Vec::new();
        let mut held = Vec::new();
        let folder = queues.path().to_path_buf();
        let code = {
            let mut pack = |tarball: &Path, run_script: &str| -> Result<String, String> {
                scripts.push(run_script.to_owned());
                held.push(tickets(&folder, "zeewin2"));
                fs::write(tarball, b"tarball").map_err(|e| e.to_string())?;
                Ok("5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into())
            };
            let mut ctx = Ctx {
                runner: &mut fake,
                out: &mut out,
                err: &mut err,
                stop: &|| false,
                id: "kluris-0000abcd-12345678".into(),
                channel: "1.99.0".into(),
                pack: &mut pack,
                root,
                queues: Some(Queues::at(&folder, Duration::from_secs(3600))),
                pace: &mut clock,
                command: "cargo xtask linux".into(),
            };
            run::linux(&options, &mut ctx)
        };
        Ran {
            code,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
            fake,
            scripts,
            queues,
            pauses: clock.pauses,
            held,
        }
    }

    fn ran(options: Options, answers: Vec<Answer>) -> Ran {
        ran_in(Path::new("."), options, answers)
    }

    fn plain(parity: bool) -> Options {
        Options {
            parity,
            parity_files: vec![],
            cargo_args: vec![],
            installer_smoke: false,
            wrapper: None,
            clippy: false,
            published: None,
            unpinned: false,
            wait: 0,
        }
    }

    #[test]
    fn an_installer_smoke_run_sends_the_smoke_and_waits_its_bound() {
        let passed = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
            == step toolchain exit 0 3s kluris-0000abcd-12345678\n\
            == step python exit 0 2s kluris-0000abcd-12345678\n\
            == step release-dist exit 0 200s kluris-0000abcd-12345678\n\
            installer-smoke: ok (Linux x86_64, kluris 3.0.0-dev, through the site wrapper)\n\
            == step installer-smoke exit 0 30s kluris-0000abcd-12345678\n\
            \n== exit 0\n";
        let options = Options {
            installer_smoke: true,
            wrapper: Some("site/install.sh".into()),
            ..plain(false)
        };
        let ran = ran(
            options,
            vec![
                free(),
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, passed)], Ok(Streamed::Exited(Some(0)))),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        let script = &ran.scripts[0];
        assert!(
            script.contains("'--wrapper' '.kluris-site/install.sh'"),
            "{script}"
        );
        assert!(!script.contains("step cargo-test"), "{script}");
        let ssh = ran.fake.calls.last().unwrap();
        assert!(
            ssh.bound >= script::bound(&smoke_spec(true)),
            "{:?}",
            ssh.bound
        );
        assert!(
            ran.err
                .lines()
                .last()
                .unwrap()
                .contains("release-dist 0 (200s), installer-smoke 0 (30s)"),
            "{}",
            ran.err
        );
    }

    fn free() -> Answer {
        said(0, "== exit 0\n")
    }

    fn failed(code: i32, stderr: &str) -> Answer {
        Answer::Capture(Ok(Captured {
            code: Some(code),
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: false,
        }))
    }

    const PASSED: &str = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
        == step toolchain: rustup toolchain install\n\
        == step toolchain exit 0 3s kluris-0000abcd-12345678\n\
        test result: ok. 12 passed\n\
        == step cargo-test exit 0 212s kluris-0000abcd-12345678\n\
        \n== exit 0\n";

    fn id_of(fake: &Fake) -> String {
        let target = fake.calls[1].args.last().unwrap();
        target
            .trim_start_matches("zeelin:.kluris-ci/incoming/")
            .trim_end_matches(".tgz")
            .to_owned()
    }

    #[test]
    fn step_lines_of_inner_output_do_not_reach_the_summary() {
        // A failing test prints the output it captured, a run script's step
        // lines among them: only the lines that end with this run's id count.
        let out = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
            == step toolchain exit 0 3s kluris-0000abcd-12345678\n\
            ---- linux::tests::scripts_run::a_run stdout ----\n\
            == step toolchain exit 0 1s\n\
            == step lifecycle exit 1 9s\n\
            == step cargo-test exit 101 2s kluris-ffffffff-00000000\n\
            == step cargo-test exit 0 212s kluris-0000abcd-12345678 extra\n\
            == step cargo-test exit 0 212s kluris-0000abcd-12345678\n\
            \n== exit 0\n";
        let ran = ran(
            plain(false),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, out)], Ok(Streamed::Exited(Some(0)))),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(
            ran.err.lines().last().unwrap(),
            "linux: zeelin run kluris-0000abcd-12345678 tree 5f54983aaaaa: toolchain 0 (3s), cargo-test 0 (212s); exit 0"
        );
    }

    #[test]
    fn a_run_asks_zeelin_sends_the_tree_and_streams_the_answer() {
        let ran = ran(
            plain(false),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(
                    vec![(Stream::Out, PASSED), (Stream::Err, "warning: x\n")],
                    Ok(Streamed::Exited(Some(0))),
                ),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(ran.out.contains("test result: ok. 12 passed"));
        assert!(ran.err.contains("warning: x"));
        let calls = &ran.fake.calls;
        assert_eq!(calls.len(), 3, "{:?}", ran.fake.called());

        assert_eq!(calls[0].program, "ssh");
        let probe = calls[0].args.join(" ");
        for word in ["ConnectTimeout=5", "BatchMode=yes", "ServerAliveInterval=5"] {
            assert!(probe.contains(word), "{probe}");
        }
        assert_eq!(
            calls[0].args.last().unwrap(),
            &script::probe_line(&Layout::zeelin())
        );
        assert!(calls[0].args.contains(&"zeelin".to_owned()));
        assert!(calls[0].bound <= Duration::from_secs(30));

        assert_eq!(calls[1].program, "scp");
        let id = id_of(&ran.fake);
        assert!(id.starts_with("kluris-"), "{id}");
        assert_eq!(
            calls[1].args.last().unwrap(),
            &format!("zeelin:{}", script::remote_tarball(&id))
        );

        assert_eq!(calls[2].program, "ssh");
        assert!(calls[2].args.join(" ").contains("ServerAliveInterval=15"));
        assert_eq!(
            calls[2].args.last().unwrap(),
            &script::bootstrap_line(&id, &Layout::zeelin())
        );
        assert!(calls[2].bound >= crate::ssh_ci::plan::bound(false));

        assert!(ran.scripts[0].contains("step cargo-test 90"));
        let summary = ran.err.lines().last().unwrap();
        assert_eq!(
            summary,
            format!(
                "linux: zeelin run {id} tree 5f54983aaaaa: toolchain 0 (3s), cargo-test 0 (212s); exit 0"
            )
        );
    }

    #[test]
    fn poker_lab_holding_zeelin_is_75_and_sends_nothing() {
        let ran = ran(
            plain(true),
            vec![said(
                75,
                "zeelin is busy: poker-lab's job holds it (~/.compile-lock/owner), and no run goes while it does\n\n== exit 75\n",
            )],
        );
        assert_eq!(ran.code, 75, "{}", ran.err);
        assert_eq!(ran.fake.calls.len(), 1, "{:?}", ran.fake.called());
        assert!(ran.scripts.is_empty(), "nothing was packed");
        assert!(
            ran.err.contains(
                "linux: zeelin is busy: poker-lab's job holds it (~/.compile-lock/owner)"
            ),
            "{}",
            ran.err
        );
    }

    #[test]
    fn zeewin2_busy_and_its_own_lock_held_are_75() {
        for line in [
            "the laptop is busy: a Kluris run (kluris-0000cf2a-be059375, process 62120) holds zeewin2",
            "zeelin's one-run lock is held by a Kluris run (kluris-1-2, process 7): one run at a time",
        ] {
            let ran = ran(
                plain(false),
                vec![said(75, &format!("{line}\n\n== exit 75\n"))],
            );
            assert_eq!(ran.code, 75, "{}", ran.err);
            assert_eq!(ran.fake.calls.len(), 1);
            assert!(ran.err.contains(&format!("linux: {line}")), "{}", ran.err);
        }
    }

    #[test]
    fn zeelin_not_answering_is_75_not_running() {
        for answer in [
            failed(
                255,
                "ssh: connect to host 10.6.0.6 port 22: Operation timed out\n",
            ),
            failed(
                255,
                "ssh: connect to host 10.6.0.6 port 22: Connection refused\n",
            ),
            timed_out(),
        ] {
            let ran = ran(plain(false), vec![answer]);
            assert_eq!(ran.code, 75, "{}", ran.err);
            assert!(ran.err.contains("linux: zeelin not running"), "{}", ran.err);
            assert_eq!(ran.fake.calls.len(), 1);
        }
    }

    #[test]
    fn a_zeelin_that_answers_but_cannot_say_is_an_error() {
        for answer in [
            failed(255, "Host key verification failed.\n"),
            said(0, "something else\n"),
        ] {
            let ran = ran(plain(false), vec![answer]);
            assert_eq!(ran.code, 1, "{}", ran.err);
            assert!(!ran.err.contains("not running"), "{}", ran.err);
            assert_eq!(ran.fake.calls.len(), 1);
        }
    }

    #[test]
    fn parity_files_are_checked_before_anything_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("parity")).unwrap();
        fs::write(dir.path().join("parity").join("test_smoke.py"), "x").unwrap();
        let options = Options {
            parity: true,
            parity_files: words("parity/missing.py"),
            cargo_args: vec![],
            installer_smoke: false,
            wrapper: None,
            clippy: false,
            published: None,
            unpinned: false,
            wait: 0,
        };
        let ran = ran_in(dir.path(), options, vec![]);
        assert_eq!(ran.code, 1);
        assert!(ran.err.contains("not in this tree"), "{}", ran.err);
        assert!(ran.fake.calls.is_empty());

        let options = Options {
            parity: true,
            parity_files: words("parity/test_smoke.py"),
            cargo_args: words("-p kluris-core"),
            installer_smoke: false,
            wrapper: None,
            clippy: false,
            published: None,
            unpinned: false,
            wait: 0,
        };
        let ran = ran_in(
            dir.path(),
            options,
            vec![
                free(),
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, PASSED)], Ok(Streamed::Exited(Some(0)))),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(ran.scripts[0].contains("'pytest' 'parity/test_smoke.py' '-q'"));
        assert!(ran.scripts[0].contains("'--no-fail-fast' '-p' 'kluris-core'"));
    }

    #[test]
    fn a_held_lock_found_by_the_bootstrap_is_75() {
        let ran = ran(
            plain(false),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(
                    vec![(
                        Stream::Out,
                        "zeelin's one-run lock is held by a Kluris run (kluris-1-2, process 7): one run at a time.\n\n== exit 75\n",
                    )],
                    Ok(Streamed::Exited(Some(75))),
                ),
            ],
        );
        assert_eq!(ran.code, 75, "{}", ran.err);
        assert_eq!(ran.fake.calls.len(), 3, "no stop for a run that ended");
    }

    #[test]
    fn a_failed_upload_runs_nothing_and_removes_its_tarball() {
        let ran = ran(
            plain(false),
            vec![
                free(),
                failed(1, "scp: write failed\n"),
                said(0, "removed\n"),
            ],
        );
        assert_eq!(ran.code, 1, "{}", ran.err);
        let calls = &ran.fake.calls;
        assert_eq!(calls.len(), 3, "{:?}", ran.fake.called());
        let id = id_of(&ran.fake);
        assert_eq!(calls[2].args.last().unwrap(), &script::cleanup_line(&id));
        assert!(
            ran.err.contains("linux: scp of the tree to zeelin failed"),
            "{}",
            ran.err
        );
        assert!(ran.err.contains("was removed"), "{}", ran.err);
    }

    fn stop_sent(fake: &Fake) -> Option<String> {
        let last = fake.calls.last()?;
        (last.program == "ssh").then(|| last.args.last().cloned().unwrap_or_default())
    }

    #[test]
    fn a_stopped_run_is_ended_there_and_is_130() {
        let ran = ran(
            plain(false),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(vec![], Ok(Streamed::Stopped)),
                said(0, ""),
            ],
        );
        assert_eq!(ran.code, 130);
        let id = id_of(&ran.fake);
        assert_eq!(stop_sent(&ran.fake), Some(script::stop_line(&id)));
    }

    #[test]
    fn a_run_past_its_bound_is_ended_there_and_is_124() {
        let ran = ran(
            plain(true),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(vec![], Ok(Streamed::TimedOut)),
                said(0, ""),
            ],
        );
        assert_eq!(ran.code, 124);
        assert!(ran.err.contains("passed its bound"), "{}", ran.err);
        let id = id_of(&ran.fake);
        assert_eq!(stop_sent(&ran.fake), Some(script::stop_line(&id)));
    }

    #[test]
    fn a_lost_connection_ends_the_run_there_too() {
        let ran = ran(
            plain(false),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(
                    vec![(Stream::Out, "== tree 5f54983a\n== step toolchain: x\n")],
                    Ok(Streamed::Exited(Some(255))),
                ),
                said(0, ""),
            ],
        );
        assert_eq!(ran.code, 255, "{}", ran.err);
        let id = id_of(&ran.fake);
        assert_eq!(stop_sent(&ran.fake), Some(script::stop_line(&id)));
    }

    // --- the queue every runner on this Mac shares: zeewin2's, the same laptop ----

    fn waiting() -> Options {
        Options {
            wait: 60,
            ..plain(false)
        }
    }

    /// A ConsensFlow run queued for zeewin2 a second before the run comes.
    fn queued_ahead(queues: &tempfile::TempDir) -> crate::ssh_ci::queue::Ticket {
        Queues::at(queues.path(), Duration::from_secs(3600))
            .join(
                "zeewin2",
                &Who {
                    tree: "consensflow".into(),
                    command: "cargo xtask windows --machine zeewin2 -- cargo test".into(),
                },
                NOW - 1_000,
            )
            .unwrap()
    }

    fn mine() -> String {
        format!("-{:010}.ticket", std::process::id())
    }

    fn passed() -> Answer {
        Answer::Stream(vec![(Stream::Out, PASSED)], Ok(Streamed::Exited(Some(0))))
    }

    const POKER_LAB: &str = "zeelin is busy: poker-lab's job holds it (~/.compile-lock/owner), and no run goes while it does";

    fn poker_lab() -> Answer {
        said(75, &format!("{POKER_LAB}\n\n== exit 75\n"))
    }

    #[test]
    fn a_linux_run_queues_in_zeewin2s_folder_behind_a_windows_run() {
        let queues = tempfile::tempdir().unwrap();
        let mut ahead = Some(queued_ahead(&queues));
        let clock = Clock {
            on_pause: Box::new(move |_| drop(ahead.take())),
            ..Clock::default()
        };
        let ran = ran_queued(
            Path::new("."),
            waiting(),
            vec![free(), said(0, ""), passed()],
            queues,
            clock,
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(ran.pauses, [10_000]);
        assert!(
            ran.err.starts_with(
                "linux: waiting for zeelin, 0 s of up to 60 min: 2nd in the queue for zeewin2; \
                 the run ahead is consensflow (cargo xtask windows --machine zeewin2 -- cargo test), here for 1 s\n"
            ),
            "{}",
            ran.err
        );
        assert_eq!(ran.held.len(), 1);
        assert_eq!(ran.held[0].len(), 1, "{:?}", ran.held);
        assert!(ran.held[0][0].ends_with(&mine()), "{:?}", ran.held);
        assert_eq!(tickets(ran.queues.path(), "zeewin2"), Vec::<String>::new());
        assert!(!ran.queues.path().join("zeelin").exists());
    }

    #[test]
    fn zeelin_not_running_is_75_at_once_and_never_waited_on() {
        let ran = ran(
            waiting(),
            vec![failed(
                255,
                "ssh: connect to host 10.6.0.6 port 22: Operation timed out\n",
            )],
        );
        assert_eq!(ran.code, 75, "{}", ran.err);
        assert!(ran.pauses.is_empty());
        assert!(ran.err.contains("linux: zeelin not running"), "{}", ran.err);
        assert_eq!(ran.fake.calls.len(), 1);
        assert_eq!(tickets(ran.queues.path(), "zeewin2"), Vec::<String>::new());
        // Nor once the wait is on.
        let later = ran_in(Path::new("."), waiting(), vec![poker_lab(), timed_out()]);
        assert_eq!(later.code, 75, "{}", later.err);
        assert_eq!(later.pauses, [30_000]);
        assert!(
            later.err.contains("linux: zeelin not running"),
            "{}",
            later.err
        );
    }

    #[test]
    fn a_busy_zeelin_at_the_head_is_waited_for_and_asked_again() {
        let ran = ran(waiting(), vec![poker_lab(), free(), said(0, ""), passed()]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(ran.pauses, [30_000]);
        assert!(
            ran.err.starts_with(&format!(
                "linux: waiting for zeelin, 0 s of up to 60 min: {POKER_LAB}\n"
            )),
            "{}",
            ran.err
        );
    }

    #[test]
    fn a_refusal_by_the_bootstrap_at_the_head_keeps_the_place_and_tries_again() {
        let held = "zeelin's one-run lock is held by a Kluris run (kluris-1-2, process 7): one run at a time\n\n== exit 75\n";
        let ran = ran(
            waiting(),
            vec![
                free(),
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, held)], Ok(Streamed::Exited(Some(75)))),
                free(),
                said(0, ""),
                passed(),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(ran.pauses, [30_000]);
        assert!(
            ran.err.contains(
                "linux: waiting for zeelin, 0 s of up to 60 min: zeelin's one-run lock is held by a Kluris run (kluris-1-2, process 7): one run at a time\n"
            ),
            "{}",
            ran.err
        );
        assert_eq!(ran.scripts.len(), 1, "packed once");
        assert_eq!(tickets(ran.queues.path(), "zeewin2"), Vec::<String>::new());
    }

    #[test]
    fn wait_0_is_refused_behind_a_live_zeewin2_ticket_and_asks_nothing() {
        let queues = tempfile::tempdir().unwrap();
        let ahead = queued_ahead(&queues);
        let ran = ran_queued(
            Path::new("."),
            plain(false),
            vec![],
            queues,
            Clock::default(),
        );
        assert_eq!(ran.code, 75);
        assert_eq!(
            ran.err,
            "linux: zeelin is busy: 1 run is queued for it (zeewin2's queue: the same laptop)\n"
        );
        assert!(ran.fake.calls.is_empty());
        drop(ahead);
    }

    #[test]
    fn a_wait_that_runs_out_is_75_saying_how_long_and_what_for() {
        let ran = ran(
            Options {
                wait: 1,
                ..plain(false)
            },
            vec![poker_lab(), poker_lab(), poker_lab()],
        );
        assert_eq!(ran.code, 75, "{}", ran.err);
        assert_eq!(
            ran.err,
            format!(
                "linux: waiting for zeelin, 0 s of up to 1 min: {POKER_LAB}\n\
                 linux: zeelin is still busy after waiting 1 min (as far as --wait 1 goes): {POKER_LAB}\n"
            )
        );
        assert!(ran.scripts.is_empty());
    }

    #[test]
    fn the_ticket_goes_however_the_run_ends() {
        let tree = "== tree 5f54983a\n== step toolchain: x\n";
        let failed_run = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\n== exit 1\n";
        let cases: Vec<(i32, Vec<Answer>)> = vec![
            (0, vec![free(), said(0, ""), passed()]),
            (
                1,
                vec![
                    free(),
                    said(0, ""),
                    Answer::Stream(
                        vec![(Stream::Out, failed_run)],
                        Ok(Streamed::Exited(Some(1))),
                    ),
                ],
            ),
            (
                124,
                vec![
                    free(),
                    said(0, ""),
                    Answer::Stream(vec![], Ok(Streamed::TimedOut)),
                    said(0, ""),
                ],
            ),
            (
                130,
                vec![
                    free(),
                    said(0, ""),
                    Answer::Stream(vec![], Ok(Streamed::Stopped)),
                    said(0, ""),
                ],
            ),
            (
                255,
                vec![
                    free(),
                    said(0, ""),
                    Answer::Stream(vec![(Stream::Out, tree)], Ok(Streamed::Exited(Some(255)))),
                    said(0, ""),
                ],
            ),
            (1, vec![free(), said(1, ""), said(0, "removed\n")]),
        ];
        for (code, answers) in cases {
            let ran = ran(waiting(), answers);
            assert_eq!(ran.code, code, "{}", ran.err);
            assert_eq!(ran.held.len(), 1);
            assert!(ran.held[0][0].ends_with(&mine()), "{code}: {:?}", ran.held);
            assert_eq!(
                tickets(ran.queues.path(), "zeewin2"),
                Vec::<String>::new(),
                "{code}"
            );
        }
        let ran = ran_queued(
            Path::new("."),
            waiting(),
            vec![poker_lab()],
            tempfile::tempdir().unwrap(),
            Clock {
                stop_after: Some(1),
                ..Clock::default()
            },
        );
        assert_eq!(ran.code, 130, "{}", ran.err);
        assert!(ran.scripts.is_empty());
        assert_eq!(tickets(ran.queues.path(), "zeewin2"), Vec::<String>::new());
    }
}
