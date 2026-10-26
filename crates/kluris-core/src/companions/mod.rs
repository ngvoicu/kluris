//! The embedded Specmint companion playbooks (names only, for now).

/// Known companions, in canonical order (v2 `companions.KNOWN`).
pub const KNOWN: [&str; 4] = [
    "specmint-core",
    "specmint-tdd",
    "specmint-core-html",
    "specmint-tdd-html",
];

/// Known companion names, de-duplicated, in canonical order; unknown names
/// are dropped (v2 `companions.normalize`).
pub fn normalize<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    KNOWN
        .iter()
        .filter(|known| names.iter().any(|n| n.as_ref() == **known))
        .map(|known| known.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_filters_dedupes_and_orders() {
        let names = [
            "specmint-tdd-html",
            "bogus",
            "specmint-core",
            "specmint-core",
        ];
        assert_eq!(
            normalize(&names),
            vec!["specmint-core", "specmint-tdd-html"]
        );
    }

    #[test]
    fn normalize_empty() {
        assert!(normalize::<&str>(&[]).is_empty());
    }
}

// --- red-phase stubs (replaced by the implementation) ---
mod stubs {
    use std::borrow::Cow;
    use std::fmt;
    use std::path::{Path, PathBuf};

    use crate::config::{BrainConfig, ConfigError, GlobalConfig};
    use crate::resolve::ResolvedBrain;

    #[derive(Debug, Clone, Default)]
    pub struct CompanionTree {
        files: Vec<(String, Cow<'static, [u8]>)>,
    }
    impl CompanionTree {
        pub fn new() -> Self {
            Self::default()
        }
        pub fn file(mut self, rel: &str, bytes: Vec<u8>) -> Self {
            self.files.push((rel.into(), Cow::Owned(bytes)));
            self
        }
        pub fn dir(self, _rel: &str) -> Self {
            self
        }
        pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
            self.files.iter().map(|(r, b)| (r.as_str(), b.as_ref()))
        }
    }
    #[derive(Debug)]
    pub struct CompanionError(String);
    impl fmt::Display for CompanionError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }
    pub fn embedded(_name: &str) -> Option<CompanionTree> {
        None
    }
    pub fn installed_dir(_name: &str, home: &Path) -> PathBuf {
        home.to_path_buf()
    }
    pub fn is_installed(_name: &str, _home: &Path) -> bool {
        false
    }
    pub fn installed(_home: &Path) -> Vec<String> {
        Vec::new()
    }
    pub fn referenced(_config: &GlobalConfig) -> Vec<String> {
        Vec::new()
    }
    pub fn verify_runtime_dir(_path: &Path) -> Result<(), CompanionError> {
        Err(CompanionError("stub".into()))
    }
    pub fn install_tree(_n: &str, _t: &CompanionTree, _h: &Path) -> Result<(), CompanionError> {
        Err(CompanionError("stub".into()))
    }
    pub fn install(_n: &str, _h: &Path) -> Result<(), CompanionError> {
        Err(CompanionError("stub".into()))
    }
    pub fn refresh(_n: &str, _h: &Path) -> Result<(), CompanionError> {
        Err(CompanionError("stub".into()))
    }
    #[derive(Debug)]
    pub struct OptInBrain {
        pub name: String,
        pub path: PathBuf,
        pub config: BrainConfig,
    }
    pub fn load_brains(_b: &[ResolvedBrain]) -> Result<Vec<OptInBrain>, ConfigError> {
        Ok(Vec::new())
    }
    pub fn enabled(_b: &[OptInBrain]) -> Vec<String> {
        Vec::new()
    }
    pub fn add(_b: &[OptInBrain], _s: &[String], _h: &Path) -> Result<Vec<String>, CompanionError> {
        Ok(Vec::new())
    }
    pub fn remove(_b: &[OptInBrain], _s: &[String]) -> Result<Vec<String>, CompanionError> {
        Ok(Vec::new())
    }
}
pub use stubs::*;
