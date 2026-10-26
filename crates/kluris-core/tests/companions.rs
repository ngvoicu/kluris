//! The Specmint companions: the embedded folders, their install into
//! `~/.kluris/companions/<name>/` (v2 `core/companions.py` and its tests
//! `tests/test_companions.py`), and the per-brain opt-ins behind
//! `kluris companion add|remove`.

use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use kluris_core::companions::{
    self, CompanionTree, KNOWN, enabled, install, install_tree, installed, installed_dir,
    is_installed, load_brains, referenced, refresh, verify_runtime_dir,
};
use kluris_core::config::{BrainEntry, GlobalConfig};
use kluris_core::resolve::ResolvedBrain;
use tempfile::TempDir;

fn vendored() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendored")
}

/// Every entry under `root` (files and folders), as sorted POSIX paths;
/// links are listed, never followed.
fn tree(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap();
            out.push(rel.to_string_lossy().replace('\\', "/"));
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            }
        }
    }
    out.sort();
    out
}

/// `{posix path: bytes}` of every file under `root`.
fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = tree(root)
        .into_iter()
        .filter(|rel| root.join(rel).is_file())
        .map(|rel| {
            let bytes = fs::read(root.join(&rel)).unwrap();
            (rel, bytes)
        })
        .collect();
    out.sort();
    out
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// v2's `_fake_vendored` folder for one companion.
fn fake(name: &str) -> CompanionTree {
    CompanionTree::new()
        .file("SKILL.md", format!("# {name}\n").into_bytes())
        .file("references/researcher.md", b"brief\n".to_vec())
        .file("assets/spec-runtime.js", b"// js\n".to_vec())
}

const FOLDER: [&str; 5] = [
    "SKILL.md",
    "assets",
    "assets/spec-runtime.js",
    "references",
    "references/researcher.md",
];

fn companions_dir(home: &Path) -> PathBuf {
    home.join(".kluris").join("companions")
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

// --- install ------------------------------------------------------------------

#[test]
fn install_copies_folder_to_home() {
    let home = TempDir::new().unwrap();
    install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();

    let dest = installed_dir("specmint-core", home.path());
    assert_eq!(dest, companions_dir(home.path()).join("specmint-core"));
    assert_eq!(
        fs::read_to_string(dest.join("SKILL.md")).unwrap(),
        "# specmint-core\n"
    );
    assert_eq!(tree(&dest), FOLDER);
    assert_eq!(names_in(&companions_dir(home.path())), ["specmint-core"]);
}

#[test]
fn install_copies_only_allowlisted_entries() {
    let home = TempDir::new().unwrap();
    let source = fake("specmint-core")
        .file("README.md", b"readme".to_vec())
        .dir("evals")
        .file("evals/evals.json", b"{}".to_vec())
        .file(".DS_Store", b"junk".to_vec())
        .file("references/.DS_Store", b"junk".to_vec())
        .file("references/__pycache__/x.pyc", b"junk".to_vec())
        .file("assets/.cache/x", b"junk".to_vec());

    install_tree("specmint-core", &source, home.path()).unwrap();

    assert_eq!(tree(&installed_dir("specmint-core", home.path())), FOLDER);
}

#[test]
fn install_without_optional_dirs() {
    let home = TempDir::new().unwrap();
    let source = CompanionTree::new().file("SKILL.md", b"# x\n".to_vec());

    install_tree("specmint-core", &source, home.path()).unwrap();

    assert_eq!(
        tree(&installed_dir("specmint-core", home.path())),
        ["SKILL.md"]
    );
}

#[test]
fn install_keeps_an_empty_allowlisted_dir() {
    // `shutil.copytree` copies an empty `references/` too.
    let home = TempDir::new().unwrap();
    let source = CompanionTree::new()
        .file("SKILL.md", b"# x\n".to_vec())
        .dir("references");

    install_tree("specmint-core", &source, home.path()).unwrap();

    assert_eq!(
        tree(&installed_dir("specmint-core", home.path())),
        ["SKILL.md", "references"]
    );
}

#[test]
fn unknown_companion_is_refused() {
    let home = TempDir::new().unwrap();
    let err = install_tree("bogus", &fake("bogus"), home.path()).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Unknown companion 'bogus'. Allowed: specmint-core, specmint-tdd, specmint-core-html, specmint-tdd-html"
    );
    assert!(!companions_dir(home.path()).exists());
    assert!(install("bogus", home.path()).is_err());
}

