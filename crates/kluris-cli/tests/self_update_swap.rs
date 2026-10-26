//! The self-update swap (`kluris_core::dist::swap`) with a failure injected
//! at each step through `SwapSteps`, under both algorithms (`windows`:
//! rename the installed binary aside, then the staged one in; otherwise one
//! rename over it): every failure leaves the installed `kluris` in place,
//! byte-identical and running, and nothing staged behind. On Windows one
//! more test fails a real swap: the staged file held open with no sharing.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use kluris_core::dist::swap::{Probe, RealSteps, SwapRequest, SwapSteps, Swapped, swap};
use tempfile::TempDir;

mod common;

const CURRENT: &str = env!("CARGO_PKG_VERSION");

fn exe_name() -> &'static str {
    if cfg!(windows) { "kluris.exe" } else { "kluris" }
}

/// An install folder holding a copy of the built `kluris`, and a release's
/// new binary extracted elsewhere.
struct Setup {
    _root: TempDir,
    prefix: PathBuf,
    installed: PathBuf,
    old: Vec<u8>,
    new_binary: PathBuf,
    new: Vec<u8>,
}

impl Setup {
    fn new() -> Setup {
        let root = TempDir::new().unwrap();
        let base = fs::canonicalize(root.path()).unwrap();
        let prefix = base.join("bin");
        fs::create_dir_all(&prefix).unwrap();
        let installed = prefix.join(exe_name());
        fs::copy(assert_cmd::cargo::cargo_bin!("kluris"), &installed).unwrap();
        let old = fs::read(&installed).unwrap();
        let extracted = base.join("extracted");
        fs::create_dir_all(&extracted).unwrap();
        let new_binary = extracted.join(exe_name());
        let new = common::fake_for("9.0.0", 0, "", &base.join("args.txt"));
        fs::write(&new_binary, &new).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&new_binary, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Setup {
            _root: root,
            prefix,
            installed,
            old,
            new_binary,
            new,
        }
    }

    fn request(&self, windows: bool) -> SwapRequest<'_> {
        SwapRequest {
            installed: &self.installed,
            new_binary: &self.new_binary,
            version: "9.0.0",
            windows,
        }
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.prefix)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The installed kluris is the old one, byte for byte, and runs.
    fn assert_old_kluris_works(&self, case: &str) {
        assert!(fs::read(&self.installed).unwrap() == self.old, "{case}: bytes");
        let out = Command::new(&self.installed).arg("--version").output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{case}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("kluris {CURRENT}\n"), "{case}");
        assert_eq!(self.names(), vec![exe_name().to_string()], "{case}: leftovers");
    }
}

/// Where to fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    /// Writing the staged copy: part of it is written, then the disk fills.
    Stage,
    /// Running the staged copy at all.
    Probe,
    /// Renaming the installed binary aside (Windows).
    Aside,
    /// Renaming the staged copy into place.
    Place,
    /// Renaming it into place and then renaming the old one back.
    PlaceAndRestore,
}

struct Faulty {
    real: RealSteps,
    fail: Fail,
    installed: PathBuf,
}

fn name_has(path: &Path, part: &str) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().contains(part))
}

impl SwapSteps for Faulty {
    fn stage(&mut self, new_binary: &Path, staged: &Path, like: &Path) -> io::Result<()> {
        if self.fail == Fail::Stage {
            fs::write(staged, b"half a bin")?;
            return Err(io::Error::other("no space left on device"));
        }
        self.real.stage(new_binary, staged, like)
    }

    fn probe(&mut self, exe: &Path) -> io::Result<Probe> {
        if self.fail == Fail::Probe {
            return Err(io::Error::other("cannot start it"));
        }
        self.real.probe(exe)
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        let fails = match self.fail {
            Fail::Aside => from == self.installed,
            Fail::Place => name_has(from, ".new-"),
            Fail::PlaceAndRestore => name_has(from, ".new-") || name_has(from, ".old-"),
            _ => false,
        };
        if fails {
            return Err(io::Error::other("access denied (injected)"));
        }
        self.real.rename(from, to)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.real.remove(path)
    }
}

fn faulty(s: &Setup, fail: Fail) -> Faulty {
    Faulty {
        real: RealSteps::default(),
        fail,
        installed: s.installed.clone(),
    }
}

