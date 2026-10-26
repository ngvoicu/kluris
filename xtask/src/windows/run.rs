//! One run: the machine chosen and asked, the tarball made, then sent and the
//! bootstrap streamed back ([`crate::ssh_ci::session`]). A run stopped here,
//! one past its bound and one whose connection was lost are ended there too
//! (the stop script), since Windows' SSH server does not end what a lost
//! session started.

use super::machine::{self, Machine, Ready};
use super::options::{self, Options};
use super::script::{self, Spec};
#[cfg(test)]
pub use crate::ssh_ci::outcome::answer;
use crate::ssh_ci::outcome::{REFUSED, say};
pub use crate::ssh_ci::session::Ctx;
use crate::ssh_ci::session::{self, Remote, Run};

/// Runs `options` and answers the status this command ends with.
pub fn windows(options: &Options, ctx: &mut Ctx) -> i32 {
    match go(options, ctx) {
        Ok(code) => code,
        Err(why) => {
            let _ = writeln!(ctx.err, "windows: {why}");
            1
        }
    }
}

fn go(options: &Options, ctx: &mut Ctx) -> Result<i32, String> {
    let parity_files = options::checked_parity_files(ctx.root, &options.parity_files)?;
    let machine = match options.machine {
        Some(name) => Machine::named(name),
        None => {
            let (machine, why) = machine::choose(ctx.runner);
            say(ctx.err, &why)?;
            machine
        }
    };
    match machine::ready(&machine, ctx.runner)? {
        Ready::Refused(why) => {
            say(ctx.err, &format!("windows: {why}"))?;
            return Ok(REFUSED);
        }
        Ready::Go(notes) => {
            for note in notes {
                say(ctx.err, &format!("windows: {note}"))?;
            }
        }
    }

    let id = ctx.id.clone();
    let spec = Spec {
        id: &id,
        channel: &ctx.channel,
        jobs: machine.jobs,
        parity: options.parity,
        parity_files: &parity_files,
        cargo_args: &options.cargo_args,
        installer_smoke: options.installer_smoke,
        wrapper: options.wrapper.is_some(),
        clippy: options.clippy,
        published: options.published.as_deref(),
    };
    let bootstrap = script::powershell_line(&script::bootstrap(&id))?;
    let stop = script::powershell_line(&script::stop_script(&id))?;
    let cleanup = script::powershell_line(&script::cleanup_script(&id))?;
    let scratch = tempfile::tempdir().map_err(|e| format!("no folder for the tarball: {e}"))?;
    let name = script::tarball_name(&id);
    let tarball = scratch.path().join(&name);
    let commit = (ctx.pack)(&tarball, &script::run_script(&spec))?;
    let remote = Remote {
        task: "windows",
        name: machine.name,
        host: machine.host,
    };
    let run = Run {
        id: &id,
        tarball_name: &name,
        target: &name,
        commit: &commit,
        bootstrap,
        bound: script::bound(&spec),
        stop: &stop,
        cleanup: &cleanup,
    };
    session::go(ctx, remote, &tarball, run)
}
