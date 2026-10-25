//! The global `config.yml` and each brain's `kluris.yml`, read and written as
//! v2.28.11 does. Every fixture under `fixtures/config/` was written by the v2
//! oracle (`parity/tools/gen_config_fixtures.py`); `*-input.yml` files are the
//! hand-edited files v2 was given to read, and the matching `*-rewritten.yml`
//! is what v2 wrote back.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use kluris_core::config::{
    AgentsConfig, BrainConfig, BrainEntry, ConfigErrorKind, DEFAULT_AGENT_COMMANDS, GlobalConfig,
    brain_config_to_yaml, global_config_path, global_config_to_yaml, home_dir, parse_brain_config,
    parse_global_config, read_brain_config, read_global_config, register_brain, unregister_brain,
    write_brain_config, write_global_config,
};

const BETA_DESCRIPTION: &str = "Équipe données — connaissances partagées über alles, naïve café façade 日本語 and a long tail to fold";

/// v2's `DEFAULT_AGENT_COMMANDS`, as v2 writes them.
const V2_DEFAULT_AGENTS: [&str; 9] = [
    "claude", "cursor", "windsurf", "copilot", "codex", "gemini", "hermes", "kilocode", "junie",
];

/// v3's default list: v2's in v2's order, `devin` in place of `windsurf`,
/// then `antigravity`. v2's default list (and its pre-Hermes one) reads as it.
const V3_DEFAULT_AGENTS: [&str; 10] = [
    "claude",
    "cursor",
    "devin",
    "copilot",
    "codex",
    "gemini",
    "hermes",
    "kilocode",
    "junie",
    "antigravity",
];

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/config")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn entry(path: &str, description: &str) -> BrainEntry {
    BrainEntry {
        path: path.into(),
        description: description.into(),
    }
}

fn global(brains: &[(&str, BrainEntry)]) -> GlobalConfig {
    GlobalConfig {
        brains: brains
            .iter()
            .map(|(n, e)| (n.to_string(), e.clone()))
            .collect::<IndexMap<_, _>>(),
    }
}

fn brain(name: &str, description: &str, agents: &[&str], companions: &[&str]) -> BrainConfig {
    BrainConfig {
        name: name.into(),
        description: description.into(),
        agents: AgentsConfig {
            commands_for: strings(agents),
        },
        companions: strings(companions),
    }
}

fn commands_yaml(agents: &[&str]) -> String {
    let items: String = agents.iter().map(|a| format!("  - {a}\n")).collect();
    format!("  commands_for:\n{items}")
}

/// A v2-written file with v2's default agent list as v3 writes it back: the
/// v3 default list.
fn as_v3_writes(v2_text: &str) -> String {
    let v2 = commands_yaml(&V2_DEFAULT_AGENTS);
    assert!(v2_text.contains(&v2), "not v2's default list: {v2_text}");
    v2_text.replace(&v2, &commands_yaml(&V3_DEFAULT_AGENTS))
}

fn three_brains() -> GlobalConfig {
    global(&[
        ("alpha", entry("/kf/brains/alpha", "alpha knowledge base")),
        ("beta", entry("/kf/brains/beta", BETA_DESCRIPTION)),
        ("gamma", entry("/kf/brains/gamma", "Registered brain")),
    ])
}

// --- the global config ------------------------------------------------------

#[test]
fn reads_v2_global_config_in_registration_order() {
    let cfg = parse_global_config(&fixture("global-three.yml")).unwrap();
    assert_eq!(cfg, three_brains());
    let names: Vec<&str> = cfg.brains.keys().map(String::as_str).collect();
    assert_eq!(names, ["alpha", "beta", "gamma"]);
}

#[test]
fn writes_global_config_byte_identical_to_v2() {
    assert_eq!(
        global_config_to_yaml(&three_brains()),
        fixture("global-three.yml")
    );
}

#[test]
fn empty_global_config_writes_an_empty_mapping_like_v2() {
    assert_eq!(
        global_config_to_yaml(&GlobalConfig::default()),
        fixture("global-empty.yml")
    );
    assert_eq!(
        parse_global_config(&fixture("global-empty.yml")).unwrap(),
        GlobalConfig::default()
    );
}

