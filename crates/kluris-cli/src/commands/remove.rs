//! `kluris remove BRAIN_NAME [--json]` (research-03 §1.12): unregister a
//! brain and refresh the skills; its files stay.

use kluris_core::register::remove;

use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::resolve::{config_path, home};
use crate::skills::{print_failures, refresh_skills, with_problems};

pub fn run(name: &str, as_json: bool) -> Result<(), CliError> {
    remove(&config_path()?, name).map_err(|e| CliError::new(e.to_string()))?;
    let home = home()?;
    let skills = refresh_skills(&home)?;
    print_failures(&skills, &home);
    if as_json {
        let doc = Json::Object(vec![
            ("ok".into(), Json::Bool(true)),
            ("name".into(), Json::Str(name.to_string())),
        ]);
        human::print(&json::to_line(&with_problems(doc, &skills)));
    } else {
        human::print(&format!("Unregistered: {name} (files preserved)\n"));
    }
    Ok(())
}
