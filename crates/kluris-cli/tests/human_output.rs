//! Human output: styles only where colour is allowed, user text never markup.

use std::path::Path;

use assert_cmd::Command;
use assert_fs::TempDir;
use assert_fs::prelude::*;

/// `kluris` with its config in `home`, a git that finds no repo above it,
/// and none of the colour variables set.
fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("GIT_CEILING_DIRECTORIES", home)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE");
    cmd
}

/// One registered brain (no git) whose description looks like Rich markup.
fn brain(home: &TempDir) {
    home.child("demo/lobe/n.md").write_str("# n\n").unwrap();
    let path = home.child("demo").path().display().to_string();
    home.child(".kluris/config.yml")
        .write_str(&format!(
            "brains:\n  demo:\n    path: '{}'\n    description: 'see [red]docs[/red] [/oops]'\n",
            path.replace('\'', "''")
        ))
        .unwrap();
}

fn stdout(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn piped_output_has_no_escapes() {
    let home = TempDir::new().unwrap();
    brain(&home);
    let text = stdout(kluris(home.path()).arg("status"));
    assert!(
        text.starts_with("\ndemo\n  Lobes: 1, Neurons: 1\n  Git: disabled\n"),
        "{text:?}"
    );
    assert!(!stdout(kluris(home.path()).arg("list")).contains('\x1b'));
}

#[test]
fn forced_colour_styles_the_brain_name_and_table_header() {
    let home = TempDir::new().unwrap();
    brain(&home);
    let text = stdout(kluris(home.path()).arg("status").env("CLICOLOR_FORCE", "1"));
    assert!(text.starts_with("\n\x1b[1mdemo\x1b[0m\n"), "{text:?}");
    let table = stdout(kluris(home.path()).arg("list").env("CLICOLOR_FORCE", "1"));
    assert!(table.contains("\x1b[1mName\x1b[0m"), "{table:?}");
    assert!(table.contains("see [red]docs[/red] [/oops]"), "{table:?}");
}

#[test]
fn no_color_wins_over_forced_colour() {
    let home = TempDir::new().unwrap();
    brain(&home);
    let text = stdout(
        kluris(home.path())
            .arg("status")
            .env("CLICOLOR_FORCE", "1")
            .env("NO_COLOR", "1"),
    );
    assert!(!text.contains('\x1b'), "{text:?}");
}

#[test]
fn json_is_never_styled() {
    let home = TempDir::new().unwrap();
    brain(&home);
    let text = stdout(
        kluris(home.path())
            .args(["status", "--json"])
            .env("CLICOLOR_FORCE", "1"),
    );
    assert_eq!(
        text,
        "{\"ok\": true, \"brains\": [{\"name\": \"demo\", \"lobes\": 1, \"neurons\": 1, \
         \"uncommitted\": \"\", \"git_enabled\": false, \"recent_commits\": []}]}\n"
    );
}

#[test]
fn usage_names_kluris_whatever_the_binary_file_is_called() {
    // clap takes the name from argv[0] (`kluris.exe` on Windows); Click's
    // usage, like v2's, says `kluris`.
    let dir = TempDir::new().unwrap();
    let copy = dir.path().join("kluris-renamed.exe");
    std::fs::copy(assert_cmd::cargo::cargo_bin!("kluris"), &copy).unwrap();
    let out = std::process::Command::new(&copy)
        .args(["status", "--help"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Usage: kluris status [OPTIONS]"), "{text}");
    let out = std::process::Command::new(&copy)
        .args(["status", "-h"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stderr).unwrap();
    assert!(
        text.contains("Try 'kluris status --help' for help."),
        "{text}"
    );
}
