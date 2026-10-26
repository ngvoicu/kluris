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

/// An in-memory HKCU `Path`; `fail_writes` makes every write fail.
#[derive(Default)]
struct FakeUserPath {
    value: Option<String>,
    writes: Vec<String>,
    fail_writes: bool,
}

impl UserPath for FakeUserPath {
    fn read(&self) -> io::Result<Option<String>> {
        Ok(self.value.clone())
    }

    fn write(&mut self, value: &str) -> io::Result<()> {
        if self.fail_writes {
            return Err(io::Error::other("registry locked"));
        }
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
        fail_writes: false,
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
        fail_writes: false,
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
    // dist 0.33.1 writes the entry as the receipt's prefix,
    // `C:\Users\u\.kluris\bin` (checked on zeewin2, 2026-10-26); an entry a
    // user retyped may differ in case, separators or a trailing `\`, and
    // PowerShell's `-in` compares without case.
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

// A failed step keeps what a retry needs (T-111 finding 5): the rc files
// go first and a failure there stops everything; then the env scripts, the
// fish file and the PATH entry, a failure stopping before the binary; the
// receipt goes last, once the binary is gone. A rerun finishes the job.
// (This replaces `a_failure_is_reported_and_the_rest_goes_on`, which required
// the receipt to be removed after a failed binary removal: that was the bug.)

#[test]
fn a_failed_binary_removal_keeps_the_receipt_and_a_retry_finishes() {
    let h = Home::installed();
    let receipt = h.receipt();
    let mut remover = |_: &Path| Err(io::Error::other("busy"));
    let report = uninstall(&request(&h, &receipt, false), None, &mut remover).unwrap();
    assert_eq!(report.failed.len(), 1, "{:?}", report.failed);
    assert!(report.failed[0].contains("busy"));
    assert!(!report.ok());
    assert!(h.prefix.join(exe_name()).exists());
    assert!(h.receipt.exists(), "the receipt a retry needs is kept");
    h.assert_rc_files_restored();

    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
    h.assert_user_things_kept();
}

#[cfg(unix)]
fn read_only(dir: &Path, yes: bool) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        dir,
        fs::Permissions::from_mode(if yes { 0o500 } else { 0o700 }),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn an_rc_file_that_cannot_be_edited_stops_before_anything_is_removed() {
    // The reviewer's case: ~/.profile is a link into a dotfiles folder the
    // atomic rewrite cannot write in.
    let h = Home::installed();
    let dotfiles = h.root.join("dotfiles");
    fs::create_dir_all(&dotfiles).unwrap();
    let profile = h.home.join(".profile");
    fs::rename(&profile, dotfiles.join("profile")).unwrap();
    std::os::unix::fs::symlink(dotfiles.join("profile"), &profile).unwrap();
    read_only(&dotfiles, true);
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    read_only(&dotfiles, false);
    assert!(!report.ok());
    // The pre-flight (T-119) names the folder the rewrite needs, for
    // ~/.profile; before it, the failed edit itself did.
    assert!(
        report
            .blocked
            .iter()
            .any(|b| b.path == dotfiles && b.problem.contains(".profile")),
        "{:?}",
        report.blocked
    );
    for kept in [
        h.prefix.join(exe_name()),
        h.receipt.clone(),
        h.config_home.join("kluris/env.sh"),
        h.config_home.join("kluris/env.fish"),
        h.home.join(".config/fish/conf.d/kluris.env.fish"),
    ] {
        assert!(
            kept.exists(),
            "{} went, though an rc edit failed",
            kept.display()
        );
    }
    assert!(report.removed.is_empty(), "{:?}", report.removed);

    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
    assert!(
        fs::symlink_metadata(&profile)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[cfg(unix)]
#[test]
fn an_env_script_that_cannot_be_removed_stops_before_the_binary() {
    let h = Home::installed();
    let folder = h.config_home.join("kluris");
    read_only(&folder, true);
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    read_only(&folder, false);
    assert!(!report.ok());
    assert!(
        h.prefix.join(exe_name()).exists(),
        "the binary a retry runs is kept"
    );
    assert!(h.receipt.exists());
    // The pre-flight (T-119) stops before the rc files are edited; before
    // it, they were.
    assert!(
        report.blocked.iter().any(|b| b.path == folder),
        "{:?}",
        report.blocked
    );
    assert!(report.edited.is_empty(), "{:?}", report.edited);

    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    h.assert_installer_files_gone();
}

#[test]
fn a_failed_path_update_keeps_the_binary_and_receipt_and_a_retry_finishes() {
    let h = Home::installed();
    let entry = h.prefix.display().to_string();
    let mut fake = FakeUserPath {
        value: Some(format!("{entry};C:\\Other")),
        fail_writes: true,
        ..FakeUserPath::default()
    };
    let receipt = h.receipt();
    let report = uninstall(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
    )
    .unwrap();
    assert!(!report.ok());
    assert!(report.failed.iter().any(|f| f.contains("registry locked")));
    assert!(h.prefix.join(exe_name()).exists());
    assert!(h.receipt.exists());

    fake.fail_writes = false;
    let receipt = h.receipt();
    let report = uninstall(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
    )
    .unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    assert_eq!(fake.value.as_deref(), Some("C:\\Other"));
    h.assert_installer_files_gone();
}

// `modify_path` and the env scripts (T-111 finding 4).

/// The receipt with `modify_path` false, as the installer writes it under
/// KLURIS_NO_MODIFY_PATH=1.
fn without_modify_path(h: &Home) {
    let text = fs::read_to_string(&h.receipt).unwrap();
    assert!(text.contains("\"modify_path\":true"));
    fs::write(
        &h.receipt,
        text.replace("\"modify_path\":true", "\"modify_path\":false"),
    )
    .unwrap();
}

/// Every file under HOME with its bytes.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                stack.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap_or_default()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn with_modify_path_false_no_path_setup_is_touched() {
    // The user manages PATH: their own env.sh (not dist's) and the profile
    // line sourcing it; the installer, run with KLURIS_NO_MODIFY_PATH=1, wrote
    // neither.
    let h = Home::installed();
    without_modify_path(&h);
    fs::write(
        h.config_home.join("kluris/env.sh"),
        "export USER_SENTINEL=1\n",
    )
    .unwrap();
    let entry = h.prefix.display().to_string();
    let mut fake = FakeUserPath {
        value: Some(format!("{entry};C:\\Other")),
        ..FakeUserPath::default()
    };
    let before = snapshot(&h.home);
    let receipt = h.receipt();
    assert!(!receipt.modify_path);
    let report = uninstall(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
    )
    .unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    assert!(report.edited.is_empty(), "{:?}", report.edited);
    assert!(report.path_entries.is_empty());
    assert!(fake.writes.is_empty(), "the user PATH was written");
    let gone = [h.prefix.join(exe_name()), h.receipt.clone()];
    let after = snapshot(&h.home);
    let expected: Vec<_> = before
        .into_iter()
        .filter(|(p, _)| !gone.contains(p))
        .collect();
    assert_eq!(after, expected, "only the binary and the receipt went");
}

#[test]
fn env_scripts_that_are_not_dists_are_kept_with_the_lines_that_source_them() {
    let h = Home::installed();
    let env_sh = h.config_home.join("kluris/env.sh");
    let mine = format!("{ENV_SH}export EDITOR=vi\n");
    fs::write(&env_sh, &mine).unwrap();
    let rc_before: Vec<(PathBuf, String)> = h
        .originals
        .iter()
        .map(|(p, _)| (p.clone(), fs::read_to_string(p).unwrap()))
        .collect();
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    assert_eq!(
        fs::read_to_string(&env_sh).unwrap(),
        mine,
        "the user's env.sh is kept"
    );
    assert!(report.kept.contains(&env_sh), "{:?}", report.kept);
    for (path, text) in rc_before {
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "{}: the line sourcing the kept env.sh stays",
            path.display()
        );
    }
    // env.fish is dist's: it and its conf.d file go.
    assert!(!h.config_home.join("kluris/env.fish").exists());
    assert!(!h.home.join(".config/fish/conf.d/kluris.env.fish").exists());
    assert!(!h.prefix.join(exe_name()).exists());
    assert!(!h.receipt.exists());
}

#[test]
fn a_fish_env_script_that_is_not_dists_is_kept_with_its_conf_d_line() {
    let h = Home::installed();
    let env_fish = h.config_home.join("kluris/env.fish");
    fs::write(&env_fish, "set -x EDITOR vi\n").unwrap();
    let conf = h.home.join(".config/fish/conf.d/kluris.env.fish");
    let conf_text = fs::read_to_string(&conf).unwrap();
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    assert_eq!(fs::read_to_string(&env_fish).unwrap(), "set -x EDITOR vi\n");
    assert_eq!(fs::read_to_string(&conf).unwrap(), conf_text);
    assert!(report.kept.contains(&env_fish), "{:?}", report.kept);
    assert!(!h.config_home.join("kluris/env.sh").exists());
    h.assert_rc_files_restored();
}

#[test]
fn uninstall_refuses_a_receipt_that_is_not_klurises() {
    let h = Home::installed();
    let edited = fs::read_to_string(&h.receipt)
        .unwrap()
        .replace(&format!("[\"{}\"]", exe_name()), "[\"notes.txt\"]");
    let receipt = Receipt::parse(&edited).unwrap();
    let before = snapshot(&h.home);
    let err = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap_err();
    assert!(
        err.to_string().contains("not a Kluris install receipt"),
        "{err}"
    );
    assert_eq!(snapshot(&h.home), before);
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
            "kept",
            "path_entries",
            "skills",
            "purged",
            "failed",
            "blocked",
            "left"
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
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
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
    // The installer writes `C:\Users\<u>\.kluris\bin` (backslashes) into
    // the receipt and the same text into PATH.
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

// Late failures (T-114): the binary and the receipt are the last two things
// removed, after every other step has succeeded, a purge included. Any
// earlier failure stops with nothing a retry needs gone, and the rerun,
// once the obstacle is cleared, finishes.

/// What a failed run must have kept for a retry, and nothing it still
/// references gone.
#[cfg(unix)]
fn assert_retry_possible(h: &Home, report: &kluris_core::dist::UninstallReport) {
    assert!(!report.ok(), "the run should have failed");
    assert!(
        h.prefix.join(exe_name()).exists(),
        "the binary a retry runs went"
    );
    assert!(h.receipt.exists(), "the receipt a retry reads went");
}

#[cfg(unix)]
#[test]
fn a_purge_that_cannot_clear_dot_kluris_keeps_the_binary_and_receipt() {
    // A read-only folder in ~/.kluris/companions: ~/.kluris cannot be removed.
    let h = Home::installed();
    let protected = h.home.join(".kluris/companions/protected");
    fs::create_dir_all(&protected).unwrap();
    fs::write(protected.join("notes.txt"), "remaining data\n").unwrap();
    read_only(&protected, true);
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, true), None, &mut remove_file).unwrap();
    read_only(&protected, false);
    assert_retry_possible(&h, &report);
    assert!(!report.purged);

    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, true), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    assert!(report.purged);
    assert!(!h.home.join(".kluris").exists());
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
}

