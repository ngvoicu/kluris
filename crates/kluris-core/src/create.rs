//! `kluris create`: v2's wizard and command (research-03 §1.1, `cli.py:492-665`).
//!
//! In v2's order: the wizard asks for what the command line left out, the
//! name and target are checked, the brain is scaffolded ([`scaffold`]), the
//! companion picker may run, then git, the maps (v2's `_run_dream_on_brain`),
//! the first commit, and the registration. Differences from v2, each
//! allowlisted:
//! - the wizard's brain type is checked like `--type` (asked again);
//! - the caller runs the wizard only on a terminal (not with `--json` or
//!   `KLURIS_NO_PROMPT=1`); without one a missing name is an error;
//! - new names follow the stricter rule ([`scaffold::new_name_error`]);
//! - `--from-config` lobe names that are paths and nested values are refused;
//! - the brain is registered last, after the commit: a failure after the
//!   scaffold (git above all) is an error naming what is left on disk, and
//!   nothing is registered (v2 raised, and registered before committing);
//! - a target that exists as a file is an error (v2 raised).
//!
//! [`scaffold`]: crate::scaffold

use std::fmt;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::brain::sandbox::{self, SymlinkLoop};
use crate::brain::store::LocalGitStore;
use crate::companions::{self, prompt::Aborted};
use crate::config::{
    BrainConfig, BrainEntry, ConfigError, read_brain_config, read_global_config, register_brain,
    write_brain_config,
};
use crate::dream::{DreamOptions, regenerate_maps};
use crate::git::Git;
use crate::pyrepr::py_path_str;
use crate::resolve::prompt_line;
use crate::scaffold::{self, DEFAULT_BRAIN_TYPE, Lobe};
use crate::yaml::Date;

/// What the command line gave (`None`: not given).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub path: Option<String>,
    pub brain_type: Option<String>,
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub no_git: bool,
}

fn missing(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(str::is_empty)
}

impl CreateInput {
    /// v2's condition for the companion picker (with a terminal): any of the
    /// name, description, path, type or git choice left out.
    pub fn wants_companion_prompt(&self) -> bool {
        missing(&self.name)
            || missing(&self.description)
            || missing(&self.path)
            || self.brain_type.is_none()
            || (!self.no_git && self.remote.is_none())
    }
}

/// `click.prompt(text, default=default, type=str)`: `text [default]: `, an
/// empty answer is the default or, without one, asked again; the answer is
/// kept as typed.
pub fn prompt_text(
    input: &mut impl BufRead,
    out: &mut impl Write,
    text: &str,
    default: Option<&str>,
) -> Result<String, Aborted> {
    let prompt = match default {
        Some(default) => format!("{text} [{default}]: "),
        None => format!("{text}: "),
    };
    loop {
        let line = prompt_line(input, out, &prompt).ok_or(Aborted)?;
        if !line.is_empty() {
            return Ok(line);
        }
        if let Some(default) = default {
            return Ok(default.to_string());
        }
    }
}

/// v2's wizard: ask for each value `given` leaves out, then end with a blank
/// line. `cwd` is the default location. The brain type is checked like
/// `--type` (Click's `Choice` message, then asked again); v2 took any text.
pub fn wizard(
    mut given: CreateInput,
    cwd: &Path,
    input: &mut impl BufRead,
    out: &mut impl Write,
) -> Result<CreateInput, Aborted> {
    if missing(&given.name) {
        let _ = out.write_all(b"\nCreate a new brain\n\n");
        given.name = Some(prompt_text(
            input,
            out,
            "  Brain name (lowercase, hyphens ok)",
            None,
        )?);
    }
    if missing(&given.description) {
        let text = "  What does this brain cover? (one sentence)";
        given.description = Some(prompt_text(input, out, text, None)?);
    }
    if missing(&given.path) {
        let cwd = cwd.display().to_string();
        given.path = Some(prompt_text(input, out, "  Location", Some(&cwd))?);
    }
    if given.brain_type.is_none() {
        let names: Vec<&str> = scaffold::BRAIN_TYPES.iter().map(|(t, _)| *t).collect();
        let text = format!("  Brain type ({})", names.join(", "));
        loop {
            let answer = prompt_text(input, out, &text, Some(DEFAULT_BRAIN_TYPE))?;
            if scaffold::is_brain_type(&answer) {
                given.brain_type = Some(answer);
                break;
            }
            let _ = writeln!(out, "Error: {}", scaffold::not_a_brain_type(&answer));
        }
    }
    if !given.no_git && given.remote.is_none() {
        let text = "  Git setup (1=local, 2=with remote, 3=no git)";
        match prompt_text(input, out, text, Some("1"))?.as_str() {
            "3" => given.no_git = true,
            choice => {
                if choice == "2" {
                    given.remote = Some(prompt_text(input, out, "  Git remote URL", None)?);
                }
                if given.branch.is_none() {
                    given.branch = Some(prompt_text(input, out, "  Default branch", Some("main"))?);
                }
            }
        }
    }
    let _ = out.write_all(b"\n");
    let _ = out.flush();
    Ok(given)
}

