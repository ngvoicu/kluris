//! `kluris self-uninstall`: remove what the installers wrote ([`super`]),
//! read from the receipt, and keep everything else: `~/.kluris/config.yml`,
//! the companions, the brains, the skills. `purge` also removes `~/.kluris`
//! and every Kluris-written skill.
//!
//! An rc file loses each line that is exactly one the installer appends
//! (`. "<env.sh>"`, or `source "<env.fish>"` in the fish conf.d file, the
//! env script named as `$HOME/...` or in full), with the empty line the
//! installer wrote before it; every other byte stays.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub use super::preflight::{Blocker, Planned, preflight};
use super::{DistEnv, Receipt, ReceiptError};
use crate::agents::install::{DestinationReport, remove_generated_skills};
use crate::fileops::{FileOps, RealFs};

/// The per-user PATH on Windows (`HKCU\Environment\Path`).
pub trait UserPath {
    /// The value as stored, environment references unexpanded; `None` when
    /// there is none.
    fn read(&self) -> io::Result<Option<String>>;
    /// Store `value` (as `REG_EXPAND_SZ`) and tell running programs.
    fn write(&mut self, value: &str) -> io::Result<()>;
}

/// What [`uninstall`] works from.
#[derive(Debug, Clone, Copy)]
pub struct UninstallRequest<'a> {
    pub env: DistEnv<'a>,
    pub receipt: &'a Receipt,
    /// Where the receipt is; its folder holds the env scripts.
    pub receipt_path: &'a Path,
    /// Also remove `~/.kluris` and every Kluris-written skill.
    pub purge: bool,
    /// `COPILOT_HOME` when set and non-empty (for the skill folders).
    pub copilot_home: Option<&'a Path>,
    /// The registered brains, `(name, path)`: a purge never removes one.
    pub brains: &'a [(String, PathBuf)],
}

/// What [`uninstall`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UninstallReport {
    /// Files and folders removed, in the order they went.
    pub removed: Vec<PathBuf>,
    /// rc files a line was taken out of.
    pub edited: Vec<PathBuf>,
    /// Files the installer's names point at that are not what it wrote, kept.
    pub kept: Vec<PathBuf>,
    /// What the pre-flight found in the way: nothing was changed.
    pub blocked: Vec<Blocker>,
    /// What is left to remove by hand after a failure once the binary was
    /// gone, one instruction each.
    pub left: Vec<String>,
    /// User PATH entries removed (Windows), as they were written.
    pub path_entries: Vec<String>,
    /// The skill folders a purge cleared something from (or failed in).
    pub skills: Vec<DestinationReport>,
    /// `~/.kluris` was removed.
    pub purged: bool,
    /// One message per thing that could not be done; the rest went on.
    pub failed: Vec<String>,
}

impl UninstallReport {
    /// Nothing failed.
    pub fn ok(&self) -> bool {
        self.failed.is_empty() && self.skills.iter().all(|d| d.failed.is_empty())
    }
}

/// Why nothing was done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UninstallError {
    /// A purge would remove these registered brains (they are in `~/.kluris`).
    BrainsInsideKluris(Vec<String>),
    /// The receipt is not the one dist writes for Kluris
    /// ([`ReceiptError::NotKluris`]): it authorises nothing.
    NotKluris(ReceiptError),
}