#[test]
fn verify_runtime_dir_accepts_the_allowlist() {
    let tmp = TempDir::new().unwrap();
    let d = tmp.path().join("c");
    fs::create_dir_all(d.join("references")).unwrap();
    fs::create_dir_all(d.join("assets")).unwrap();
    fs::write(d.join("SKILL.md"), "x").unwrap();
    verify_runtime_dir(&d).unwrap();
}

#[test]
fn verify_runtime_dir_rejects_entries_outside_allowlist() {
    type Make = fn(&Path);
    let cases: [(&str, Make); 4] = [
        ("README.md", |d| fs::write(d.join("README.md"), "x").unwrap()),
        ("evals", |d| fs::create_dir(d.join("evals")).unwrap()),
        (".DS_Store", |d| {
            fs::write(d.join("references").join(".DS_Store"), "x").unwrap()
        }),
        (".hidden", |d| {
            fs::create_dir(d.join("references").join(".hidden")).unwrap()
        }),
    ];
    for (what, make) in cases {
        let tmp = TempDir::new().unwrap();
        let d = tmp.path().join("c");
        fs::create_dir_all(d.join("references")).unwrap();
        fs::write(d.join("SKILL.md"), "x").unwrap();
        verify_runtime_dir(&d).unwrap();
        make(&d);
        let err = verify_runtime_dir(&d).expect_err(what);
        assert!(err.to_string().contains(what), "{what}: {err}");
    }
}

#[test]
fn verify_runtime_dir_rejects_allowlisted_name_of_wrong_kind() {
    let tmp = TempDir::new().unwrap();
    let d = tmp.path().join("c");
    fs::create_dir(&d).unwrap();
    fs::write(d.join("SKILL.md"), "x").unwrap();
    fs::write(d.join("assets"), "not a dir").unwrap();

    let err = verify_runtime_dir(&d).unwrap_err();
    assert!(err.to_string().contains("assets"), "{err}");
}

#[test]
fn verify_runtime_dir_needs_skill_md() {
    let tmp = TempDir::new().unwrap();
    let d = tmp.path().join("c");
    fs::create_dir_all(d.join("SKILL.md")).unwrap();
    let err = verify_runtime_dir(&d).unwrap_err();
    assert!(err.to_string().contains("SKILL.md"), "{err}");
}

#[test]
fn verify_runtime_dir_rejects_links() {
    let tmp = TempDir::new().unwrap();
    let outside = tmp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("leak.md"), "secret").unwrap();
    let d = tmp.path().join("c");
    fs::create_dir_all(d.join("references")).unwrap();
    fs::write(d.join("SKILL.md"), "x").unwrap();
    if !symlink_file(&outside.join("leak.md"), &d.join("references").join("leak.md")) {
        eprintln!("symlinks unavailable; skipped");
        return;
    }
    let err = verify_runtime_dir(&d).unwrap_err();
    assert!(err.to_string().contains("link"), "{err}");

    let d2 = tmp.path().join("c2");
    fs::create_dir(&d2).unwrap();
    fs::write(d2.join("SKILL.md"), "x").unwrap();
    assert!(symlink_dir(&outside, &d2.join("references")));
    assert!(verify_runtime_dir(&d2).is_err());
}