/// What `create` works with, after the wizard.
#[derive(Debug, Clone, Copy)]
pub struct CreateRequest<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    /// `--path`, as given (relative to the working directory).
    pub path: Option<&'a str>,
    pub brain_type: &'a str,
    pub remote: Option<&'a str>,
    pub branch: &'a str,
    pub no_git: bool,
    /// `--from-config`, as given.
    pub from_config: Option<&'a str>,
    /// The global config file.
    pub config_path: &'a Path,
    /// The HOME companions install under.
    pub home: &'a Path,
    /// The local date, written into the new files.
    pub today: Date,
}

/// A brain `create` made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub name: String,
    /// As registered (`str(brain_path)`).
    pub path: String,
    pub brain_type: String,
    /// v2's count: the lobes of the brain type, whatever `--from-config` gave.
    pub lobes: usize,
    pub companions: Vec<String>,
    /// v2's `Warning: dream failed: ...` lines.
    pub warnings: Vec<String>,
}

/// Why `create` stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateError {
    /// Nothing was written; the message is v2's (or the decided fix's).
    Refused(String),
    Config(ConfigError),
    /// End of input at the companion picker (the scaffold is left, as in v2).
    Aborted,
    /// A step after the scaffold failed: the files are left at `path`, and
    /// the brain is not registered.
    Left {
        path: String,
        cause: String,
    },
}

impl fmt::Display for CreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CreateError::Refused(message) => f.write_str(message),
            CreateError::Config(e) => write!(f, "{e}"),
            CreateError::Aborted => f.write_str("Aborted!"),
            CreateError::Left { path, cause } => write!(
                f,
                "{cause} (the brain files are left in {path} and the brain is not registered: \
                 delete that directory, or fix the problem and run 'kluris register {path}')"
            ),
        }
    }
}

impl std::error::Error for CreateError {}

impl From<ConfigError> for CreateError {
    fn from(e: ConfigError) -> Self {
        CreateError::Config(e)
    }
}

fn resolve(path: &Path) -> Result<PathBuf, CreateError> {
    sandbox::resolve(path).map_err(|SymlinkLoop| {
        CreateError::Refused(format!("Symlink loop from '{}'", path.display()))
    })
}

/// v2's target checks (5-7): the brain's path, refused when `--path` is not
/// a usable directory, the target is inside a registered brain, or the
/// target is not an empty directory.
fn target_path(
    req: &CreateRequest,
    brains: &crate::config::GlobalConfig,
) -> Result<PathBuf, CreateError> {
    let brain_path = match req.path.filter(|p| !p.is_empty()) {
        Some(given) => {
            let base = resolve(Path::new(given))?;
            if !base.is_dir() {
                return Err(CreateError::Refused(format!(
                    "--path '{given}' is not a directory."
                )));
            }
            if base.join("kluris.yml").exists() {
                return Err(CreateError::Refused(format!(
                    "--path '{given}' is already a brain. Pass the parent directory, not the brain itself."
                )));
            }
            base.join(req.name)
        }
        None => resolve(Path::new(req.name))?,
    };
    let resolved = resolve(&brain_path)?;
    for entry in brains.brains.values() {
        let Ok(existing) = sandbox::resolve(Path::new(&py_path_str(&entry.path))) else {
            continue;
        };
        if sandbox::is_within(&resolved, &existing) {
            return Err(CreateError::Refused(format!(
                "Target path {} is inside existing brain '{}'. Create the brain outside of other brains.",
                brain_path.display(),
                entry.path
            )));
        }
    }
    if brain_path.exists() {
        if !brain_path.is_dir() {
            return Err(CreateError::Refused(format!(
                "{} already exists and is not a directory. \
                 Choose a different name or use --path to specify a location.",
                brain_path.display()
            )));
        }
        let not_empty = fs::read_dir(&brain_path).is_ok_and(|mut d| d.next().is_some());
        if not_empty {
            if brain_path.join("kluris.yml").exists() {
                return Err(CreateError::Refused(format!(
                    "{} already contains a kluris.yml. Use a different name.",
                    brain_path.display()
                )));
            }
            return Err(CreateError::Refused(format!(
                "{} already exists and is not empty. \
                 Choose a different name or use --path to specify a location.",
                brain_path.display()
            )));
        }
    }
    Ok(brain_path)
}

