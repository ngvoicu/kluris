//! `kluris self-uninstall` in a temp HOME holding what dist 0.33.1's
//! installers write, and `kluris_core::dist::uninstall` with an in-memory
//! `UserPath` for the Windows user PATH.
//!
//! The installer files are exact copies of what `kluris-installer.sh`
//! (dist 0.33.1, `install-path = "~/.kluris/bin"`, the flat layout) wrote
//! into a temp HOME on 2026-10-26: the env scripts go to the receipt's folder
//! (`~/.config/kluris/env.sh`, `env.fish`), every rc file gets an empty line
//! then `. "$HOME/.config/kluris/env.sh"`, and the fish conf.d file the same
//! with `source`. Only what that installer writes is removed: env scripts
//! beside the binary (`~/.kluris/bin/env`, where older dist versions put
//! them) were never written for Kluris and are left alone.
//!
//! The opt-in test `real_windows_user_path_round_trip` changes the real
//! HKCU PATH of the machine it runs on and restores it; it runs only with
//! `--ignored` on Windows (`cargo xtask windows -- --test self_uninstall --
//! --ignored`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use kluris_core::agents::render::MARKER;
use kluris_core::dist::{
    DistEnv, Receipt, UninstallError, UninstallRequest, UserPath, remove_path_entry, uninstall,
};
use tempfile::TempDir;

const ENV_SH: &str = r#"#!/bin/sh
# add binaries to PATH if they aren't added yet
# affix colons on either side of $PATH to simplify matching
case ":${PATH}:" in
    *:"$HOME/.kluris/bin":*)
        ;;
    *)
        # Prepending path in case a system-installed binary needs to be overridden
        export PATH="$HOME/.kluris/bin:$PATH"
        ;;
esac
"#;

const ENV_FISH: &str = r#"if not contains "$HOME/.kluris/bin" $PATH
    # Prepending path in case a system-installed binary needs to be overridden
    set -x PATH "$HOME/.kluris/bin" $PATH
end
"#;

const SH_LINE: &str = r#". "$HOME/.config/kluris/env.sh""#;
const FISH_LINE: &str = r#"source "$HOME/.config/kluris/env.fish""#;
const LEGACY_SH_LINE: &str = r#". "$HOME/.kluris/bin/env""#;

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "kluris.exe"
    } else {
        "kluris"
    }
}

/// `echo "" >> rc; echo "<line>" >> rc`, as the installer appends.
fn appended(original: &str, line: &str) -> String {
    format!("{original}\n{line}\n")
}

/// A Kluris-written skill (v3's marker after the frontmatter).
fn generated_skill(name: &str) -> String {
    format!("---\nname: {name}\ndescription: brain {name}\n---\n\n{MARKER}\n\n# Brain\n")
}

struct Home {
    _root: TempDir,
    root: PathBuf,
    home: PathBuf,
    zdotdir: PathBuf,
    config_home: PathBuf,
    prefix: PathBuf,
    receipt: PathBuf,
    /// rc file -> its bytes before the installer touched it.
    originals: Vec<(PathBuf, String)>,
}