#[test]
fn every_failed_step_keeps_the_installed_kluris() {
    for windows in [false, true] {
        let mut fails = vec![Fail::Stage, Fail::Probe, Fail::Place];
        if windows {
            fails.push(Fail::Aside);
        }
        for fail in fails {
            let case = format!("{fail:?} (windows: {windows})");
            let s = Setup::new();
            let err = swap(&s.request(windows), &mut faulty(&s, fail))
                .expect_err(&case)
                .to_string();
            assert!(err.contains("unchanged"), "{case}: {err}");
            s.assert_old_kluris_works(&case);
        }
    }
}

#[test]
fn a_failed_rollback_deletes_nothing_and_says_where_kluris_is() {
    let s = Setup::new();
    let err = swap(&s.request(true), &mut faulty(&s, Fail::PlaceAndRestore))
        .expect_err("both renames fail")
        .to_string();
    let names = s.names();
    let old = names
        .iter()
        .find(|n| n.starts_with(&format!(".{}.old-", exe_name())))
        .unwrap_or_else(|| panic!("the old binary is kept: {names:?}"));
    assert!(fs::read(s.prefix.join(old)).unwrap() == s.old);
    assert!(err.contains(old) && err.contains(exe_name()), "{err}");
    assert!(!names.iter().any(|n| n.contains(".new-")), "{names:?}");
}

#[test]
fn a_good_swap_puts_the_new_binary_in_place() {
    for windows in [false, true] {
        let s = Setup::new();
        let Swapped { old } = swap(&s.request(windows), &mut RealSteps::default())
            .unwrap_or_else(|e| panic!("windows: {windows}: {e}"));
        assert!(fs::read(&s.installed).unwrap() == s.new, "windows: {windows}");
        let names = s.names();
        if windows {
            // The old binary stays beside the new one (a running exe cannot
            // be deleted), under the name the sweep knows.
            let old = old.expect("an old binary");
            let name = old.file_name().unwrap().to_string_lossy().into_owned();
            let rest = name
                .strip_prefix(&format!(".{}.old-", exe_name()))
                .unwrap_or_else(|| panic!("{name}"));
            let (pid, n) = rest.split_once('-').unwrap();
            assert_eq!(pid, std::process::id().to_string());
            assert!(n.chars().all(|c| c.is_ascii_digit()), "{name}");
            assert!(fs::read(&old).unwrap() == s.old);
            let mut expected = vec![exe_name().to_string(), name];
            expected.sort();
            assert_eq!(names, expected);
        } else {
            assert_eq!(old, None);
            assert_eq!(names, vec![exe_name().to_string()]);
        }
    }
}

#[cfg(unix)]
#[test]
fn the_staged_copy_keeps_the_installed_mode() {
    use std::os::unix::fs::PermissionsExt;
    let s = Setup::new();
    fs::set_permissions(&s.installed, fs::Permissions::from_mode(0o750)).unwrap();
    swap(&s.request(false), &mut RealSteps::default()).unwrap();
    let mode = fs::metadata(&s.installed).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o750);
}

/// A real failed swap: the staged file is held open with no sharing, so
/// Windows refuses to rename it into place, and the old binary goes back.
#[cfg(windows)]
#[test]
fn a_real_failed_swap_on_windows_restores_kluris() {
    use std::os::windows::fs::OpenOptionsExt;
    struct Locking(RealSteps);
    impl SwapSteps for Locking {
        fn stage(&mut self, new_binary: &Path, staged: &Path, like: &Path) -> io::Result<()> {
            self.0.stage(new_binary, staged, like)
        }
        fn probe(&mut self, exe: &Path) -> io::Result<Probe> {
            self.0.probe(exe)
        }
        fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            if name_has(from, ".new-") {
                let _held = fs::OpenOptions::new()
                    .read(true)
                    .share_mode(0)
                    .open(from)?;
                return self.0.rename(from, to);
            }
            self.0.rename(from, to)
        }
        fn remove(&mut self, path: &Path) -> io::Result<()> {
            self.0.remove(path)
        }
    }
    let s = Setup::new();
    let err = swap(&s.request(true), &mut Locking(RealSteps::default()))
        .expect_err("the held file cannot be renamed")
        .to_string();
    assert!(err.contains("unchanged"), "{err}");
    s.assert_old_kluris_works("held staged file");
}
