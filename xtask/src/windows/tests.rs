//! Unit tests of `cargo xtask windows`: the words, the machines, the PowerShell
//! the machine is sent, the answer a run ends with, and a whole run against a
//! fake ssh. Nothing here reaches the network; the pack tests run git on a
//! repository made for the case.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use super::machine::{self, FLOOR_GIB, Machine, Ready};
use super::options::{self, Name, Options, Parsed};
use super::remote::{Stream, Streamed};
use super::run::{self, Ctx};
use super::script::{self, Spec};
use crate::ssh_ci::fake::{Answer, Fake, said, timed_out};

fn words(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_owned).collect()
}

/// The script a `powershell -EncodedCommand` line carries, decoded.
fn decoded(line: &str) -> String {
    let encoded = line
        .rsplit(' ')
        .next()
        .expect("the line ends with the encoded script");
    let bytes = STANDARD.decode(encoded).expect("base64");
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16(&units).expect("UTF-16")
}

fn spec<'a>(parity: bool, cargo_args: &'a [String]) -> Spec<'a> {
    Spec {
        id: "kluris-0000abcd-12345678",
        channel: "1.99.0",
        jobs: 2,
        parity,
        parity_files: &[],
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
        ..spec(false, &[])
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
    fn no_words_is_the_default_machine_without_parity() {
        assert_eq!(
            run_of(""),
            Options {
                machine: None,
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
        let options = run_of("--installer-smoke");
        assert!(options.installer_smoke && !options.parity, "{options:?}");
        assert_eq!(options.wrapper, None);
        let options = run_of("--machine zeewin --installer-smoke --wrapper ../site/install.ps1");
        assert_eq!(options.machine, Some(Name::Zeewin));
        assert_eq!(options.wrapper.as_deref(), Some("../site/install.ps1"));
        assert_eq!(
            run_of("--wrapper=x.ps1 --installer-smoke")
                .wrapper
                .as_deref(),
            Some("x.ps1"),
            "the = spelling, in any order"
        );
        for (line, says) in [
            ("--wrapper x.ps1", "--wrapper goes with --installer-smoke"),
            ("--installer-smoke --parity", "not with --parity"),
            ("--installer-smoke -- -p kluris-core", "runs no cargo test"),
            ("--installer-smoke --wrapper", "--wrapper takes"),
            (
                "--installer-smoke --wrapper --machine zeewin",
                "--wrapper takes",
            ),
            ("--installer-smoke=yes", "--installer-smoke takes no value"),
            ("--installer-smoke --wrapper a.ps1 --wrapper b.ps1", "once"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn the_wrapper_is_a_ps1_file_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("install.ps1");
        fs::write(&file, "# the site's wrapper\r\n").unwrap();
        let given = file.to_string_lossy().into_owned();
        assert_eq!(
            options::wrapper_file(&given),
            Ok((
                ".kluris-site/install.ps1".to_owned(),
                "# the site's wrapper\r\n".to_owned()
            ))
        );
        fs::write(dir.path().join("install.sh"), "x").unwrap();
        for (path, says) in [
            (dir.path().join("install.sh"), "a .ps1 file"),
            (dir.path().join("missing.ps1"), "could not read"),
        ] {
            let refused = options::wrapper_file(&path.to_string_lossy()).expect_err("refused");
            assert!(
                refused.contains(says) && refused.contains("--wrapper"),
                "{refused}"
            );
        }
    }

    #[test]
    fn machine_and_parity_in_both_spellings() {
        let options = run_of("--machine zeewin --parity");
        assert_eq!(options.machine, Some(Name::Zeewin));
        assert!(options.parity);
        assert_eq!(run_of("--machine=zeewin2").machine, Some(Name::Zeewin2));
    }

    #[test]
    fn words_after_the_dashes_go_to_cargo_test_as_they_are() {
        let options = run_of("--machine zeewin -- -p kluris-core -- --nocapture --parity");
        assert_eq!(
            options.cargo_args,
            words("-p kluris-core -- --nocapture --parity")
        );
        assert!(!options.parity, "--parity after -- is cargo's");
    }

    #[test]
    fn help() {
        assert_eq!(options::parse(&words("--help")), Ok(Parsed::Help));
        assert_eq!(options::parse(&words("-h")), Ok(Parsed::Help));
    }

    #[test]
    fn refusals() {
        for (line, says) in [
            ("--machine zeewin3", "zeewin, zeewin2"),
            ("--machine", "--machine takes"),
            ("--machine --parity", "--machine takes"),
            ("--bogus", "unknown option: --bogus"),
            ("--parity=yes", "--parity takes no value"),
            ("test", "after --"),
            ("--machine zeewin --machine zeewin2", "once"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn parity_files_need_parity_and_take_paths() {
        let options = run_of("--parity --parity-files parity/test_smoke.py");
        assert_eq!(options.parity_files, words("parity/test_smoke.py"));
        let options =
            run_of("--parity-files parity/a.py,parity/b.py --parity --parity-files parity/c.py");
        assert_eq!(
            options.parity_files,
            words("parity/a.py parity/b.py parity/c.py"),
            "repeated and comma-separated both collect, in the order given"
        );
        assert_eq!(
            run_of("--parity --parity-files=parity/test_dream.py").parity_files,
            words("parity/test_dream.py"),
            "the = spelling too"
        );
        let options = run_of("--parity --parity-files parity/a.py -- -p kluris-core");
        assert_eq!(options.parity_files, words("parity/a.py"));
        assert_eq!(options.cargo_args, words("-p kluris-core"));
        for (line, says) in [
            ("--parity-files parity/test_smoke.py", "--parity"),
            ("--parity --parity-files", "takes"),
            ("--parity --parity-files --machine zeewin", "takes"),
            ("--parity --parity-files=", "takes"),
            ("--parity --parity-files parity/a.py,", "takes"),
        ] {
            let refused = options::parse(&words(line)).expect_err(line);
            assert!(refused.contains(says), "{line:?}: {refused}");
        }
    }

    #[test]
    fn parity_files_are_checked_against_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("parity").join("sub")).unwrap();
        fs::write(root.join("parity").join("test_x.py"), "x").unwrap();
        fs::write(root.join("parity").join("sub").join("test_y.py"), "y").unwrap();
        fs::write(root.join("parity").join("notes.txt"), "n").unwrap();
        let given = |files: &str| -> Vec<String> { files.split(' ').map(str::to_owned).collect() };
        assert_eq!(
            options::checked_parity_files(root, &given("parity/test_x.py parity/sub/test_y.py"))
                .unwrap(),
            words("parity/test_x.py parity/sub/test_y.py"),
            "files under parity/ in subfolders count too"
        );
        assert!(
            options::checked_parity_files(root, &[]).unwrap().is_empty(),
            "no --parity-files is no check"
        );
        for (file, says) in [
            ("test_x.py", "under parity/"),
            ("src/test_x.py", "under parity/"),
            ("parity/../outside.py", "under parity/"),
            ("parity/", "under parity/"),
            ("/parity/test_x.py", "under parity/"),
            ("parity/notes.txt", ".py"),
            ("parity/missing.py", "not in this tree"),
            ("parity", "under parity/"),
        ] {
            let refused = options::checked_parity_files(root, &given(file)).expect_err(file);
            assert!(refused.contains(says), "{file}: {refused}");
            assert!(refused.contains("--parity-files"), "{file}: {refused}");
        }
    }
}

// --- the machines -------------------------------------------------------------

mod machines {
    use super::*;

    fn zeewin2() -> Machine {
        Machine::named(Name::Zeewin2)
    }

    #[test]
    fn the_two_machines() {
        let zeewin = Machine::named(Name::Zeewin);
        assert_eq!((zeewin.name, zeewin.host), ("zeewin", "zeewin"));
        assert!(zeewin.shared.is_none());
        let laptop = zeewin2();
        assert_eq!((laptop.name, laptop.host), ("zeewin2", "zeewin2"));
        let shared = laptop.shared.expect("zeewin2 is shared with poker-lab");
        assert_eq!(
            (shared.linux, shared.lock, shared.floor_gib),
            ("zeelin", ".compile-lock/owner", 50)
        );
        assert_eq!(FLOOR_GIB, 50);
        assert!(zeewin.jobs < laptop.jobs, "zeewin is short of memory");
    }

    #[test]
    fn zeewin_is_asked_nothing_before_a_run() {
        let mut fake = Fake::default();
        let ready = machine::ready(&Machine::named(Name::Zeewin), &mut fake).unwrap();
        assert_eq!(ready, Ready::Go(vec![]));
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn zeewin2_is_refused_while_poker_lab_holds_its_lock() {
        let mut fake = Fake::new(vec![said(0, "held\n")]);
        let Ready::Refused(why) = machine::ready(&zeewin2(), &mut fake).unwrap() else {
            panic!("not refused")
        };
        assert!(
            why.contains("poker-lab") && why.contains("~/.compile-lock/owner"),
            "{why}"
        );
        assert_eq!(fake.calls.len(), 1, "nothing is asked of Windows then");
    }

    #[test]
    fn the_poker_lab_check_is_bounded_and_only_looks() {
        let mut fake = Fake::new(vec![said(0, "free\n"), said(0, "free 600\n")]);
        machine::ready(&zeewin2(), &mut fake).unwrap();
        let zeelin = &fake.calls[0];
        assert_eq!(zeelin.program, "ssh");
        let line = zeelin.args.join(" ");
        assert!(
            line.contains("ConnectTimeout=5") && line.contains("BatchMode=yes"),
            "{line}"
        );
        assert!(zeelin.args.contains(&"zeelin".to_owned()), "{line}");
        assert!(
            zeelin.bound <= Duration::from_secs(20),
            "{:?}",
            zeelin.bound
        );
        let asked = zeelin.args.last().unwrap();
        assert_eq!(
            asked,
            "if [ -e ~/.compile-lock/owner ]; then echo held; else echo free; fi"
        );
        let state = &fake.calls[1];
        assert!(state.bound <= Duration::from_secs(60), "{:?}", state.bound);
        assert!(state.args.contains(&"zeewin2".to_owned()));
    }

    #[test]
    fn zeelin_not_answering_is_ubuntu_not_running_and_passes() {
        for zeelin in [
            said(255, ""),
            timed_out(),
            Answer::Capture(Err("no ssh".into())),
        ] {
            let mut fake = Fake::new(vec![zeelin, said(0, "free 600\n")]);
            let Ready::Go(notes) = machine::ready(&zeewin2(), &mut fake).unwrap() else {
                panic!("refused")
            };
            assert_eq!(notes.len(), 1, "{notes:?}");
            assert!(notes[0].contains("Ubuntu is not running"), "{notes:?}");
        }
    }

    #[test]
    fn zeewin2_is_refused_under_the_disk_floor() {
        let mut fake = Fake::new(vec![said(0, "free\n"), said(0, "free 49\r\n")]);
        let Ready::Refused(why) = machine::ready(&zeewin2(), &mut fake).unwrap() else {
            panic!("not refused")
        };
        assert!(why.contains("50 GiB") && why.contains("49"), "{why}");

        let mut fake = Fake::new(vec![said(0, "free\n"), said(0, "free 50\r\n")]);
        assert_eq!(
            machine::ready(&zeewin2(), &mut fake).unwrap(),
            Ready::Go(vec![])
        );
    }

    #[test]
    fn a_held_run_lock_is_left_to_the_lock_itself() {
        let mut fake = Fake::new(vec![
            said(0, "free\n"),
            said(0, "free 600\r\nheld 0000abcd-12345678 4242\r\n"),
        ]);
        assert_eq!(
            machine::ready(&zeewin2(), &mut fake).unwrap(),
            Ready::Go(vec![])
        );
    }

    #[test]
    fn a_windows_that_does_not_answer_is_an_error() {
        let mut fake = Fake::new(vec![said(0, "free\n"), said(255, "")]);
        let error = machine::ready(&zeewin2(), &mut fake).unwrap_err();
        assert!(error.contains("zeewin2"), "{error}");
        let mut fake = Fake::new(vec![said(0, "free\n"), said(0, "nonsense")]);
        assert!(machine::ready(&zeewin2(), &mut fake).is_err());
    }

    #[test]
    fn the_state_script_reads_c_and_the_shared_lock_owner() {
        let script = script::state_script();
        assert!(script.contains("Get-PSDrive -Name C"), "{script}");
        assert!(
            script.contains(r"consensflow-build\.windows-run.id"),
            "{script}"
        );
        assert!(!script.contains("Open("), "the probe never opens the lock");
    }

    fn chosen(answers: Vec<Answer>) -> (Machine, String, Fake) {
        let mut fake = Fake::new(answers);
        let (machine, why) = machine::choose(&mut fake);
        (machine, why, fake)
    }

    #[test]
    fn the_default_is_zeewin2_while_it_is_free_and_allowed() {
        let (machine, why, _) = chosen(vec![said(0, "free\n"), said(0, "free 600\n")]);
        assert_eq!(machine.name, "zeewin2");
        assert_eq!(why, "windows: on zeewin2");
    }

    #[test]
    fn the_default_is_zeewin_when_zeewin2_is_not() {
        for (answers, says) in [
            (vec![said(0, "held\n")], "poker-lab"),
            (vec![said(0, "free\n"), said(0, "free 10\n")], "50 GiB"),
            (
                vec![
                    said(0, "free\n"),
                    said(0, "free 600\nheld 0000abcd-12345678 4242\n"),
                ],
                "0000abcd-12345678",
            ),
            (vec![said(0, "free\n"), said(255, "")], "did not say"),
        ] {
            let (machine, why, _) = chosen(answers);
            assert_eq!(machine.name, "zeewin", "{why}");
            assert!(
                why.starts_with("windows: on zeewin, as zeewin2 is not free: "),
                "{why}"
            );
            assert!(why.contains(says), "{why}");
        }
    }
}

// --- the scripts ----------------------------------------------------------------

mod scripts {
    use super::*;

    /// Words no script may hold: they would change the machine's settings, its
    /// PATH, Defender, the firewall or the registry, install software, or log in.
    const FORBIDDEN: [&str; 22] = [
        "setx",
        "SetEnvironmentVariable",
        "Set-ItemProperty",
        "New-ItemProperty",
        "reg add",
        "reg.exe",
        "HKLM",
        "HKCU",
        "netsh",
        "Set-MpPreference",
        "Add-MpPreference",
        "NetFirewall",
        "Set-ExecutionPolicy",
        "msiexec",
        "winget",
        "choco",
        "RunAs",
        "rustup-init",
        "self update",
        "rustup default",
        "login",
        "Register-ScheduledTask",
    ];

    fn every_script() -> Vec<(&'static str, String)> {
        let extra = words("-p kluris-core");
        vec![
            ("bootstrap", script::bootstrap("kluris-0000abcd-12345678")),
            ("run", script::run_script(&spec(false, &[]))),
            ("run --parity", script::run_script(&spec(true, &extra))),
            (
                "run --installer-smoke",
                script::run_script(&smoke_spec(true)),
            ),
            ("stop", script::stop_script("kluris-0000abcd-12345678")),
            (
                "cleanup",
                script::cleanup_script("kluris-0000abcd-12345678"),
            ),
            ("state", script::state_script()),
        ]
    }

    #[test]
    fn no_script_changes_the_machine() {
        for (name, text) in every_script() {
            let lower = text.to_lowercase();
            for word in FORBIDDEN {
                assert!(
                    !lower.contains(&word.to_lowercase()),
                    "{name} holds {word:?}"
                );
            }
            // PATH is changed for the run's own process only.
            for (at, _) in text.match_indices("Path") {
                let line = text[..at].rsplit('\n').next().unwrap_or("").to_owned()
                    + text[at..].split('\n').next().unwrap_or("");
                if line.contains("Path =") {
                    assert!(
                        line.trim_start().starts_with("$env:Path = "),
                        "{name}: {line}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_bootstrap_takes_the_shared_lock_first() {
        let text = script::bootstrap("kluris-0000abcd-12345678");
        let at = |what: &str| {
            text.find(what)
                .unwrap_or_else(|| panic!("the bootstrap has no {what:?}:\n{text}"))
        };
        let open = at("[IO.File]::Open(");
        assert!(text[open..].starts_with(
            "[IO.File]::Open((Join-Path $lockdir '.windows-run.lock'), 'OpenOrCreate', 'ReadWrite', 'None')"
        ));
        assert!(text.contains("$lockdir = Join-Path $env:USERPROFILE 'consensflow-build'"));
        for later in [
            "Set-Content",
            "tar.exe",
            "-xmzf",
            "$run =",
            ".kluris-run.ps1",
        ] {
            assert!(open < at(later), "{later:?} comes before the lock");
        }
        // Before the lock only the lock's own folder is made.
        let before = &text[..open];
        assert_eq!(before.matches("New-Item").count(), 1, "{before}");
        assert!(before.contains("New-Item -ItemType Directory -Force $lockdir"));
    }

    #[test]
    fn a_held_lock_is_refused_with_75_naming_the_holder() {
        let text = script::bootstrap("kluris-0000abcd-12345678");
        assert!(text.contains("Exit-Run 75"), "{text}");
        assert!(text.contains("a ConsensFlow run"), "{text}");
        assert!(text.contains("a Kluris run"), "{text}");
        // ConsensFlow's own record of an older run is honoured as its scripts do.
        assert!(
            text.contains("$_.StartTime -le (Get-Item $running).LastWriteTime"),
            "{text}"
        );
        for never in ["Start-Sleep", "while", "Remove-Item -LiteralPath $lockfile"] {
            assert!(!text.contains(never), "the bootstrap holds {never:?}");
        }
    }

    #[test]
    fn the_bootstrap_cleans_up_in_a_finally() {
        let text = script::bootstrap("kluris-0000abcd-12345678");
        let finally = text.find("} finally {").expect("a finally");
        let cleanup = &text[finally..];
        assert!(
            cleanup.contains("Remove-Item -LiteralPath $tarball -Force"),
            "{cleanup}"
        );
        assert!(cleanup.contains("$lock.Dispose()"), "{cleanup}");
        assert!(
            cleanup.contains("if ($held)"),
            "only its own records: {cleanup}"
        );
        assert!(
            text.contains("$tarball = Join-Path $env:USERPROFILE 'kluris-0000abcd-12345678.tgz'")
        );
        assert!(text.contains(
            r"$run = Join-Path $env:USERPROFILE '.kluris-ci\runs\kluris-0000abcd-12345678'"
        ));
        assert!(
            text.contains("Set-Content -Path $owner -Value @('kluris-0000abcd-12345678', $PID)")
        );
    }

    #[test]
    fn what_goes_on_the_command_line_fits_it() {
        for text in [
            script::bootstrap("kluris-0000abcd-12345678"),
            script::stop_script("kluris-0000abcd-12345678"),
            script::state_script(),
        ] {
            let line = script::powershell_line(&text).unwrap();
            assert!(line.starts_with("powershell -NoProfile -NonInteractive -EncodedCommand "));
            assert!(line.len() <= script::ENCODED_LIMIT + 60, "{}", line.len());
            assert_eq!(decoded(&line), text);
        }
        let long = "x".repeat(script::ENCODED_LIMIT);
        assert!(script::powershell_line(&long).is_err());
    }

    #[test]
    fn the_run_is_bounded_in_processes_and_time() {
        let text = script::run_script(&spec(true, &[]));
        assert!(text.contains(&format!("[KlurisJob]::Enter({})", script::PROCESS_LIMIT)));
        const { assert!(script::PROCESS_LIMIT <= 64) };
        assert!(
            text.contains("0x2000 | 0x8"),
            "kill on close and the process limit"
        );
        assert!(text.contains("Exit-Run 1"), "no run without its job");
        assert!(text.contains("WaitForExit($minutes * 60000)"), "{text}");
        assert!(text.contains("taskkill /PID $process.Id /T /F"), "{text}");
        for step in script::steps(&spec(true, &[])) {
            assert!(step.minutes > 0 && step.minutes <= 120, "{step:?}");
            assert!(
                text.contains(&format!("Invoke-Step '{}' {} ", step.name, step.minutes)),
                "{step:?} is not run with its bound"
            );
        }
        let all: u32 = script::steps(&spec(true, &[]))
            .iter()
            .map(|s| s.minutes)
            .sum();
        assert!(script::bound(&spec(true, &[])) > Duration::from_secs(u64::from(all) * 60));
        assert!(script::bound(&spec(true, &[])) < Duration::from_secs(8 * 3600));
    }

    #[test]
    fn the_steps_of_a_run() {
        let names = |parity| {
            script::steps(&spec(parity, &[]))
                .iter()
                .map(|step| step.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(false), ["toolchain", "cargo-test"]);
        assert_eq!(
            names(true),
            ["toolchain", "cargo-test", "release-build", "parity"]
        );
        let smoke: Vec<_> = script::steps(&smoke_spec(false))
            .iter()
            .map(|step| step.name)
            .collect();
        assert_eq!(
            smoke,
            [
                "toolchain",
                "python",
                "release-dist",
                "installer-smoke",
                "release-versions",
                "lifecycle"
            ]
        );
    }

    #[test]
    fn an_installer_smoke_is_bounded_too() {
        let spec = smoke_spec(true);
        let text = script::run_script(&spec);
        for step in script::steps(&spec) {
            assert!(step.minutes > 0 && step.minutes <= 120, "{step:?}");
            assert!(
                text.contains(&format!("Invoke-Step '{}' {} ", step.name, step.minutes)),
                "{step:?} is not run with its bound:\n{text}"
            );
        }
        let all: u32 = script::steps(&spec).iter().map(|s| s.minutes).sum();
        assert!(script::bound(&spec) > Duration::from_secs(u64::from(all) * 60));
        assert!(script::bound(&spec) < script::bound(&spec_with_parity()));
    }

    fn spec_with_parity() -> Spec<'static> {
        spec(true, &[])
    }

    #[test]
    fn an_installer_smoke_builds_the_release_and_runs_the_smoke_script() {
        let text = script::run_script(&smoke_spec(false));
        for line in [
            "$env:KLURIS_TOOLS_DIR = $tools",
            "$env:UV_PYTHON_INSTALL_DIR = Join-Path $tools 'python'",
            "$env:UV_PYTHON_PREFERENCE = 'only-managed'",
            "Invoke-Step 'python' 10 $uv 'python install 3.11'",
            "Invoke-Step 'release-dist' 60 $powershell '-NoProfile -ExecutionPolicy Bypass -File scripts\\release\\build-local.ps1'",
            "Invoke-Step 'installer-smoke' 20 $powershell \"-NoProfile -ExecutionPolicy Bypass -File scripts\\release\\installer-smoke.ps1 -Release target\\distrib -Python `\"$python`\" -Record `\"$record`\"\"",
            "$record = Join-Path $run 'user-path.saved'",
        ] {
            assert!(text.contains(line), "no {line:?} in\n{text}");
        }
        assert!(!text.contains("-Wrapper"), "{text}");
        assert!(
            !text.contains("Invoke-Step 'cargo-test'") && !text.contains("pytest"),
            "an installer smoke runs no tests: {text}"
        );
        // Whatever the smoke step's end (past its bound it is killed, its
        // `finally` never runs), the user PATH it saved is written back.
        let smoke = text.find("Invoke-Step 'installer-smoke'").unwrap();
        let restore = text
            .find("scripts\\release\\user-path.ps1 -Restore")
            .expect("the run restores the saved user PATH");
        assert!(restore > smoke, "{text}");
        assert!(
            text[smoke..].contains("if (Test-Path -LiteralPath $record)"),
            "{text}"
        );

        let with = script::run_script(&smoke_spec(true));
        assert!(
            with.contains("-Wrapper .kluris-site\\install.ps1"),
            "{with}"
        );
    }

    #[test]
    fn an_installer_smoke_runs_the_life_cycle_on_two_built_releases() {
        let text = script::run_script(&smoke_spec(false));
        for line in [
            "Invoke-Step 'release-versions' 60 $powershell '-NoProfile -ExecutionPolicy Bypass -File scripts\\release\\build-versions.ps1 -Out target\\releases -Versions 3.0.0,3.0.1'",
            "Invoke-Step 'lifecycle' 20 $powershell \"-NoProfile -ExecutionPolicy Bypass -File scripts\\release\\lifecycle-smoke.ps1 -Releases target\\releases -Old 3.0.0 -New 3.0.1 -Python `\"$python`\" -Record `\"$record`\"\"",
        ] {
            assert!(text.contains(line), "no {line:?} in\n{text}");
        }
        // The life cycle runs whatever the installer smoke's end, after its
        // PATH record is written back, and its own record is written back too.
        let smoke = text.find("Invoke-Step 'installer-smoke'").unwrap();
        let versions = text.find("Invoke-Step 'release-versions'").unwrap();
        let lifecycle = text.find("Invoke-Step 'lifecycle'").unwrap();
        assert!(smoke < versions && versions < lifecycle, "{text}");
        let restores: Vec<usize> = text
            .match_indices("scripts\\release\\user-path.ps1 -Restore")
            .map(|(at, _)| at)
            .collect();
        assert!(
            restores.iter().any(|at| *at > smoke && *at < versions),
            "{text}"
        );
        assert!(restores.iter().any(|at| *at > lifecycle), "{text}");
        assert!(
            !text[smoke..versions].contains("if ($python -and $state.last -eq 0) { Invoke-Step 'release-versions'"),
            "the life cycle does not wait on the installer smoke's result: {text}"
        );
    }

    #[test]
    fn the_stop_restores_the_user_path_an_ended_smoke_left() {
        let text = script::stop_script("kluris-0000abcd-12345678");
        assert!(
            text.contains("runs\\kluris-0000abcd-12345678\\user-path.saved"),
            "{text}"
        );
        assert!(
            text.contains("scripts\\release\\user-path.ps1") && text.contains("-Restore"),
            "{text}"
        );
        assert!(text.trim_end().ends_with("exit 0"));
    }

    #[test]
    fn tools_and_caches_stay_in_klurises_own_folder() {
        let text = script::run_script(&spec(true, &[]));
        for line in [
            "$ci = Join-Path $env:USERPROFILE '.kluris-ci'",
            "$env:RUSTUP_HOME = Join-Path $tools 'rustup'",
            "$env:CARGO_HOME = Join-Path $tools 'cargo'",
            "$env:CARGO_TARGET_DIR = Join-Path $cache 'target'",
            "$env:CARGO_BUILD_JOBS = '2'",
            "$env:UV_CACHE_DIR = Join-Path $cache 'uv'",
            "$env:UV_PYTHON_INSTALL_DIR = Join-Path $tools 'python'",
            "$env:UV_PYTHON_PREFERENCE = 'only-managed'",
            "$env:UV_PROJECT_ENVIRONMENT = Join-Path $cache 'parity-venv'",
            "$env:KLURIS_PARITY_ORACLE_DIR = Join-Path $cache 'oracle'",
            r"$env:KLURIS_V3_BIN = Join-Path $env:CARGO_TARGET_DIR 'release\kluris.exe'",
        ] {
            assert!(text.contains(line), "no {line:?} in\n{text}");
        }
        assert!(text.contains(script::UV_SHA256) && text.contains(script::UV_VERSION));
        assert!(text.contains("Get-FileHash -Algorithm SHA256"));
        assert!(
            text.contains("Missing on"),
            "a machine without rustup is told, not fixed"
        );
    }

    #[test]
    fn the_commands_of_a_run() {
        let extra = words("-p kluris-core --test list -- --nocapture \"two words\"");
        let text = script::run_script(&spec(true, &extra));
        assert!(text.contains(
            "Invoke-Step 'toolchain' 30 $rustup 'toolchain install 1.99.0 --profile minimal --component clippy --component rustfmt --no-self-update'"
        ), "{text}");
        assert!(text.contains(
            "$cargo 'test --workspace --no-fail-fast -p kluris-core --test list -- --nocapture \"\\\"two\" \"words\\\"\"'"
        ), "{text}");
        assert!(
            text.contains("$cargo 'build --release --package kluris'"),
            "{text}"
        );
        assert!(text.contains("$uv 'run --project parity --locked pytest parity -q'"));
        let plain = script::run_script(&spec(false, &[]));
        assert!(
            plain.contains("$cargo 'test --workspace --no-fail-fast'"),
            "{plain}"
        );
        assert!(!plain.contains("pytest") && !plain.contains("uv.exe"));
    }

    #[test]
    fn the_parity_step_runs_the_files_parity_files_named() {
        let files = words("parity/test_smoke.py parity/test_dream.py");
        let text = script::run_script(&Spec {
            parity_files: &files,
            ..spec(true, &[])
        });
        assert!(
            text.contains(
                "Invoke-Step 'parity' 90 $uv 'run --project parity --locked pytest parity/test_smoke.py parity/test_dream.py -q'"
            ),
            "{text}"
        );
        assert!(
            !text.contains("pytest parity -q"),
            "the named files replace the whole-suite argument: {text}"
        );
        // With none named the whole suite runs, as before.
        let text = script::run_script(&spec(true, &[]));
        assert!(
            text.contains(
                "Invoke-Step 'parity' 90 $uv 'run --project parity --locked pytest parity -q'"
            ),
            "{text}"
        );
    }

    #[test]
    fn old_runs_are_swept_under_the_lock() {
        let text = script::run_script(&spec(false, &[]));
        assert!(text.contains("(Get-Date).AddDays(-1)"), "{text}");
        assert!(
            text.contains("$_.Name -ne 'kluris-0000abcd-12345678'"),
            "{text}"
        );
        assert!(text.contains("-Filter 'kluris-*.tgz'"), "{text}");
    }

    #[test]
    fn the_upload_cleanup_removes_only_its_own_tarball() {
        let text = script::cleanup_script("kluris-0000abcd-12345678");
        assert!(
            text.contains("$tarball = Join-Path $env:USERPROFILE 'kluris-0000abcd-12345678.tgz'"),
            "{text}"
        );
        assert_eq!(text.matches("Remove-Item").count(), 1, "{text}");
        assert!(
            text.contains("Remove-Item -LiteralPath $tarball -Force"),
            "{text}"
        );
        for never in ["consensflow-build", "Open(", "taskkill", "Recurse", "*"] {
            assert!(!text.contains(never), "the cleanup holds {never:?}: {text}");
        }
        for said in ["'removed'", "'absent'", "\"failed: "] {
            assert!(text.contains(said), "{said}: {text}");
        }
        let line = script::powershell_line(&text).unwrap();
        assert_eq!(decoded(&line), text);
    }

    #[test]
    fn the_stop_ends_only_its_own_run() {
        let text = script::stop_script("kluris-0000abcd-12345678");
        assert!(
            text.contains(
                "if ($of[0] -eq 'kluris-0000abcd-12345678') { taskkill /PID ($of[1]) /T /F"
            )
        );
        assert!(text.contains("'kluris-0000abcd-12345678.tgz'"));
        assert!(text.trim_end().ends_with("exit 0"));
    }

    #[test]
    fn run_ids_are_unique_and_never_consensflows() {
        let a = script::run_id();
        let b = script::run_id();
        assert_ne!(a, b);
        for id in [a, b] {
            let hex = id.strip_prefix("kluris-").expect("a Kluris id");
            let parts: Vec<&str> = hex.split('-').collect();
            assert_eq!(parts.len(), 2, "{id}");
            assert!(
                parts
                    .iter()
                    .all(|p| p.len() == 8 && p.bytes().all(|b| b.is_ascii_hexdigit())),
                "{id}"
            );
        }
        assert_eq!(script::tarball_name("kluris-1-2"), "kluris-1-2.tgz");
    }

    #[test]
    fn exit_lines() {
        assert_eq!(script::exit_said("== exit 0"), Some(0));
        assert_eq!(script::exit_said("== exit 75\r"), Some(75));
        assert_eq!(
            script::exit_said("== exit -1073741819"),
            Some(-1_073_741_819)
        );
        for line in [
            "== exit",
            "== exit x",
            "== exit 7 8",
            " == exit 7",
            "== exit -",
        ] {
            assert_eq!(script::exit_said(line), None, "{line:?}");
        }
    }

    #[test]
    fn windows_command_lines() {
        for (args, line) in [
            (vec!["a", "b"], "a b"),
            (vec!["two words"], "\"two words\""),
            (vec![""], "\"\""),
            (vec!["say \"hi\""], "\"say \\\"hi\\\"\""),
            (vec![r"C:\a b\"], "\"C:\\a b\\\\\""),
            (vec![r"a\\b"], r"a\\b"),
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            assert_eq!(script::command_line(&args), line, "{args:?}");
        }
    }
}

// --- the answer -------------------------------------------------------------------

mod answers {
    use super::*;

    #[test]
    fn the_machine_says_the_status() {
        assert_eq!(run::answer(Some(1), Some(75), false).0, 75);
        assert_eq!(run::answer(Some(1), Some(101), true).0, 101);
        assert_eq!(run::answer(Some(0), Some(0), true), (0, None));
    }

    #[test]
    fn a_pass_needs_the_tree_said() {
        let (code, note) = run::answer(Some(0), Some(0), false);
        assert_eq!(code, 1);
        assert!(note.unwrap().contains("== tree"));
    }

    #[test]
    fn statuses_no_program_ends_with_are_1() {
        let (code, note) = run::answer(Some(1), Some(-1_073_741_819), true);
        assert_eq!(code, 1);
        assert!(note.unwrap().contains("-1073741819"));
    }

    #[test]
    fn without_the_line_ssh_is_the_answer_but_never_0() {
        assert_eq!(run::answer(Some(255), None, false).0, 255);
        assert_eq!(run::answer(Some(0), None, true).0, 1);
        assert_eq!(run::answer(None, None, false).0, 1);
        assert!(run::answer(Some(255), None, false).1.is_some());
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
        packed: Vec<PathBuf>,
        run_script: String,
    }

    fn ran(options: Options, answers: Vec<Answer>) -> Ran {
        let dir = tempfile::tempdir().unwrap();
        ran_rooted(dir.path(), options, answers)
    }

    fn ran_rooted(root: &Path, options: Options, answers: Vec<Answer>) -> Ran {
        let mut fake = Fake::new(answers);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut packed = Vec::new();
        let mut run_script = String::new();
        let code = {
            let mut pack = |tarball: &Path, script: &str| -> Result<String, String> {
                assert!(script.contains("Invoke-Step 'cargo-test'"));
                run_script = script.to_owned();
                packed.push(tarball.to_path_buf());
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
            run::windows(&options, &mut ctx)
        };
        Ran {
            code,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
            fake,
            packed,
            run_script,
        }
    }

    fn on(name: Name, parity: bool) -> Options {
        Options {
            machine: Some(name),
            parity,
            parity_files: vec![],
            cargo_args: vec![],
            installer_smoke: false,
            wrapper: None,
        }
    }

    const PASSED: &str = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\
        == step toolchain: rustup toolchain install\r\n\
        == step toolchain exit 0 3s\r\n\
        test result: ok. 12 passed\n\
        == step cargo-test exit 0 412s\r\n\
        \r\n== exit 0\r\n";

    #[test]
    fn a_run_on_zeewin_sends_the_tree_and_streams_the_answer() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![
                said(0, ""),
                Answer::Stream(
                    vec![(Stream::Out, PASSED), (Stream::Err, "warning: x\n")],
                    Ok(Streamed::Exited(Some(1))),
                ),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(ran.out.contains("test result: ok. 12 passed"));
        assert!(ran.err.contains("warning: x"));
        let calls = &ran.fake.calls;
        assert_eq!(calls.len(), 2, "{:?}", ran.fake.called());
        assert_eq!(calls[0].program, "scp");
        let target = calls[0].args.last().unwrap();
        assert!(
            target.starts_with("zeewin:kluris-") && target.ends_with(".tgz"),
            "{target}"
        );
        assert_eq!(
            calls[0].args[calls[0].args.len() - 2],
            ran.packed[0].to_string_lossy()
        );
        assert_eq!(calls[1].program, "ssh");
        assert!(calls[1].args.contains(&"zeewin".to_owned()));
        let line = calls[1].args.join(" ");
        assert!(line.contains("ServerAliveInterval"), "{line}");
        let bootstrap = decoded(calls[1].args.last().unwrap());
        let id = target
            .trim_start_matches("zeewin:")
            .trim_end_matches(".tgz");
        assert_eq!(bootstrap, script::bootstrap(id));
        assert!(calls[1].bound >= script::bound(&spec(false, &[])));
        let summary = ran.err.lines().last().unwrap();
        assert!(
            summary.starts_with(&format!("windows: zeewin run {id} tree 5f54983aaaaa")),
            "{summary}"
        );
        assert!(
            summary.contains("toolchain 0 (3s), cargo-test 0 (412s)"),
            "{summary}"
        );
        assert!(summary.ends_with("exit 0"), "{summary}");
    }

    #[test]
    fn an_installer_smoke_run_sends_the_smoke_and_waits_its_bound() {
        let passed = "== tree 5f54983aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\
            == step toolchain exit 0 3s\r\n\
            == step python exit 0 9s\r\n\
            == step release-dist exit 0 300s\r\n\
            installer-smoke: user PATH restored: 120 bytes, REG_EXPAND_SZ, byte for byte identical to the saved value\r\n\
            == step installer-smoke exit 0 40s\r\n\
            \r\n== exit 0\r\n";
        let mut fake = Fake::new(vec![
            said(0, ""),
            Answer::Stream(vec![(Stream::Out, passed)], Ok(Streamed::Exited(Some(1)))),
        ]);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut run_script = String::new();
        let dir = tempfile::tempdir().unwrap();
        let options = Options {
            installer_smoke: true,
            wrapper: Some("site/install.ps1".into()),
            ..on(Name::Zeewin, false)
        };
        let code = {
            let mut pack = |tarball: &Path, script: &str| -> Result<String, String> {
                run_script = script.to_owned();
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
                root: dir.path(),
            };
            run::windows(&options, &mut ctx)
        };
        let err = String::from_utf8(err).unwrap();
        assert_eq!(code, 0, "{err}");
        assert!(
            run_script.contains("installer-smoke.ps1 -Release"),
            "{run_script}"
        );
        assert!(
            run_script.contains("-Wrapper .kluris-site\\install.ps1"),
            "{run_script}"
        );
        assert!(
            !run_script.contains("Invoke-Step 'cargo-test'"),
            "{run_script}"
        );
        let ssh = &fake.calls[1];
        assert!(
            ssh.bound >= script::bound(&smoke_spec(true)),
            "{:?}",
            ssh.bound
        );
        assert!(
            ssh.bound < script::bound(&spec(true, &[])),
            "{:?}",
            ssh.bound
        );
        let summary = err.lines().last().unwrap();
        assert!(
            summary.contains("release-dist 0 (300s), installer-smoke 0 (40s)"),
            "{summary}"
        );
    }

    #[test]
    fn a_held_lock_is_75_with_the_holder_named() {
        let ran = ran(
            on(Name::Zeewin, true),
            vec![
                said(0, ""),
                Answer::Stream(
                    vec![(
                        Stream::Out,
                        "The machine's one-run lock is held by a ConsensFlow run (0000abcd-12345678, process 4242): one run at a time, not waiting.\r\n\n== exit 75\r\n",
                    )],
                    Ok(Streamed::Exited(Some(1))),
                ),
            ],
        );
        assert_eq!(ran.code, 75);
        assert!(ran.out.contains("ConsensFlow run (0000abcd-12345678"));
        assert!(ran.err.lines().last().unwrap().ends_with("exit 75"));
        assert_eq!(ran.fake.calls.len(), 2, "nothing waits, nothing retries");
    }

    #[test]
    fn zeewin2_refused_sends_nothing_and_is_75() {
        let ran = ran(on(Name::Zeewin2, true), vec![said(0, "held\n")]);
        assert_eq!(ran.code, 75);
        assert!(ran.err.contains("poker-lab"), "{}", ran.err);
        assert_eq!(ran.fake.calls.len(), 1);
        assert!(ran.packed.is_empty());

        let ran = ran_low_disk();
        assert_eq!(ran.code, 75);
        assert!(ran.err.contains("50 GiB"), "{}", ran.err);
    }

    fn ran_low_disk() -> Ran {
        ran(
            on(Name::Zeewin2, false),
            vec![said(0, "free\n"), said(0, "free 12\n")],
        )
    }

    #[test]
    fn a_refused_parity_file_sends_nothing() {
        let ran = ran(
            Options {
                machine: Some(Name::Zeewin),
                parity: true,
                parity_files: words("parity/not_there.py"),
                cargo_args: vec![],
                installer_smoke: false,
                wrapper: None,
            },
            vec![],
        );
        assert_eq!(ran.code, 1);
        assert!(
            ran.err.contains("--parity-files") && ran.err.contains("parity/not_there.py"),
            "{}",
            ran.err
        );
        assert!(
            ran.fake.calls.is_empty() && ran.packed.is_empty(),
            "refused before the machine is even asked: {:?}",
            ran.fake.called()
        );
    }

    #[test]
    fn the_parity_files_reach_the_remote_pytest_line() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("parity")).unwrap();
        fs::write(root.join("parity").join("test_smoke.py"), "x").unwrap();
        let ran = ran_rooted(
            root,
            Options {
                machine: Some(Name::Zeewin),
                parity: true,
                parity_files: words("parity/test_smoke.py"),
                cargo_args: vec![],
                installer_smoke: false,
                wrapper: None,
            },
            vec![
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, PASSED)], Ok(Streamed::Exited(Some(0)))),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(
            ran.run_script
                .contains("$uv 'run --project parity --locked pytest parity/test_smoke.py -q'"),
            "{}",
            ran.run_script
        );
    }

    #[test]
    fn no_machine_takes_zeewin_when_zeewin2_is_held() {
        let ran = ran(
            Options {
                machine: None,
                parity: false,
                parity_files: vec![],
                cargo_args: vec![],
                installer_smoke: false,
                wrapper: None,
            },
            vec![
                said(0, "held\n"),
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, PASSED)], Ok(Streamed::Exited(Some(0)))),
            ],
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(
            ran.err
                .starts_with("windows: on zeewin, as zeewin2 is not free"),
            "{}",
            ran.err
        );
        assert!(
            ran.fake.calls[1]
                .args
                .last()
                .unwrap()
                .starts_with("zeewin:")
        );
    }

    #[test]
    fn a_failed_upload_runs_nothing_and_removes_its_tarball() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![said(1, ""), said(0, "removed\r\n")],
        );
        assert_eq!(ran.code, 1);
        assert!(ran.err.contains("scp"), "{}", ran.err);
        let calls = &ran.fake.calls;
        assert_eq!(calls.len(), 2, "{:?}", ran.fake.called());
        let id = calls[0]
            .args
            .last()
            .unwrap()
            .trim_start_matches("zeewin:")
            .trim_end_matches(".tgz")
            .to_owned();
        let cleanup = &calls[1];
        assert_eq!(cleanup.program, "ssh");
        assert!(cleanup.args.contains(&"zeewin".to_owned()));
        assert!(
            cleanup.bound <= Duration::from_secs(30),
            "{:?}",
            cleanup.bound
        );
        let line = cleanup.args.join(" ");
        assert!(
            line.contains("BatchMode=yes") && line.contains("ConnectTimeout="),
            "{line}"
        );
        assert_eq!(
            decoded(cleanup.args.last().unwrap()),
            script::cleanup_script(&id)
        );
        assert!(
            ran.err.contains(&format!("{id}.tgz")) && ran.err.contains("removed"),
            "{}",
            ran.err
        );
    }

    #[test]
    fn an_upload_cleanup_that_finds_nothing_says_so() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![said(1, ""), said(0, "absent\n")],
        );
        assert_eq!(ran.code, 1);
        assert!(ran.err.contains("was not there"), "{}", ran.err);
    }

    #[test]
    fn an_upload_cleanup_that_cannot_reach_the_machine_says_so() {
        for cleanup in [said(255, ""), timed_out()] {
            let ran = ran(
                on(Name::Zeewin2, false),
                vec![
                    said(0, "free\n"),
                    said(0, "free 600\n"),
                    timed_out(),
                    cleanup,
                ],
            );
            assert_eq!(ran.code, 1);
            assert_eq!(ran.fake.calls.len(), 4, "{:?}", ran.fake.called());
            assert!(ran.err.contains("not done in 10 min"), "{}", ran.err);
            assert!(ran.err.contains("could not reach zeewin2"), "{}", ran.err);
            assert!(ran.err.contains("sweep"), "{}", ran.err);
        }
    }

    #[test]
    fn an_upload_cleanup_that_fails_there_says_why() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![said(1, ""), said(0, "failed: access denied\n")],
        );
        assert_eq!(ran.code, 1);
        assert!(
            ran.err.contains("could not be removed: access denied"),
            "{}",
            ran.err
        );
    }

    fn stop_sent(fake: &Fake) -> String {
        let stop = fake.calls.last().unwrap();
        assert_eq!(stop.program, "ssh");
        assert!(stop.bound <= Duration::from_secs(120));
        decoded(stop.args.last().unwrap())
    }

    #[test]
    fn a_stopped_run_is_ended_there_and_is_130() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![
                said(0, ""),
                Answer::Stream(vec![(Stream::Out, "== tree x\n")], Ok(Streamed::Stopped)),
                said(0, ""),
            ],
        );
        assert_eq!(ran.code, 130);
        let id = ran.fake.calls[0]
            .args
            .last()
            .unwrap()
            .trim_start_matches("zeewin:")
            .trim_end_matches(".tgz")
            .to_owned();
        assert_eq!(stop_sent(&ran.fake), script::stop_script(&id));
    }

    #[test]
    fn a_run_past_its_bound_is_ended_there_and_is_124() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![
                said(0, ""),
                Answer::Stream(vec![], Ok(Streamed::TimedOut)),
                said(0, ""),
            ],
        );
        assert_eq!(ran.code, 124);
        assert!(stop_sent(&ran.fake).contains("taskkill"));
        assert!(ran.err.contains("bound"), "{}", ran.err);
    }

    #[test]
    fn a_lost_connection_ends_the_run_there_too() {
        let ran = ran(
            on(Name::Zeewin, false),
            vec![
                said(0, ""),
                Answer::Stream(
                    vec![(Stream::Out, "== tree x\nCompiling")],
                    Ok(Streamed::Exited(Some(255))),
                ),
                said(255, ""),
            ],
        );
        assert_eq!(ran.code, 255);
        stop_sent(&ran.fake);
        assert!(ran.err.contains("may still be going on"), "{}", ran.err);
    }
}

