//! One run: the machine chosen, the run's turn in its queue waited for
//! ([`crate::ssh_ci::queue`]) and the machine asked at the head, the tarball
//! made, then sent and the bootstrap streamed back ([`crate::ssh_ci::session`]);
//! a bootstrap that refuses the run with 75 sends it back to its place in the
//! queue, to wait on. A run stopped here,
//! one past its bound and one whose connection was lost are ended there too
//! (the stop script), since Windows' SSH server does not end what a lost
//! session started.

use super::machine::{self, Machine};
use super::options::{self, Options};
use super::script::{self, Spec};
#[cfg(test)]
pub use crate::ssh_ci::outcome::answer;
use crate::ssh_ci::outcome::{REFUSED, STOPPED, say};
use crate::ssh_ci::queue::{Place, Turn, Waited, Who};
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
            let now = ctx.pace.now_ms();
            let (machine, why) = machine::choose(ctx.runner, ctx.queues.as_ref(), now);
            say(ctx.err, &why)?;
            machine
        }
    };
    // The run's place in the machine's queue, kept until the run is over: the
    // ticket goes with `turn` however the run ends.
    let who = Who::of(ctx.root, &ctx.command);
    let (mut turn, refused) = Turn::take(
        ctx.queues.as_ref(),
        Place::of(machine.name),
        &who,
        options.wait,
        ctx.pace,
        &mut |line| say(ctx.err, &format!("windows: {line}")),
    )?;
    if let Some(why) = refused {
        say(ctx.err, &format!("windows: {why}"))?;
        return Ok(REFUSED);
    }

    let id = ctx.id.clone();
    let channel = ctx.channel.clone();
    let spec = Spec {
        id: &id,
        channel: &channel,
        jobs: machine.jobs,
        parity: options.parity,
        parity_files: &parity_files,
        cargo_args: &options.cargo_args,
        installer_smoke: options.installer_smoke,
        wrapper: options.wrapper.is_some(),
        clippy: options.clippy,
        published: options.published.as_deref(),
        unpinned: options.unpinned,
    };
    let bootstrap = script::powershell_line(&script::bootstrap(&id))?;
    let stop = script::powershell_line(&script::stop_script(&id))?;
    let cleanup = script::powershell_line(&script::cleanup_script(&id))?;
    let scratch = tempfile::tempdir().map_err(|e| format!("no folder for the tarball: {e}"))?;
    let name = script::tarball_name(&id);
    let tarball = scratch.path().join(&name);
    let mut commit = None;
    loop {
        let waited = turn.wait(
            &mut || machine::look_ready(&machine, &mut *ctx.runner),
            &mut *ctx.pace,
            &mut |line| say(&mut *ctx.err, &format!("windows: {line}")),
        )?;
        match waited {
            Waited::Stopped => return Ok(STOPPED),
            Waited::Refused(why) => {
                say(ctx.err, &format!("windows: {why}"))?;
                return Ok(REFUSED);
            }
            Waited::Go(notes) => {
                for note in notes {
                    say(ctx.err, &format!("windows: {note}"))?;
                }
            }
        }
        // Packed once, when the run first goes: a refused run sends the same
        // tree again.
        let commit = match &commit {
            Some(commit) => commit,
            None => commit.insert((ctx.pack)(&tarball, &script::run_script(&spec))?),
        };
        let remote = Remote {
            task: "windows",
            name: machine.name,
            host: machine.host,
        };
        let run = Run {
            id: &id,
            tarball_name: &name,
            target: &name,
            commit,
            bootstrap: bootstrap.clone(),
            bound: script::bound(&spec),
            stop: &stop,
            cleanup: &cleanup,
        };
        let sent = session::go(ctx, remote, &tarball, run)?;
        match sent.refused {
            // The machine's own lock is the last word: the run waits on,
            // keeping its place, unless it was told not to wait.
            Some(why) if options.wait > 0 => turn.refused_there(why),
            _ => return Ok(sent.code),
        }
    }
}