#[cfg(unix)]
#[test]
fn a_receipt_folder_that_cannot_lose_the_receipt_keeps_the_binary() {
    // The KLURIS_NO_MODIFY_PATH=1 install of the probe: no env scripts to
    // go, and the receipt's folder read-only.
    let h = Home::installed();
    without_modify_path(&h);
    let folder = h.config_home.join("kluris");
    read_only(&folder, true);
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    read_only(&folder, false);
    assert_retry_possible(&h, &report);
    assert!(report.removed.is_empty(), "{:?}", report.removed);

    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    assert!(!h.prefix.join(exe_name()).exists());
    assert!(!h.receipt.exists());
}

#[cfg(unix)]
#[test]
fn a_fish_conf_d_that_cannot_be_cleaned_keeps_the_env_scripts() {
    // conf.d read-only: its kluris.env.fish, which sources env.fish, stays,
    // so env.fish (and everything after it) must stay too.
    let h = Home::installed();
    let conf_d = h.home.join(".config/fish/conf.d");
    read_only(&conf_d, true);
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    read_only(&conf_d, false);
    assert_retry_possible(&h, &report);
    assert!(conf_d.join("kluris.env.fish").exists());
    assert!(
        h.config_home.join("kluris/env.fish").exists(),
        "env.fish went while conf.d/kluris.env.fish still sources it"
    );
    assert!(h.config_home.join("kluris/env.sh").exists());

    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, false), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?}", report.failed);
    h.assert_installer_files_gone();
    h.assert_rc_files_restored();
}

