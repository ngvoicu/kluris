//! Written and hand-edited configs as `kluris list --json` reads them.
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
