//! `kluris commit` needs git >= 2.38 (`git hook run`, and `merge-tree
//! --write-tree` for `sync`): an older git is a clean error before anything
//! is read or written.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;

use kluris_core::dream::DreamOptions;
use kluris_core::git::SystemGit;
use kluris_core::sync::commit::{CommitError, CommitRequest, commit};
use kluris_core::yaml::Date;

#[test]
fn an_older_git_is_a_clean_error_naming_the_version_needed() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("git");
    fs::write(
        &fake,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'git version 2.30.1'; exit 0; fi\necho \"unexpected: $*\" >&2\nexit 99\n",
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let brain = dir.path().join("brain");
    fs::create_dir_all(brain.join("notes")).unwrap();
    fs::write(brain.join("notes").join("a.md"), "# A\n").unwrap();

    let paths = vec!["notes/a.md".to_string()];
    let error = commit(
        &brain,
        &SystemGit::new().program(&fake),
        &CommitRequest {
            paths: &paths,
            message: None,
            agent: None,
            no_verify: false,
            maps: DreamOptions {
                name: "demo",
                description: "",
                today: Date::new(2026, 1, 1).unwrap(),
            },
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        CommitError::GitTooOld {
            found: "git version 2.30.1".into()
        }
    );
    let text = error.to_string();
    assert!(text.contains("2.38") && text.contains("2.30.1"), "{text}");
    assert!(!brain.join("notes").join("map.md").exists());
}