#[test]
fn install_twice_over_stray_entry_at_companion_path() {
    for kind in ["file", "symlink", "dangling-symlink", "file-symlink"] {
        let tmp = TempDir::new().unwrap();
        let home = tmp.path().join("home");
        let dest = companions_dir(&home).join("specmint-core");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let elsewhere = tmp.path().join("elsewhere");
        match kind {
            "file" => fs::write(&dest, "stray").unwrap(),
            "symlink" => {
                fs::create_dir(&elsewhere).unwrap();
                fs::write(elsewhere.join("keep.md"), "mine").unwrap();
                if !symlink_dir(&elsewhere, &dest) {
                    eprintln!("symlinks unavailable; skipped");
                    return;
                }
            }
            "dangling-symlink" => {
                if !symlink_dir(&tmp.path().join("missing"), &dest) {
                    return;
                }
            }
            _ => {
                fs::write(tmp.path().join("target.md"), "mine").unwrap();
                if !symlink_file(&tmp.path().join("target.md"), &dest) {
                    return;
                }
            }
        }

        install_tree("specmint-core", &fake("specmint-core"), &home).unwrap();
        install_tree("specmint-core", &fake("specmint-core"), &home).unwrap();

        let meta = fs::symlink_metadata(&dest).unwrap();
        assert!(meta.is_dir() && !meta.file_type().is_symlink(), "{kind}");
        assert_eq!(tree(&dest), FOLDER, "{kind}");
        assert_eq!(names_in(&companions_dir(&home)), ["specmint-core"], "{kind}");
        if kind == "symlink" {
            assert_eq!(
                fs::read_to_string(elsewhere.join("keep.md")).unwrap(),
                "mine",
                "the link's target is left alone"
            );
        }
        if kind == "file-symlink" {
            assert_eq!(
                fs::read_to_string(tmp.path().join("target.md")).unwrap(),
                "mine"
            );
        }
    }
}

#[test]
fn install_leaves_no_leftover_after_stray_backup_and_staging() {
    for as_dir in [false, true] {
        let home = TempDir::new().unwrap();
        let parent = companions_dir(home.path());
        fs::create_dir_all(&parent).unwrap();
        for stray in [".specmint-core.old", ".specmint-core.tmp"] {
            if as_dir {
                fs::create_dir_all(parent.join(stray).join("references")).unwrap();
                fs::write(parent.join(stray).join("references").join("x"), "x").unwrap();
            } else {
                fs::write(parent.join(stray), "stray").unwrap();
            }
        }

        install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();

        assert_eq!(names_in(&parent), ["specmint-core"]);
        assert_eq!(tree(&parent.join("specmint-core")), FOLDER);
    }
}

#[test]
fn refresh_replaces_old_single_file_install() {
    let home = TempDir::new().unwrap();
    let old = companions_dir(home.path()).join("specmint-core");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("SKILL.md"), "old single-file playbook").unwrap();

    install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();

    assert_eq!(tree(&old), FOLDER);
    assert_eq!(
        fs::read_to_string(old.join("SKILL.md")).unwrap(),
        "# specmint-core\n"
    );
    assert_eq!(names_in(&companions_dir(home.path())), ["specmint-core"]);
}

#[test]
fn refresh_replaces_old_single_file_install_with_the_embedded_folder() {
    let home = TempDir::new().unwrap();
    let old = companions_dir(home.path()).join("specmint-tdd-html");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("SKILL.md"), "old single-file playbook").unwrap();

    refresh("specmint-tdd-html", home.path()).unwrap();

    assert_eq!(files(&old), files(&vendored().join("specmint-tdd-html")));
}

#[test]
fn refresh_drops_files_no_longer_vendored() {
    let home = TempDir::new().unwrap();
    install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();
    let dest = installed_dir("specmint-core", home.path());
    fs::write(dest.join("references").join("retired.md"), "gone").unwrap();

    install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();

    assert_eq!(tree(&dest), FOLDER);
}

#[test]
fn install_keeps_existing_dir_when_the_source_has_no_skill_md() {
    let home = TempDir::new().unwrap();
    install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();
    let skill = installed_dir("specmint-core", home.path()).join("SKILL.md");
    fs::write(&skill, "old").unwrap();

    let source = CompanionTree::new().file("references/researcher.md", b"brief\n".to_vec());
    let err = install_tree("specmint-core", &source, home.path()).unwrap_err();

    assert!(err.to_string().contains("SKILL.md"), "{err}");
    assert_eq!(fs::read_to_string(&skill).unwrap(), "old");
    assert_eq!(names_in(&companions_dir(home.path())), ["specmint-core"]);
}

