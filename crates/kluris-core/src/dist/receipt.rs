//! dist's install receipt: one JSON line,
//! `{"binaries":["kluris"],...,"install_prefix":"<dir>","modify_path":true,
//! ...,"version":"3.0.0"}` (the shell installer ends it with `\n`, the
//! PowerShell one with `\r\n`).
//!
//! A receipt says what `self-uninstall` removes and what `self-update`
//! replaces, so [`Receipt::read`] accepts only Kluris's own, as dist 0.33.1
//! writes it ([`Receipt::check_kluris`]); anything else is
//! [`ReceiptError::NotKluris`] and no command changes anything.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{APP_NAME, REPO_NAME, REPO_OWNER};

/// A receipt as read.
#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    /// The folder the binaries are in (flat layout).
    pub install_prefix: PathBuf,
    /// The installed binaries' file names (`kluris`, or `kluris.exe`).
    pub binaries: Vec<String>,
    /// The installed version, without a `v`.
    pub version: String,
    /// Whether the installer edited PATH.
    pub modify_path: bool,
    /// Every field, in the file's order, for rewriting it.
    fields: Map<String, Value>,
    /// The line ending the file had (`\n`, `\r\n`, or none).
    line_end: String,
}

/// Why a receipt could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptError {
    /// No file there.
    Missing(PathBuf),
    /// The file is there but did not read.
    Unreadable(PathBuf, String),
    /// Not a receipt: not JSON, or no `install_prefix` / `version`.
    Invalid(PathBuf, String),
    /// A receipt, but not the one dist writes for Kluris: why.
    NotKluris(PathBuf, String),
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReceiptError::Missing(p) => write!(f, "No install receipt at {}", p.display()),
            ReceiptError::Unreadable(p, e) => {
                write!(f, "Cannot read the install receipt {}: {e}", p.display())
            }
            ReceiptError::Invalid(p, e) => {
                write!(f, "The install receipt {} is not valid: {e}", p.display())
            }
            ReceiptError::NotKluris(p, why) => write!(
                f,
                "The receipt at {} is not a Kluris install receipt; nothing was changed ({why}).",
                p.display()
            ),
        }
    }
}

impl std::error::Error for ReceiptError {}