// The pre-flight (T-118; SPEC decision of 2026-10-26): the whole removal
// plan is checked before the first change. A file that cannot be read or
// checked (anything but NotFound), a folder an entry must leave that is not
// writable and searchable, an immutable flag (macOS) or a read-only
// attribute (Windows) stops the uninstall with nothing changed and each
// blocker named with its fix; the rerun, once it is cleared, finishes.

/// Every file under `root`, with its bytes or link target.
fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.file_type().is_symlink() {
                files.push((
                    path.clone(),
                    fs::read_link(&path)
                        .unwrap()
                        .into_os_string()
                        .into_encoded_bytes(),
                ));
            } else if meta.is_dir() {
                files.push((path.clone(), b"<dir>".to_vec()));
                stack.push(path);
            } else {
                files.push((path.clone(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[cfg(unix)]
fn chmod(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

/// Run the uninstall with `obstacle` in place (`undo` clears it), assert it
/// was blocked before any change, then rerun.
fn assert_blocked_then_finishes(
    h: &Home,
    purge: bool,
    obstacle: impl FnOnce(),
    undo: impl FnOnce(),
    blocked_path: &Path,
) {
    assert_blocked_then_finishes_with(h, purge, &[], obstacle, undo, blocked_path);
}

/// [`assert_blocked_then_finishes`] with these registered brains.
fn assert_blocked_then_finishes_with(
    h: &Home,
    purge: bool,
    brains: &[(String, PathBuf)],
    obstacle: impl FnOnce(),
    undo: impl FnOnce(),
    blocked_path: &Path,
) {
    let before = tree(&h.root);
    obstacle();
    let receipt = h.receipt();
    let report = uninstall(
        &UninstallRequest {
            brains,
            ..request(h, &receipt, purge)
        },
        None,
        &mut remove_file,
    )
    .unwrap();
    undo();
    assert!(!report.ok(), "the uninstall should have been blocked");
    assert!(
        report
            .blocked
            .iter()
            .any(|b| b.path == blocked_path || b.path.starts_with(blocked_path)),
        "no blocker names {}: {:?}",
        blocked_path.display(),
        report.blocked
    );
    assert!(
        report.blocked.iter().all(|b| !b.fix.is_empty()),
        "{:?}",
        report.blocked
    );
    assert!(
        report.removed.is_empty() && report.edited.is_empty(),
        "{report:?}"
    );
    assert_eq!(
        tree(&h.root),
        before,
        "the blocked uninstall changed something"
    );

    let receipt = h.receipt();
    let report = uninstall(
        &UninstallRequest {
            brains,
            ..request(h, &receipt, purge)
        },
        None,
        &mut remove_file,
    )
    .unwrap();
    assert!(
        report.ok(),
        "the rerun: {:?} {:?}",
        report.failed,
        report.blocked
    );
    assert!(!h.prefix.join(exe_name()).exists());
    assert!(!h.receipt.exists());
}

#[cfg(unix)]
#[test]
fn an_unsearchable_fish_conf_d_blocks_before_any_change() {
    let h = Home::installed();
    let conf_d = h.home.join(".config/fish/conf.d");
    assert_blocked_then_finishes(
        &h,
        false,
        || chmod(&conf_d, 0o400),
        || chmod(&conf_d, 0o700),
        &conf_d,
    );
    h.assert_installer_files_gone();
}

#[cfg(unix)]
#[test]
fn an_unsearchable_profile_target_blocks_before_any_change() {
    let h = Home::installed();
    let dotfiles = h.root.join("dotfiles");
    fs::create_dir_all(&dotfiles).unwrap();
    let profile = h.home.join(".profile");
    fs::rename(&profile, dotfiles.join("profile")).unwrap();
    std::os::unix::fs::symlink(dotfiles.join("profile"), &profile).unwrap();
    assert_blocked_then_finishes(
        &h,
        false,
        || chmod(&dotfiles, 0o400),
        || chmod(&dotfiles, 0o700),
        &profile,
    );
    h.assert_rc_files_restored();
}

#[cfg(unix)]
#[test]
fn a_read_only_dot_kluris_blocks_a_purge_before_any_change() {
    let h = Home::installed();
    let kluris = h.home.join(".kluris");
    assert_blocked_then_finishes(
        &h,
        true,
        || chmod(&kluris, 0o500),
        || chmod(&kluris, 0o700),
        &kluris,
    );
    assert!(!kluris.exists());
}

#[cfg(unix)]
#[test]
fn a_protected_folder_in_the_bin_folder_blocks_a_purge_before_any_change() {
    let h = Home::installed();
    let protected = h.prefix.join("protected");
    fs::create_dir_all(&protected).unwrap();
    fs::write(protected.join("notes.txt"), "remaining data\n").unwrap();
    assert_blocked_then_finishes(
        &h,
        true,
        || chmod(&protected, 0o500),
        || chmod(&protected, 0o700),
        &protected,
    );
    assert!(!h.home.join(".kluris").exists());
}

#[cfg(unix)]
#[test]
fn a_read_only_home_blocks_a_purge_before_any_change() {
    let h = Home::installed();
    without_modify_path(&h);
    let home = h.home.clone();
    assert_blocked_then_finishes(
        &h,
        true,
        || chmod(&home, 0o500),
        || chmod(&home, 0o700),
        &home,
    );
    assert!(!h.home.join(".kluris").exists());
}

#[cfg(target_os = "macos")]
fn uchg(path: &Path, on: bool) {
    let status = std::process::Command::new("chflags")
        .arg(if on { "uchg" } else { "nouchg" })
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

#[cfg(target_os = "macos")]
#[test]
fn an_immutable_receipt_blocks_before_any_change() {
    let h = Home::installed();
    let receipt = h.receipt.clone();
    assert_blocked_then_finishes(
        &h,
        false,
        || uchg(&receipt, true),
        || uchg(&receipt, false),
        &receipt,
    );
    h.assert_installer_files_gone();
}

/// A failure after the binary is gone (a race the pre-flight could not
/// see), injected through FileOps: exit 1, and the report says exactly what
/// is left to remove by hand; no shell file still sources a removed script
/// and the PATH entry is gone.
#[test]
fn a_failure_after_the_binary_is_gone_says_what_is_left() {
    use kluris_core::dist::uninstall_with;
    use kluris_core::fileops::FileOps;
    struct ReceiptStuck<'a>(&'a Path);
    impl FileOps for ReceiptStuck<'_> {
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            if path == self.0 {
                return Err(io::Error::other("injected: the receipt is held"));
            }
            fs::remove_file(path)
        }
    }
    let h = Home::installed();
    let entry = h.prefix.display().to_string();
    let mut fake = FakeUserPath {
        value: Some(format!("{entry};C:\\Other")),
        ..FakeUserPath::default()
    };
    let receipt = h.receipt();
    let ops = ReceiptStuck(&h.receipt);
    let report = uninstall_with(
        &request(&h, &receipt, false),
        Some(&mut fake),
        &mut remove_file,
        &ops,
    )
    .unwrap();
    assert!(!report.ok());
    assert!(!h.prefix.join(exe_name()).exists(), "the binary went first");
    assert!(h.receipt.exists());
    assert_eq!(
        fake.value.as_deref(),
        Some("C:\\Other"),
        "the PATH entry is gone"
    );
    h.assert_rc_files_restored();
    assert!(!h.home.join(".config/fish/conf.d/kluris.env.fish").exists());
    assert_eq!(
        report.left,
        vec![format!("remove the file {}", h.receipt.display())],
        "{:?}",
        report.left
    );
}

#[cfg(windows)]
#[test]
fn a_read_only_receipt_blocks_before_any_change() {
    let h = Home::installed();
    let receipt = h.receipt.clone();
    let set = |on: bool| {
        let mut perms = fs::metadata(&receipt).unwrap().permissions();
        perms.set_readonly(on);
        fs::set_permissions(&receipt, perms).unwrap();
    };
    assert_blocked_then_finishes(&h, false, || set(true), || set(false), &h.receipt);
    h.assert_installer_files_gone();
}

/// The command prints the blockers, changes nothing and exits 1; the JSON
/// lists them as `blocked: [{path, problem, fix}]`.
#[cfg(unix)]
#[test]
fn cli_a_blocked_uninstall_lists_the_blockers_and_changes_nothing() {
    let h = Home::installed();
    let conf_d = h.home.join(".config/fish/conf.d");
    let before = tree(&h.root);
    chmod(&conf_d, 0o400);
    let out = h.kluris(&["self-uninstall", "--json"]);
    let text = h.kluris(&["self-uninstall"]);
    chmod(&conf_d, 0o700);
    assert_eq!(tree(&h.root), before);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["removed"], serde_json::json!([]));
    let blocked = v["blocked"].as_array().unwrap();
    assert!(!blocked.is_empty(), "{v}");
    for b in blocked {
        let keys: Vec<&String> = b.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["path", "problem", "fix"], "{b}");
    }
    assert!(
        blocked
            .iter()
            .any(|b| b["path"].as_str().unwrap().starts_with(&slashed(&conf_d))),
        "{v}"
    );
    assert_eq!(text.status.code(), Some(1));
    let text = stdout(&text);
    assert!(text.contains("Nothing was changed"), "{text}");
    assert!(text.contains("conf.d"), "{text}");
    assert!(!text.contains("uninstalled"), "{text}");
}

// Folders the user made links (T-129, SPEC decision 2026-10-26 on the
// pre-flight): a purge checks the very tree it removes, through a linked
// ~/.kluris as the removal goes through it; a folder the uninstall would
// remove once empty is kept when it is a link, the link and its target
// both, and listed in `kept`; the files Kluris installed in it still go.

/// Move `home/<rel>` to `root/dotfiles/<rel's name>` and link it back;
/// returns (link, target).
#[cfg(unix)]
fn relink(h: &Home, rel: &str) -> (PathBuf, PathBuf) {
    let link = h.home.join(rel);
    let target = h.root.join("dotfiles").join(link.file_name().unwrap());
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::rename(&link, &target).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    (link, target)
}

#[cfg(unix)]
fn assert_link_kept(link: &Path, target: &Path) {
    assert!(
        fs::symlink_metadata(link).unwrap().file_type().is_symlink(),
        "{} is no longer the user's link",
        link.display()
    );
    assert_eq!(fs::read_link(link).unwrap(), target);
    assert!(target.is_dir(), "{} went", target.display());
}

/// A linked ~/.kluris with a protected folder in its companions: blocked
/// with nothing changed, then a purge that keeps the link and its folder.
#[cfg(unix)]
fn assert_linked_dot_kluris_purge(
    brains: impl Fn(&Home) -> Vec<(String, PathBuf)>,
    protect: impl Fn(&Path, bool),
) {
    let h = Home::installed();
    let (link, target) = relink(&h, ".kluris");
    let protected = link.join("companions/protected");
    fs::create_dir_all(&protected).unwrap();
    fs::write(protected.join("notes.txt"), "remaining data\n").unwrap();
    let brains = brains(&h);
    assert_blocked_then_finishes_with(
        &h,
        true,
        &brains,
        || protect(&protected, true),
        || protect(&protected, false),
        &protected,
    );
    assert_link_kept(&link, &target);
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        0,
        "the purge left files in ~/.kluris"
    );
    h.assert_installer_files_gone();
    assert!(h.home.join("brains/b1/brain.md").exists());
}