#[test]
fn install_keeps_existing_dir_on_copy_failure() {
    let home = TempDir::new().unwrap();
    install_tree("specmint-core", &fake("specmint-core"), home.path()).unwrap();
    let dest = installed_dir("specmint-core", home.path());
    fs::write(dest.join("references").join("researcher.md"), "old").unwrap();

    // `references/a` cannot be both a file and a folder: the staging copy fails.
    let source = fake("specmint-core")
        .file("references/a", b"file".to_vec())
        .file("references/a/b", b"nested".to_vec());
    assert!(install_tree("specmint-core", &source, home.path()).is_err());

    assert_eq!(
        fs::read_to_string(dest.join("references").join("researcher.md")).unwrap(),
        "old"
    );
    assert_eq!(names_in(&companions_dir(home.path())), ["specmint-core"]);
}

#[test]
fn refresh_overwrites_user_modifications() {
    let home = TempDir::new().unwrap();
    install("specmint-core", home.path()).unwrap();
    let skill = installed_dir("specmint-core", home.path()).join("SKILL.md");
    fs::write(&skill, "garbage").unwrap();

    refresh("specmint-core", home.path()).unwrap();

    assert_eq!(
        fs::read(&skill).unwrap(),
        fs::read(vendored().join("specmint-core").join("SKILL.md")).unwrap()
    );
}

#[test]
fn refresh_idempotent_back_to_back() {
    let home = TempDir::new().unwrap();
    refresh("specmint-core", home.path()).unwrap();
    refresh("specmint-core", home.path()).unwrap();
    assert!(is_installed("specmint-core", home.path()));
    assert_eq!(names_in(&companions_dir(home.path())), ["specmint-core"]);
}

#[test]
fn installed_lists_existing_paths_and_is_installed_needs_skill_md() {
    let home = TempDir::new().unwrap();
    assert!(installed(home.path()).is_empty());
    let parent = companions_dir(home.path());
    fs::create_dir_all(&parent).unwrap();
    // v2 `installed()`: the path exists, whatever it is.
    fs::write(parent.join("specmint-tdd-html"), "stray").unwrap();
    install("specmint-core", home.path()).unwrap();
    fs::create_dir(parent.join("specmint-tdd")).unwrap();

    assert_eq!(
        installed(home.path()),
        ["specmint-core", "specmint-tdd", "specmint-tdd-html"]
    );
    assert!(is_installed("specmint-core", home.path()));
    assert!(!is_installed("specmint-tdd", home.path()));
    assert!(!is_installed("specmint-tdd-html", home.path()));
    assert!(!is_installed("specmint-core-html", home.path()));
}

// --- the embedded folders --------------------------------------------------------

#[test]
fn embedded_trees_equal_vendored_byte_for_byte() {
    for name in KNOWN {
        let embedded = companions::embedded(name).expect(name);
        let mut got: Vec<(String, Vec<u8>)> = embedded
            .files()
            .map(|(rel, bytes)| (rel.to_string(), bytes.to_vec()))
            .collect();
        got.sort();
        assert_eq!(got, files(&vendored().join(name)), "{name}");
    }
    assert!(companions::embedded("bogus").is_none());
}

#[test]
fn every_embedded_companion_installs_as_vendored() {
    let home = TempDir::new().unwrap();
    for name in KNOWN {
        install(name, home.path()).unwrap();
        let dest = installed_dir(name, home.path());
        assert_eq!(tree(&dest), tree(&vendored().join(name)), "{name}");
        assert_eq!(files(&dest), files(&vendored().join(name)), "{name}");
        verify_runtime_dir(&dest).unwrap();
    }
    assert_eq!(installed(home.path()), KNOWN);
}

