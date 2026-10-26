//! One run on zeelin: the probe (zeelin running, and none of the three locks
//! held), the tarball made, then sent and the bootstrap streamed back
//! ([`crate::ssh_ci::session`]). A run stopped here, one past its bound and one
//! whose connection was lost are ended there too (the stop script): sshd does
//! not end a session without a terminal when its connection goes.

use std::time::Duration;

use super::options::Options;
use super::script::{self, HOST, JOBS, Layout, Spec, TASK_LIMIT};
use crate::ssh_ci::outcome::{REFUSED, exit_said, say};
use crate::ssh_ci::plan;
use crate::ssh_ci::runner::Call;
pub use crate::ssh_ci::session::Ctx;
use crate::ssh_ci::session::{self, Remote, Run};
use crate::ssh_ci::words::checked_parity_files;

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
    if let Some(why) = refused(ctx, &layout)? {
        for line in why {
            say(ctx.err, &format!("linux: {line}"))?;
        }
        return Ok(REFUSED);
    }

    let id = plan::run_id();
    let spec = Spec {
        channel: &ctx.channel,
        jobs: JOBS,
        tasks: TASK_LIMIT,
        parity: options.parity,
        parity_files: &parity_files,
        cargo_args: &options.cargo_args,
        installer_smoke: options.installer_smoke,
        wrapper: options.wrapper.is_some(),
        clippy: options.clippy,
        published: options.published.as_deref(),
    };
    let scratch = tempfile::tempdir().map_err(|e| format!("no folder for the tarball: {e}"))?;
    let name = plan::tarball_name(&id);
    let tarball = scratch.path().join(&name);
    let commit = (ctx.pack)(&tarball, &script::run_script(&spec))?;
    let target = script::remote_tarball(&id);
    let stop = script::stop_line(&id);
    let cleanup = script::cleanup_line(&id);
    let remote = Remote {
        task: "linux",
        name: HOST,
        host: HOST,
    };
    let run = Run {
        id: &id,
        tarball_name: &name,
        target: &target,
        commit: &commit,
        bootstrap: script::bootstrap_line(&id, &layout),
        bound: script::bound(&spec),
        stop: &stop,
        cleanup: &cleanup,
    };
    session::go(ctx, remote, &tarball, run)
}

/// Why a run may not go to zeelin now, or none. zeelin not answering is a
/// refusal (the laptop's Ubuntu is not running); one that answered without
/// saying is the error.
fn refused(ctx: &mut Ctx, layout: &Layout) -> Result<Option<Vec<String>>, String> {
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
    let answer = ctx.runner.capture(&Call::new("ssh", args, PROBE_BOUND))?;
    if answer.timed_out {
        return Ok(Some(vec![format!(
            "zeelin not running: no answer in {}s, so nothing was sent",
            PROBE_BOUND.as_secs()
        )]));
    }
    let said: Vec<&str> = answer
        .stdout
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .collect();
    match (said.last().copied().and_then(exit_said), answer.code) {
        (Some(0), Some(0)) => Ok(None),
        (Some(REFUSED), _) => Ok(Some(
            said[..said.len() - 1]
                .iter()
                .map(|line| (*line).to_owned())
                .collect(),
        )),
        (_, Some(255)) if not_running(&answer.stderr) => Ok(Some(vec![format!(
            "zeelin not running: {}, so nothing was sent",
            answer.stderr.trim()
        )])),
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
