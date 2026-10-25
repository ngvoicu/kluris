//! Minimal config reading for `kluris list`: the global config
//! (`~/.kluris/config.yml`) and a brain's `kluris.yml` companions.
//!
//! Validation follows v2's pydantic models: string fields must be YAML strings,
//! unknown keys are ignored, `path` is required.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use indexmap::IndexMap;
use yaml_rust2::yaml::Hash;
use yaml_rust2::{Yaml, YamlLoader};

use crate::companions;

/// One registered brain (v2 `BrainEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainEntry {
    /// Exactly as stored in config.yml; never re-resolved.
    pub path: String,
    pub description: String,
}

/// The global config (v2 `GlobalConfig`); brains in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalConfig {
    pub brains: IndexMap<String, BrainEntry>,
}

/// A config file that exists but cannot be read or does not validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub reason: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid config file {}: {}",
            self.path.display(),
            self.reason
        )
    }
}

impl std::error::Error for ConfigError {}

/// The global config path: `KLURIS_CONFIG` when set and non-empty, else
/// `<home>/.kluris/config.yml` where home is `HOME` when set and non-empty,
/// else the OS home. `None` when no home can be found.
pub fn global_config_path(
    kluris_config: Option<&OsStr>,
    home: Option<&OsStr>,
    os_home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = kluris_config.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    let home = match home.filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => os_home?,
    };
    Some(home.join(".kluris").join("config.yml"))
}

/// Parse a YAML document the way v2's `yaml.safe_load(text) or {}` does:
/// an empty (or null) document is an empty mapping; more than one document
/// is an error.
fn load_mapping(text: &str) -> Result<Hash, String> {
    let mut docs = YamlLoader::load_from_str(text).map_err(|e| e.to_string())?;
    if docs.len() > 1 {
        return Err("expected a single YAML document".into());
    }
    match docs.pop() {
        None | Some(Yaml::Null) => Ok(Hash::new()),
        Some(Yaml::Hash(map)) => Ok(map),
        Some(_) => Err("the top level is not a mapping".into()),
    }
}

fn get<'a>(map: &'a Hash, key: &str) -> Option<&'a Yaml> {
    map.get(&Yaml::String(key.to_string()))
}

/// A pydantic `str` field: only a YAML string validates.
fn string_field(map: &Hash, key: &str, default: Option<&str>) -> Result<String, String> {
    match get(map, key) {
        Some(Yaml::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("'{key}' must be a string")),
        None => default
            .map(str::to_string)
            .ok_or_else(|| format!("'{key}' is required")),
    }
}

/// A pydantic `list[str]` field with a default of `[]`.
fn string_list_field(map: &Hash, key: &str) -> Result<Vec<String>, String> {
    match get(map, key) {
        None => Ok(Vec::new()),
        Some(Yaml::Array(items)) => items
            .iter()
            .map(|item| match item {
                Yaml::String(s) => Ok(s.clone()),
                _ => Err(format!("'{key}' must be a list of strings")),
            })
            .collect(),
        Some(_) => Err(format!("'{key}' must be a list of strings")),
    }
}

/// Parse the global config text (an empty document is an empty config).
pub fn parse_global_config(text: &str) -> Result<GlobalConfig, String> {
    let root = load_mapping(text)?;
    let mut brains = IndexMap::new();
    match get(&root, "brains") {
        None => {}
        Some(Yaml::Hash(entries)) => {
            for (name, entry) in entries {
                let Yaml::String(name) = name else {
                    return Err("brain names must be strings".into());
                };
                let Yaml::Hash(entry) = entry else {
                    return Err(format!("brain '{name}' must be a mapping"));
                };
                let path = string_field(entry, "path", None)
                    .map_err(|e| format!("brain '{name}': {e}"))?;
                let description = string_field(entry, "description", Some(""))
                    .map_err(|e| format!("brain '{name}': {e}"))?;
                brains.insert(name.clone(), BrainEntry { path, description });
            }
        }
        Some(_) => return Err("'brains' must be a mapping".into()),
    }
    Ok(GlobalConfig { brains })
}

/// Read the global config; a missing file is an empty config.
pub fn read_global_config(path: &Path) -> Result<GlobalConfig, ConfigError> {
    let error = |reason: String| ConfigError {
        path: path.to_path_buf(),
        reason,
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(GlobalConfig::default()),
        Err(e) => return Err(error(e.to_string())),
    };
    parse_global_config(&text).map_err(error)
}