#[test]
fn vendored_folders_match_the_allowlist() {
    for name in KNOWN {
        let root = vendored().join(name);
        let top = names_in(&root);
        assert!(top.contains(&"SKILL.md".to_string()), "{name}");
        for entry in &top {
            assert!(
                ["SKILL.md", "references", "assets"].contains(&entry.as_str()),
                "{name}: {entry}"
            );
        }
        assert!(root.join("references").is_dir(), "{name}");
        for rel in tree(&root) {
            let last = rel.rsplit('/').next().unwrap_or(&rel);
            assert!(!last.starts_with('.'), "{name}: {rel}");
            assert_ne!(last, "__pycache__", "{name}: {rel}");
            let meta = fs::symlink_metadata(root.join(&rel)).unwrap();
            assert!(!meta.file_type().is_symlink(), "{name}: {rel}");
        }
        verify_runtime_dir(&root).unwrap();
        if name.ends_with("-html") {
            assert!(root.join("assets").join("spec-runtime.js").is_file());
            assert!(root.join("assets").join("spec-styles.css").is_file());
        }
    }
}

// --- opt-ins ----------------------------------------------------------------------

const V2_KLURIS_YML: &str = "name: a\ndescription: a knowledge base\nagents:\n  commands_for:\n  - claude\n  - cursor\n  - windsurf\n  - copilot\n  - codex\n  - gemini\n  - hermes\n  - kilocode\n  - junie\ncompanions: []\n";

fn brain(root: &Path, name: &str, kluris_yml: Option<&str>) -> ResolvedBrain {
    let path = root.join(name);
    fs::create_dir_all(&path).unwrap();
    if let Some(text) = kluris_yml {
        fs::write(path.join("kluris.yml"), text).unwrap();
    }
    ResolvedBrain {
        name: name.to_string(),
        entry: BrainEntry {
            path: path.to_string_lossy().into_owned(),
            description: String::new(),
        },
    }
}

fn yml(name: &str, companions: &[&str]) -> String {
    let list = if companions.is_empty() {
        " []\n".to_string()
    } else {
        format!(
            "\n{}",
            companions
                .iter()
                .map(|c| format!("- {c}\n"))
                .collect::<String>()
        )
    };
    format!(
        "name: {name}\ndescription: ''\nagents:\n  commands_for:\n  - claude\ncompanions:{list}"
    )
}

#[test]
fn load_brains_reads_every_kluris_yml_first() {
    let tmp = TempDir::new().unwrap();
    let a = brain(tmp.path(), "a", Some(&yml("a", &["specmint-tdd"])));
    let missing = brain(tmp.path(), "b", None);
    let invalid = brain(tmp.path(), "c", Some("name: [x\n"));

    let loaded = load_brains(std::slice::from_ref(&a)).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "a");
    assert_eq!(loaded[0].config.companions, ["specmint-tdd"]);

    let err = load_brains(&[a.clone(), missing]).unwrap_err();
    assert!(err.to_string().starts_with("Cannot access config file "), "{err}");
    assert!(err.to_string().contains("kluris.yml"), "{err}");
    let err = load_brains(&[a, invalid]).unwrap_err();
    assert!(err.to_string().starts_with("Invalid config file "), "{err}");
}

#[test]
fn enabled_is_the_union_in_known_order() {
    let tmp = TempDir::new().unwrap();
    let brains = load_brains(&[
        brain(tmp.path(), "a", Some(&yml("a", &["specmint-tdd-html", "bogus"]))),
        brain(tmp.path(), "b", Some(&yml("b", &["specmint-tdd", "specmint-core"]))),
        brain(tmp.path(), "c", Some(&yml("c", &[]))),
    ])
    .unwrap();
    assert_eq!(
        enabled(&brains),
        ["specmint-core", "specmint-tdd", "specmint-tdd-html"]
    );
}

#[test]
fn add_installs_the_files_and_opts_every_brain_in() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let a = brain(tmp.path(), "a", Some(&yml("a", &["specmint-tdd-html", "bogus"])));
    let b = brain(tmp.path(), "b", Some(&yml("b", &["specmint-core"])));
    let brains = load_brains(&[a, b]).unwrap();

    let changed = companions::add(
        &brains,
        &["specmint-core".to_string(), "specmint-tdd".to_string()],
        &home,
    )
    .unwrap();

    // v2 rewrites every resolved brain, even one that had the companion.
    assert_eq!(changed, ["a", "b"]);
    assert_eq!(
        fs::read_to_string(tmp.path().join("a").join("kluris.yml")).unwrap(),
        yml("a", &["specmint-core", "specmint-tdd", "specmint-tdd-html"])
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("b").join("kluris.yml")).unwrap(),
        yml("b", &["specmint-core", "specmint-tdd"])
    );
    assert_eq!(installed(&home), ["specmint-core", "specmint-tdd"]);
}

