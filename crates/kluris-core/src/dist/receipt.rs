//! dist's install receipt: one JSON line,
//! `{"binaries":["kluris"],...,"install_prefix":"<dir>","modify_path":true,
//! ...,"version":"3.0.0"}` (the shell installer ends it with `\n`, the
//! PowerShell one with `\r\n`).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

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
        }
    }
}

impl std::error::Error for ReceiptError {}

impl Receipt {
    /// Read the receipt at `path`.
    pub fn read(path: &Path) -> Result<Receipt, ReceiptError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ReceiptError::Missing(path.to_path_buf()));
            }
            Err(e) => return Err(ReceiptError::Unreadable(path.to_path_buf(), e.to_string())),
        };
        Receipt::parse(&text).map_err(|e| ReceiptError::Invalid(path.to_path_buf(), e))
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