/// Validate a brain config the way v2's `BrainConfig` model does and return
/// its `companions` list as written.
fn brain_config_companions(text: &str) -> Result<Vec<String>, String> {
    let root = load_mapping(text)?;
    string_field(&root, "name", None)?;
    string_field(&root, "description", Some(""))?;
    match get(&root, "agents") {
        None => {}
        Some(Yaml::Hash(agents)) => {
            string_list_field(agents, "commands_for")?;
        }
        Some(_) => return Err("'agents' must be a mapping".into()),
    }
    string_list_field(&root, "companions")
}

/// Companions from a brain's `kluris.yml` text, normalized; `None` when the
/// text does not validate as a brain config (v2 `_read_companion_state`).
pub fn parse_brain_companions(text: &str) -> Option<Vec<String>> {
    brain_config_companions(text)
        .ok()
        .map(|names| companions::normalize(&names))
}

/// Companions of the brain at `brain_path`; `None` when its `kluris.yml` is
/// missing, unreadable or invalid.
pub fn brain_companions(brain_path: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(brain_path.join("kluris.yml")).ok()?;
    parse_brain_companions(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn config_path_prefers_kluris_config() {
        let got = global_config_path(
            Some(OsStr::new("/custom/c.yml")),
            Some(OsStr::new("/h")),
            Some(p("/os")),
        );
        assert_eq!(got, Some(p("/custom/c.yml")));
    }

    #[test]
    fn config_path_ignores_empty_kluris_config_and_uses_home() {
        let got = global_config_path(Some(OsStr::new("")), Some(OsStr::new("/h")), Some(p("/os")));
        assert_eq!(got, Some(p("/h").join(".kluris").join("config.yml")));
    }

    #[test]
    fn config_path_falls_back_to_os_home() {
        let got = global_config_path(None, Some(OsStr::new("")), Some(p("/os")));
        assert_eq!(got, Some(p("/os").join(".kluris").join("config.yml")));
        assert_eq!(global_config_path(None, None, None), None);
    }

    #[test]
    fn empty_global_config_has_no_brains() {
        assert_eq!(parse_global_config(""), Ok(GlobalConfig::default()));
        assert_eq!(
            parse_global_config("# nothing\n"),
            Ok(GlobalConfig::default())
        );
        assert_eq!(
            parse_global_config("brains: {}\n"),
            Ok(GlobalConfig::default())
        );
    }

    #[test]
    fn global_config_keeps_order_defaults_description_ignores_extras() {
        let text = "brains:\n  zed:\n    path: /b/zed\n    description: last\n    type: legacy\n  alpha:\n    path: /b/alpha\nrepo: x\n";
        let cfg = parse_global_config(text).unwrap();
        let names: Vec<&str> = cfg.brains.keys().map(String::as_str).collect();
        assert_eq!(names, ["zed", "alpha"]);
        assert_eq!(
            cfg.brains["zed"],
            BrainEntry {
                path: "/b/zed".into(),
                description: "last".into()
            }
        );
        assert_eq!(cfg.brains["alpha"].description, "");
    }

    #[test]
    fn global_config_rejects_invalid_shapes() {
        for text in [
            "brains: null\n",
            "brains: []\n",
            "brains:\n  a:\n    description: no path\n",
            "brains:\n  a:\n    path: 12\n",
            "brains:\n  a:\n    path: /x\n    description: [1]\n",
            "- a list\n",
            "brains: [unclosed\n",
        ] {
            assert!(parse_global_config(text).is_err(), "should reject {text:?}");
        }
    }

    #[test]
    fn read_global_config_missing_file_is_empty() {
        let dir = std::env::temp_dir().join("kluris-core-missing-config-test");
        assert_eq!(
            read_global_config(&dir.join("nope.yml")),
            Ok(GlobalConfig::default())
        );
    }

    #[test]
    fn brain_companions_are_normalized() {
        let text =
            "name: b\ncompanions:\n- specmint-tdd\n- bogus\n- specmint-core\n- specmint-core\n";
        assert_eq!(
            parse_brain_companions(text),
            Some(vec![
                "specmint-core".to_string(),
                "specmint-tdd".to_string()
            ])
        );
    }

    #[test]
    fn brain_companions_default_to_empty() {
        let text = "name: b\ndescription: d\nagents:\n  commands_for:\n  - claude\n";
        assert_eq!(parse_brain_companions(text), Some(vec![]));
    }

    #[test]
    fn invalid_brain_config_is_unknown() {
        for text in [
            "",
            "description: no name\n",
            "name: 5\n",
            "name: b\ncompanions: null\n",
            "name: b\ncompanions: [1]\n",
            "name: b\nagents: null\n",
            "name: b\nagents:\n  commands_for: [1]\n",
            "name: b\ndescription: [x]\n",
            "name: [unclosed\n",
        ] {
            assert_eq!(
                parse_brain_companions(text),
                None,
                "should be unknown: {text:?}"
            );
        }
    }
}