// --- the tarball --------------------------------------------------------------------

mod tarball {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    }

    fn tar_program() -> PathBuf {
        match std::env::var_os("SystemRoot") {
            Some(root) if cfg!(windows) => Path::new(&root).join("System32").join("tar.exe"),
            _ => PathBuf::from("tar"),
        }
    }

    /// `name` from the tarball, or none.
    fn member(tarball: &Path, name: &str) -> Option<String> {
        let out = Command::new(tar_program())
            .arg("-xOzf")
            .arg(tarball)
            .arg(name)
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8(out.stdout).unwrap())
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "target/\nparity/.oracle/\n").unwrap();
        fs::write(root.join("kept.txt"), "committed\n").unwrap();
        fs::create_dir_all(root.join("parity")).unwrap();
        fs::write(root.join("parity").join("harness.py"), "old\n").unwrap();
        git(root, &["init", "-q"]);
        git(root, &["add", "-A"]);
        git(root, &["commit", "-q", "-m", "one"]);
        dir
    }

    #[test]
    fn a_clean_tree_is_its_head() {
        let dir = repo();
        let tarball = dir.path().join("out.tgz");
        let head = git(dir.path(), &["rev-parse", "HEAD"]);
        let commit = super::super::pack::pack(dir.path(), &tarball, "the run script\n").unwrap();
        assert_eq!(commit, head);
        assert_eq!(member(&tarball, "kept.txt").as_deref(), Some("committed\n"));
        assert_eq!(
            member(&tarball, ".kluris-run.ps1").as_deref(),
            Some("the run script\n")
        );
        assert_eq!(member(&tarball, ".kluris-tree"), Some(format!("{head}\n")));
    }

    #[test]
    fn files_beside_the_tree_go_in_the_tarball() {
        let dir = repo();
        let tarball = dir.path().join("out.tgz");
        let extra = [(
            ".kluris-site/install.ps1".to_owned(),
            "# wrapper: x\r\n".to_owned(),
        )];
        super::super::pack::pack_with_files(dir.path(), &tarball, "the run script\n", &extra)
            .unwrap();
        assert_eq!(
            member(&tarball, ".kluris-site/install.ps1").as_deref(),
            Some("# wrapper: x\r\n")
        );
        assert_eq!(
            member(&tarball, ".kluris-run.ps1").as_deref(),
            Some("the run script\n")
        );
        assert_eq!(member(&tarball, "kept.txt").as_deref(), Some("committed\n"));
    }

    #[test]
    fn uncommitted_tracked_changes_go_and_nothing_else() {
        let dir = repo();
        let root = dir.path();
        fs::write(root.join("parity").join("harness.py"), "changed\n").unwrap();
        fs::write(root.join("untracked.txt"), "new\n").unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target").join("big"), "x").unwrap();
        fs::create_dir_all(root.join("parity").join(".oracle")).unwrap();
        fs::write(root.join("parity").join(".oracle").join("v"), "x").unwrap();
        let head = git(root, &["rev-parse", "HEAD"]);
        let status_before = git(root, &["status", "--porcelain"]);

        let tarball = dir.path().join("target").join("out.tgz");
        let commit = super::super::pack::pack(root, &tarball, "s").unwrap();
        assert_ne!(commit, head, "the tree with its changes is another commit");
        assert_eq!(
            member(&tarball, "parity/harness.py").as_deref(),
            Some("changed\n")
        );
        assert_eq!(member(&tarball, "untracked.txt"), None);
        assert_eq!(member(&tarball, "target/big"), None);
        assert_eq!(member(&tarball, "parity/.oracle/v"), None);
        // The checkout is left as it was: no stash, no change.
        assert_eq!(git(root, &["status", "--porcelain"]), status_before);
        assert_eq!(git(root, &["stash", "list"]), "");
        assert_eq!(
            fs::read_to_string(root.join("parity").join("harness.py")).unwrap(),
            "changed\n"
        );
    }

    #[test]
    fn tracked_files_under_excluded_folders_do_not_go() {
        let dir = repo();
        let root = dir.path();
        fs::create_dir_all(root.join("parity").join(".venv")).unwrap();
        fs::write(root.join("parity").join(".venv").join("pyvenv.cfg"), "x").unwrap();
        git(root, &["add", "-f", "parity/.venv/pyvenv.cfg"]);
        git(root, &["commit", "-q", "-m", "a venv by mistake"]);
        let tarball = dir.path().join("out.tgz");
        super::super::pack::pack(root, &tarball, "s").unwrap();
        assert_eq!(member(&tarball, "parity/.venv/pyvenv.cfg"), None);
        assert_eq!(member(&tarball, "kept.txt").as_deref(), Some("committed\n"));
    }

    #[test]
    fn the_tarball_holds_the_committed_bytes_whatever_autocrlf_says() {
        // Git for Windows sets core.autocrlf=true system-wide, and git archive
        // then writes CRLF (zeewin, 2026-10-25).
        let dir = repo();
        let root = dir.path();
        git(root, &["config", "core.autocrlf", "true"]);
        fs::write(root.join("parity").join("harness.py"), "changed\n").unwrap();
        let tarball = dir.path().join("out.tgz");
        super::super::pack::pack(root, &tarball, "s\n").unwrap();
        assert_eq!(member(&tarball, "kept.txt").as_deref(), Some("committed\n"));
        assert_eq!(
            member(&tarball, "parity/harness.py").as_deref(),
            Some("changed\n")
        );
    }

    #[test]
    fn the_toolchain_channel_is_read_from_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.99.0\"\ncomponents = [\"rustfmt\"]\n",
        )
        .unwrap();
        assert_eq!(super::super::pack::channel(dir.path()).unwrap(), "1.99.0");
        fs::write(dir.path().join("rust-toolchain.toml"), "[toolchain]\n").unwrap();
        assert!(super::super::pack::channel(dir.path()).is_err());
    }
}