#[test]
fn stale_global_keys_are_ignored_and_dropped_on_write() {
    let cfg = parse_global_config(&fixture("global-stale-input.yml")).unwrap();
    assert_eq!(cfg, three_brains());

    let mut cfg = cfg;
    cfg.brains.insert(
        "delta".into(),
        entry("/kf/brains/delta", "delta knowledge base"),
    );
    assert_eq!(
        global_config_to_yaml(&cfg),
        fixture("global-stale-rewritten.yml")
    );
}

#[test]
fn missing_or_empty_global_config_has_no_brains() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.yml");
    assert_eq!(read_global_config(&path).unwrap(), GlobalConfig::default());
    fs::write(&path, "").unwrap();
    assert_eq!(read_global_config(&path).unwrap(), GlobalConfig::default());
    fs::write(&path, "# only a comment\n").unwrap();
    assert_eq!(read_global_config(&path).unwrap(), GlobalConfig::default());
}

#[test]
fn invalid_global_config_is_an_error_value() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.yml");
    for text in [
        "brains: null\n",
        "brains:\n  a:\n    description: no path\n",
        "brains:\n  a:\n    path: 12\n",
        "brains: [unclosed\n",
    ] {
        fs::write(&path, text).unwrap();
        let err = read_global_config(&path).unwrap_err();
        assert_eq!(err.kind, ConfigErrorKind::Invalid, "{text:?}");
        assert_eq!(err.path, path);
    }
}

// --- brain kluris.yml -------------------------------------------------------

#[test]
fn reads_v2_brain_configs() {
    let cases = [
        (
            "brain-created.yml",
            brain("alpha", "alpha knowledge base", &V3_DEFAULT_AGENTS, &[]),
        ),
        (
            "brain-registered.yml",
            brain("gamma", "Registered brain", &V3_DEFAULT_AGENTS, &[]),
        ),
        (
            "brain-unicode-companions.yml",
            brain(
                "beta",
                BETA_DESCRIPTION,
                &V3_DEFAULT_AGENTS,
                &["specmint-core", "specmint-tdd-html"],
            ),
        ),
        (
            "brain-custom-agents.yml",
            brain(
                "delta",
                "d",
                &["claude", "devin", "antigravity", "bogus-agent"],
                &["specmint-core"],
            ),
        ),
    ];
    for (name, want) in cases {
        assert_eq!(parse_brain_config(&fixture(name)).unwrap(), want, "{name}");
    }
}

#[test]
fn writes_brain_config_byte_identical_to_v2_for_the_same_model() {
    let cases = [
        (
            "brain-created.yml",
            brain("alpha", "alpha knowledge base", &V2_DEFAULT_AGENTS, &[]),
        ),
        (
            "brain-unicode-companions.yml",
            brain(
                "beta",
                BETA_DESCRIPTION,
                &V2_DEFAULT_AGENTS,
                &["specmint-core", "specmint-tdd-html"],
            ),
        ),
        (
            "brain-custom-agents.yml",
            brain(
                "delta",
                "d",
                &["claude", "devin", "antigravity", "bogus-agent"],
                &["specmint-core"],
            ),
        ),
        (
            "brain-minimal-rewritten.yml",
            brain("alpha", "", &V2_DEFAULT_AGENTS, &["specmint-core"]),
        ),
    ];
    for (name, model) in cases {
        assert_eq!(brain_config_to_yaml(&model), fixture(name), "{name}");
    }
}

#[test]
fn v2_brain_config_round_trips_with_the_v3_default_agents() {
    for name in [
        "brain-created.yml",
        "brain-registered.yml",
        "brain-unicode-companions.yml",
    ] {
        let v2 = fixture(name);
        let cfg = parse_brain_config(&v2).unwrap();
        assert_eq!(brain_config_to_yaml(&cfg), as_v3_writes(&v2), "{name}");
    }
    // No windsurf: the round trip is exact.
    let v2 = fixture("brain-custom-agents.yml");
    assert_eq!(brain_config_to_yaml(&parse_brain_config(&v2).unwrap()), v2);
}

