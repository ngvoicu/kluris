//! A receipt Kluris's installer did not write, or one edited to name other
//! files, never authorises a change: `kluris self-uninstall` and `kluris
//! self-update` refuse before touching anything (T-111 finding 3; the first
//! two cases are the reviewer's probe, /tmp/kluris-t111-ownership-probe.py).
//!
//! The receipt they accept is dist 0.33.1's own, as its installers write it
//! (see `dist_receipt`): `source` kluris/kluris/ngvoicu/github, `provider`
//! cargo-dist, `binaries` exactly `["kluris"]` (`["kluris.exe"]` on
//! Windows), no cdylibs, cstaticlibs or aliases, the flat layout, an
//! absolute `install_prefix` and a boolean `modify_path`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

const REFUSED: &str = "is not a Kluris install receipt; nothing was changed";

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "kluris.exe"
    } else {
        "kluris"
    }
}

/// dist 0.33.1's receipt for an install into `prefix`.
fn dist_receipt(prefix: &Path) -> Value {
    json!({
        "binaries": [exe_name()],
        "binary_aliases": {},
        "cdylibs": [],
        "cstaticlibs": [],
        "install_layout": "flat",
        "install_prefix": prefix.display().to_string(),
        "modify_path": true,
        "provider": {"source": "cargo-dist", "version": "0.33.1"},
        "source": {"app_name": "kluris", "name": "kluris", "owner": "ngvoicu", "release_type": "github"},
        "version": "3.0.0"
    })
}

/// A HOME with a receipt, the files it names, an rc line and the user's own
/// file `victim`.
struct Case {
    _root: TempDir,
    home: PathBuf,
    receipt: PathBuf,
    receipt_text: String,
    profile: PathBuf,
    profile_text: String,
    victim: PathBuf,
    binary: PathBuf,
}

/// `prefix_rel` under HOME holds `notes.txt` (the user's) and a `kluris`
/// binary; `edit` turns dist's receipt into the case's.
fn case(prefix_rel: &str, edit: impl FnOnce(&mut Value, &Path)) -> Case {
    let root = TempDir::new().unwrap();
    let home = fs::canonicalize(root.path()).unwrap().join("home");
    let prefix = home.join(prefix_rel);
    let config = home.join(".config").join("kluris");
    fs::create_dir_all(&prefix).unwrap();
    fs::create_dir_all(&config).unwrap();
    let victim = prefix.join("notes.txt");
    fs::write(&victim, "USER OWNED DATA\n").unwrap();
    let binary = prefix.join(exe_name());
    fs::write(&binary, b"installed binary").unwrap();
    fs::write(config.join("env.sh"), "export USER_SENTINEL=1\n").unwrap();
    let profile = home.join(".profile");
    let profile_text = "export A=1\n\n. \"$HOME/.config/kluris/env.sh\"\n".to_string();
    fs::write(&profile, &profile_text).unwrap();
    let mut receipt = dist_receipt(&prefix);
    edit(&mut receipt, &home);
    let receipt_text = format!("{receipt}\n");
    let receipt_path = config.join("kluris-receipt.json");
    fs::write(&receipt_path, &receipt_text).unwrap();
    Case {
        _root: root,
        home,
        receipt: receipt_path,
        receipt_text,
        profile,
        profile_text,
        victim,
        binary,
    }
}

impl Case {
    fn run(&self, args: &[&str]) -> Output {
        Command::new(assert_cmd::cargo::cargo_bin!("kluris"))
            .args(args)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("LOCALAPPDATA", self.home.join("AppData").join("Local"))
            .env(
                "KLURIS_CONFIG",
                self.home.join(".kluris").join("config.yml"),
            )
            // Nothing listens there: a command past the receipt check would
            // fail on the network, not with the refusal.
            .env("KLURIS_GITHUB_API_URL", "http://127.0.0.1:9")
            .env("KLURIS_NO_PROMPT", "1")
            .env("NO_COLOR", "1")
            .env_remove("ZDOTDIR")
            .env_remove("KLURIS_GITHUB_TOKEN")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    /// The command refused with the message and changed nothing.
    fn assert_refused(&self, args: &[&str]) {
        let out = self.run(args);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.status.code(), Some(1), "{args:?}: {text}");
        assert!(text.contains(REFUSED), "{args:?}: {text}");
        assert!(
            text.contains(&self.receipt.display().to_string().replace('\\', "\\\\"))
                || text.contains(&self.receipt.display().to_string()),
            "the refusal names the receipt: {text}"
        );
        assert_eq!(
            fs::read_to_string(&self.victim).unwrap(),
            "USER OWNED DATA\n"
        );
        assert!(self.binary.exists(), "{args:?} removed the binary");
        assert_eq!(
            fs::read_to_string(&self.receipt).unwrap(),
            self.receipt_text
        );
        assert_eq!(
            fs::read_to_string(&self.profile).unwrap(),
            self.profile_text
        );
        assert!(self.home.join(".config/kluris/env.sh").exists());
    }

