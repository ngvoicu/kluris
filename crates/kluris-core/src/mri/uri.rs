//! The `file://` link `kluris mri` prints: Python 3.11's `Path.as_uri()` of
//! the resolved page, or under WSL the Windows path `wslpath -w` gives
//! (v2's `_is_wsl` and `_windows_path_if_wsl`, `cli.py:1525-1553`).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

/// `urllib.parse.quote_from_bytes(bytes, safe)`: letters, digits, `_.-~` and
/// `safe` kept, every other byte `%XX` (upper-case hex).
fn quote(bytes: &[u8], safe: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"_.-~".contains(&b) || safe.contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `PurePosixPath(path).as_uri()` / `PureWindowsPath(path).as_uri()` of an
/// absolute path: `file://` + the quoted path; on Windows `file:///C:/...`
/// for a drive and `file://server/share/...` for a UNC path.
pub fn file_uri(path: &Path) -> String {
    if cfg!(windows) {
        let text = path.to_string_lossy().replace('\\', "/");
        let bytes = text.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' {
            let rest = text[2..].trim_start_matches('/');
            format!("file:///{}/{}", &text[..2], quote(rest.as_bytes(), b"/"))
        } else {
            format!("file:{}", quote(bytes, b"/"))
        }
    } else {
        format!(
            "file://{}",
            quote(path.as_os_str().as_encoded_bytes(), b"/")
        )
    }
}

/// v2's `_is_wsl`: `WSL_DISTRO_NAME` is set (and not empty), or
/// `/proc/version` mentions `microsoft` in any case.
pub fn is_wsl(distro: Option<&OsStr>, proc_version: Option<&str>) -> bool {
    distro.is_some_and(|d| !d.is_empty())
        || proc_version.is_some_and(|v| v.to_lowercase().contains("microsoft"))
}

/// The link v2 builds from `wslpath -w`: `file://` + the UNC path without its
/// leading backslashes, backslashes turned into slashes.
pub fn wsl_file_uri(windows_path: &str) -> String {
    format!(
        "file://{}",
        windows_path.trim_start_matches('\\').replace('\\', "/")
    )
}

/// v2's `_windows_path_if_wsl`: under WSL, `wslpath -w <path>`'s trimmed
/// output when it succeeds and says something; `None` otherwise.
pub fn windows_path_if_wsl(path: &Path) -> Option<String> {
    let proc_version = std::fs::read_to_string("/proc/version").ok();
    if !is_wsl(
        std::env::var_os("WSL_DISTRO_NAME").as_deref(),
        proc_version.as_deref(),
    ) {
        return None;
    }
    let out = Command::new("wslpath")
        .arg("-w")
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim_matches(crate::yaml::is_py_space);
    (!text.is_empty()).then(|| text.to_string())
}

/// The link `kluris mri` prints for the page at `out` (as given: relative to
/// the working directory): the WSL form when that applies, else the
/// `as_uri()` of the resolved path.
pub fn page_uri(out: &Path) -> String {
    match windows_path_if_wsl(out) {
        Some(windows) => wsl_file_uri(&windows),
        None => {
            let resolved =
                crate::brain::sandbox::resolve(out).unwrap_or_else(|_| out.to_path_buf());
            file_uri(&resolved)
        }
    }
}