impl Receipt {
    /// Read the receipt at `path`: Kluris's own for this platform
    /// ([`Receipt::check_kluris`]), or an error.
    pub fn read(path: &Path) -> Result<Receipt, ReceiptError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ReceiptError::Missing(path.to_path_buf()));
            }
            Err(e) => return Err(ReceiptError::Unreadable(path.to_path_buf(), e.to_string())),
        };
        let receipt =
            Receipt::parse(&text).map_err(|e| ReceiptError::Invalid(path.to_path_buf(), e))?;
        receipt
            .check_kluris(cfg!(windows))
            .map_err(|why| ReceiptError::NotKluris(path.to_path_buf(), why))?;
        Ok(receipt)
    }

    /// Whether this is the receipt dist 0.33.1's installers write for Kluris
    /// (`windows`: the PowerShell installer's), field by field; the first
    /// that is not says why. `source` kluris/kluris/ngvoicu/github,
    /// `provider.source` cargo-dist, `binaries` exactly `["kluris"]`
    /// (`["kluris.exe"]`), no cdylibs, cstaticlibs or aliases, the flat
    /// layout, an absolute `install_prefix`, a boolean `modify_path`. Then
    /// the one file a receipt names is `<install_prefix>/kluris(.exe)`.
    pub fn check_kluris(&self, windows: bool) -> Result<(), String> {
        let f = &self.fields;
        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
        let source = f.get("source");
        for (key, want) in [
            ("app_name", APP_NAME),
            ("name", REPO_NAME),
            ("owner", REPO_OWNER),
            ("release_type", "github"),
        ] {
            if text(source.and_then(|s| s.get(key))).as_deref() != Some(want) {
                return Err(format!("source.{key} is not \"{want}\""));
            }
        }
        if text(f.get("provider").and_then(|p| p.get("source"))).as_deref() != Some("cargo-dist") {
            return Err("provider.source is not \"cargo-dist\"".into());
        }
        let exe = if windows {
            format!("{APP_NAME}.exe")
        } else {
            APP_NAME.to_string()
        };
        match f.get("binaries") {
            Some(Value::Array(items)) if items.len() == 1 && items[0].as_str() == Some(&exe) => {}
            _ => return Err(format!("binaries is not [\"{exe}\"]")),
        }
        for key in ["cdylibs", "cstaticlibs"] {
            match f.get(key) {
                None => {}
                Some(Value::Array(items)) if items.is_empty() => {}
                _ => return Err(format!("{key} is not empty")),
            }
        }
        match f.get("binary_aliases") {
            None => {}
            Some(Value::Object(aliases)) if aliases.is_empty() => {}
            _ => return Err("binary_aliases is not empty".into()),
        }
        if text(f.get("install_layout")).as_deref() != Some("flat") {
            return Err("install_layout is not \"flat\"".into());
        }
        if !matches!(f.get("modify_path"), Some(Value::Bool(_))) {
            return Err("modify_path is not true or false".into());
        }
        if !is_absolute(&self.install_prefix.to_string_lossy(), windows) {
            return Err("install_prefix is not an absolute path".into());
        }
        Ok(())
    }

    /// Parse a receipt's text.
    pub fn parse(text: &str) -> Result<Receipt, String> {
        let body = text.trim_start_matches('\u{feff}');
        let line_end = if body.ends_with("\r\n") {
            "\r\n"
        } else if body.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let fields = match serde_json::from_str::<Value>(body).map_err(|e| e.to_string())? {
            Value::Object(fields) => fields,
            _ => return Err("not a JSON object".into()),
        };
        let string = |key: &str| match fields.get(key) {
            Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
            _ => Err(format!("no `{key}`")),
        };
        let install_prefix = PathBuf::from(string("install_prefix")?);
        let version = string("version")?;
        let binaries = match fields.get("binaries") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .filter(|b| is_plain_name(b))
                .collect(),
            _ => Vec::new(),
        };
        let modify_path = !matches!(fields.get("modify_path"), Some(Value::Bool(false)));
        Ok(Receipt {
            install_prefix,
            binaries,
            version,
            modify_path,
            fields,
            line_end: line_end.to_string(),
        })
    }

    /// The receipt's text with `version` set to `version`, everything else
    /// as it was (the installers' compact form).
    pub fn text_with_version(&self, version: &str) -> String {
        let mut fields = self.fields.clone();
        fields.insert("version".into(), Value::String(version.into()));
        let mut text = Value::Object(fields).to_string();
        text.push_str(&self.line_end);
        text
    }

    /// Rewrite the receipt at `path` with `version`: a sibling temp file
    /// renamed over it.
    pub fn write_version(&self, path: &Path, version: &str) -> io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, self.text_with_version(version))?;
        fs::rename(&tmp, path).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }
}

/// `path` is absolute on that platform: `/...` on Unix; `C:\...` (or
/// `C:/...`) or a `\\server\share` path on Windows.
fn is_absolute(path: &str, windows: bool) -> bool {
    if !windows {
        return path.starts_with('/');
    }
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    drive || path.starts_with("\\\\")
}

