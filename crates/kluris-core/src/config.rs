//! The global config (`~/.kluris/config.yml`) and each brain's `kluris.yml`,
//! read and written exactly as v2's `core/config.py` does.
//!
//! Reading follows v2's pydantic models: `yaml.safe_load(text) or {}`, string
//! fields must be YAML strings, unknown keys are ignored (and so dropped by
//! the next write). Writing is `yaml.dump(model_dump(), sort_keys=False)`:
//! the [`Profile::Config`] emitter, in the models' field order.
//!
//! Two exceptions: a brain's `search: {language: ...}`, which v2 ignores, is
//! read for `kluris search` and written back after `companions` (PAR-021);
//! its `sync: {auto_fetch: ...}`, also ignored by v2, is read for the fresh
//! reads of `wake-up` and `search` and written back after `search` (PAR-072).

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::{fmt, fs, io};

use indexmap::IndexMap;

use crate::atomic;
use crate::companions;
use crate::yaml::{self, Map, Profile, Value};

/// The default `commands_for`: v2's `DEFAULT_AGENT_COMMANDS` in v2's order
/// with `devin` in place of `windsurf` (Windsurf became Devin), then
/// `antigravity`.
pub const DEFAULT_AGENT_COMMANDS: [&str; 10] = [
    "claude",
    "cursor",
    "devin",
    "copilot",
    "codex",
    "gemini",
    "hermes",
    "kilocode",
    "junie",
    "antigravity",
];

/// Earlier default lists: a `commands_for` with exactly one of these sets of
/// names reads as [`DEFAULT_AGENT_COMMANDS`], so upgraded brains get the new
/// agents (v2 did the same for its pre-Hermes list).
const LEGACY_DEFAULT_AGENT_COMMANDS: [&[&str]; 3] = [
    // v2 before Hermes (`_LEGACY_DEFAULT_AGENT_COMMANDS`).
    &[
        "claude", "cursor", "windsurf", "copilot", "codex", "gemini", "kilocode", "junie",
    ],
    // v2.28 (`DEFAULT_AGENT_COMMANDS`).
    &[
        "claude", "cursor", "windsurf", "copilot", "codex", "gemini", "hermes", "kilocode", "junie",
    ],
    // v2.28 with windsurf read as devin.
    &[
        "claude", "cursor", "devin", "copilot", "codex", "gemini", "hermes", "kilocode", "junie",
    ],
];

/// One registered brain (v2 `BrainEntry`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainEntry {
    /// Exactly as stored in config.yml (v2 stores `str(path)`); never
    /// re-resolved.
    pub path: String,
    pub description: String,
}

/// The global config (v2 `GlobalConfig`); brains in file order, which is
/// registration order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalConfig {
    pub brains: IndexMap<String, BrainEntry>,
}

/// Agent installation settings for a brain (v2 `AgentsConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsConfig {
    /// Agent names as written, except `windsurf`, which reads as `devin`.
    /// Unknown names are kept.
    pub commands_for: Vec<String>,
}

impl Default for AgentsConfig {
    fn default() -> AgentsConfig {
        AgentsConfig {
            commands_for: DEFAULT_AGENT_COMMANDS.map(String::from).to_vec(),
        }
    }
}

/// A brain's local `kluris.yml` (v2 `BrainConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrainConfig {
    pub name: String,
    pub description: String,
    pub agents: AgentsConfig,
    /// As written; [`companions::normalize`] gives the known ones.
    pub companions: Vec<String>,
    /// Kluris 3's `search:` settings (v2 ignores the key).
    pub search: SearchConfig,
    /// Kluris 3.1's `sync:` settings (v2 ignores the key).
    pub sync: SyncConfig,
}

/// A brain's `search:` settings in `kluris.yml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchConfig {
    /// `search.language` as written (`en`, `ro`); `None` when absent or not
    /// a string, and then not written back.
    pub language: Option<String>,
}

/// A brain's `sync:` settings in `kluris.yml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncConfig {
    /// `sync.auto_fetch` as written (`false` turns the fresh reads off);
    /// `None` when absent or not a boolean, and then not written back.
    pub auto_fetch: Option<bool>,
}

