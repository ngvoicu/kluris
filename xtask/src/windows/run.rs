//! One run: the machine chosen and asked, the tarball made and sent, the
//! bootstrap streamed back, and the answer. A run stopped here, one past its
//! bound and one whose connection was lost are ended there too (the stop
//! script), since Windows' SSH server does not end what a lost session started.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use super::machine::{self, Machine, Ready};
use super::options::{self, Options};
use super::remote::{Call, Runner, Stream, Streamed};
use super::script::{self, EXIT_MARK, STEP_MARK, Spec, TREE_MARK};

/// A machine that cannot take the run now: its lock is held, or zeewin2 is refused.
pub const REFUSED: i32 = 75;
/// This side was stopped (Ctrl-C).
pub const STOPPED: i32 = 130;
/// The run passed its bound.
pub const TIMED_OUT: i32 = 124;

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
    /// The tree's toolchain channel.
    pub channel: String,
    /// Writes the tarball (with the run script in it) and answers the commit.
    pub pack: &'a mut dyn FnMut(&Path, &str) -> Result<String, String>,
    /// The worktree, for the `--parity-files` check before anything is sent.
    pub root: &'a Path,
}

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

    let id = script::run_id();
    let spec = Spec {
        id: &id,
        channel: &ctx.channel,
        jobs: machine.jobs,
        parity: options.parity,
        parity_files: &parity_files,
        cargo_args: &options.cargo_args,
    };
    let bootstrap = script::powershell_line(&script::bootstrap(&id))?;
    let stop = script::powershell_line(&script::stop_script(&id))?;
    let scratch = tempfile::tempdir().map_err(|e| format!("no folder for the tarball: {e}"))?;
    let tarball = scratch.path().join(script::tarball_name(&id));
    let commit = (ctx.pack)(&tarball, &script::run_script(&spec))?;
    if (ctx.stop)() {
        return Ok(STOPPED);
    }

    let upload = Call::new(
        "scp",
        vec![
            "-q".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            tarball.to_string_lossy().into_owned(),
            format!("{}:{}", machine.host, script::tarball_name(&id)),
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
            &format!("windows: scp of the tree to {} failed: {why}", machine.name),
        )?;
        let cleaned = remove_upload(ctx, &machine, &id)?;
        say(ctx.err, &format!("windows: {cleaned}"))?;
        return Ok(1);
    }
    if (ctx.stop)() {
        end_there(ctx, &machine, &stop);
        return Ok(STOPPED);
    }

    let mut args: Vec<String> = ["-o", "BatchMode=yes"].map(str::to_owned).to_vec();
    args.extend(KEEPALIVE.map(str::to_owned));
    args.push(machine.host.to_owned());
    args.push(bootstrap);
    let call = Call::new("ssh", args, script::bound(&spec));
    let mut said = Said::default();
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
            let exit = said.last_line.as_deref().and_then(script::exit_said);
            let (code, note) = answer(ssh, exit, said.tree.is_some());
            if let Some(note) = note {
                say(ctx.err, &format!("windows: {note}"))?;
            }
            if exit.is_none() {
                // The session ended without the run's last line: it may go on there.
                if !end_there(ctx, &machine, &stop) {
                    say(
                        ctx.err,
                        "windows: the run may still be going on there, holding the lock until its steps' bounds end it",
                    )?;
                }
            }
            code
        }
        Ok(Streamed::Stopped) => {
            end_there(ctx, &machine, &stop);
            STOPPED
        }
        Ok(Streamed::TimedOut) => {
            say(
                ctx.err,
                &format!(
                    "windows: the run passed its bound of {} min and was ended",
                    call.bound.as_secs() / 60
                ),
            )?;
            end_there(ctx, &machine, &stop);
            TIMED_OUT
        }
        Err(why) => {
            end_there(ctx, &machine, &stop);
            return Err(why);
        }
    };
    say(ctx.err, &summary(&machine, &id, &commit, &said, code))?;
    Ok(code)
}

