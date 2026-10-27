//! One run on zeelin: its turn waited for in zeewin2's queue (the same
//! laptop, [`crate::ssh_ci::queue`]), the probe at the head (zeelin running,
//! and none of the three locks held), the tarball made, then sent and the
//! bootstrap streamed back ([`crate::ssh_ci::session`]); a bootstrap that
//! refuses the run with 75 sends it back to its place in the queue. A run stopped here, one past its bound and one
//! whose connection was lost are ended there too (the stop script): sshd does
//! not end a session without a terminal when its connection goes.

use std::time::Duration;

use super::options::Options;
use super::script::{self, HOST, JOBS, Layout, Spec, TASK_LIMIT};
use crate::ssh_ci::outcome::{REFUSED, STOPPED, exit_said, say};
use crate::ssh_ci::plan;
use crate::ssh_ci::queue::{Look, Place, Turn, Waited, Who};
use crate::ssh_ci::runner::{Call, Runner};
pub use crate::ssh_ci::session::Ctx;
use crate::ssh_ci::session::{self, Remote, Run};
use crate::ssh_ci::words::checked_parity_files;

/// The queue a Linux run waits in: zeewin2's, the same laptop.
const PLACE: Place<'static> = Place {
    queue: "zeewin2",
    machine: HOST,
};

/// How long zeelin is given to answer the probe: to connect, and in all.
const PROBE_CONNECT_S: u32 = 5;
const PROBE_BOUND: Duration = Duration::from_secs(30);

/// Runs `options` and answers the status this command ends with.
pub fn linux(options: &Options, ctx: &mut Ctx) -> i32 {
    match go(options, ctx) {
        Ok(code) => code,
        Err(why) => {
            let _ = writeln!(ctx.err, "linux: {why}");
            1
        }
    }
}

fn go(options: &Options, ctx: &mut Ctx) -> Result<i32, String> {
    let parity_files = checked_parity_files(ctx.root, &options.parity_files)?;
    let layout = Layout::zeelin();
    // zeelin is the laptop zeewin2 is: a Linux run waits in zeewin2's queue,
    // and ConsensFlow's and Kluris's zeewin2 runs wait behind it. The ticket
    // goes with `turn` however the run ends.
    let who = Who::of(ctx.root, &ctx.command);
    let (mut turn, refused) = Turn::take(
        ctx.queues.as_ref(),
        PLACE,
        &who,
        options.wait,
        ctx.pace,
        &mut |line| say(ctx.err, &format!("linux: {line}")),
    )?;
    if let Some(why) = refused {
        say(ctx.err, &format!("linux: {why}"))?;
        return Ok(REFUSED);
    }

    let id = ctx.id.clone();
    let channel = ctx.channel.clone();
    let spec = Spec {
        channel: &channel,
        jobs: JOBS,
        tasks: TASK_LIMIT,
        parity: options.parity,
        parity_files: &parity_files,
        cargo_args: &options.cargo_args,
        installer_smoke: options.installer_smoke,
        wrapper: options.wrapper.is_some(),
        clippy: options.clippy,
        published: options.published.as_deref(),
        unpinned: options.unpinned,
    };
    let scratch = tempfile::tempdir().map_err(|e| format!("no folder for the tarball: {e}"))?;
    let name = plan::tarball_name(&id);
    let tarball = scratch.path().join(&name);
    let target = script::remote_tarball(&id);
    let stop = script::stop_line(&id);
    let cleanup = script::cleanup_line(&id);
    let mut commit = None;
    loop {
        let waited = turn.wait(
            &mut || look(&mut *ctx.runner, &layout),
            &mut *ctx.pace,
            &mut |line| say(&mut *ctx.err, &format!("linux: {line}")),
        )?;
        match waited {
            Waited::Stopped => return Ok(STOPPED),
            Waited::Refused(why) => {
                for line in why.lines() {
                    say(ctx.err, &format!("linux: {line}"))?;
                }
                return Ok(REFUSED);
            }
            Waited::Go(notes) => {
                for note in notes {
                    say(ctx.err, &format!("linux: {note}"))?;
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
            task: "linux",
            name: HOST,
            host: HOST,
        };
        let run = Run {
            id: &id,
            tarball_name: &name,
            target: &target,
            commit,
            bootstrap: script::bootstrap_line(&id, &layout),
            bound: script::bound(&spec),
            stop: &stop,
            cleanup: &cleanup,
        };
        let sent = session::go(ctx, remote, &tarball, run)?;
        match sent.refused {
            // The locks asked again under Kluris's own are the last word: the
            // run waits on, keeping its place, unless it was told not to wait.
            Some(why) if options.wait > 0 => turn.refused_there(why),
            _ => return Ok(sent.code),
        }
    }
}

/// Whether a run may go to zeelin now, as the run at the head of its queue
/// asks: the probe's refusal is a busy machine, waited for; zeelin not
/// answering is [`Look::Gone`] (the laptop's Ubuntu is not running: never
/// waited on); one that answered without saying is the error.
fn look(runner: &mut dyn Runner, layout: &Layout) -> Result<Look, String> {
    let args = vec![
        "-o".to_owned(),
        format!("ConnectTimeout={PROBE_CONNECT_S}"),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "ServerAliveInterval=5".to_owned(),
        "-o".to_owned(),
        "ServerAliveCountMax=1".to_owned(),
        HOST.to_owned(),
        script::probe_line(layout),
    ];
    let answer = runner.capture(&Call::new("ssh", args, PROBE_BOUND))?;
    if answer.timed_out {
        return Ok(Look::Gone(format!(
            "zeelin not running: no answer in {}s, so nothing was sent",
            PROBE_BOUND.as_secs()
        )));
    }
    let said: Vec<&str> = answer
        .stdout
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    match (said.last().copied().and_then(exit_said), answer.code) {
        (Some(0), Some(0)) => Ok(Look::Go(Vec::new())),
        (Some(REFUSED), _) => Ok(Look::Busy(said[..said.len() - 1].join("\n"))),
        (_, Some(255)) if not_running(&answer.stderr) => Ok(Look::Gone(format!(
            "zeelin not running: {}, so nothing was sent",
            answer.stderr.trim()
        ))),
        (_, code) => Err(format!(
            "zeelin did not say whether a run may go (ssh ended with {}: {}; it said {:?})",
            code.map_or_else(|| "no status".to_owned(), |code| code.to_string()),
            answer.stderr.trim(),
            answer.stdout
        )),
    }
}

/// Whether ssh could not reach zeelin at all: WSL stops Ubuntu (and its sshd)
/// when nothing keeps it running, and the laptop may be asleep or away.
fn not_running(stderr: &str) -> bool {
    stderr.contains("connect to host")
}