#[cfg(unix)]
#[test]
fn a_linked_dot_kluris_with_a_protected_folder_blocks_a_purge_before_any_change() {
    assert_linked_dot_kluris_purge(
        |_| Vec::new(),
        |dir, on| chmod(dir, if on { 0o555 } else { 0o755 }),
    );
}

#[cfg(unix)]
#[test]
fn a_linked_dot_kluris_blocks_a_purge_with_a_registered_brain_before_any_change() {
    assert_linked_dot_kluris_purge(
        |h| vec![("b1".to_string(), h.home.join("brains/b1"))],
        |dir, on| chmod(dir, if on { 0o555 } else { 0o755 }),
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_linked_dot_kluris_with_an_immutable_folder_blocks_a_purge_before_any_change() {
    assert_linked_dot_kluris_purge(|_| Vec::new(), uchg);
}

#[cfg(unix)]
#[test]
fn a_linked_dot_kluris_is_kept_by_a_purge() {
    let h = Home::installed();
    let (link, target) = relink(&h, ".kluris");
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, true), None, &mut remove_file).unwrap();
    assert!(report.ok(), "{:?} {:?}", report.failed, report.blocked);
    assert!(report.purged);
    assert!(report.kept.contains(&link), "{:?}", report.kept);
    assert_link_kept(&link, &target);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    h.assert_installer_files_gone();
}

