//! The binary's basics: version, exit codes, the JSON error envelope, `list`.

use std::path::Path;

use assert_cmd::Command;
use assert_fs::TempDir;
use assert_fs::prelude::*;
use predicates::prelude::*;

/// `kluris` with HOME, USERPROFILE and KLURIS_CONFIG inside `home`, so the
/// developer's real config is never read.
fn kluris(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("kluris"));
    cmd.env("HOME", home)
        .env("USERPROFILE", home)
        .env("KLURIS_CONFIG", home.join(".kluris").join("config.yml"))
        .env("KLURIS_NO_PROMPT", "1")
        .env("NO_COLOR", "1");
    cmd
}

/// A path as it appears inside a JSON string (`\` escaped).
fn json_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

/// A path as a single-quoted YAML scalar.
fn yaml_path(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

#[test]
fn version_prints_kluris_3_dev() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .arg("--version")
        .assert()
        .success()
        .stdout("kluris 3.0.0-dev\n");
}

#[test]
fn no_arguments_prints_usage_and_exits_2() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn unknown_command_with_json_is_the_envelope() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .args(["bogus", "--json"])
        .assert()
        .code(1)
        .stdout("{\"ok\": false, \"error\": \"No such command 'bogus'.\"}\n");
}

#[test]
fn unknown_command_without_json_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .arg("bogus")
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("No such command 'bogus'."));
}

#[test]
fn unknown_option_with_json_is_the_envelope() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .args(["list", "--bogus", "--json"])
        .assert()
        .code(1)
        .stdout("{\"ok\": false, \"error\": \"No such option '--bogus'.\"}\n");
}

#[test]
fn unknown_option_without_json_exits_2() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .args(["list", "--bogus"])
        .assert()
        .code(2)
        .stdout("")
        .stderr(predicate::str::contains("No such option '--bogus'."));
}

#[test]
fn root_level_json_flag_is_a_usage_error() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .arg("--json")
        .assert()
        .code(2)
        .stdout("");
}

#[test]
fn list_json_without_config() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout("{\"ok\": true, \"brains\": []}\n");
}

#[test]
fn list_without_config_says_no_brains() {
    let home = TempDir::new().unwrap();
    kluris(home.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "No brains registered. Run 'kluris create <name>' to create one.",
        ));
}

/// Four brains: normalized companions, no companions key, missing kluris.yml,
/// a kluris.yml without `name`; plus a stale path and a non-ASCII description.
fn write_four_brains(home: &TempDir) -> [std::path::PathBuf; 4] {
    let a = home.child("brains/a");
    a.child("kluris.yml")
        .write_str(
            "name: a\ncompanions:\n- specmint-tdd\n- bogus\n- specmint-core\n- specmint-core\n",
        )
        .unwrap();
    let b = home.child("brains/b");
    b.child("kluris.yml")
        .write_str("name: b\ndescription: B\n")
        .unwrap();
    let c = home.child("brains/c");
    c.create_dir_all().unwrap();
    let d = home.child("brains/d");
    d.child("kluris.yml")
        .write_str("description: nameless\n")
        .unwrap();
    home.child(".kluris/config.yml")
        .write_str(&format!(
            "brains:\n  a:\n    path: {}\n    description: Grüße — 🚀\n  b:\n    path: {}\n  c:\n    path: {}\n    description: no kluris.yml\n  d:\n    path: {}\n    description: nameless\n",
            yaml_path(a.path()),
            yaml_path(b.path()),
            yaml_path(c.path()),
            yaml_path(d.path()),
        ))
        .unwrap();
    [
        a.to_path_buf(),
        b.to_path_buf(),
        c.to_path_buf(),
        d.to_path_buf(),
    ]
}

#[test]
fn list_json_with_brains_matches_v2_shape() {
    let home = TempDir::new().unwrap();
    let [a, b, c, d] = write_four_brains(&home);
    let expected = format!(
        concat!(
            "{{\"ok\": true, \"brains\": [",
            "{{\"name\": \"a\", \"path\": \"{}\", \"description\": \"Gr\\u00fc\\u00dfe \\u2014 \\ud83d\\ude80\", \"companions\": [\"specmint-core\", \"specmint-tdd\"]}}, ",
            "{{\"name\": \"b\", \"path\": \"{}\", \"description\": \"\", \"companions\": []}}, ",
            "{{\"name\": \"c\", \"path\": \"{}\", \"description\": \"no kluris.yml\", \"companions\": null}}, ",
            "{{\"name\": \"d\", \"path\": \"{}\", \"description\": \"nameless\", \"companions\": null}}",
            "]}}\n"
        ),
        json_path(&a),
        json_path(&b),
        json_path(&c),
        json_path(&d),
    );
    kluris(home.path())
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn list_table_shows_columns_and_companion_states() {
    let home = TempDir::new().unwrap();
    write_four_brains(&home);
    kluris(home.path()).arg("list").assert().success().stdout(
        predicate::str::contains("Registered Brains")
            .and(predicate::str::contains("Name"))
            .and(predicate::str::contains("Path"))
            .and(predicate::str::contains("Description"))
            .and(predicate::str::contains("Companions"))
            .and(predicate::str::contains("specmint-core, specmint-tdd"))
            .and(predicate::str::contains("(none)"))
            .and(predicate::str::contains("(unknown)"))
            .and(predicate::str::contains("Grüße — 🚀")),
    );
}

#[test]
fn list_reads_home_config_when_kluris_config_is_unset_or_empty() {
    let home = TempDir::new().unwrap();
    home.child(".kluris/config.yml")
        .write_str("brains:\n  only:\n    path: /nowhere/only\n")
        .unwrap();
    let expected = "{\"ok\": true, \"brains\": [{\"name\": \"only\", \"path\": \"/nowhere/only\", \"description\": \"\", \"companions\": null}]}\n";
    kluris(home.path())
        .env_remove("KLURIS_CONFIG")
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(expected);
    kluris(home.path())
        .env("KLURIS_CONFIG", "")
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn list_with_empty_config_file() {
    let home = TempDir::new().unwrap();
    home.child(".kluris/config.yml").write_str("").unwrap();
    kluris(home.path())
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout("{\"ok\": true, \"brains\": []}\n");
}

#[test]
fn invalid_config_is_a_kluris_error() {
    let home = TempDir::new().unwrap();
    home.child(".kluris/config.yml")
        .write_str("brains: null\n")
        .unwrap();
    let out = kluris(home.path())
        .args(["list", "--json"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("{\"ok\": false, \"error\": \""), "{text}");
    assert!(
        text.ends_with("\"}\n") && text.lines().count() == 1,
        "{text}"
    );

    kluris(home.path())
        .arg("list")
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with("Error: "));
}