impl std::fmt::Display for UninstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UninstallError::BrainsInsideKluris(names) => write!(
                f,
                "--purge would remove ~/.kluris, which holds the registered brain(s) {}. \
                 Move them out of ~/.kluris (or run without --purge); nothing was removed.",
                names.join(", ")
            ),
            UninstallError::NotKluris(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for UninstallError {}

/// Remove what the installers wrote, in an order that keeps what a retry
/// needs (T-111):
///
/// 0. Nothing is done unless the receipt is Kluris's
///    ([`Receipt::check_kluris`]); a purge that would remove a registered
///    brain is refused too.
/// 1. With `modify_path` (the installer set PATH up), the rc files lose the
///    line sourcing env.sh, unless env.sh is there and is not dist's (then it
///    and its lines are kept and reported). A failure stops here: env
///    scripts, receipt and binary all stay.
/// 2. The fish conf.d file loses its line (the same rule for env.fish); a
///    failure stops, as it still sources env.fish.
/// 3. The env scripts dist wrote, and on Windows the user PATH entry.
/// 4. With a purge: the Kluris-written skills, then everything in
///    `~/.kluris` but the folder the binary is in.
///
/// Then the binary and the receipt, the last two things removed, once
/// every step before has succeeded (a failure anywhere stops and keeps
/// both): the receipt's folder is first checked to take a removal; 5. the
/// binary, through `remove_binary` (the running one may need the
/// platform's self-delete; one that will not go keeps the receipt), its
/// emptied folder and under a purge what is left of `~/.kluris`; 6. the
/// receipt.
///
/// Without `modify_path` (KLURIS_NO_MODIFY_PATH=1) no rc file, env script,
/// fish file or PATH entry is touched: the user manages PATH. A rerun after
/// any failure picks up where this one stopped.
pub fn uninstall(
    req: &UninstallRequest,
    user_path: Option<&mut dyn UserPath>,
    remove_binary: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> Result<UninstallReport, UninstallError> {
    uninstall_with(req, user_path, remove_binary, &RealFs)
}

/// [`uninstall`], removing and rewriting through `ops`.
pub fn uninstall_with(
    req: &UninstallRequest,
    user_path: Option<&mut dyn UserPath>,
    remove_binary: &mut dyn FnMut(&Path) -> io::Result<()>,
    _ops: &dyn FileOps,
) -> Result<UninstallReport, UninstallError> {
    req.receipt.check_kluris(req.env.windows).map_err(|why| {
        UninstallError::NotKluris(ReceiptError::NotKluris(req.receipt_path.to_path_buf(), why))
    })?;
    let home = req.env.home;
    let kluris_dir = home.join(".kluris");
    if req.purge {
        let inside: Vec<String> = req
            .brains
            .iter()
            .filter(|(_, path)| is_within(path, &kluris_dir))
            .map(|(name, _)| name.clone())
            .collect();
        if !inside.is_empty() {
            return Err(UninstallError::BrainsInsideKluris(inside));
        }
    }

    let mut report = UninstallReport::default();
    let prefix = req.receipt.install_prefix.as_path();
    let receipt_dir = req.receipt_path.parent().unwrap_or(Path::new(""));
    let env_sh = receipt_dir.join("env.sh");
    let env_fish = receipt_dir.join("env.fish");

    if req.receipt.modify_path {
        let exprs = install_dir_exprs(prefix, home);
        let sh_ours = dists_script(&env_sh, &exprs, env_sh_text, &mut report);
        let fish_ours = dists_script(&env_fish, &exprs, env_fish_text, &mut report);
        if !report.failed.is_empty() {
            return Ok(report);
        }

        // 1. The rc files.
        if sh_ours {
            let sh_lines = source_lines(".", &[&env_sh], home);
            let zdir = req.env.zdotdir.unwrap_or(home);
            for rc in [
                home.join(".profile"),
                home.join(".bashrc"),
                home.join(".bash_profile"),
                home.join(".bash_login"),
                zdir.join(".zshrc"),
                zdir.join(".zshenv"),
            ] {
                edit_rc(&rc, &sh_lines, false, &mut report);
            }
            if !report.failed.is_empty() {
                return Ok(report);
            }
        }

        // 2. The fish conf.d file. A failure stops here: it still sources
        // env.fish.
        if fish_ours {
            let fish_lines = source_lines("source", &[&env_fish], home);
            let fish_conf = home
                .join(".config")
                .join("fish")
                .join("conf.d")
                .join(format!("{}.env.fish", super::APP_NAME));
            edit_rc(&fish_conf, &fish_lines, true, &mut report);
            if !report.failed.is_empty() {
                return Ok(report);
            }
        }

        // 3. The env scripts and the Windows user PATH entry.
        if fish_ours {
            remove_file(&env_fish, &mut report);
        }
        if sh_ours {
            remove_file(&env_sh, &mut report);
        }
        if let Some(user_path) = user_path {
            match user_path.read() {
                Ok(Some(value)) => {
                    if let Some((new_value, removed)) = remove_path_entry(&value, prefix) {
                        match user_path.write(&new_value) {
                            Ok(()) => report.path_entries.extend(removed),
                            Err(e) => report
                                .failed
                                .push(format!("cannot update the user PATH: {e}")),
                        }
                    }
                }
                Ok(None) => {}
                Err(e) => report
                    .failed
                    .push(format!("cannot read the user PATH: {e}")),
            }
        }
    }
    if !report.ok() {
        return Ok(report);
    }

    // 4. A purge: the skills, then ~/.kluris but the folder the binary is in.
    let binary = prefix.join(req.env.binary_name());
    if req.purge {
        report.skills = remove_generated_skills(home, req.copilot_home)
            .into_iter()
            .filter(|d| !d.removed.is_empty() || !d.kept.is_empty() || !d.failed.is_empty())
            .collect();
        if !report.ok() {
            return Ok(report);
        }
        clear_kluris_dir(&kluris_dir, prefix, &binary, &mut report);
        if !report.ok() {
            return Ok(report);
        }
    }

    // The receipt goes last, so its folder must take a removal now: one
    // that would not is found before the binary is gone.
    if let Err(e) = probe_removable(receipt_dir) {
        report.failed.push(format!(
            "cannot remove the receipt {}: {e}",
            req.receipt_path.display()
        ));
        return Ok(report);
    }

    // 5. The binary, by its own name in the receipt's prefix, then its
    // empty folder (and under a purge what is left of ~/.kluris). A binary
    // that will not go keeps the receipt.
    if fs::symlink_metadata(&binary).is_ok() {
        match remove_binary(&binary) {
            Ok(()) => report.removed.push(binary),
            Err(e) => {
                report
                    .failed
                    .push(format!("cannot remove {}: {e}", binary.display()));
                return Ok(report);
            }
        }
    }
    remove_dir_if_empty(prefix, &mut report);
    if req.purge {
        if fs::symlink_metadata(&kluris_dir).is_ok() {
            match fs::remove_dir_all(&kluris_dir) {
                Ok(()) => {
                    report.removed.push(kluris_dir);
                    report.purged = true;
                }
                Err(e) => report
                    .failed
                    .push(format!("cannot remove {}: {e}", kluris_dir.display())),
            }
        } else {
            report.purged = true;
        }
    }

    // 6. The receipt.
    remove_file(req.receipt_path, &mut report);
    remove_dir_if_empty(receipt_dir, &mut report);
    Ok(report)
}

/// A purge's ~/.kluris before the binary goes: every entry but the one that
/// holds the install prefix (`~/.kluris/bin`) or is the binary itself.
fn clear_kluris_dir(kluris_dir: &Path, prefix: &Path, binary: &Path, report: &mut UninstallReport) {
    let entries = match fs::read_dir(kluris_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return,
        Err(e) => {
            report
                .failed
                .push(format!("cannot read {}: {e}", kluris_dir.display()));
            return;
        }
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for path in paths {
        if prefix.starts_with(&path) || path == binary {
            continue;
        }
        let removed = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path),
            Ok(_) => fs::remove_file(&path),
            Err(e) => Err(e),
        };
        match removed {
            Ok(()) => report.removed.push(path),
            Err(e) => {
                report
                    .failed
                    .push(format!("cannot remove {}: {e}", path.display()));
                return;
            }
        }
    }
}

/// Whether a file in `dir` can be removed: a probe file made and removed
/// there (the folder's own permission decides both).
fn probe_removable(dir: &Path) -> io::Result<()> {
    let probe = dir.join(format!(".{}-uninstall-probe", super::APP_NAME));
    fs::write(&probe, b"")?;
    fs::remove_file(&probe)
}

/// How the installer names the install folder inside its env scripts:
/// `$HOME/<rest>` under HOME (its `replace_home`), else the folder itself.
fn install_dir_exprs(prefix: &Path, home: &Path) -> Vec<String> {
    let mut exprs = Vec::new();
    if let Ok(rel) = prefix.strip_prefix(home) {
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        exprs.push(format!("$HOME/{}", rel.join("/")));
    }
    exprs.push(prefix.to_string_lossy().into_owned());
    exprs
}

/// dist 0.33.1's env.sh for the install folder `expr`
/// (`write_env_script_sh`).
fn env_sh_text(expr: &str) -> String {
    format!(
        "#!/bin/sh\n\
         # add binaries to PATH if they aren't added yet\n\
         # affix colons on either side of $PATH to simplify matching\n\
         case \":${{PATH}}:\" in\n    \
         *:\"{expr}\":*)\n        \
         ;;\n    \
         *)\n        \
         # Prepending path in case a system-installed binary needs to be overridden\n        \
         export PATH=\"{expr}:$PATH\"\n        \
         ;;\n\
         esac\n"
    )
}

/// dist 0.33.1's env.fish for the install folder `expr`
/// (`write_env_script_fish`).
fn env_fish_text(expr: &str) -> String {
    format!(
        "if not contains \"{expr}\" $PATH\n    \
         # Prepending path in case a system-installed binary needs to be overridden\n    \
         set -x PATH \"{expr}\" $PATH\n\
         end\n"
    )
}

/// Whether the env script at `path` may go with the lines sourcing it: it
/// is not there, or it is exactly what dist writes for one of `exprs`. One
/// that is anything else is the user's: kept, and said in `report.kept`.
fn dists_script(
    path: &Path,
    exprs: &[String],
    text_for: fn(&str) -> String,
    report: &mut UninstallReport,
) -> bool {
    let text = match fs::read(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return true,
        Err(e) => {
            report
                .failed
                .push(format!("cannot read {}: {e}", path.display()));
            return false;
        }
    };
    if exprs.iter().any(|expr| text_for(expr).as_bytes() == text) {
        true
    } else {
        report.kept.push(path.to_path_buf());
        false
    }
}

/// `value` (a `;`-separated PATH) without the entries naming `prefix`, and
/// those entries as written; `None` when there is none. Entries match as
/// Windows compares paths: without case, `/` as `\`, a trailing `\` ignored.
/// Every other entry, empty ones included, stays as it was.
pub fn remove_path_entry(value: &str, prefix: &Path) -> Option<(String, Vec<String>)> {
    let want = windows_key(&prefix.to_string_lossy());
    let (removed, kept): (Vec<&str>, Vec<&str>) = value
        .split(';')
        .partition(|entry| !entry.is_empty() && windows_key(entry) == want);
    if removed.is_empty() {
        return None;
    }
    Some((
        kept.join(";"),
        removed.into_iter().map(str::to_string).collect(),
    ))
}

fn windows_key(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// Whether `path` is `dir` or inside it (lexically, and once more with both
/// resolved when they exist).
fn is_within(path: &Path, dir: &Path) -> bool {
    if path.starts_with(dir) {
        return true;
    }
    match (fs::canonicalize(path), fs::canonicalize(dir)) {
        (Ok(p), Ok(d)) => p.starts_with(d),
        _ => false,
    }
}

/// The lines that source `scripts`: `<command> "<script>"`, the script named
/// with `$HOME` in front when under HOME (as the installer writes it) and
/// in full.
fn source_lines(command: &str, scripts: &[&Path], home: &Path) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    for script in scripts {
        let mut exprs = Vec::new();
        if let Ok(rel) = script.strip_prefix(home) {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            exprs.push(format!("$HOME/{}", rel.join("/")));
        }
        exprs.push(script.to_string_lossy().into_owned());
        for expr in exprs {
            let line = format!("{command} \"{expr}\"").into_bytes();
            if !lines.contains(&line) {
                lines.push(line);
            }
        }
    }
    lines
}

/// Take `lines` out of the rc file `path` ([`without_lines`]). A file left
/// with only whitespace is removed when `remove_if_empty` (the installer's
/// own fish conf.d file).
fn edit_rc(path: &Path, lines: &[Vec<u8>], remove_if_empty: bool, report: &mut UninstallReport) {
    if !fs::metadata(path).is_ok_and(|m| m.is_file()) {
        return;
    }
    let text = match fs::read(path) {
        Ok(text) => text,
        Err(e) => {
            report
                .failed
                .push(format!("cannot read {}: {e}", path.display()));
            return;
        }
    };
    let Some(new_text) = without_lines(&text, lines) else {
        return;
    };
    if remove_if_empty && new_text.iter().all(u8::is_ascii_whitespace) {
        remove_file(path, report);
        return;
    }
    match crate::atomic::write_atomic(path, &new_text) {
        Ok(()) => report.edited.push(path.to_path_buf()),
        Err(e) => report
            .failed
            .push(format!("cannot write {}: {e}", path.display())),
    }
}

/// `text` without each line (ignoring its `\n` / `\r\n`) that equals one of
/// `lines`, nor the empty line right before it; `None` when there is none.
fn without_lines(text: &[u8], lines: &[Vec<u8>]) -> Option<Vec<u8>> {
    fn content(line: &[u8]) -> &[u8] {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        line.strip_suffix(b"\r").unwrap_or(line)
    }
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut changed = false;
    for line in text.split_inclusive(|b| *b == b'\n') {
        if lines.iter().any(|l| l.as_slice() == content(line)) {
            changed = true;
            if kept.last().is_some_and(|prev| content(prev).is_empty()) {
                kept.pop();
            }
            continue;
        }
        kept.push(line);
    }
    changed.then(|| kept.concat())
}

/// Remove the file (or symlink) at `path` when there is one.
fn remove_file(path: &Path, report: &mut UninstallReport) {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_dir() => match fs::remove_file(path) {
            Ok(()) => report.removed.push(path.to_path_buf()),
            Err(e) => report
                .failed
                .push(format!("cannot remove {}: {e}", path.display())),
        },
        _ => {}
    }
}

