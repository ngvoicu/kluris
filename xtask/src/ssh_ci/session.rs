//! The part of a run that is the same on every machine once it may go: the
//! tarball sent by scp (and, when that fails, what it left there removed), the
//! bootstrap streamed back and its answer, and the stop that ends a run there
//! when it was stopped here, passed its bound or lost its connection (neither
//! Windows' SSH server nor a session without a terminal ends what a lost
//! session started).

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use super::outcome::{REFUSED, STOPPED, Said, TIMED_OUT, answer, exit_said, say, summary};
use super::queue::{Pace, Queues};
use super::runner::{Call, Runner, Stream, Streamed};

const UPLOAD_BOUND: Duration = Duration::from_secs(10 * 60);
const STOP_BOUND: Duration = Duration::from_secs(60);
/// The cleanup of a failed upload: a short connect and a short whole.
const CLEANUP_CONNECT_S: u32 = 10;
const CLEANUP_BOUND: Duration = Duration::from_secs(30);
/// ssh ends a session whose machine stops answering for this long (15 s × 8).
const KEEPALIVE: [&str; 4] = [
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=8",
];

/// What a run needs of this side.
pub struct Ctx<'a> {
    pub runner: &'a mut dyn Runner,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// Whether this side was asked to stop.
    pub stop: &'a dyn Fn() -> bool,
    /// The run's id ([`crate::ssh_ci::plan::run_id`]): its folder, tarball
    /// and lock records there, and the word its step lines end with.
    pub id: String,
    /// The tree's toolchain channel.
    pub channel: String,
    /// Writes the tarball (with the run script in it) and answers the commit.
    pub pack: &'a mut dyn FnMut(&Path, &str) -> Result<String, String>,
    /// The worktree, for the `--parity-files` check before anything is sent,
    /// and its folder's name for the run's ticket.
    pub root: &'a Path,
    /// The machines' queues ([`super::queue`]): none where no folder is known.
    pub queues: Option<Queues>,
    /// The clock and the pauses of the wait for a turn.
    pub pace: &'a mut dyn Pace,
    /// The command as it was typed, for the run's ticket.
    pub command: String,
}

/// The machine a run goes to, as its lines name it.
#[derive(Debug, Clone, Copy)]
pub struct Remote<'a> {
    /// The task, the first word of each line this side says: `windows`, `linux`.
    pub task: &'a str,
    pub name: &'a str,
    pub host: &'a str,
}

/// The run's tarball and the lines its machine is sent.
pub struct Run<'a> {
    pub id: &'a str,
    /// The tarball's name, there.
    pub tarball_name: &'a str,
    /// The tarball's path there, as scp names it after `host:`.
    pub target: &'a str,
    /// The commit the tarball was made from.
    pub commit: &'a str,
    /// The bootstrap's line, how long it may take, and the stop's and the
    /// upload cleanup's lines.
    pub bootstrap: String,
    pub bound: Duration,
    pub stop: &'a str,
    pub cleanup: &'a str,
}

/// How a run sent to its machine ended.
#[derive(Debug, PartialEq, Eq)]
pub struct Sent {
    /// The status this command ends with.
    pub code: i32,
    /// The bootstrap refused the run with 75 before unpacking anything (the
    /// machine's own lock, or a check under it), and what it said: the run
    /// may wait on for its turn.
    pub refused: Option<String>,
}

impl From<i32> for Sent {
    fn from(code: i32) -> Self {
        Self {
            code,
            refused: None,
        }
    }
}