impl Home {
    /// A HOME after a dist 0.33.1 install (ZDOTDIR set), with the user's
    /// config, companions, a brain and skills.
    fn installed() -> Home {
        let root_dir = TempDir::new().unwrap();
        let root = fs::canonicalize(root_dir.path()).unwrap();
        let home = root.join("home");
        let zdotdir = root.join("zdot");
        let config_home = home.join(".config");
        let prefix = home.join(".kluris").join("bin");
        let receipt_home = config_home.join("kluris");
        for d in [
            &prefix,
            &zdotdir,
            &receipt_home,
            &home.join(".config/fish/conf.d"),
        ] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(prefix.join(exe_name()), b"installed binary").unwrap();
        fs::write(receipt_home.join("env.sh"), ENV_SH).unwrap();
        fs::write(receipt_home.join("env.fish"), ENV_FISH).unwrap();
        let receipt = receipt_home.join("kluris-receipt.json");
        fs::write(
            &receipt,
            format!(
                "{{\"binaries\":[\"{}\"],\"binary_aliases\":{{}},\"cdylibs\":[],\"cstaticlibs\":[],\
                 \"install_layout\":\"flat\",\"install_prefix\":{},\"modify_path\":true,\
                 \"provider\":{{\"source\":\"cargo-dist\",\"version\":\"0.33.1\"}},\
                 \"source\":{{\"app_name\":\"kluris\",\"name\":\"kluris\",\"owner\":\"ngvoicu\",\
                 \"release_type\":\"github\"}},\"version\":\"3.0.0\"}}\n",
                exe_name(),
                serde_json::to_string(&prefix.display().to_string()).unwrap()
            ),
        )
        .unwrap();
        let originals = vec![
            (home.join(".profile"), "export A=1\n# keep me\n".to_string()),
            (
                home.join(".bashrc"),
                // A line that only looks like ours stays.
                format!("alias ll='ls -l'\n{SH_LINE} # mine\n  {SH_LINE}\n"),
            ),
            (zdotdir.join(".zshrc"), "setopt foo\n\n".to_string()),
        ];
        for (path, original) in &originals {
            fs::write(path, appended(original, SH_LINE)).unwrap();
        }
        fs::write(
            home.join(".config/fish/conf.d/kluris.env.fish"),
            appended("", FISH_LINE),
        )
        .unwrap();

        // The user's own things.
        let kluris = home.join(".kluris");
        fs::write(kluris.join("config.yml"), "brains: {}\n").unwrap();
        fs::create_dir_all(kluris.join("companions/specmint-core")).unwrap();
        fs::write(kluris.join("companions/specmint-core/SKILL.md"), "# c\n").unwrap();
        fs::create_dir_all(home.join("brains/b1")).unwrap();
        fs::write(home.join("brains/b1/brain.md"), "# b1\n").unwrap();
        for folder in [".agents/skills", ".claude/skills"] {
            let dir = home.join(folder).join("kluris-b1");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("SKILL.md"), generated_skill("kluris-b1")).unwrap();
        }
        let own = home.join(".agents/skills/kluris-notes");
        fs::create_dir_all(&own).unwrap();
        fs::write(
            own.join("SKILL.md"),
            "---\nname: kluris-notes\ndescription: mine\n---\n",
        )
        .unwrap();
        Home {
            _root: root_dir,
            root,
            home,
            zdotdir,
            config_home,
            prefix,
            receipt,
            originals,
        }
    }

    /// Env scripts beside the binary and an rc line to them, as dist before
    /// 0.33 wrote for other apps; no Kluris installer ever wrote these.
    fn with_files_dist_never_wrote(self) -> Home {
        fs::write(self.prefix.join("env"), ENV_SH).unwrap();
        fs::write(self.prefix.join("env.fish"), ENV_FISH).unwrap();
        let bash_profile = self.home.join(".bash_profile");
        fs::write(&bash_profile, appended("PS1='$ '\n", LEGACY_SH_LINE)).unwrap();
        self
    }

    fn env(&self) -> DistEnv<'_> {
        DistEnv {
            home: &self.home,
            xdg_config_home: Some(&self.config_home),
            local_app_data: None,
            zdotdir: Some(&self.zdotdir),
            windows: cfg!(windows),
        }
    }

    fn receipt(&self) -> Receipt {
        Receipt::read(&self.receipt).unwrap()
    }

    fn cmd(&self, exe: &Path) -> Command {
        let mut cmd = Command::new(exe);
        cmd.env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("ZDOTDIR", &self.zdotdir)
            .env(
                "KLURIS_CONFIG",
                self.home.join(".kluris").join("config.yml"),
            )
            .env("NO_COLOR", "1")
            .env("GIT_CEILING_DIRECTORIES", &self.root)
            .env_remove("KLURIS_NO_PROMPT")
            .env_remove("COPILOT_HOME")
            .stdin(Stdio::null());
        cmd
    }

    fn kluris(&self, args: &[&str]) -> Output {
        self.cmd(Path::new(assert_cmd::cargo::cargo_bin!("kluris")))
            .args(args)
            .output()
            .unwrap()
    }

    fn assert_installer_files_gone(&self) {
        for gone in [
            self.prefix.join(exe_name()),
            self.prefix.clone(),
            self.config_home.join("kluris/env.sh"),
            self.config_home.join("kluris/env.fish"),
            self.receipt.clone(),
            self.config_home.join("kluris"),
            self.home.join(".config/fish/conf.d/kluris.env.fish"),
        ] {
            assert!(!gone.exists(), "{} is still there", gone.display());
        }
    }

    fn assert_rc_files_restored(&self) {
        for (path, original) in &self.originals {
            assert_eq!(
                fs::read_to_string(path).unwrap(),
                *original,
                "{}",
                path.display()
            );
        }
    }

    fn assert_user_things_kept(&self) {
        for kept in [
            ".kluris/config.yml",
            ".kluris/companions/specmint-core/SKILL.md",
            "brains/b1/brain.md",
            ".agents/skills/kluris-b1/SKILL.md",
            ".claude/skills/kluris-b1/SKILL.md",
            ".agents/skills/kluris-notes/SKILL.md",
            ".config/fish/conf.d",
        ] {
            assert!(self.home.join(kept).exists(), "{kept} was removed");
        }
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// `p` as the JSON lists it (`/`-joined).
fn slashed(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// An in-memory HKCU `Path`.
#[derive(Default)]
struct FakeUserPath {
    value: Option<String>,
    writes: Vec<String>,
}

impl UserPath for FakeUserPath {
    fn read(&self) -> io::Result<Option<String>> {
        Ok(self.value.clone())
    }

    fn write(&mut self, value: &str) -> io::Result<()> {
        self.writes.push(value.to_string());
        self.value = Some(value.to_string());
        Ok(())
    }
}

fn request<'a>(h: &'a Home, receipt: &'a Receipt, purge: bool) -> UninstallRequest<'a> {
    UninstallRequest {
        env: h.env(),
        receipt,
        receipt_path: &h.receipt,
        purge,
        copilot_home: None,
        brains: &[],
    }
}

fn remove_file(p: &Path) -> io::Result<()> {
    fs::remove_file(p)
}

#[test]
fn removes_exactly_what_the_installer_wrote() {
    let h = Home::installed();
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
    h.assert_user_things_kept();
    assert!(h.home.join(".kluris").is_dir(), "~/.kluris itself stays");
    let mut edited = report.edited.clone();
    edited.sort();
    let mut expected = vec![
        h.home.join(".bashrc"),
        h.home.join(".profile"),
        h.zdotdir.join(".zshrc"),
    ];
    expected.sort();
    assert_eq!(edited, expected);
    assert!(report.removed.contains(&h.prefix.join(exe_name())));
    assert!(!report.purged);
}

#[test]
fn a_rc_file_without_a_final_newline_keeps_the_newline_the_installer_added() {
    // `echo "" >>` ended the user's last line; that newline stays.
    let h = Home::installed();
    let rc = h.home.join(".bash_login");
    fs::write(&rc, format!("export B=2\n{SH_LINE}\n")).unwrap();
    let receipt = h.receipt();
    uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert_eq!(fs::read_to_string(&rc).unwrap(), "export B=2\n");
}

#[test]
fn crlf_rc_files_and_lines_in_the_middle() {
    let h = Home::installed();
    let rc = h.home.join(".zshenv");
    fs::write(
        h.zdotdir.join(".zshenv"),
        format!("a\r\n\r\n{SH_LINE}\r\nb\r\n"),
    )
    .unwrap();
    let receipt = h.receipt();
    uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert_eq!(
        fs::read_to_string(h.zdotdir.join(".zshenv")).unwrap(),
        "a\r\nb\r\n"
    );
    assert!(
        !rc.exists(),
        "ZDOTDIR is set: $HOME/.zshenv is not one of them"
    );
}

#[test]
fn zsh_files_follow_zdotdir_and_home_without_it() {
    let h = Home::installed();
    let home_zshrc = h.home.join(".zshrc");
    let text = appended("keep\n", SH_LINE);
    fs::write(&home_zshrc, &text).unwrap();
    let receipt = h.receipt();
    uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert_eq!(
        fs::read_to_string(&home_zshrc).unwrap(),
        text,
        "ZDOTDIR wins"
    );

    let h = Home::installed();
    let home_zshrc = h.home.join(".zshrc");
    fs::write(&home_zshrc, &text).unwrap();
    let receipt = h.receipt();
    let mut req = request(&h, &receipt, false);
    req.env.zdotdir = None;
    uninstall(&req, None, &mut remove_file).unwrap();
    assert_eq!(fs::read_to_string(&home_zshrc).unwrap(), "keep\n");
}

#[test]
fn files_the_installer_never_wrote_are_kept() {
    let h = Home::installed().with_files_dist_never_wrote();
    let bash_profile = fs::read(h.home.join(".bash_profile")).unwrap();
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(fs::read_to_string(h.prefix.join("env")).unwrap(), ENV_SH);
    assert_eq!(
        fs::read_to_string(h.prefix.join("env.fish")).unwrap(),
        ENV_FISH
    );
    assert_eq!(
        fs::read(h.home.join(".bash_profile")).unwrap(),
        bash_profile
    );
    assert!(!h.prefix.join(exe_name()).exists());
    assert!(!h.receipt.exists());
    h.assert_rc_files_restored();
}

#[test]
fn a_fish_conf_file_with_the_users_own_lines_keeps_them() {
    let h = Home::installed();
    let conf = h.home.join(".config/fish/conf.d/kluris.env.fish");
    fs::write(&conf, format!("set -x X 1\n\n{FISH_LINE}\n")).unwrap();
    let receipt = h.receipt();
    uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert_eq!(fs::read_to_string(&conf).unwrap(), "set -x X 1\n");
}

#[test]
fn a_custom_xdg_config_home_is_followed() {
    // The receipt and env scripts live under $XDG_CONFIG_HOME/kluris; the rc
    // line names the env script with $HOME in front when it is under HOME.
    let h = Home::installed();
    let xdg = h.home.join("xdg");
    fs::create_dir_all(xdg.join("kluris")).unwrap();
    for name in ["env.sh", "env.fish", "kluris-receipt.json"] {
        fs::rename(
            h.config_home.join("kluris").join(name),
            xdg.join("kluris").join(name),
        )
        .unwrap();
    }
    let line = r#". "$HOME/xdg/kluris/env.sh""#;
    fs::write(h.home.join(".profile"), appended("p\n", line)).unwrap();
    let receipt = Receipt::read(&xdg.join("kluris/kluris-receipt.json")).unwrap();
    let receipt_path = xdg.join("kluris/kluris-receipt.json");
    let mut req = request(&h, &receipt, false);
    req.env.xdg_config_home = Some(&xdg);
    req.receipt_path = &receipt_path;
    uninstall(&req, None, &mut remove_file).unwrap();
    assert_eq!(fs::read_to_string(h.home.join(".profile")).unwrap(), "p\n");
    assert!(!xdg.join("kluris").exists());
}

#[test]
fn the_user_path_entry_is_removed_through_user_path() {
    let h = Home::installed();
    let receipt = h.receipt();
    let entry = h.prefix.display().to_string();
    let mut fake = FakeUserPath {
        value: Some(format!("{entry};%USERPROFILE%\\bin;C:\\Other")),
        writes: Vec::new(),
    };
    let report = uninstall(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
    )
    .unwrap();
    assert_eq!(
        fake.writes,
        vec!["%USERPROFILE%\\bin;C:\\Other".to_string()]
    );
    assert_eq!(report.path_entries, vec![entry]);

    // Nothing of ours on PATH: nothing written.
    let h = Home::installed();
    let receipt = h.receipt();
    let mut fake = FakeUserPath {
        value: Some("C:\\Other;%USERPROFILE%\\.kluris\\bin2".into()),
        writes: Vec::new(),
    };
    let report = uninstall(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
    )
    .unwrap();
    assert!(fake.writes.is_empty());
    assert!(report.path_entries.is_empty());

    // No PATH value at all.
    let h = Home::installed();
    let receipt = h.receipt();
    let mut fake = FakeUserPath::default();
    uninstall(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
    )
    .unwrap();
    assert!(fake.writes.is_empty());
}

#[test]
fn remove_path_entry_matches_case_and_separators_and_keeps_the_rest() {
    let prefix = Path::new(r"C:\Users\u\.kluris\bin");
    // The installer's receipt has `C:\Users\u\.kluris/bin`; PowerShell's
    // `-in` compares without case.
    assert_eq!(
        remove_path_entry(r"c:\users\u\.KLURIS/bin\;D:\x;;E:\y", prefix),
        Some((
            r"D:\x;;E:\y".to_string(),
            vec![r"c:\users\u\.KLURIS/bin\".to_string()]
        ))
    );
    assert_eq!(
        remove_path_entry(
            r"D:\x;C:\Users\u\.kluris\bin;C:\Users\u\.kluris\bin",
            prefix
        ),
        Some((
            r"D:\x".to_string(),
            vec![
                r"C:\Users\u\.kluris\bin".to_string(),
                r"C:\Users\u\.kluris\bin".to_string()
            ]
        ))
    );
    assert_eq!(
        remove_path_entry(r"D:\x;C:\Users\u\.kluris\bin2", prefix),
        None
    );
    assert_eq!(remove_path_entry("", prefix), None);
}

#[test]
fn purge_removes_dot_kluris_and_kluris_written_skills_only() {
    let h = Home::installed();
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, true), None, &mut remove_file).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(report.purged);
    assert!(!h.home.join(".kluris").exists());
    assert!(!h.home.join(".agents/skills/kluris-b1").exists());
    assert!(!h.home.join(".claude/skills/kluris-b1").exists());
    assert!(h.home.join(".agents/skills/kluris-notes/SKILL.md").exists());
    assert!(h.home.join("brains/b1/brain.md").exists());
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
    let removed: Vec<String> = report
        .skills
        .iter()
        .flat_map(|d| d.removed.clone())
        .collect();
    assert_eq!(
        removed,
        vec!["kluris-b1".to_string(), "kluris-b1".to_string()]
    );
}

#[test]
fn purge_refuses_while_a_brain_lives_inside_dot_kluris() {
    let h = Home::installed();
    let inside = h.home.join(".kluris/brains/b2");
    fs::create_dir_all(&inside).unwrap();
    let receipt = h.receipt();
    let brains = vec![("b2".to_string(), inside.clone())];
    let mut req = request(&h, &receipt, true);
    req.brains = &brains;
    match uninstall(&req, None, &mut remove_file) {
        Err(UninstallError::BrainsInsideKluris(names)) => assert_eq!(names, vec!["b2".to_string()]),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // Nothing was touched.
    assert!(h.prefix.join(exe_name()).exists());
    assert!(h.receipt.exists());
    assert!(inside.is_dir());
}

#[test]
fn the_binary_goes_through_the_callers_remover() {
    let h = Home::installed();
    let receipt = h.receipt();
    let mut asked = Vec::new();
    let mut remover = |p: &Path| {
        asked.push(p.to_path_buf());
        fs::remove_file(p)
    };
    uninstall(&request(&h, &receipt, false), None, &mut remover).unwrap();
    assert_eq!(asked, vec![h.prefix.join(exe_name())]);
}

#[test]
fn a_failure_is_reported_and_the_rest_goes_on() {
    let h = Home::installed();
    let receipt = h.receipt();
    let mut remover = |_: &Path| Err(io::Error::other("busy"));
    let report = uninstall(&request(&h, &receipt, false), None, &mut remover).unwrap();
    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(report.failed[0].contains("busy"));
    assert!(h.prefix.join(exe_name()).exists());
    h.assert_rc_files_restored();
    assert!(!h.receipt.exists());
}

// The command.

#[test]
fn cli_uninstall_json() {
    let h = Home::installed();
    let out = h.kluris(&["self-uninstall", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["purged"], false);
    assert_eq!(v["failed"], serde_json::json!([]));
    let removed: Vec<&str> = v["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(
        removed.contains(&slashed(&h.prefix.join(exe_name())).as_str()),
        "{removed:?}"
    );
    assert!(
        removed.contains(&slashed(&h.receipt).as_str()),
        "{removed:?}"
    );
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [
            "ok",
            "removed",
            "edited",
            "path_entries",
            "skills",
            "purged",
            "failed"
        ]
    );
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
    h.assert_user_things_kept();
}

#[test]
fn cli_uninstall_human() {
    let h = Home::installed();
    let out = h.kluris(&["self-uninstall"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("config.yml"), "{text}");
    assert!(text.contains("uninstalled"), "{text}");
    h.assert_installer_files_gone();
}

#[test]
fn cli_the_running_installed_copy_removes_itself() {
    let h = Home::installed();
    let exe = h.prefix.join(exe_name());
    fs::copy(assert_cmd::cargo::cargo_bin!("kluris"), &exe).unwrap();
    let out = h
        .cmd(&exe)
        .args(["self-uninstall", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // Windows deletes the relocated copy once the process has exited.
    for _ in 0..50 {
        if !exe.exists() && !h.prefix.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(!exe.exists());
    assert!(!h.prefix.exists());
    h.assert_user_things_kept();
}

#[test]
fn cli_without_a_receipt_refuses() {
    let h = Home::installed();
    fs::remove_file(&h.receipt).unwrap();
    let out = h.kluris(&["self-uninstall"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("receipt") && err.contains(&h.receipt.display().to_string()),
        "{err}"
    );
    assert!(h.prefix.join(exe_name()).exists());
}

#[test]
fn cli_purge_without_yes_is_an_error_when_not_interactive() {
    let h = Home::installed();
    for args in [
        vec!["self-uninstall", "--purge"],
        vec!["self-uninstall", "--purge", "--json"],
    ] {
        let out = h.kluris(&args);
        assert_eq!(out.status.code(), Some(1));
        let msg = if args.contains(&"--json") {
            let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
            v["error"].as_str().unwrap().to_string()
        } else {
            stderr(&out)
        };
        assert!(msg.contains("--yes"), "{msg}");
    }
    // Nothing removed.
    assert!(h.prefix.join(exe_name()).exists());
    assert!(h.receipt.exists());
    h.assert_user_things_kept();
}

#[test]
fn cli_purge_with_yes() {
    let h = Home::installed();
    let out = h.kluris(&["self-uninstall", "--purge", "--yes", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["purged"], true);
    assert!(!h.home.join(".kluris").exists());
    assert!(!h.home.join(".agents/skills/kluris-b1").exists());
    assert!(h.home.join(".agents/skills/kluris-notes/SKILL.md").exists());
    assert!(h.home.join("brains/b1/brain.md").exists());
    h.assert_installer_files_gone();
}

#[test]
fn cli_purge_refuses_a_brain_inside_dot_kluris() {
    let h = Home::installed();
    let inside = h.home.join(".kluris/brains/b2");
    fs::create_dir_all(&inside).unwrap();
    fs::write(
        h.home.join(".kluris/config.yml"),
        format!(
            "brains:\n  b2:\n    path: '{}'\n    description: ''\n",
            inside.display().to_string().replace('\'', "''")
        ),
    )
    .unwrap();
    let out = h.kluris(&["self-uninstall", "--purge", "--yes"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("b2"), "{}", stderr(&out));
    assert!(inside.is_dir());
    assert!(h.prefix.join(exe_name()).exists());
}

#[test]
fn self_commands_are_in_help() {
    let h = Home::installed();
    let out = h.kluris(&["self-uninstall", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    for flag in ["--purge", "--yes", "--json"] {
        assert!(stdout(&out).contains(flag), "{}", stdout(&out));
    }
    let root = h.kluris(&["--help"]);
    for name in ["self-update", "self-uninstall"] {
        assert!(stdout(&root).contains(name), "{}", stdout(&root));
    }
    let table = h.kluris(&["help", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&table.stdout).unwrap();
    let names: Vec<&str> = v["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(&names[names.len() - 2..], ["self-update", "self-uninstall"]);
}

/// The real HKCU `Path`, read and written here with the Win32 API directly
/// (not through Kluris's code), so the check does not trust what it checks.
#[cfg(windows)]
mod real_registry {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, RegCloseKey, RegDeleteValueW,
        RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn open() -> HKEY {
        let mut key: HKEY = std::ptr::null_mut();
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide("Environment").as_ptr(),
                0,
                KEY_READ | KEY_WRITE,
                &mut key,
            )
        };
        assert_eq!(rc, ERROR_SUCCESS, "open HKCU\\Environment");
        key
    }

    /// `(type, bytes)` of `Path`, exactly as stored; `None` when absent.
    pub fn read_raw() -> Option<(u32, Vec<u8>)> {
        let key = open();
        let name = wide("Path");
        let mut kind = 0u32;
        let mut len = 0u32;
        let rc = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut len,
            )
        };
        if rc == ERROR_FILE_NOT_FOUND {
            unsafe { RegCloseKey(key) };
            return None;
        }
        assert_eq!(rc, ERROR_SUCCESS);
        let mut data = vec![0u8; len as usize];
        let rc = unsafe {
            RegQueryValueExW(
                key,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut len,
            )
        };
        assert_eq!(rc, ERROR_SUCCESS);
        data.truncate(len as usize);
        unsafe { RegCloseKey(key) };
        Some((kind, data))
    }

    /// Put `saved` back exactly (or remove `Path` when there was none).
    pub fn restore_raw(saved: &Option<(u32, Vec<u8>)>) {
        let key = open();
        let name = wide("Path");
        let rc = match saved {
            Some((kind, data)) => unsafe {
                RegSetValueExW(
                    key,
                    name.as_ptr(),
                    0,
                    *kind,
                    data.as_ptr(),
                    data.len() as u32,
                )
            },
            None => unsafe { RegDeleteValueW(key, name.as_ptr()) },
        };
        unsafe { RegCloseKey(key) };
        assert_eq!(rc, ERROR_SUCCESS, "restore the user PATH");
    }

    /// The value as text (no trailing NULs).
    pub fn text(raw: &Option<(u32, Vec<u8>)>) -> Option<String> {
        raw.as_ref().map(|(_, data)| {
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
                .trim_end_matches('\0')
                .to_string()
        })
    }

    pub fn set_expand(value: &str) {
        let data: Vec<u8> = wide(value).iter().flat_map(|u| u.to_le_bytes()).collect();
        restore_raw(&Some((REG_EXPAND_SZ, data)));
    }

    pub const EXPAND: u32 = REG_EXPAND_SZ;
}

/// The real HKCU PATH through the shipped binary: save it exactly, add the
/// install folder in front (as the PowerShell installer does), run
/// `kluris self-uninstall`, check that only that entry left and the value is
/// `REG_EXPAND_SZ`, and always put the saved value back, compared byte for
/// byte.
#[cfg(windows)]
#[test]
#[ignore = "changes the real user PATH (restored); run with --ignored on a test machine"]
fn real_windows_user_path_round_trip() {
    let saved = real_registry::read_raw();
    struct Restore(Option<(u32, Vec<u8>)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            real_registry::restore_raw(&self.0);
            assert!(
                real_registry::read_raw() == self.0,
                "the saved PATH is back byte for byte"
            );
        }
    }
    let guard = Restore(saved.clone());

    let h = Home::installed();
    // The installer writes `C:\Users\<u>\.kluris/bin` into the receipt and
    // the folder it made into PATH.
    let entry = h.prefix.display().to_string();
    let rest = real_registry::text(&saved).unwrap_or_default();
    let with_ours = if rest.is_empty() {
        entry.clone()
    } else {
        format!("{entry};{rest}")
    };
    real_registry::set_expand(&with_ours);

    let out = h.kluris(&["self-uninstall", "--json"]);
    let after = real_registry::read_raw();
    drop(guard);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["path_entries"], serde_json::json!([entry]));
    assert_eq!(real_registry::text(&after).unwrap_or_default(), rest);
    assert_eq!(after.map(|(kind, _)| kind), Some(real_registry::EXPAND));
}
