//! What Kluris's installers (dist 0.33.1's `kluris-installer.sh` / `.ps1`,
//! `install-path = "~/.kluris/bin"`) leave on a machine, for `kluris
//! self-update` and `kluris self-uninstall` (research-02 §1, §5).
//!
//! - The binary goes to `~/.kluris/bin` (the flat layout: the receipt's
//!   `install_prefix` is that folder).
//! - The install receipt is `kluris-receipt.json` in the receipt folder:
//!   `$XDG_CONFIG_HOME/kluris` or `~/.config/kluris` on Unix; on Windows
//!   `$XDG_CONFIG_HOME\kluris` when set, else `%LOCALAPPDATA%\kluris`.
//! - Unix PATH setup: `env.sh` and `env.fish` in the receipt folder, an
//!   empty line then `. "$HOME/.config/kluris/env.sh"` appended to the rc
//!   files, and `~/.config/fish/conf.d/kluris.env.fish` sourcing `env.fish`.
//!   (dist before 0.33 put env scripts beside the binary; no Kluris
//!   installer did, so nothing there is touched.)
//! - Windows PATH setup: the install folder in front of the user PATH
//!   (`HKCU\Environment\Path`, `REG_EXPAND_SZ`).
//!
//! Every name derives from [`APP_NAME`], dist's app name.

mod receipt;
mod uninstall;

use std::path::{Path, PathBuf};

pub use receipt::{Receipt, ReceiptError};
pub use uninstall::{
    UninstallError, UninstallReport, UninstallRequest, UserPath, remove_path_entry, uninstall,
};

/// dist's app name: the receipt, env-script and archive names follow it.
pub const APP_NAME: &str = "kluris";

/// The GitHub repository releases come from.
pub const REPO_OWNER: &str = "ngvoicu";
pub const REPO_NAME: &str = "kluris";

/// Where the installers' files are, from the environment.
#[derive(Debug, Clone, Copy)]
pub struct DistEnv<'a> {
    /// HOME (`config::home_dir`).
    pub home: &'a Path,
    /// `XDG_CONFIG_HOME` when set and non-empty.
    pub xdg_config_home: Option<&'a Path>,
    /// `LOCALAPPDATA` when set and non-empty (Windows).
    pub local_app_data: Option<&'a Path>,
    /// `ZDOTDIR` when set and non-empty: where the zsh rc files are.
    pub zdotdir: Option<&'a Path>,
    /// The PowerShell installer's rules (Windows) or the shell installer's.
    pub windows: bool,
}

impl DistEnv<'_> {
    /// The receipt folder (also the env scripts' folder).
    pub fn receipt_home(&self) -> Option<PathBuf> {
        let base = match (self.windows, self.xdg_config_home) {
            (_, Some(xdg)) => xdg.to_path_buf(),
            (true, None) => self.local_app_data?.to_path_buf(),
            (false, None) => self.home.join(".config"),
        };
        Some(base.join(APP_NAME))
    }

    /// The install receipt.
    pub fn receipt_path(&self) -> Option<PathBuf> {
        Some(
            self.receipt_home()?
                .join(format!("{APP_NAME}-receipt.json")),
        )
    }

    /// The binary's file name.
    pub fn binary_name(&self) -> String {
        if self.windows {
            format!("{APP_NAME}.exe")
        } else {
            APP_NAME.to_string()
        }
    }
}

/// The release archive for `target`: `kluris-<target>.zip` on Windows,
/// `kluris-<target>.tar.gz` elsewhere.
pub fn archive_name(target: &str) -> String {
    if target.contains("windows") {
        format!("{APP_NAME}-{target}.zip")
    } else {
        format!("{APP_NAME}-{target}.tar.gz")
    }
}

/// The binary inside that archive: dist puts everything in a top-level
/// `kluris-<target>/` folder.
pub fn binary_in_archive(target: &str) -> String {
    let exe = if target.contains("windows") {
        format!("{APP_NAME}.exe")
    } else {
        APP_NAME.to_string()
    };
    format!("{APP_NAME}-{target}/{exe}")
}

/// The release checksums asset (`<hex> *<name>` lines).
pub const SUMS_ASSET: &str = "sha256.sum";

#[cfg(test)]
mod tests {
    use super::*;

    // dist 0.33.1's real layouts: `tar -tzf kluris-aarch64-apple-darwin.tar.gz`
    // lists `kluris-aarch64-apple-darwin/kluris` (its installer unpacks with
    // `--strip-components 1`); the Windows zip is flat: kluris-installer.ps1
    // copies `$tmp\kluris.exe` straight after `Expand-Archive`, and dist's own
    // `cargo-dist-x86_64-pc-windows-msvc.zip` has `dist.exe` at its root.
    #[test]
    fn the_binary_is_where_dist_puts_it() {
        for target in [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
        ] {
            assert_eq!(binary_in_archive(target), format!("kluris-{target}/kluris"));
            assert_eq!(archive_name(target), format!("kluris-{target}.tar.gz"));
        }
        for target in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
            assert_eq!(binary_in_archive(target), "kluris.exe");
            assert_eq!(archive_name(target), format!("kluris-{target}.zip"));
        }
    }

    #[test]
    fn the_receipt_is_where_dist_writes_it() {
        let unix = DistEnv {
            home: Path::new("/home/u"),
            xdg_config_home: None,
            local_app_data: None,
            zdotdir: None,
            windows: false,
        };
        assert_eq!(
            unix.receipt_path(),
            Some(PathBuf::from("/home/u/.config/kluris/kluris-receipt.json"))
        );
        let windows = DistEnv {
            home: Path::new(r"C:\Users\u"),
            local_app_data: Some(Path::new(r"C:\Users\u\AppData\Local")),
            windows: true,
            ..unix
        };
        assert_eq!(
            windows.receipt_path(),
            Some(
                PathBuf::from(r"C:\Users\u\AppData\Local")
                    .join("kluris")
                    .join("kluris-receipt.json")
            )
        );
        assert_eq!(windows.binary_name(), "kluris.exe");
    }
}