impl BrainConfig {
    /// `BrainConfig(name=name)`: every other field at its default.
    pub fn new(name: impl Into<String>) -> BrainConfig {
        BrainConfig {
            name: name.into(),
            description: String::new(),
            agents: AgentsConfig::default(),
            companions: Vec::new(),
            search: SearchConfig::default(),
            sync: SyncConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigErrorKind {
    /// The file does not exist (only an error for a brain's `kluris.yml`).
    NotFound,
    /// Reading or writing failed.
    Io,
    /// The file is not YAML or does not validate.
    Invalid,
}

/// A config file that cannot be read, validated or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub path: PathBuf,
    pub kind: ConfigErrorKind,
    pub reason: String,
}

impl ConfigError {
    fn new(path: &Path, kind: ConfigErrorKind, reason: impl Into<String>) -> ConfigError {
        ConfigError {
            path: path.to_path_buf(),
            kind,
            reason: reason.into(),
        }
    }

    /// A failed read: a missing file is [`ConfigErrorKind::NotFound`].
    fn read(path: &Path, err: &io::Error) -> ConfigError {
        let kind = if err.kind() == io::ErrorKind::NotFound {
            ConfigErrorKind::NotFound
        } else {
            ConfigErrorKind::Io
        };
        ConfigError::new(path, kind, err.to_string())
    }

    /// A failed write (a missing directory included).
    fn write(path: &Path, err: &io::Error) -> ConfigError {
        ConfigError::new(path, ConfigErrorKind::Io, err.to_string())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            ConfigErrorKind::Invalid => "Invalid config file",
            ConfigErrorKind::NotFound | ConfigErrorKind::Io => "Cannot access config file",
        };
        write!(f, "{what} {}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for ConfigError {}

// --- paths ------------------------------------------------------------------

/// The home directory: `HOME` when set and non-empty, else the OS home
/// (`os_home`, e.g. `std::env::home_dir()`). One rule on every OS, also on
/// Windows, where v2 used `$HOME` for skills and `Path.home()` for the config.
pub fn home_dir(home: Option<&OsStr>, os_home: Option<PathBuf>) -> Option<PathBuf> {
    match home.filter(|h| !h.is_empty()) {
        Some(h) => Some(PathBuf::from(h)),
        None => os_home,
    }
}

/// The global config path: `KLURIS_CONFIG` verbatim when set and non-empty,
/// else `<home>/.kluris/config.yml` ([`home_dir`]). `None` when no home can
/// be found.
pub fn global_config_path(
    kluris_config: Option<&OsStr>,
    home: Option<&OsStr>,
    os_home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = kluris_config.filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(path));
    }
    Some(home_dir(home, os_home)?.join(".kluris").join("config.yml"))
}

// --- reading ----------------------------------------------------------------

/// Parse a YAML document the way v2's `yaml.safe_load(text) or {}` does
/// (YAML 1.1 types): any falsy document (empty, `null`, `false`, `0`, `""`,
/// `[]`) is an empty mapping; more than one document is an error.
fn load_mapping(text: &str) -> Result<Map, String> {
    let doc = yaml::load(text).map_err(|e| e.to_string())?;
    if doc.is_falsy() {
        return Ok(Map::new());
    }
    doc.into_map()
        .map_err(|_| "the top level is not a mapping".into())
}

/// A pydantic `str` field: only a YAML string validates.
fn string_field(map: &Map, key: &str, default: Option<&str>) -> Result<String, String> {
    match map.get(key) {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(_) => Err(format!("'{key}' must be a string")),
        None => default
            .map(str::to_string)
            .ok_or_else(|| format!("'{key}' is required")),
    }
}

/// A pydantic `list[str]` field; `None` when the key is missing.
fn string_list_field(map: &Map, key: &str) -> Result<Option<Vec<String>>, String> {
    let error = || format!("'{key}' must be a list of strings");
    match map.get(key) {
        None => Ok(None),
        Some(Value::Seq(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_string).ok_or_else(error))
            .collect::<Result<_, _>>()
            .map(Some),
        Some(_) => Err(error()),
    }
}

/// Parse the global config text (an empty document is an empty config).
pub fn parse_global_config(text: &str) -> Result<GlobalConfig, String> {
    let root = load_mapping(text)?;
    let mut brains = IndexMap::new();
    match root.get("brains") {
        None => {}
        Some(Value::Map(entries)) => {
            for (name, entry) in entries {
                let Value::Map(entry) = entry else {
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
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(GlobalConfig::default()),
        Err(e) => return Err(ConfigError::read(path, &e)),
    };
    parse_global_config(&text).map_err(|e| ConfigError::new(path, ConfigErrorKind::Invalid, e))
}

/// v2's `AgentsConfig` validation plus the v3 rename: an earlier default set
/// (compared as written, like v2's `frozenset`) becomes the default list;
/// any other list is kept, with each `windsurf` read as `devin`.
fn agent_commands(names: Vec<String>) -> Vec<String> {
    let written: BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let is_legacy = LEGACY_DEFAULT_AGENT_COMMANDS
        .iter()
        .any(|legacy| written == legacy.iter().copied().collect());
    if is_legacy {
        return AgentsConfig::default().commands_for;
    }
    names
        .into_iter()
        .map(|n| if n == "windsurf" { "devin".into() } else { n })
        .collect()
}

/// Parse a brain's `kluris.yml` text the way v2's `BrainConfig` model does.
pub fn parse_brain_config(text: &str) -> Result<BrainConfig, String> {
    let root = load_mapping(text)?;
    let name = string_field(&root, "name", None)?;
    let description = string_field(&root, "description", Some(""))?;
    let agents = match root.get("agents") {
        None => AgentsConfig::default(),
        Some(Value::Map(agents)) => match string_list_field(agents, "commands_for")? {
            None => AgentsConfig::default(),
            Some(names) => AgentsConfig {
                commands_for: agent_commands(names),
            },
        },
        Some(_) => return Err("'agents' must be a mapping".into()),
    };
    let companions = string_list_field(&root, "companions")?.unwrap_or_default();
    // Never an error: v2 ignores the key, so a brain v2 reads must read here.
    let language = match root.get("search") {
        Some(Value::Map(search)) => match search.get("language") {
            Some(Value::Str(language)) => Some(language.clone()),
            _ => None,
        },
        _ => None,
    };
    let auto_fetch = match root.get("sync") {
        Some(Value::Map(sync)) => match sync.get("auto_fetch") {
            Some(Value::Bool(on)) => Some(*on),
            _ => None,
        },
        _ => None,
    };
    Ok(BrainConfig {
        name,
        description,
        agents,
        companions,
        search: SearchConfig { language },
        sync: SyncConfig { auto_fetch },
    })
}

/// The `kluris.yml` path of the brain at `brain_path`.
fn brain_config_file(brain_path: &Path) -> PathBuf {
    brain_path.join("kluris.yml")
}

/// Read the `kluris.yml` of the brain at `brain_path`; a missing file is a
/// [`ConfigErrorKind::NotFound`] error (v2 raises).
pub fn read_brain_config(brain_path: &Path) -> Result<BrainConfig, ConfigError> {
    let path = brain_config_file(brain_path);
    let text = fs::read_to_string(&path).map_err(|e| ConfigError::read(&path, &e))?;
    parse_brain_config(&text).map_err(|e| ConfigError::new(&path, ConfigErrorKind::Invalid, e))
}

/// Companions from a brain's `kluris.yml` text, normalized; `None` when the
/// text does not validate as a brain config (v2 `_read_companion_state`).
pub fn parse_brain_companions(text: &str) -> Option<Vec<String>> {
    parse_brain_config(text)
        .ok()
        .map(|cfg| companions::normalize(&cfg.companions))
}

/// Companions of the brain at `brain_path`; `None` when its `kluris.yml` is
/// missing, unreadable or invalid.
pub fn brain_companions(brain_path: &Path) -> Option<Vec<String>> {
    read_brain_config(brain_path)
        .ok()
        .map(|cfg| companions::normalize(&cfg.companions))
}

// --- writing ----------------------------------------------------------------

fn str_value(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn str_seq(items: &[String]) -> Value {
    Value::Seq(items.iter().map(|s| str_value(s)).collect())
}

/// The global config as v2 writes it (`model_dump` order: brains, then each
/// entry's `path` and `description`).
pub fn global_config_to_yaml(config: &GlobalConfig) -> String {
    let brains = config
        .brains
        .iter()
        .map(|(name, entry)| {
            let fields = Map::from([
                ("path".to_string(), str_value(&entry.path)),
                ("description".to_string(), str_value(&entry.description)),
            ]);
            (name.clone(), Value::Map(fields))
        })
        .collect();
    let root = Map::from([("brains".to_string(), Value::Map(brains))]);
    yaml::emit(&root, Profile::Config)
}

/// A brain's `kluris.yml` as v2 writes it (`model_dump` order: name,
/// description, agents.commands_for, companions), then `search` when it has
/// a language and `sync` when it has `auto_fetch`.
pub fn brain_config_to_yaml(config: &BrainConfig) -> String {
    let agents = Map::from([(
        "commands_for".to_string(),
        str_seq(&config.agents.commands_for),
    )]);
    let mut root = Map::from([
        ("name".to_string(), str_value(&config.name)),
        ("description".to_string(), str_value(&config.description)),
        ("agents".to_string(), Value::Map(agents)),
        ("companions".to_string(), str_seq(&config.companions)),
    ]);
    if let Some(language) = &config.search.language {
        let search = Map::from([("language".to_string(), str_value(language))]);
        root.insert("search".to_string(), Value::Map(search));
    }
    if let Some(auto_fetch) = config.sync.auto_fetch {
        let sync = Map::from([("auto_fetch".to_string(), Value::Bool(auto_fetch))]);
        root.insert("sync".to_string(), Value::Map(sync));
    }
    yaml::emit(&root, Profile::Config)
}

/// Write the global config, creating its directory (`~/.kluris/`) when
/// missing.
pub fn write_global_config(path: &Path, config: &GlobalConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| ConfigError::write(path, &e))?;
    }
    write_atomic(path, &global_config_to_yaml(config)).map_err(|e| ConfigError::write(path, &e))
}

/// Write the `kluris.yml` of the brain at `brain_path` (which must exist).
pub fn write_brain_config(brain_path: &Path, config: &BrainConfig) -> Result<(), ConfigError> {
    let path = brain_config_file(brain_path);
    write_atomic(&path, &brain_config_to_yaml(config)).map_err(|e| ConfigError::write(&path, &e))
}

/// Add (or replace, keeping its position like a Python dict) a brain in the
/// global config at `path`.
pub fn register_brain(path: &Path, name: &str, entry: BrainEntry) -> Result<(), ConfigError> {
    let mut config = read_global_config(path)?;
    config.brains.insert(name.to_string(), entry);
    write_global_config(path, &config)
}

/// Remove a brain from the global config at `path`, keeping the others'
/// order; an unknown name still rewrites the file, as v2 does.
pub fn unregister_brain(path: &Path, name: &str) -> Result<(), ConfigError> {
    let mut config = read_global_config(path)?;
    config.brains.shift_remove(name);
    write_global_config(path, &config)
}

/// `text` (LF) with the line endings of the file it replaces: CRLF when that
/// file's first line break is CRLF, else LF; a new file is LF (decision #3).
fn with_endings_of(target: &Path, text: &str) -> String {
    let existing = fs::read(target).unwrap_or_default();
    let crlf = match existing.iter().position(|&b| b == b'\n') {
        Some(i) => i > 0 && existing[i - 1] == b'\r',
        None => false,
    };
    if crlf {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// Replace `path` with `text` through [`atomic::write_atomic`] (a symlink
/// is followed and kept, permissions are kept, the old file is left as it was
/// on failure), keeping the old file's line endings as v2's in-place
/// `write_text` keeps the file.
fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let target = atomic::resolve_link_target(path)?;
    atomic::write_atomic(path, with_endings_of(&target, text).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn falsy_documents_are_an_empty_config() {
        for text in ["false\n", "0\n", "''\n", "[]\n", "null\n", "~\n"] {
            assert_eq!(
                parse_global_config(text),
                Ok(GlobalConfig::default()),
                "{text:?}"
            );
        }
    }

    #[test]
    fn yaml_1_1_scalars_are_not_strings() {
        for text in [
            "brains:\n  a:\n    path: /x\n    description: yes\n",
            "brains:\n  a:\n    path: 2026-01-02\n",
            "brains:\n  a:\n    path: 0755\n",
            "brains:\n  1:\n    path: /x\n",
        ] {
            assert!(parse_global_config(text).is_err(), "should reject {text:?}");
        }
        for text in [
            "name: yes\n",
            "name: b\ncompanions: [on]\n",
            "name: b\ndescription: 2026-01-02\n",
        ] {
            assert_eq!(
                parse_brain_companions(text),
                None,
                "should be unknown: {text:?}"
            );
        }
    }

    #[test]
    fn legacy_set_is_compared_as_a_set() {
        for legacy in LEGACY_DEFAULT_AGENT_COMMANDS {
            let mut names: Vec<String> = legacy.iter().map(|s| s.to_string()).collect();
            names.push("claude".into());
            assert_eq!(agent_commands(names), AgentsConfig::default().commands_for);
            let names: Vec<String> = legacy[1..].iter().map(|s| s.to_string()).collect();
            assert_eq!(agent_commands(names).len(), legacy.len() - 1);
        }
    }
}