#[test]
fn non_ascii_description_round_trips_byte_identically() {
    let v2 = fixture("global-three.yml");
    let cfg = parse_global_config(&v2).unwrap();
    assert_eq!(cfg.brains["beta"].description, BETA_DESCRIPTION);
    assert_eq!(global_config_to_yaml(&cfg), v2);
    assert!(v2.contains("\"\\xC9quipe donn\\xE9es \\u2014"));
}

#[test]
fn stale_brain_keys_are_ignored_and_dropped_on_write() {
    let mut cfg = parse_brain_config(&fixture("brain-stale-input.yml")).unwrap();
    assert_eq!(
        cfg,
        brain("alpha", "alpha knowledge base", &V3_DEFAULT_AGENTS, &[])
    );
    cfg.companions.push("specmint-tdd".into());
    assert_eq!(
        brain_config_to_yaml(&cfg),
        as_v3_writes(&fixture("brain-stale-rewritten.yml"))
    );
}

#[test]
fn missing_companions_and_agents_take_v2_defaults() {
    let mut cfg = parse_brain_config(&fixture("brain-minimal-input.yml")).unwrap();
    assert_eq!(cfg, brain("alpha", "", &V3_DEFAULT_AGENTS, &[]));
    cfg.companions.push("specmint-core".into());
    assert_eq!(
        brain_config_to_yaml(&cfg),
        as_v3_writes(&fixture("brain-minimal-rewritten.yml"))
    );
    assert_eq!(
        parse_brain_config("name: x\nagents: {}\n").unwrap().agents,
        AgentsConfig::default()
    );
}

#[test]
fn defaults_are_v2_defaults_with_devin_then_antigravity() {
    assert_eq!(DEFAULT_AGENT_COMMANDS[..], V3_DEFAULT_AGENTS[..]);
    assert_eq!(
        AgentsConfig::default().commands_for,
        strings(&V3_DEFAULT_AGENTS)
    );
    assert_eq!(
        BrainConfig::new("n"),
        brain("n", "", &V3_DEFAULT_AGENTS, &[])
    );
}

#[test]
fn key_order_in_the_file_does_not_change_the_written_order() {
    let cfg = parse_brain_config(&fixture("brain-custom-agents-input.yml")).unwrap();
    let mut want = brain(
        "delta",
        "d",
        &["claude", "devin", "antigravity", "bogus-agent"],
        &[],
    );
    assert_eq!(cfg, want);
    want.companions.push("specmint-core".into());
    assert_eq!(
        brain_config_to_yaml(&want),
        fixture("brain-custom-agents.yml")
    );
}

#[test]
fn windsurf_is_read_as_devin_and_unknown_agents_are_kept() {
    let text =
        "name: b\nagents:\n  commands_for:\n  - windsurf\n  - bogus\n  - claude\n  - devin\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for,
        strings(&["devin", "bogus", "claude", "devin"])
    );
    // Only agent names: a companion or brain named windsurf is left alone.
    let text = "name: windsurf\ndescription: windsurf\ncompanions:\n- windsurf\n";
    let cfg = parse_brain_config(text).unwrap();
    assert_eq!(cfg.name, "windsurf");
    assert_eq!(cfg.description, "windsurf");
    assert_eq!(cfg.companions, strings(&["windsurf"]));
}

