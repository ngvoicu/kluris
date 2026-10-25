use std::fs;

use kluris_core::list::{BrainListing, list_brains};

#[test]
fn lists_brains_in_config_order_with_companion_state() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let with_cfg = root.join("with-cfg");
    let no_cfg = root.join("no-cfg");
    fs::create_dir_all(&with_cfg).unwrap();
    fs::create_dir_all(&no_cfg).unwrap();
    fs::write(
        with_cfg.join("kluris.yml"),
        "name: with-cfg\ncompanions:\n- specmint-tdd\n- specmint-core\n",
    )
    .unwrap();
    let config = root.join("config.yml");
    let text = format!(
        "brains:\n  with-cfg:\n    path: '{}'\n    description: has one\n  no-cfg:\n    path: '{}'\n  gone:\n    path: /does/not/exist\n    description: stale\n",
        with_cfg.display(),
        no_cfg.display()
    );
    fs::write(&config, text).unwrap();

    let got = list_brains(&config).unwrap();
    assert_eq!(
        got,
        vec![
            BrainListing {
                name: "with-cfg".into(),
                path: with_cfg.display().to_string(),
                description: "has one".into(),
                companions: Some(vec!["specmint-core".into(), "specmint-tdd".into()]),
            },
            BrainListing {
                name: "no-cfg".into(),
                path: no_cfg.display().to_string(),
                description: String::new(),
                companions: None,
            },
            BrainListing {
                name: "gone".into(),
                path: "/does/not/exist".into(),
                description: "stale".into(),
                companions: None,
            },
        ]
    );
}

#[test]
fn missing_config_lists_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(list_brains(&tmp.path().join("config.yml")).unwrap(), vec![]);
}

#[test]
fn invalid_config_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.yml");
    fs::write(&config, "brains: null\n").unwrap();
    let err = list_brains(&config).unwrap_err();
    assert_eq!(err.path, config);
}
