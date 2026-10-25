//! The two Windows machines, and whether a run may go to one now.
//!
//! - **zeewin** (AGV-COMPUTE1, `ssh zeewin`): the old PC, short of memory, so
//!   its builds run with fewer jobs. Nothing is asked of it before a run: its
//!   lock refuses a run while another goes on.
//! - **zeewin2** (the ASUS laptop's Windows, `ssh zeewin2`): the owner's laptop
//!   and poker-lab's compute box. A run is refused (75) while poker-lab's
//!   `~/.compile-lock/owner` exists on the laptop's Ubuntu (`ssh zeelin`; one
//!   that does not answer is an Ubuntu that is not running, and the check
//!   passes), and while C: has under [`FLOOR_GIB`] free. The lock is only looked
//!   at: never taken, broken or waited on.
//!
//! With no `--machine` a run goes to zeewin2 while it is allowed and no run of
//! Kluris or ConsensFlow holds it, else to zeewin.

use std::time::Duration;

use super::options::Name;
use super::remote::{Call, Captured, Runner};
use super::script;

/// The least C: of zeewin2 must have free for a run, in GiB.
pub const FLOOR_GIB: u64 = 50;

/// How long zeelin is given to answer.
const ZEELIN_CONNECT_S: u32 = 5;
const ZEELIN_BOUND: Duration = Duration::from_secs(15);
/// How long a Windows machine is given to say its state.
const STATE_BOUND: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Machine {
    pub name: &'static str,
    pub host: &'static str,
    /// `CARGO_BUILD_JOBS` there.
    pub jobs: u32,
    pub shared: Option<Shared>,
}

/// A machine another project uses too, and whose disk is its owner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shared {
    /// The ssh host of the Linux beside it, where poker-lab's lock is.
    pub linux: &'static str,
    /// That lock, from the Linux user's home.
    pub lock: &'static str,
    pub floor_gib: u64,
}

impl Machine {
    pub fn named(name: Name) -> Self {
        match name {
            // 7.4 GiB, of which about 2.3 are free while it idles.
            Name::Zeewin => Self {
                name: "zeewin",
                host: "zeewin",
                jobs: 2,
                shared: None,
            },
            Name::Zeewin2 => Self {
                name: "zeewin2",
                host: "zeewin2",
                jobs: 8,
                shared: Some(Shared {
                    linux: "zeelin",
                    lock: ".compile-lock/owner",
                    floor_gib: FLOOR_GIB,
                }),
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Ready {
    /// It may, with what is to be said of it.
    Go(Vec<String>),
    /// It may not, and why: the run ends with 75, having sent nothing.
    Refused(String),
}

/// Whether a run may go to `machine` now. A machine that could not say is the error.
pub fn ready(machine: &Machine, runner: &mut dyn Runner) -> Result<Ready, String> {
    Ok(look(machine, runner)?.0)
}

/// [`ready`], and the holder of the shared lock if a run holds it.
fn look(machine: &Machine, runner: &mut dyn Runner) -> Result<(Ready, Option<String>), String> {
    let Some(shared) = &machine.shared else {
        return Ok((Ready::Go(Vec::new()), None));
    };
    let mut notes = Vec::new();
    match poker_lab(shared, runner) {
        Lock::Held => {
            let why = format!(
                "{} is busy: poker-lab's job holds it (~/{} on {}), and no run goes while it does",
                machine.name, shared.lock, shared.linux
            );
            return Ok((Ready::Refused(why), None));
        }
        Lock::Free => {}
        Lock::NoAnswer(why) => notes.push(format!(
            "{} did not answer ({why}), so Ubuntu is not running there and no poker-lab job holds the laptop: going on",
            shared.linux
        )),
    }
    let state = state(machine, runner)?;
    if state.free_gib < shared.floor_gib {
        let why = format!(
            "{} is short of disk: a run needs {} GiB free on C:, and it has {}",
            machine.name, shared.floor_gib, state.free_gib
        );
        return Ok((Ready::Refused(why), state.holder));
    }
    Ok((Ready::Go(notes), state.holder))
}

enum Lock {
    Held,
    Free,
    NoAnswer(String),
}

fn poker_lab(shared: &Shared, runner: &mut dyn Runner) -> Lock {
    let args = [
        "-o".to_owned(),
        format!("ConnectTimeout={ZEELIN_CONNECT_S}"),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        "ServerAliveInterval=5".to_owned(),
        "-o".to_owned(),
        "ServerAliveCountMax=1".to_owned(),
        shared.linux.to_owned(),
        format!(
            "if [ -e ~/{} ]; then echo held; else echo free; fi",
            shared.lock
        ),
    ];
    match runner.capture(&Call::new("ssh", args.to_vec(), ZEELIN_BOUND)) {
        Ok(Captured {
            code: Some(0),
            stdout,
            ..
        }) => match stdout.trim() {
            "held" => Lock::Held,
            "free" => Lock::Free,
            said => Lock::NoAnswer(format!("it said {said:?}")),
        },
        Ok(Captured {
            timed_out: true, ..
        }) => Lock::NoAnswer(format!("no answer in {}s", ZEELIN_BOUND.as_secs())),
        Ok(answer) => Lock::NoAnswer(format!(
            "ssh ended with {}{}",
            status(answer.code),
            said_as(&answer.stderr)
        )),
        Err(why) => Lock::NoAnswer(why),
    }
}

/// What a Windows machine says of itself.
#[derive(Debug, PartialEq, Eq)]
struct State {
    free_gib: u64,
    /// The shared lock's record, while a run holds it.
    holder: Option<String>,
}

fn state(machine: &Machine, runner: &mut dyn Runner) -> Result<State, String> {
    let line = script::powershell_line(&script::state_script())?;
    let args = vec![
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        machine.host.to_owned(),
        line,
    ];
    let answer = runner.capture(&Call::new("ssh", args, STATE_BOUND))?;
    let unsaid = |why: String| format!("{} did not say what it has free ({why})", machine.name);
    if answer.timed_out {
        return Err(unsaid(format!("no answer in {}s", STATE_BOUND.as_secs())));
    }
    if answer.code != Some(0) {
        return Err(unsaid(format!(
            "ssh ended with {}{}",
            status(answer.code),
            said_as(&answer.stderr)
        )));
    }
    let free_gib = answer
        .stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("free ")?.trim().parse().ok())
        .ok_or_else(|| unsaid(format!("it said {:?}", answer.stdout)))?;
    let holder = answer
        .stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("held ").map(str::to_owned));
    Ok(State { free_gib, holder })
}

/// The machine a run goes to with no `--machine`, and the line that says which and why.
pub fn choose(runner: &mut dyn Runner) -> (Machine, String) {
    let laptop = Machine::named(Name::Zeewin2);
    let why = match look(&laptop, runner) {
        Ok((Ready::Go(_), None)) => return (laptop, "windows: on zeewin2".into()),
        Ok((Ready::Go(_), Some(holder))) => format!("a run holds its lock ({holder})"),
        Ok((Ready::Refused(why), _)) | Err(why) => why,
    };
    (
        Machine::named(Name::Zeewin),
        format!("windows: on zeewin, as zeewin2 is not free: {why}"),
    )
}

fn status(code: Option<i32>) -> String {
    code.map_or_else(|| "no status".into(), |code| format!("status {code}"))
}

fn said_as(stderr: &str) -> String {
    let said = stderr.trim();
    if said.is_empty() {
        String::new()
    } else {
        format!(": {said}")
    }
}