#[test]
fn v2_default_lists_migrate_to_the_v3_default() {
    let cfg = parse_brain_config(&fixture("brain-legacy-input.yml")).unwrap();
    assert_eq!(
        cfg,
        brain("gamma", "Registered brain", &V3_DEFAULT_AGENTS, &[])
    );
    let mut cfg = cfg;
    cfg.companions.push("specmint-core".into());
    assert_eq!(
        brain_config_to_yaml(&cfg),
        as_v3_writes(&fixture("brain-legacy-rewritten.yml"))
    );
    // v2.28's 9 names, with windsurf or with devin, in any order and with
    // repeats (compared as a set): the v3 default.
    for names in [
        "[claude, cursor, windsurf, copilot, codex, gemini, hermes, kilocode, junie]",
        "[claude, cursor, devin, copilot, codex, gemini, hermes, kilocode, junie]",
        "[junie, kilocode, hermes, gemini, codex, copilot, devin, cursor, claude]",
        "[claude, claude, cursor, windsurf, copilot, codex, gemini, hermes, kilocode, junie]",
        "[gemini, windsurf, claude, cursor, copilot, codex, kilocode, junie]",
    ] {
        let text = format!("name: b\nagents:\n  commands_for: {names}\n");
        assert_eq!(
            parse_brain_config(&text).unwrap().agents,
            AgentsConfig::default(),
            "{names}"
        );
    }
    // Any other list is kept exactly as written (windsurf still reads as
    // devin): a subset, a legacy set plus one more, both windsurf and devin.
    let text = "name: b\nagents:\n  commands_for: [claude, cursor, devin, copilot, codex, gemini, hermes, kilocode]\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for,
        strings(&[
            "claude", "cursor", "devin", "copilot", "codex", "gemini", "hermes", "kilocode"
        ])
    );
    let text = "name: b\nagents:\n  commands_for: [claude, cursor, windsurf, devin, copilot, codex, gemini, hermes, kilocode, junie]\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for,
        strings(&[
            "claude", "cursor", "devin", "devin", "copilot", "codex", "gemini", "hermes",
            "kilocode", "junie"
        ])
    );
    let text = "name: b\nagents:\n  commands_for: [claude, cursor, devin, copilot, codex, gemini, hermes, kilocode, junie, bogus]\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for.len(),
        10
    );
    let text = "name: b\nagents:\n  commands_for: [claude, cursor]\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for,
        strings(&["claude", "cursor"])
    );
    let text = "name: b\nagents:\n  commands_for: [claude, cursor, windsurf, copilot, codex, gemini, kilocode, junie, bogus]\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for.len(),
        9
    );
    // With devin instead of windsurf it is not v2's legacy set.
    let text = "name: b\nagents:\n  commands_for: [claude, cursor, devin, copilot, codex, gemini, kilocode, junie]\n";
    assert_eq!(
        parse_brain_config(text).unwrap().agents.commands_for,
        strings(&[
            "claude", "cursor", "devin", "copilot", "codex", "gemini", "kilocode", "junie"
        ])
    );
}

#[test]
fn invalid_brain_configs_are_errors() {
    for text in [
        "",
        "description: no name\n",
        "name: 5\n",
        "name: yes\n",
        "name: b\ncompanions: null\n",
        "name: b\ncompanions: [1]\n",
        "name: b\nagents: null\n",
        "name: b\nagents:\n  commands_for: null\n",
        "name: b\nagents:\n  commands_for: [1]\n",
        "name: b\ndescription: [x]\n",
        "- a list\n",
        "name: [unclosed\n",
    ] {
        assert!(parse_brain_config(text).is_err(), "should reject {text:?}");
    }
}

#[test]
fn missing_brain_config_is_a_not_found_error() {
    let tmp = tempfile::tempdir().unwrap();
    let err = read_brain_config(tmp.path()).unwrap_err();
    assert_eq!(err.kind, ConfigErrorKind::NotFound);
    assert_eq!(err.path, tmp.path().join("kluris.yml"));
}

// --- writing to disk --------------------------------------------------------

fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn write_global_config_creates_the_kluris_dir_and_leaves_no_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("home").join(".kluris").join("config.yml");
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        fixture("global-three.yml")
    );
    assert_eq!(dir_names(path.parent().unwrap()), ["config.yml"]);

    // Overwrite in place.
    write_global_config(&path, &GlobalConfig::default()).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "brains: {}\n");
    assert_eq!(dir_names(path.parent().unwrap()), ["config.yml"]);
}