/// The lobes `--from-config` gives, else the brain type's.
fn lobes(req: &CreateRequest) -> Result<Vec<Lobe>, CreateError> {
    if let Some(file) = req.from_config.filter(|f| !f.is_empty()) {
        let text = fs::read_to_string(file)
            .map_err(|e| CreateError::Refused(format!("--from-config: cannot read {file}: {e}")))?;
        if let Some(lobes) =
            scaffold::structure_from_config(&text, file).map_err(CreateError::Refused)?
        {
            return Ok(lobes);
        }
    }
    Ok(scaffold::type_lobes(req.brain_type))
}

/// Create the brain: check it, scaffold it, ask for companions (`pick`, run
/// only when the caller wants v2's picker), set up git, write the maps,
/// commit, register. Installing the agent skills is the caller's last step.
pub fn create(
    req: &CreateRequest,
    git: &impl Git,
    pick: impl FnOnce() -> Result<Vec<String>, Aborted>,
) -> Result<Created, CreateError> {
    if let Some(message) = scaffold::new_name_error(req.name) {
        return Err(CreateError::Refused(message));
    }
    let config = read_global_config(req.config_path)?;
    if let Some(entry) = config.brains.get(req.name) {
        return Err(CreateError::Refused(format!(
            "A brain named '{}' is already registered at {}. Choose a different name.",
            req.name, entry.path
        )));
    }
    let brain_path = target_path(req, &config)?;
    let lobes = lobes(req)?;
    let description = match req.description.filter(|d| !d.is_empty()) {
        Some(d) => d.to_string(),
        None => format!("{} knowledge base", req.name),
    };
    let path = brain_path.display().to_string();
    let left = |cause: String| CreateError::Left {
        path: path.clone(),
        cause,
    };

    let mut brain_config = BrainConfig::new(req.name);
    brain_config.description = description.clone();
    scaffold::scaffold_brain(&brain_path, &brain_config, &lobes, req.today)
        .map_err(|e| left(e.to_string()))?;

    let selected = companions::normalize(&pick().map_err(|Aborted| CreateError::Aborted)?);
    if !selected.is_empty() {
        brain_config.companions = selected.clone();
        write_brain_config(&brain_path, &brain_config).map_err(|e| left(e.to_string()))?;
        for name in &selected {
            companions::install(name, req.home).map_err(|e| left(e.to_string()))?;
        }
    }

    if !req.no_git {
        git.init(&brain_path).map_err(|e| left(e.to_string()))?;
        if req.branch != "main" {
            git.checkout_new_branch(&brain_path, req.branch)
                .map_err(|e| left(e.to_string()))?;
        }
        if let Some(remote) = req.remote.filter(|r| !r.is_empty()) {
            git.remote_add(&brain_path, "origin", remote)
                .map_err(|e| left(e.to_string()))?;
        }
    }

    let warnings = regenerate(&brain_path, req.today);

    if !req.no_git {
        git.add(&brain_path, "-A")
            .map_err(|e| left(e.to_string()))?;
        git.commit(&brain_path, &format!("brain: initialize {}", req.name))
            .map_err(|e| left(e.to_string()))?;
    }

    let entry = BrainEntry {
        path: path.clone(),
        description,
    };
    register_brain(req.config_path, req.name, entry).map_err(|e| left(e.to_string()))?;

    Ok(Created {
        name: req.name.to_string(),
        path,
        brain_type: req.brain_type.to_string(),
        lobes: scaffold::type_lobes(req.brain_type).len(),
        companions: selected,
        warnings,
    })
}

/// v2's `_run_dream_on_brain` on the new brain; what fails is a warning.
fn regenerate(brain_path: &Path, today: Date) -> Vec<String> {
    let config = match read_brain_config(brain_path) {
        Ok(config) => config,
        Err(e) => return vec![format!("Warning: dream failed: {e}")],
    };
    let mut store = LocalGitStore::open(brain_path);
    let options = DreamOptions {
        name: &config.name,
        description: &config.description,
        today,
    };
    regenerate_maps(&mut store, &options)
        .into_iter()
        .map(|s| format!("Warning: dream failed: {}: {}", s.rel, s.reason))
        .collect()
}