/// Remove the folder `path` when it is there and empty.
fn remove_dir_if_empty(path: &Path, report: &mut UninstallReport) {
    let empty = fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_none());
    if empty && fs::remove_dir(path).is_ok() {
        report.removed.push(path.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|l| l.as_bytes().to_vec()).collect()
    }

    /// An rc file is rewritten atomically (a new file renamed over it, so a
    /// failed write cannot truncate it), and a symlinked rc file stays a
    /// link to the file that gets the edit.
    #[cfg(unix)]
    #[test]
    fn edit_rc_replaces_the_file_and_keeps_a_symlink() {
        use std::os::unix::fs::{MetadataExt, symlink};
        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("dotfiles-zshrc");
        fs::write(
            &real,
            "alias ll='ls -l'\n\n. \"$HOME/.config/kluris/env.sh\"\n",
        )
        .unwrap();
        let rc = dir.path().join(".zshrc");
        symlink(&real, &rc).unwrap();
        let inode = fs::metadata(&real).unwrap().ino();
        let mut report = UninstallReport::default();
        edit_rc(
            &rc,
            &lines(&[". \"$HOME/.config/kluris/env.sh\""]),
            false,
            &mut report,
        );
        assert_eq!(report.failed, Vec::<String>::new());
        assert_eq!(report.edited, std::slice::from_ref(&rc));
        assert!(fs::symlink_metadata(&rc).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "alias ll='ls -l'\n");
        assert_ne!(fs::metadata(&real).unwrap().ino(), inode);
    }

    #[test]
    fn without_lines_takes_the_line_and_the_empty_line_before_it() {
        let ours = lines(&["X"]);
        assert_eq!(without_lines(b"a\n\nX\n", &ours), Some(b"a\n".to_vec()));
        assert_eq!(without_lines(b"a\nX\n", &ours), Some(b"a\n".to_vec()));
        assert_eq!(without_lines(b"\nX\n", &ours), Some(Vec::new()));
        assert_eq!(without_lines(b"a\n\nX", &ours), Some(b"a\n".to_vec()));
        assert_eq!(without_lines(b"X \nX#\n X\n", &ours), None);
        assert_eq!(
            without_lines(b"a\r\n\r\nX\r\nb\r\n", &ours),
            Some(b"a\r\nb\r\n".to_vec())
        );
    }

    #[test]
    fn source_lines_name_the_script_both_ways() {
        let home = Path::new("/h");
        assert_eq!(
            source_lines(".", &[Path::new("/h/.config/kluris/env.sh")], home),
            lines(&[
                ". \"$HOME/.config/kluris/env.sh\"",
                ". \"/h/.config/kluris/env.sh\""
            ])
        );
        assert_eq!(
            source_lines("source", &[Path::new("/x/env.fish")], home),
            lines(&["source \"/x/env.fish\""])
        );
    }
}