#[test]
fn write_brain_config_replaces_kluris_yml_and_leaves_no_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("kluris.yml"), "name: old\n").unwrap();
    let model = brain(
        "delta",
        "d",
        &["claude", "devin", "antigravity", "bogus-agent"],
        &["specmint-core"],
    );
    write_brain_config(tmp.path(), &model).unwrap();
    assert_eq!(
        fs::read_to_string(tmp.path().join("kluris.yml")).unwrap(),
        fixture("brain-custom-agents.yml")
    );
    assert_eq!(dir_names(tmp.path()), ["kluris.yml"]);
    assert_eq!(read_brain_config(tmp.path()).unwrap(), model);
}

#[test]
fn write_brain_config_does_not_create_the_brain_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("no-such-brain");
    let err = write_brain_config(&missing, &BrainConfig::new("x")).unwrap_err();
    assert_eq!(err.kind, ConfigErrorKind::Io);
    assert!(!missing.exists());
}

#[test]
fn write_error_is_a_value_and_keeps_the_old_file() {
    let tmp = tempfile::tempdir().unwrap();
    // The parent of the config path is a file.
    let blocker = tmp.path().join("blocker");
    fs::write(&blocker, "x").unwrap();
    let err = write_global_config(&blocker.join("config.yml"), &three_brains()).unwrap_err();
    assert_eq!(err.kind, ConfigErrorKind::Io);
    assert_eq!(fs::read_to_string(&blocker).unwrap(), "x");
}

#[cfg(unix)]
#[test]
fn write_follows_a_symlinked_config_and_keeps_its_mode() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("dotfiles").join("config.yml");
    fs::create_dir_all(real.parent().unwrap()).unwrap();
    fs::write(&real, "brains: {}\n").unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
    let kluris_dir = tmp.path().join(".kluris");
    fs::create_dir(&kluris_dir).unwrap();
    let link = kluris_dir.join("config.yml");
    symlink(&real, &link).unwrap();

    write_global_config(&link, &three_brains()).unwrap();
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&real).unwrap(),
        fixture("global-three.yml")
    );
    assert_eq!(
        fs::metadata(&real).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(dir_names(real.parent().unwrap()), ["config.yml"]);
}

// --- line endings (decision #3: a file keeps its endings, new files are LF)

fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

#[test]
fn global_config_rewrite_keeps_the_existing_line_endings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.yml");
    let want = fixture("global-three.yml");

    // A new file is LF.
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), want.as_bytes());

    // An existing LF file stays LF.
    fs::write(&path, "brains: {}\n").unwrap();
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), want.as_bytes());

    // An existing CRLF file stays CRLF, every line of it.
    fs::write(&path, b"brains: {}\r\n").unwrap();
    let cfg = read_global_config(&path).unwrap();
    write_global_config(&path, &cfg).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"brains: {}\r\n");
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), crlf(&want).as_bytes());
    // ... and reads back to the same model.
    assert_eq!(read_global_config(&path).unwrap(), three_brains());
    assert_eq!(dir_names(tmp.path()), ["config.yml"]);
}

#[test]
fn brain_config_rewrite_keeps_the_existing_line_endings() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("kluris.yml");
    let model = brain(
        "delta",
        "d",
        &["claude", "devin", "antigravity", "bogus-agent"],
        &["specmint-core"],
    );
    let want = fixture("brain-custom-agents.yml");

    write_brain_config(tmp.path(), &model).unwrap();
    assert_eq!(fs::read(&file).unwrap(), want.as_bytes());

    fs::write(&file, "name: old\n").unwrap();
    write_brain_config(tmp.path(), &model).unwrap();
    assert_eq!(fs::read(&file).unwrap(), want.as_bytes());

    fs::write(&file, crlf(&fixture("brain-custom-agents-input.yml"))).unwrap();
    let cfg = read_brain_config(tmp.path()).unwrap();
    let mut with_companion = cfg.clone();
    with_companion.companions.push("specmint-core".into());
    write_brain_config(tmp.path(), &with_companion).unwrap();
    assert_eq!(fs::read(&file).unwrap(), crlf(&want).as_bytes());
    assert_eq!(read_brain_config(tmp.path()).unwrap(), model);
    assert_eq!(dir_names(tmp.path()), ["kluris.yml"]);
}

