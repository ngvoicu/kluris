//! Written and hand-edited configs as `kluris list --json` reads them, and
//! configs in YAML's rarer syntax as `kluris list` reads or refuses them.
//!
//! `fixtures/config-reading/` holds the files `list` reads: `config.yml`
//! (stale keys on `alpha` and at the top level; the brains folder written
//! as `/kf/brains`) and each brain's `kluris.yml` (`alpha` with stale `type`
//! and `git` keys, `beta` with a folded non-ASCII description and two
//! companions, then hand-written: custom agents and an unknown companion,
//! the pre-Hermes agent list, a name alone, and `companions: null`).

mod common;

use std::fs;

use common::{Sandbox, assert_unchanged, copy_tree, fixture, tree_files, write};

/// The brains `config.yml` registers, each at `/kf/brains/<name>`.
const BRAINS: [&str; 6] = ["alpha", "beta", "custom", "legacy", "minimal", "broken"];

#[test]
fn list_json_reads_written_and_stale_configs() {
    let sb = Sandbox::new();
    let brains = sb.work.join("brains");
    copy_tree(&fixture("config-reading/brains"), &brains);
    // Each brain's path as the OS writes it, as `create --path` stores it.
    let mut config = fs::read_to_string(fixture("config-reading/config.yml")).unwrap();
    for name in BRAINS {
        let placeholder = format!("/kf/brains/{name}\n");
        assert!(
            config.contains(&placeholder),
            "no {placeholder:?} in config.yml"
        );
        let path = brains.join(name);
        config = config.replace(&placeholder, &format!("{}\n", path.display()));
    }
    write(&sb.config, config.as_bytes());
    let before = tree_files(&sb.work);

    let run = sb.kluris_shown(&["list", "--json"]);

    insta::assert_snapshot!("list_json", run.render(&sb.root));
    assert_unchanged(&before, &tree_files(&sb.work), "list");
}

// --- YAML 1.1 syntax corners ---------------------------------------------------

/// Configs in flow style, by scenario id: `list --json` reads the brains
/// of the first two; the third gives a description that is a list.
const FLOW_CONFIGS: &[(&str, &str)] = &[
    // `alpha:{` is a key and a flow mapping (a `:` followed by a flow
    // indicator ends a plain scalar in a flow collection).
    ("glued-flow-mapping", "brains: {alpha:{path: /missing}}\n"),
    (
        "two-flow-brains",
        "brains: {alpha:{path: /missing, description: d}, beta: {path: /b}}\n",
    ),
    (
        "flow-in-block",
        "brains:\n  alpha: {path: /missing, description: [a,b]}\n",
    ),
];

#[test]
fn list_json_reads_flow_style_configs() {
    for (id, text) in FLOW_CONFIGS {
        let sb = Sandbox::new();
        write(&sb.config, text.as_bytes());
        let before = tree_files(&sb.work);

        let run = sb.kluris_shown(&["list", "--json"]);

        insta::assert_snapshot!(format!("flow_config__{id}"), run.render(&sb.root));
        assert_unchanged(&before, &tree_files(&sb.work), id);
    }
}

/// Configs that do not read, by scenario id: YAML that does not parse and
/// scalars that do not construct.
fn unreadable_configs() -> Vec<(&'static str, String)> {
    let rows: &[(&str, &str)] = &[
        ("anchor-before-question-mark", "brains: {}\nextra: [&a?x]\n"),
        ("block-scalar-in-flow", "brains: {}\nextra: {!!str &a: |}\n"),
        ("non-printable", "brains: {}\nextra: \"a\x07\"\n"),
        // An int or float refuses U+001C..U+001F around the number.
        ("int-x1c", "brains: {}\nextra: !!int \"\\x1c1\"\n"),
        ("float-x1f", "brains: {}\nextra: !!float \"1\\x1f\"\n"),
    ];
    let mut rows: Vec<(&str, String)> = rows.iter().map(|(id, t)| (*id, t.to_string())).collect();
    // An int refuses more than 4,300 decimal digits.
    rows.push((
        "4301-digits",
        format!("brains: {{}}\nextra: {}\n", "1".repeat(4301)),
    ));
    rows
}

#[test]
fn list_fails_on_configs_that_do_not_read() {
    for (id, text) in unreadable_configs() {
        for (mode, args) in [("text", &["list"][..]), ("json", &["list", "--json"])] {
            let sb = Sandbox::new();
            write(&sb.config, text.as_bytes());
            let before = tree_files(&sb.work);

            let run = sb.kluris_shown(args);

            insta::assert_snapshot!(
                format!("unreadable_config__{id}__{mode}"),
                run.render(&sb.root)
            );
            assert_unchanged(&before, &tree_files(&sb.work), id);
        }
    }
}
