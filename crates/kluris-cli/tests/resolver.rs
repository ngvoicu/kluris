//! The resolver's terminal side in the shipped binary: no test-only command,
//! and the TTY decision for a stdin that is the null device.

use std::path::Path;
use std::process::{Command, Stdio};

use assert_cmd::prelude::*;
use assert_fs::TempDir;
use assert_fs::prelude::*;

/// `kluris` with its config in `home` and prompting allowed (so only the
/// stdin decides whether the picker may run).
fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("GIT_CEILING_DIRECTORIES", home)
        .env("NO_COLOR", "1")
        .env_remove("KLURIS_NO_PROMPT")
        .env_remove("KLURIS_TEST_HOOKS");
    cmd
}

/// Two registered brains, `a` and `b`, both existing directories.
fn two_brains(home: &TempDir) {
    let mut config = String::from("brains:\n");
    for name in ["a", "b"] {
        home.child(name).create_dir_all().unwrap();
        let path = home.child(name).path().display().to_string();
        config.push_str(&format!(
            "  {name}:\n    path: '{}'\n    description: ''\n",
            path.replace('\'', "''")
        ));
    }
    home.child(".kluris/config.yml").write_str(&config).unwrap();
}

#[test]
fn the_shipped_binary_has_no_resolve_hook() {
    // `kluris __resolve` was a parity-only command behind KLURIS_TEST_HOOKS=1;
    // the binary now carries nothing test-only, so it is v2's unknown command
    // whatever the environment says.
    let home = TempDir::new().unwrap();
    two_brains(&home);
    for json in [false, true] {
        let mut cmd = kluris(home.path());
        cmd.env("KLURIS_TEST_HOOKS", "1")
            .args(["__resolve", "--allow-all", "--brain", "all"]);
        if json {
            cmd.arg("--json");
            cmd.assert()
                .code(1)
                .stdout("{\"ok\": false, \"error\": \"No such command '__resolve'.\"}\n");
        } else {
            cmd.assert()
                .code(2)
                .stdout("")
                .stderr(predicates::str::contains(
                    "Error: No such command '__resolve'.",
                ));
        }
    }
}

#[test]
fn a_null_stdin_is_not_a_terminal() {
    // `Stdio::null()` is /dev/null, or NUL on Windows, where v2's `isatty()`
    // is true and it shows the picker, then aborts on end of input (PAR-022).
    // v3 asks whether stdin is a console: NUL is not one, so this is the
    // non-interactive error on every OS, exit 1 as in v2.
    let home = TempDir::new().unwrap();
    two_brains(&home);
    kluris(home.path())
        .arg("status")
        .stdin(Stdio::null())
        .assert()
        .code(1)
        .stdout("")
        .stderr("Error: Multiple brains registered. Pass --brain NAME or 'all'. Available: a, b\n");
    kluris(home.path())
        .arg("wake-up")
        .stdin(Stdio::null())
        .assert()
        .code(1)
        .stdout("")
        .stderr("Error: Multiple brains registered. Pass --brain NAME. Available: a, b\n");
}
