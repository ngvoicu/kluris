//! Written and hand-edited configs as `kluris list --json` reads them: the
//! reading scenario of `parity/test_config_roundtrip.py`.
//!
//! `fixtures/config-reading/` holds the files `list` reads: `config.yml`
//! (stale keys on `alpha` and at the top level; the brains folder written
//! as `/kf/brains`) and each brain's `kluris.yml` (`alpha` with stale `type`
//! and `git` keys, `beta` with a folded non-ASCII description and two
//! companions, then hand-written: custom agents and an unknown companion,
//! the pre-Hermes agent list, a name alone, and `companions: null`).

mod common;

use std::fs;

use common::{Sandbox, copy_tree, fixture, printed_paths, tree_dump, write};

#[test]
fn list_json_reads_written_and_stale_configs() {
    let sb = Sandbox::new();
    let brains = sb.work.join("brains");
    copy_tree(&fixture("config-reading/brains"), &brains);
    let config = fs::read_to_string(fixture("config-reading/config.yml")).unwrap();
    let config = config.replace("/kf/brains", &brains.display().to_string());
    write(&sb.config, config.as_bytes());
    let before = tree_dump(&sb.work);

    let run = sb.kluris(&["list", "--json"]);

    printed_paths(|| insta::assert_snapshot!("list_json", run.render(&sb.root)));
    assert_eq!(tree_dump(&sb.work), before, "list must not write anything");
}