/// Removes what a failed upload left of the run's tarball there, and says how it went.
fn remove_upload(ctx: &mut Ctx, machine: &Machine, id: &str) -> Result<String, String> {
    let name = script::tarball_name(id);
    let args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        format!("ConnectTimeout={CLEANUP_CONNECT_S}"),
        machine.host.into(),
        script::powershell_line(&script::cleanup_script(id))?,
    ];
    let sweep = "the next run's sweep removes it after a day";
    let answer = ctx.runner.capture(&Call::new("ssh", args, CLEANUP_BOUND));
    Ok(match answer {
        Ok(answer) if answer.code == Some(0) => match answer.stdout.trim() {
            "removed" => format!(
                "the run's partial tarball {name} on {} was removed",
                machine.name
            ),
            "absent" => format!(
                "the run's tarball {name} was not there on {}: nothing to remove",
                machine.name
            ),
            said => match said.strip_prefix("failed: ") {
                Some(why) => format!(
                    "the run's tarball {name} on {} could not be removed: {why}; {sweep}",
                    machine.name
                ),
                None => format!(
                    "the cleanup of {name} on {} said {said:?}; {sweep}",
                    machine.name
                ),
            },
        },
        Ok(answer) if answer.timed_out => format!(
            "could not reach {} to remove {name} (no answer in {}s); {sweep}",
            machine.name,
            CLEANUP_BOUND.as_secs()
        ),
        Ok(answer) => format!(
            "could not reach {} to remove {name} (ssh ended with {:?}: {}); {sweep}",
            machine.name,
            answer.code,
            answer.stderr.trim()
        ),
        Err(why) => format!(
            "could not reach {} to remove {name} ({why}); {sweep}",
            machine.name
        ),
    })
}

/// Sends the stop; whether the machine said it was done.
fn end_there(ctx: &mut Ctx, machine: &Machine, stop: &str) -> bool {
    let args = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        machine.host.into(),
        stop.into(),
    ];
    matches!(
        ctx.runner.capture(&Call::new("ssh", args, STOP_BOUND)),
        Ok(answer) if answer.code == Some(0)
    )
}

/// The status a run that ssh ended by itself comes to, and what is to be said
/// of it besides. `said` is the machine's own last line (`== exit N`): it is the
/// answer whatever ssh's status is, since ssh to a PowerShell answers 1 for any
/// failure. Without it ssh's status is the answer if it is a failure, and never
/// 0. A pass needs the tree said: a run that never unpacked passed nothing. A
/// status no program here ends with (Windows' -1073741819) is 1.
pub fn answer(ssh: Option<i32>, said: Option<i32>, tree_said: bool) -> (i32, Option<String>) {
    let holds = |status: i32| (0..=255).contains(&status);
    match said {
        Some(0) if !tree_said => (
            1,
            Some(format!(
                "no `{TREE_MARK}…` line said which tree the machine ran, so its 0 is not a pass: answering 1"
            )),
        ),
        Some(status) if holds(status) => (status, None),
        Some(status) => (
            1,
            Some(format!(
                "the run ended with status {status}, which no status here holds: answering 1"
            )),
        ),
        None => {
            let missing = format!("no `{EXIT_MARK}N` line ended the output");
            match ssh {
                Some(code) if code != 0 && holds(code) => (
                    code,
                    Some(format!("{missing}, so ssh's status is the answer: {code}")),
                ),
                Some(code) => (
                    1,
                    Some(format!("{missing}, and ssh ended with {code}: answering 1")),
                ),
                None => (
                    1,
                    Some(format!(
                        "{missing}, and ssh was ended by a signal: answering 1"
                    )),
                ),
            }
        }
    }
}

/// What the machine's output said, read as it goes by: the last line, the
/// tree, and each step's end (`== step NAME exit CODE SECONDSs`).
#[derive(Default)]
struct Said {
    partial: Vec<u8>,
    last_line: Option<String>,
    tree: Option<String>,
    steps: Vec<(String, String, String)>,
}

impl Said {
    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                let line = String::from_utf8_lossy(&self.partial).trim_end().to_owned();
                self.partial.clear();
                self.line(line);
            } else {
                self.partial.push(byte);
            }
        }
    }

    fn finish(&mut self) {
        if !self.partial.is_empty() {
            let line = String::from_utf8_lossy(&self.partial).trim_end().to_owned();
            self.partial.clear();
            self.line(line);
        }
    }

    fn line(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        if let Some(tree) = line.strip_prefix(TREE_MARK) {
            self.tree = Some(tree.trim().to_owned());
        }
        if let Some(rest) = line.strip_prefix(STEP_MARK)
            && let [name, "exit", code, seconds] = rest.split_whitespace().collect::<Vec<_>>()[..]
        {
            self.steps
                .push((name.to_owned(), code.to_owned(), seconds.to_owned()));
        }
        self.last_line = Some(line);
    }
}

fn summary(machine: &Machine, id: &str, commit: &str, said: &Said, code: i32) -> String {
    let tree: String = commit.chars().take(12).collect();
    let steps = if said.steps.is_empty() {
        "no step ran".to_owned()
    } else {
        said.steps
            .iter()
            .map(|(name, code, seconds)| format!("{name} {code} ({seconds})"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "windows: {} run {id} tree {tree}: {steps}; exit {code}",
        machine.name
    )
}

fn say(to: &mut dyn Write, line: &str) -> Result<(), String> {
    writeln!(to, "{line}")
        .and_then(|()| to.flush())
        .map_err(|e: io::Error| format!("could not write: {e}"))
}