/// Sends the tarball at `tarball`, then the bootstrap, and answers how it ended.
pub fn go(ctx: &mut Ctx, remote: Remote, tarball: &Path, run: Run) -> Result<Sent, String> {
    if (ctx.stop)() {
        return Ok(STOPPED.into());
    }
    let upload = Call::new(
        "scp",
        vec![
            "-q".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            tarball.to_string_lossy().into_owned(),
            format!("{}:{}", remote.host, run.target),
        ],
        UPLOAD_BOUND,
    );
    let sent = ctx.runner.capture(&upload)?;
    if sent.code != Some(0) {
        let why = if sent.timed_out {
            format!("not done in {} min", UPLOAD_BOUND.as_secs() / 60)
        } else {
            format!("it ended with {:?}: {}", sent.code, sent.stderr.trim())
        };
        say(
            ctx.err,
            &format!(
                "{}: scp of the tree to {} failed: {why}",
                remote.task, remote.name
            ),
        )?;
        let cleaned = remove_upload(ctx, remote, run.tarball_name, run.cleanup);
        say(ctx.err, &format!("{}: {cleaned}", remote.task))?;
        return Ok(1.into());
    }
    if (ctx.stop)() {
        end_there(ctx, remote, run.stop);
        return Ok(STOPPED.into());
    }

    let mut args: Vec<String> = ["-o", "BatchMode=yes"].map(str::to_owned).to_vec();
    args.extend(KEEPALIVE.map(str::to_owned));
    args.push(remote.host.to_owned());
    args.push(run.bootstrap);
    let call = Call::new("ssh", args, run.bound);
    let mut said = Said::of_run(run.id);
    let out = &mut *ctx.out;
    let err = &mut *ctx.err;
    let ended = ctx.runner.stream(
        &call,
        &mut |stream, bytes| {
            let to: &mut dyn Write = match stream {
                Stream::Out => {
                    said.push(bytes);
                    &mut *out
                }
                Stream::Err => &mut *err,
            };
            to.write_all(bytes).and_then(|()| to.flush())
        },
        ctx.stop,
    );
    said.finish();
    let code = match ended {
        Ok(Streamed::Exited(ssh)) => {
            let exit = said.last_line.as_deref().and_then(exit_said);
            let (code, note) = answer(ssh, exit, said.tree.is_some());
            if let Some(note) = note {
                say(ctx.err, &format!("{}: {note}", remote.task))?;
            }
            if exit.is_none() {
                // The session ended without the run's last line: it may go on there.
                if !end_there(ctx, remote, run.stop) {
                    say(
                        ctx.err,
                        &format!(
                            "{}: the run may still be going on there, holding the lock until its steps' bounds end it",
                            remote.task
                        ),
                    )?;
                }
            }
            code
        }
        Ok(Streamed::Stopped) => {
            end_there(ctx, remote, run.stop);
            STOPPED
        }
        Ok(Streamed::TimedOut) => {
            say(
                ctx.err,
                &format!(
                    "{}: the run passed its bound of {} min and was ended",
                    remote.task,
                    call.bound.as_secs() / 60
                ),
            )?;
            end_there(ctx, remote, run.stop);
            TIMED_OUT
        }
        Err(why) => {
            end_there(ctx, remote, run.stop);
            return Err(why);
        }
    };
    say(
        ctx.err,
        &summary(remote.task, remote.name, run.id, run.commit, &said, code),
    )?;
    // The machine said 75 itself before it said a tree: its bootstrap refused
    // the run before anything was unpacked.
    let exit = said.last_line.as_deref().and_then(exit_said);
    let refused = (exit == Some(REFUSED) && said.tree.is_none()).then(|| {
        said.before_last
            .clone()
            .unwrap_or_else(|| format!("{} refused the run with {REFUSED}", remote.name))
    });
    Ok(Sent { code, refused })
}

/// Removes what a failed upload left of the run's tarball there with the
/// `cleanup` line (which says `removed`, `absent` or `failed: <why>`), and says
/// how it went.
fn remove_upload(ctx: &mut Ctx, remote: Remote, name: &str, cleanup: &str) -> String {
    let args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={CLEANUP_CONNECT_S}"),
        remote.host.into(),
        cleanup.into(),
    ];
    let machine = remote.name;
    let sweep = "the next run's sweep removes it after a day";
    match ctx.runner.capture(&Call::new("ssh", args, CLEANUP_BOUND)) {
        Ok(answer) if answer.code == Some(0) => match answer.stdout.trim() {
            "removed" => format!("the run's partial tarball {name} on {machine} was removed"),
            "absent" => {
                format!("the run's tarball {name} was not there on {machine}: nothing to remove")
            }
            said => match said.strip_prefix("failed: ") {
                Some(why) => format!(
                    "the run's tarball {name} on {machine} could not be removed: {why}; {sweep}"
                ),
                None => format!("the cleanup of {name} on {machine} said {said:?}; {sweep}"),
            },
        },
        Ok(answer) if answer.timed_out => format!(
            "could not reach {machine} to remove {name} (no answer in {}s); {sweep}",
            CLEANUP_BOUND.as_secs()
        ),
        Ok(answer) => format!(
            "could not reach {machine} to remove {name} (ssh ended with {:?}: {}); {sweep}",
            answer.code,
            answer.stderr.trim()
        ),
        Err(why) => format!("could not reach {machine} to remove {name} ({why}); {sweep}"),
    }
}

/// Sends the stop; whether the machine said it was done.
fn end_there(ctx: &mut Ctx, remote: Remote, stop: &str) -> bool {
    let args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        remote.host.into(),
        stop.into(),
    ];
    matches!(
        ctx.runner.capture(&Call::new("ssh", args, STOP_BOUND)),
        Ok(answer) if answer.code == Some(0)
    )
}