/// A file name with no folder part (`kluris`, `kluris.exe`): a receipt never
/// names a path to remove outside its install prefix.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':'])
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIST: &str = "{\"binaries\":[\"kluris\"],\"binary_aliases\":{},\"cdylibs\":[],\"cstaticlibs\":[],\"install_layout\":\"flat\",\"install_prefix\":\"/home/u/.kluris/bin\",\"modify_path\":true,\"provider\":{\"source\":\"cargo-dist\",\"version\":\"0.33.1\"},\"source\":{\"app_name\":\"kluris\",\"name\":\"kluris\",\"owner\":\"ngvoicu\",\"release_type\":\"github\"},\"version\":\"3.0.0\"}\n";

    #[test]
    fn reads_dists_receipt() {
        let r = Receipt::parse(DIST).unwrap();
        assert_eq!(r.install_prefix, PathBuf::from("/home/u/.kluris/bin"));
        assert_eq!(r.binaries, vec!["kluris".to_string()]);
        assert_eq!(r.version, "3.0.0");
        assert!(r.modify_path);
    }

    #[test]
    fn rewriting_changes_only_the_version() {
        let r = Receipt::parse(DIST).unwrap();
        assert_eq!(
            r.text_with_version("3.1.0"),
            DIST.replace("\"version\":\"3.0.0\"", "\"version\":\"3.1.0\"")
        );
        // kluris-installer.ps1's receipt, as dist 0.33.1 wrote it on zeewin2
        // (2026-10-26): `kluris.exe`, the prefix in backslashes, CRLF, no BOM.
        let windows = DIST
            .replace("[\"kluris\"]", "[\"kluris.exe\"]")
            .replace("/home/u/.kluris/bin", "C:\\\\Users\\\\u\\\\.kluris\\\\bin")
            .replace('\n', "\r\n");
        let r = Receipt::parse(&windows).unwrap();
        assert_eq!(
            r.install_prefix,
            PathBuf::from("C:\\Users\\u\\.kluris\\bin")
        );
        assert_eq!(r.binaries, vec!["kluris.exe".to_string()]);
        assert_eq!(
            r.text_with_version("3.1.0"),
            windows.replace("\"version\":\"3.0.0\"", "\"version\":\"3.1.0\"")
        );
    }

    // T-111 finding 3: only a receipt that is dist's, for Kluris, says what
    // may be removed or replaced.
    #[test]
    fn dists_receipt_is_klurises_on_its_platform() {
        let unix = Receipt::parse(DIST).unwrap();
        assert_eq!(unix.check_kluris(false), Ok(()));
        // The Unix receipt names `kluris`, which no Windows install has.
        assert!(unix.check_kluris(true).is_err());
        let windows = Receipt::parse(
            &DIST
                .replace("[\"kluris\"]", "[\"kluris.exe\"]")
                .replace("/home/u/.kluris/bin", "C:\\\\Users\\\\u\\\\.kluris\\\\bin")
                .replace('\n', "\r\n"),
        )
        .unwrap();
        assert_eq!(windows.check_kluris(true), Ok(()));
        assert!(windows.check_kluris(false).is_err());
    }

    #[test]
    fn a_receipt_that_is_not_dists_for_kluris_is_refused_with_why() {
        for (from, to, why) in [
            ("[\"kluris\"]", "[\"notes.txt\"]", "binaries"),
            ("[\"kluris\"]", "[\"kluris\",\"x\"]", "binaries"),
            ("\"cdylibs\":[]", "\"cdylibs\":[\"libx.so\"]", "cdylibs"),
            (
                "\"cstaticlibs\":[]",
                "\"cstaticlibs\":[\"libx.a\"]",
                "cstaticlibs",
            ),
            (
                "\"binary_aliases\":{}",
                "\"binary_aliases\":{\"k\":[\"x\"]}",
                "binary_aliases",
            ),
            ("\"flat\"", "\"hierarchical\"", "install_layout"),
            (
                "\"/home/u/.kluris/bin\"",
                "\"home/u/.kluris/bin\"",
                "install_prefix",
            ),
            ("\"cargo-dist\"", "\"homebrew\"", "provider"),
            ("\"app_name\":\"kluris\"", "\"app_name\":\"x\"", "source"),
            (
                "\"name\":\"kluris\",\"owner\"",
                "\"name\":\"x\",\"owner\"",
                "source",
            ),
            ("\"ngvoicu\"", "\"someone\"", "source"),
            (
                "\"release_type\":\"github\"",
                "\"release_type\":\"x\"",
                "source",
            ),
            (
                "\"modify_path\":true",
                "\"modify_path\":\"yes\"",
                "modify_path",
            ),
        ] {
            assert_eq!(DIST.matches(from).count(), 1, "{from}");
            let r = Receipt::parse(&DIST.replace(from, to)).unwrap();
            let err = r.check_kluris(false).unwrap_err();
            assert!(err.contains(why), "{to}: {err}");
        }
        for windows_prefix in ["C:\\\\x", "\\\\\\\\server\\\\share\\\\bin"] {
            let r = Receipt::parse(
                &DIST
                    .replace("[\"kluris\"]", "[\"kluris.exe\"]")
                    .replace("\"/home/u/.kluris/bin\"", &format!("\"{windows_prefix}\"")),
            )
            .unwrap();
            assert_eq!(r.check_kluris(true), Ok(()), "{windows_prefix}");
        }
        let relative = Receipt::parse(
            &DIST
                .replace("[\"kluris\"]", "[\"kluris.exe\"]")
                .replace("\"/home/u/.kluris/bin\"", "\"kluris\\\\bin\""),
        )
        .unwrap();
        assert!(relative.check_kluris(true).is_err());
    }

    #[test]
    fn not_a_receipt() {
        assert!(Receipt::parse("[]").is_err());
        assert!(Receipt::parse("{\"version\":\"1\"}").is_err());
        assert!(Receipt::parse("{\"install_prefix\":\"/x\"}").is_err());
        assert!(Receipt::parse("nope").is_err());
    }

    #[test]
    fn binaries_with_a_folder_part_are_ignored() {
        let r = Receipt::parse(
            "{\"binaries\":[\"kluris\",\"../x\",\"a/b\",\"c:\\\\d\"],\"install_prefix\":\"/p\",\"version\":\"1\"}",
        )
        .unwrap();
        assert_eq!(r.binaries, vec!["kluris".to_string()]);
    }
}
