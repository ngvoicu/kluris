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

use super::{DistEnv, Receipt};
use crate::agents::install::{DestinationReport, remove_generated_skills};

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
        }
    }
}

impl std::error::Error for UninstallError {}

/// Remove what the installers wrote. The binaries go through
/// `remove_binary` (the running one may need the platform's self-delete);
/// the user PATH through `user_path` when given (Windows). Failures are
/// reported and the rest goes on; only a purge that would remove a brain
/// stops before anything is touched.
pub fn uninstall(
    req: &UninstallRequest,
    user_path: Option<&mut dyn UserPath>,
    remove_binary: &mut dyn FnMut(&Path) -> io::Result<()>,
) -> Result<UninstallReport, UninstallError> {
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

    // rc files: the sh line in each, the fish line in the conf.d file.
    let sh_lines = source_lines(".", &[&env_sh], home);
    let zdir = req.env.zdotdir.unwrap_or(home);
    let rc_files = [
        home.join(".profile"),
        home.join(".bashrc"),
        home.join(".bash_profile"),
        home.join(".bash_login"),
        zdir.join(".zshrc"),
        zdir.join(".zshenv"),
    ];
    for rc in &rc_files {
        edit_rc(rc, &sh_lines, false, &mut report);
    }
    let fish_lines = source_lines("source", &[&env_fish], home);
    let fish_conf = home
        .join(".config")
        .join("fish")
        .join("conf.d")
        .join(format!("{}.env.fish", super::APP_NAME));
    edit_rc(&fish_conf, &fish_lines, true, &mut report);

    for script in [&env_sh, &env_fish] {
        remove_file(script, &mut report);
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

    if req.purge {
        report.skills = remove_generated_skills(home, req.copilot_home)
            .into_iter()
            .filter(|d| !d.removed.is_empty() || !d.kept.is_empty() || !d.failed.is_empty())
            .collect();
    }

    remove_file(req.receipt_path, &mut report);
    remove_dir_if_empty(receipt_dir, &mut report);

    let binaries = if req.receipt.binaries.is_empty() {
        vec![req.env.binary_name()]
    } else {
        req.receipt.binaries.clone()
    };
    for name in binaries {
        let path = prefix.join(name);
        if fs::symlink_metadata(&path).is_err() {
            continue;
        }
        match remove_binary(&path) {
            Ok(()) => report.removed.push(path),
            Err(e) => report
                .failed
                .push(format!("cannot remove {}: {e}", path.display())),
        }
    }
    remove_dir_if_empty(prefix, &mut report);

    if req.purge && fs::symlink_metadata(&kluris_dir).is_ok() {
        match fs::remove_dir_all(&kluris_dir) {
            Ok(()) => {
                report.removed.push(kluris_dir);
                report.purged = true;
            }
            Err(e) => report
                .failed
                .push(format!("cannot remove {}: {e}", kluris_dir.display())),
        }
    } else if req.purge {
        report.purged = true;
    }
    Ok(report)
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