/// An install into linked folders: the uninstall finishes, removes the
/// files it installed there and keeps the link and its target.
#[cfg(unix)]
fn assert_linked_folder_kept(rel: &str, purge: bool) {
    let h = Home::installed();
    if purge {
        // Nothing of the user's in ~/.kluris but the bin folder.
        fs::remove_dir_all(h.home.join(".kluris/companions")).unwrap();
        fs::remove_file(h.home.join(".kluris/config.yml")).unwrap();
    }
    let (link, target) = relink(&h, rel);
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, purge), None, &mut remove_file).unwrap();
    assert!(
        report.ok(),
        "{:?} {:?} {:?}",
        report.failed,
        report.left,
        report.blocked
    );
    assert!(report.left.is_empty(), "{:?}", report.left);
    assert!(report.kept.contains(&link), "{:?}", report.kept);
    assert_link_kept(&link, &target);
    assert_eq!(
        fs::read_dir(&target).unwrap().count(),
        0,
        "the installer's files in {} were not removed",
        target.display()
    );
    assert!(!h.receipt.exists() && !h.prefix.join(exe_name()).exists());
    h.assert_rc_files_restored();
}

#[cfg(unix)]
#[test]
fn a_linked_receipt_folder_is_kept_with_its_target() {
    assert_linked_folder_kept(".config/kluris", false);
}