#[test]
fn the_first_line_break_decides_the_endings() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.yml");
    let want = fixture("global-three.yml");
    fs::write(&path, b"brains:\r\n  a:\n    path: /x\n").unwrap();
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), crlf(&want).as_bytes());
    fs::write(&path, b"brains:\n  a:\r\n    path: /x\r\n").unwrap();
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), want.as_bytes());
    // No line break at all: LF.
    fs::write(&path, b"brains: {}").unwrap();
    write_global_config(&path, &three_brains()).unwrap();
    assert_eq!(fs::read(&path).unwrap(), want.as_bytes());
}

// --- symlinks (v2's in-place write goes through the link, also a dangling one)

#[cfg(unix)]
fn assert_link(link: &Path, to: &Path) {
    let meta = fs::symlink_metadata(link).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "{} was replaced",
        link.display()
    );
    assert_eq!(fs::read_link(link).unwrap(), to);
}

#[cfg(unix)]
#[test]
fn write_through_a_symlink_keeps_the_target_endings() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real.yml");
    fs::write(&real, b"brains: {}\r\n").unwrap();
    let link = tmp.path().join("config.yml");
    symlink("real.yml", &link).unwrap();
    write_global_config(&link, &three_brains()).unwrap();
    assert_link(&link, Path::new("real.yml"));
    assert_eq!(
        fs::read(&real).unwrap(),
        crlf(&fixture("global-three.yml")).as_bytes()
    );
}

#[cfg(unix)]
#[test]
fn write_through_a_dangling_relative_symlink_creates_its_target() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join("dotfiles")).unwrap();
    let kluris_dir = tmp.path().join(".kluris");
    fs::create_dir(&kluris_dir).unwrap();
    let link = kluris_dir.join("config.yml");
    let to = Path::new("../dotfiles/kluris-config.yml");
    symlink(to, &link).unwrap();

    write_global_config(&link, &three_brains()).unwrap();
    assert_link(&link, to);
    let real = tmp.path().join("dotfiles").join("kluris-config.yml");
    assert_eq!(
        fs::read_to_string(&real).unwrap(),
        fixture("global-three.yml")
    );
    assert_eq!(dir_names(&kluris_dir), ["config.yml"]);
    assert_eq!(dir_names(real.parent().unwrap()), ["kluris-config.yml"]);
}

#[cfg(unix)]
#[test]
fn write_through_a_dangling_absolute_symlink_creates_its_target() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let brain_dir = tmp.path().join("brain");
    fs::create_dir(&brain_dir).unwrap();
    let real = tmp.path().join("elsewhere.yml");
    let link = brain_dir.join("kluris.yml");
    symlink(&real, &link).unwrap();

    write_brain_config(&brain_dir, &BrainConfig::new("x")).unwrap();
    assert_link(&link, &real);
    assert_eq!(
        read_brain_config(&brain_dir).unwrap(),
        BrainConfig::new("x")
    );
    assert_eq!(dir_names(&brain_dir), ["kluris.yml"]);
}

#[cfg(unix)]
#[test]
fn write_follows_a_chain_of_symlinks() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    // config.yml -> a/hop1 -> ../b/hop2 -> <root>/final.yml (absent)
    let link = root.join("config.yml");
    symlink("a/hop1", &link).unwrap();
    symlink("../b/hop2", root.join("a").join("hop1")).unwrap();
    symlink(root.join("final.yml"), root.join("b").join("hop2")).unwrap();

    write_global_config(&link, &three_brains()).unwrap();
    assert_link(&link, Path::new("a/hop1"));
    assert_link(&root.join("a").join("hop1"), Path::new("../b/hop2"));
    assert_link(&root.join("b").join("hop2"), &root.join("final.yml"));
    assert_eq!(
        fs::read_to_string(root.join("final.yml")).unwrap(),
        fixture("global-three.yml")
    );

    // Again, now that the end of the chain exists.
    write_global_config(&link, &GlobalConfig::default()).unwrap();
    assert_link(&link, Path::new("a/hop1"));
    assert_eq!(
        fs::read_to_string(root.join("final.yml")).unwrap(),
        "brains: {}\n"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_loop_is_an_error_and_keeps_the_links() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a.yml");
    let b = tmp.path().join("b.yml");
    symlink("b.yml", &a).unwrap();
    symlink("a.yml", &b).unwrap();
    let err = write_global_config(&a, &three_brains()).unwrap_err();
    assert_eq!(err.kind, ConfigErrorKind::Io);
    assert_link(&a, Path::new("b.yml"));
    assert_link(&b, Path::new("a.yml"));
    assert_eq!(dir_names(tmp.path()), ["a.yml", "b.yml"]);
}

