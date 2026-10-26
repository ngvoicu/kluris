//! `kluris remove BRAIN_NAME [--json]` (research-03 §1.12): unregister a
//! brain, stop and remove its sync service, and refresh the skills; its
//! files stay.

use kluris_core::register::remove;
use kluris_core::service;

use crate::error::CliError;
use crate::output::human;
use crate::output::json::{self, Json};
use crate::platform::service::{SystemRunner, env as service_env};
use crate::resolve::{config_path, home};
use crate::skills::{print_failures, refresh_skills, with_problems};

/// What became of the brain's sync service.
enum Service {
    None,
    Removed(String),
    /// It would not stop: why, with what to run.
    Failed(String),
}

/// Stop and remove the brain's sync service (`kluris sync
/// --uninstall-service`); like the skills, a failure never fails the remove.
fn remove_service(name: &str) -> Service {
    let hint = format!("run `kluris sync --uninstall-service --brain {name}`");
    let env = match service_env() {
        Ok(env) => env,
        Err(e) => return Service::Failed(format!("{}; {hint}", e.message())),
    };
    match service::remove_brain_service(&env, name, &SystemRunner) {
        Ok(None) => Service::None,
        Ok(Some(service)) => Service::Removed(service),
        Err(e) => Service::Failed(format!(
            "could not stop the sync service {}: {e}; {hint}",
            env.name(name)
        )),
    }
}

pub fn run(name: &str, as_json: bool) -> Result<(), CliError> {
    remove(&config_path()?, name).map_err(|e| CliError::new(e.to_string()))?;
    let service = remove_service(name);
    let home = home()?;
    let skills = refresh_skills(&home)?;
    print_failures(&skills, &home);
    if as_json {
        let mut fields = vec![
            ("ok".into(), Json::Bool(true)),
            ("name".into(), Json::Str(name.to_string())),
        ];
        match &service {
            Service::None => {}
            Service::Removed(s) => fields.push(("service".into(), Json::Str(s.clone()))),
            Service::Failed(why) => fields.push(("service_failed".into(), Json::Str(why.clone()))),
        }
        human::print(&json::to_line(&with_problems(
            Json::Object(fields),
            &skills,
        )));
    } else {
        let mut out = String::new();
        if let Service::Removed(s) = &service {
            out.push_str(&format!("Stopped and removed the sync service {s}.\n"));
        }
        out.push_str(&format!("Unregistered: {name} (files preserved)\n"));
        human::print(&out);
        if let Service::Failed(why) = &service {
            eprintln!("Warning: {why}");
        }
    }
    Ok(())
}