#[test]
fn add_rewrites_a_v2_kluris_yml_keeping_crlf() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let a = brain(tmp.path(), "a", Some(&V2_KLURIS_YML.replace('\n', "\r\n")));
    let brains = load_brains(&[a]).unwrap();

    companions::add(&brains, &["specmint-core".to_string()], &home).unwrap();

    let expected = "name: a\ndescription: a knowledge base\nagents:\n  commands_for:\n  - claude\n  - cursor\n  - devin\n  - copilot\n  - codex\n  - gemini\n  - hermes\n  - kilocode\n  - junie\n  - antigravity\ncompanions:\n- specmint-core\n";
    assert_eq!(
        fs::read_to_string(tmp.path().join("a").join("kluris.yml")).unwrap(),
        expected.replace('\n', "\r\n")
    );
}

#[test]
fn remove_touches_only_brains_that_had_the_companion() {
    // research-03 §5 #25: v2 rewrites (and reports) every resolved brain.
    let tmp = TempDir::new().unwrap();
    let a = brain(tmp.path(), "a", Some(&yml("a", &["specmint-core", "specmint-tdd"])));
    let b = brain(tmp.path(), "b", Some(V2_KLURIS_YML));
    let c = brain(tmp.path(), "c", Some(&yml("c", &["specmint-tdd", "bogus"])));
    let brains = load_brains(&[a, b, c]).unwrap();

    let changed = companions::remove(&brains, &["specmint-core".to_string()]).unwrap();

    assert_eq!(changed, ["a"]);
    assert_eq!(
        fs::read_to_string(tmp.path().join("a").join("kluris.yml")).unwrap(),
        yml("a", &["specmint-tdd"])
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("b").join("kluris.yml")).unwrap(),
        V2_KLURIS_YML,
        "a brain without the companion is not rewritten"
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("c").join("kluris.yml")).unwrap(),
        yml("c", &["specmint-tdd", "bogus"])
    );

    let brains = load_brains(&[brain(tmp.path(), "c", None)]).unwrap();
    let changed = companions::remove(
        &brains,
        &["specmint-tdd".to_string(), "specmint-core".to_string()],
    )
    .unwrap();
    assert_eq!(changed, ["c"]);
    assert_eq!(
        fs::read_to_string(tmp.path().join("c").join("kluris.yml")).unwrap(),
        yml("c", &[]),
        "a brain that had it is normalized as v2 does (unknown names dropped)"
    );
}

#[test]
fn remove_keeps_the_runtime_files() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let brains = load_brains(&[brain(tmp.path(), "a", Some(&yml("a", &[])))]).unwrap();
    companions::add(&brains, &["specmint-core".to_string()], &home).unwrap();
    let brains = load_brains(&[brain(tmp.path(), "a", None)]).unwrap();
    assert_eq!(
        companions::remove(&brains, &["specmint-core".to_string()]).unwrap(),
        ["a"]
    );
    assert!(is_installed("specmint-core", &home));
}

#[test]
fn referenced_reads_known_companions_from_brain_configs() {
    let tmp = TempDir::new().unwrap();
    let a = brain(
        tmp.path(),
        "a",
        Some("name: a\ncompanions:\n- specmint-tdd-html\n- unknown\n- specmint-core\n"),
    );
    let b = brain(tmp.path(), "b", Some("name: [broken\n"));
    let c = brain(tmp.path(), "c", None);
    let mut brains = IndexMap::new();
    for r in [a, b, c] {
        brains.insert(r.name, r.entry);
    }
    brains.insert(
        "gone".to_string(),
        BrainEntry {
            path: tmp.path().join("gone").to_string_lossy().into_owned(),
            description: String::new(),
        },
    );

    assert_eq!(
        referenced(&GlobalConfig { brains }),
        ["specmint-core", "specmint-tdd-html"]
    );
}