    fn assert_every_command_refuses(&self) {
        for args in [
            &["self-uninstall", "--yes", "--json"][..],
            &["self-uninstall"][..],
            &["self-uninstall", "--purge", "--yes", "--json"][..],
            &["self-update", "--check", "--json"][..],
            &["self-update", "--json"][..],
            &["self-update", "--version", "3.0.1"][..],
        ] {
            self.assert_refused(args);
        }
    }
}

#[test]
fn a_foreign_receipt_authorises_nothing() {
    // The reviewer's first probe: another app's receipt naming a file in
    // ~/Documents.
    let c = case("Documents", |r, _| {
        r["binaries"] = json!(["notes.txt"]);
        r["source"] = json!({"owner": "someone-else", "name": "other-app"});
    });
    c.assert_every_command_refuses();
}

#[test]
fn an_edited_kluris_receipt_authorises_nothing() {
    // The reviewer's second probe: Kluris's receipt with `binaries` edited.
    let c = case(".kluris/bin", |r, _| r["binaries"] = json!(["notes.txt"]));
    c.assert_every_command_refuses();
}

#[test]
fn every_field_that_says_whose_install_it_is_must_be_dists() {
    type Edit = Box<dyn FnOnce(&mut Value, &Path)>;
    let other_exe = if cfg!(windows) {
        "kluris"
    } else {
        "kluris.exe"
    };
    let cases: Vec<(&str, Edit)> = vec![
        (
            "another binary too",
            Box::new(|r, _| r["binaries"] = json!([exe_name(), "notes.txt"])),
        ),
        ("no binaries", Box::new(|r, _| r["binaries"] = json!([]))),
        (
            "the other platform's binary",
            Box::new(move |r, _| r["binaries"] = json!([other_exe])),
        ),
        (
            "a binary with a folder",
            Box::new(|r, _| r["binaries"] = json!([format!("../{}", exe_name())])),
        ),
        (
            "a cdylib",
            Box::new(|r, _| r["cdylibs"] = json!(["libnotes.so"])),
        ),
        (
            "a cstaticlib",
            Box::new(|r, _| r["cstaticlibs"] = json!(["libnotes.a"])),
        ),
        (
            "an alias",
            Box::new(|r, _| r["binary_aliases"] = json!({"kluris": ["notes.txt"]})),
        ),
        (
            "the hierarchical layout",
            Box::new(|r, _| r["install_layout"] = json!("hierarchical")),
        ),
        (
            "no layout",
            Box::new(|r, _| {
                r.as_object_mut().unwrap().remove("install_layout");
            }),
        ),
        (
            "a relative prefix",
            Box::new(|r, _| r["install_prefix"] = json!(".kluris/bin")),
        ),
        (
            "a ~ prefix",
            Box::new(|r, _| r["install_prefix"] = json!("~/.kluris/bin")),
        ),
        (
            "another provider",
            Box::new(|r, _| r["provider"]["source"] = json!("homebrew")),
        ),
        (
            "no provider",
            Box::new(|r, _| {
                r.as_object_mut().unwrap().remove("provider");
            }),
        ),
        (
            "another app name",
            Box::new(|r, _| r["source"]["app_name"] = json!("kluris-cli")),
        ),
        (
            "another package name",
            Box::new(|r, _| r["source"]["name"] = json!("notes")),
        ),
        (
            "another owner",
            Box::new(|r, _| r["source"]["owner"] = json!("someone-else")),
        ),
        (
            "another release type",
            Box::new(|r, _| r["source"]["release_type"] = json!("axodotdev")),
        ),
        (
            "no source",
            Box::new(|r, _| {
                r.as_object_mut().unwrap().remove("source");
            }),
        ),
        (
            "no modify_path",
            Box::new(|r, _| {
                r.as_object_mut().unwrap().remove("modify_path");
            }),
        ),
        (
            "a modify_path that is no boolean",
            Box::new(|r, _| r["modify_path"] = json!("no")),
        ),
    ];
    for (what, edit) in cases {
        let c = case(".kluris/bin", edit);
        let out = c.run(&["self-uninstall", "--yes", "--json"]);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(out.status.code(), Some(1), "{what}: {text}");
        assert!(text.contains(REFUSED), "{what}: {text}");
        assert!(
            c.binary.exists() && c.victim.exists(),
            "{what}: removed something"
        );
        assert_eq!(
            fs::read_to_string(&c.receipt).unwrap(),
            c.receipt_text,
            "{what}"
        );
        let out = c.run(&["self-update", "--check", "--json"]);
        assert_eq!(out.status.code(), Some(1), "{what}: self-update --check");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(REFUSED)
                || String::from_utf8_lossy(&out.stderr).contains(REFUSED),
            "{what}: self-update --check: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

#[test]
fn dists_own_receipt_is_accepted() {
    // The control: the receipt as dist writes it gets past the check (the
    // uninstall then removes the binary, and only it of the prefix).
    let c = case(".kluris/bin", |_, _| {});
    let out = c.run(&["self-uninstall", "--yes", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(!c.binary.exists());
    assert_eq!(fs::read_to_string(&c.victim).unwrap(), "USER OWNED DATA\n");
}
