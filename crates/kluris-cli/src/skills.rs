//! The agent skills every brain-registering command refreshes (v2's
//! `_do_install`, whose report these commands do not print), and the note
//! `create` and `register` print about the new skill.

use std::path::Path;

use kluris_core::agents::install::{InstallRequest, install_skills, read_brain_configs};
use kluris_core::config::read_global_config;

use crate::error::CliError;
use crate::output::human::{Painter, Style};
use crate::resolve::{config_path, copilot_home};

/// Re-render every registered brain's agent skills under `home`.
pub fn refresh_skills(home: &Path) -> Result<(), CliError> {
    let config = read_global_config(&config_path()?).map_err(|e| CliError::new(e.to_string()))?;
    let brains = read_brain_configs(&config);
    let copilot_home = copilot_home();
    install_skills(&InstallRequest {
        config: &config,
        brains: &brains,
        home,
        copilot_home: copilot_home.as_deref(),
    });
    Ok(())
}

/// v2's `_print_skill_usage`.
pub fn skill_usage(painter: Painter, name: &str) -> String {
    let skill = format!("kluris-{name}");
    let title = painter.paint(
        Style::Green,
        &painter.bold(&format!("Agent skill installed: {skill}")),
    );
    format!(
        "{title}\n  Claude/Codex/Gemini/etc: /{skill}\n  Hermes: /skill {skill}  (or start with \
         `hermes -s {skill}`)\n  Created a new Hermes profile later? Run `kluris doctor` to \
         refresh Kluris skills.\n"
    )
}