#[cfg(unix)]
#[test]
fn a_dangling_symlink_into_a_missing_dir_is_an_error_and_keeps_the_link() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("config.yml");
    symlink("no-such-dir/config.yml", &link).unwrap();
    let err = write_global_config(&link, &three_brains()).unwrap_err();
    assert_eq!(err.kind, ConfigErrorKind::Io);
    assert_link(&link, Path::new("no-such-dir/config.yml"));
    assert_eq!(dir_names(tmp.path()), ["config.yml"]);
}

#[test]
fn register_keeps_position_and_unregister_keeps_order() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(".kluris").join("config.yml");
    register_brain(&path, "alpha", entry("/kf/brains/alpha", "first")).unwrap();
    register_brain(&path, "beta", entry("/kf/brains/beta", BETA_DESCRIPTION)).unwrap();
    register_brain(
        &path,
        "gamma",
        entry("/kf/brains/gamma", "Registered brain"),
    )
    .unwrap();
    // Same name again: a Python dict keeps the key's position.
    register_brain(
        &path,
        "alpha",
        entry("/kf/brains/alpha", "alpha knowledge base"),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        fixture("global-three.yml")
    );

    unregister_brain(&path, "beta").unwrap();
    unregister_brain(&path, "never-registered").unwrap();
    let names: Vec<String> = read_global_config(&path)
        .unwrap()
        .brains
        .into_keys()
        .collect();
    assert_eq!(names, ["alpha", "gamma"]);
}

// --- paths ------------------------------------------------------------------

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn kluris_config_overrides_the_config_path() {
    assert_eq!(
        global_config_path(
            Some(OsStr::new("/custom/c.yml")),
            Some(OsStr::new("/h")),
            Some(p("/os"))
        ),
        Some(p("/custom/c.yml"))
    );
    // Used verbatim, even relative (v2 `Path(env_path)`).
    assert_eq!(
        global_config_path(Some(OsStr::new("rel/c.yml")), None, None),
        Some(p("rel/c.yml"))
    );
    // Empty is unset (v2 `if env_path:`).
    assert_eq!(
        global_config_path(Some(OsStr::new("")), Some(OsStr::new("/h")), Some(p("/os"))),
        Some(p("/h").join(".kluris").join("config.yml"))
    );
}

#[test]
fn home_is_home_when_set_else_the_os_home() {
    assert_eq!(
        home_dir(Some(OsStr::new("/h")), Some(p("/os"))),
        Some(p("/h"))
    );
    assert_eq!(home_dir(None, Some(p("/os"))), Some(p("/os")));
    assert_eq!(
        home_dir(Some(OsStr::new("")), Some(p("/os"))),
        Some(p("/os"))
    );
    assert_eq!(home_dir(None, None), None);
    assert_eq!(
        global_config_path(None, Some(OsStr::new("/h")), Some(p("/os"))),
        Some(p("/h").join(".kluris").join("config.yml"))
    );
    assert_eq!(
        global_config_path(None, None, Some(p("/os"))),
        Some(p("/os").join(".kluris").join("config.yml"))
    );
    assert_eq!(global_config_path(None, None, None), None);
}

#[test]
fn brain_paths_are_stored_as_given() {
    // Never re-resolved or normalized: a Windows path and a trailing slash survive.
    let cfg = global(&[
        ("w", entry(r"C:\Users\me\brains\w", "")),
        ("t", entry("/kf/brains/t/", "")),
    ]);
    let text = global_config_to_yaml(&cfg);
    assert_eq!(parse_global_config(&text).unwrap(), cfg);
}
