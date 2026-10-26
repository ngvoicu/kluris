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
use crate::ssh_ci::fake::{Answer, Fake, said, timed_out};
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
    fn an_installer_smoke_builds_the_release_and_runs_the_smoke_script() {
        let spec = smoke_spec(false);
        let steps: Vec<_> = script::steps(&spec).iter().map(|step| step.name).collect();
        assert_eq!(
            steps,
            ["toolchain", "python", "release-dist", "installer-smoke"]
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
    }

    fn ran_in(root: &Path, options: Options, answers: Vec<Answer>) -> Ran {
        let mut fake = Fake::new(answers);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut scripts = Vec::new();
        let code = {
            let mut pack = |tarball: &Path, run_script: &str| -> Result<String, String> {
                scripts.push(run_script.to_owned());
                fs::write(tarball, b"tarball").map_err(|e| e.to_string())?;
                Ok("5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into())
            };
            let mut ctx = Ctx {
                runner: &mut fake,
                out: &mut out,
                err: &mut err,
                stop: &|| false,
                channel: "1.99.0".into(),
                pack: &mut pack,
                root,
            };
            run::linux(&options, &mut ctx)
        };
        Ran {
            code,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
            fake,
            scripts,
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
        }
    }

    #[test]
    fn an_installer_smoke_run_sends_the_smoke_and_waits_its_bound() {
        let passed = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
            == step toolchain exit 0 3s\n\
            == step python exit 0 2s\n\
            == step release-dist exit 0 200s\n\
            installer-smoke: ok (Linux x86_64, kluris 3.0.0-dev, through the site wrapper)\n\
            == step installer-smoke exit 0 30s\n\
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
        == step toolchain exit 0 3s\n\
        test result: ok. 12 passed\n\
        == step cargo-test exit 0 212s\n\
        \n== exit 0\n";

    fn id_of(fake: &Fake) -> String {
        let target = fake.calls[1].args.last().unwrap();
        target
            .trim_start_matches("zeelin:.kluris-ci/incoming/")
            .trim_end_matches(".tgz")
            .to_owned()
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
            "zeelin's one-run lock is held by a Kluris run (kluris-1-2, process 7): one run at a time, not waiting",
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
                        "zeelin's one-run lock is held by a Kluris run (kluris-1-2, process 7): one run at a time, not waiting.\n\n== exit 75\n",
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
}
