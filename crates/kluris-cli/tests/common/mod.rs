//! Helpers shared by the CLI tests: the golden-snapshot normalizations
//! ([`golden`]), the sandboxed runs golden tests snapshot ([`sandbox`]), the
//! brains a sandbox reads ([`brains`]), the fixture brains of the dream and
//! MRI goldens ([`dream_brains`]), runs with stdin on a pseudo-terminal
//! ([`pty`], Unix), the files a snapshot shows by reference ([`shared`]),
//! the frozen homes an earlier Kluris set up ([`upgrade`]) and a fake
//! release binary for the self-update tests.

#![allow(dead_code, unused_imports)]

pub mod brains;
pub mod dream_brains;
pub mod golden;
#[cfg(unix)]
pub mod pty;
pub mod sandbox;
pub mod shared;
pub mod upgrade;

pub use golden::{
    PASSTHROUGH_ENV, PINNED_ENV, PINNED_GIT_DATE, decode, file_body, git_dump, git_state,
    native_path, normalize, normalize_json, tree_dump, tree_files,
};
pub use sandbox::{
    Run, Sandbox, changes_dump, command_line, copy_tree, fixture, printed_paths, unstored, write,
    write_files,
};

use std::path::{Path, PathBuf};

/// A release's new binary for the self-update tests: it answers
/// `--version` with `version_line` (exit `version_code`); started any other
/// way it writes how it was started (the program as given, then the
/// arguments, one per line) to `args_file`, prints `out` and exits `code`.
///
/// Unix: a shell script. Windows: the `fake_kluris` example (an exe must be
/// one) with its settings appended as a trailer. `cargo test --workspace`
/// (and `cargo xtask windows`) builds the examples; a filtered `cargo test
/// -p kluris --test self_update` does not, and `--examples` does not help
/// (it builds a test-harness copy, `fake_kluris-<hash>`), so on Windows run
/// `cargo build -p kluris --examples` first.
pub struct Fake<'a> {
    pub version_line: &'a str,
    pub version_code: i32,
    pub code: i32,
    pub out: &'a str,
    pub args_file: &'a Path,
}

impl Fake<'_> {
    /// A fake for release `version` that runs `doctor` with exit `code`.
    pub fn bytes(&self) -> Vec<u8> {
        if cfg!(windows) {
            let mut bytes = std::fs::read(fake_exe()).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; build the example first: cargo build -p kluris --examples (cargo test --examples builds only a test-harness copy, not this binary)",
                    fake_exe().display()
                )
            });
            bytes.extend_from_slice(b"\n#KLURIS-FAKE#");
            let config = serde_json::json!({
                "version_line": self.version_line,
                "version_code": self.version_code,
                "code": self.code,
                "out": self.out,
                "args_file": self.args_file.display().to_string(),
            });
            bytes.extend_from_slice(config.to_string().as_bytes());
            bytes
        } else {
            format!(
                "#!/bin/sh\nif [ \"$#\" = 1 ] && [ \"$1\" = --version ]; then\n    printf '%s\\n' '{}'\n    exit {}\nfi\nprintf '%s\\n' \"$0\" \"$@\" > '{}'\nprintf '%s\\n' '{}'\nexit {}\n",
                self.version_line,
                self.version_code,
                self.args_file.display(),
                self.out,
                self.code
            )
            .into_bytes()
        }
    }
}

/// A well-behaved release binary for `version`.
pub fn fake_for(version: &str, code: i32, out: &str, args_file: &Path) -> Vec<u8> {
    Fake {
        version_line: &format!("kluris {version}"),
        version_code: 0,
        code,
        out,
        args_file,
    }
    .bytes()
}

/// Bytes no OS here can run: a header of the other platform's format.
pub fn wrong_format() -> Vec<u8> {
    let mut bytes = if cfg!(windows) {
        b"\x7fELF\x02\x01\x01\0".to_vec()
    } else {
        b"MZ\x90\0\x03\0\0\0".to_vec()
    };
    bytes.extend_from_slice(&[0u8; 256]);
    bytes
}

/// `target/<profile>/examples/fake_kluris.exe`.
fn fake_exe() -> PathBuf {
    let kluris = Path::new(assert_cmd::cargo::cargo_bin!("kluris"));
    let dir = kluris.parent().unwrap_or(Path::new("."));
    dir.join("examples").join("fake_kluris.exe")
}
