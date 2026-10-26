//! stub
use std::path::{Path, PathBuf};
use indexmap::IndexMap;
use crate::config::{BrainConfig, ConfigError, GlobalConfig};
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestinationRole { Universal, Agent(&'static str), Deselected(&'static str), Migration }
#[derive(Debug, Clone)]
pub struct DestinationReport { pub path: PathBuf, pub role: DestinationRole, pub written: Vec<String>, pub removed: Vec<String>, pub failed: Vec<String> }
#[derive(Debug, Clone, Default)]
pub struct InstallReport { pub skills: Vec<String>, pub agents: Vec<String>, pub destinations: Vec<DestinationReport>, pub brain_errors: Vec<(String, String)> }
impl InstallReport {
    pub fn ok(&self) -> bool { false }
    pub fn total_written(&self) -> usize { 0 }
    pub fn failures(&self) -> Vec<(PathBuf, String)> { Vec::new() }
}
pub struct InstallRequest<'a> { pub config: &'a GlobalConfig, pub brains: &'a IndexMap<String, Result<BrainConfig, ConfigError>>, pub home: &'a Path, pub copilot_home: Option<&'a Path> }
pub fn read_brain_configs(_config: &GlobalConfig) -> IndexMap<String, Result<BrainConfig, ConfigError>> { IndexMap::new() }
pub fn install_skills(_req: &InstallRequest) -> InstallReport { InstallReport::default() }