#[cfg(unix)]
#[test]
fn a_linked_bin_folder_is_kept_with_its_target() {
    assert_linked_folder_kept(".kluris/bin", false);
}

#[cfg(unix)]
#[test]
fn a_linked_receipt_folder_is_kept_by_a_purge() {
    assert_linked_folder_kept(".config/kluris", true);
}

/// A linked ~/.kluris/bin is in ~/.kluris, which a purge removes: the link
/// goes with ~/.kluris (never followed), its target and everything the
/// installer did not put there stay.
#[cfg(unix)]
#[test]
fn a_purge_removes_dot_kluris_with_a_linked_bin_folder_in_it_but_not_the_target() {
    let h = Home::installed();
    let (_link, target) = relink(&h, ".kluris/bin");
    fs::write(target.join("mine.txt"), "the user's\n").unwrap();
    let receipt = h.receipt();
    let report = uninstall(&request(&h, &receipt, true), None, &mut remove_file).unwrap();
    assert!(
        report.ok(),
        "{:?} {:?} {:?}",
        report.failed,
        report.left,
        report.blocked
    );
    assert!(!h.home.join(".kluris").exists());
    assert!(!target.join(exe_name()).exists());
    assert_eq!(
        fs::read_to_string(target.join("mine.txt")).unwrap(),
        "the user's\n"
    );
}
