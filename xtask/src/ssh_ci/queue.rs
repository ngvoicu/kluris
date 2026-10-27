//! The first-come queue every runner on this Mac keeps for the household
//! machines: ConsensFlow's (`tools/xtask/src/windows/queue.rs` in that
//! repository, whose module doc is the protocol), kept here to the letter, so
//! that a ConsensFlow run and a Kluris run see each other's tickets and are
//! served in the order they came.
//!
//! - **Where:** a folder for each machine in `~/.cf-windows-queues` (or the
//!   folder `CF_WINDOWS_QUEUES` names), a ticket file in it for each run that
//!   waits or goes on there, named `<arrival ms, 15 digits>-<process, 10
//!   digits>.ticket` (so the names sort in the order of coming) and holding two
//!   lines: the tree's folder name and the command.
//! - **Alive:** a thread touches the ticket every [`BEAT`] from the run's
//!   coming to its end; one not touched for [`DEAD_AFTER_MS`] is a run that was
//!   killed, and whoever looks takes it away. A live ticket is never removed
//!   but by its own run, when the run ends, however it ends (the guard is
//!   dropped).
//! - **Order:** a run goes when no live ticket older than its own is there and
//!   the machine is ready (each task's own checks, asked only at the head of
//!   the queue); it holds its ticket while it goes on, so the runs behind wait
//!   for it.
//! - **The clock:** a ticket is named for the moment its run came or after the
//!   newest live ticket, whichever is later, so a clock set back puts no run
//!   ahead of those that were there; a ticket touched more than [`AHEAD_MS`]
//!   ahead of the clock is a run killed before it was set back, and holds no
//!   one back. A Mac that slept past [`DEAD_AFTER_MS`] finds its ticket gone and
//!   makes it again under the same name.
//! - **`--wait MINUTES`:** [`DEFAULT_WAIT_MINUTES`] unless given, at most
//!   [`MAX_WAIT_MINUTES`]. A wait that runs out ends with 75 saying how long it
//!   waited and what for. `--wait 0` asks once and takes no ticket: while a live
//!   run is queued for the machine it is refused before the machine is asked.
//! - **The looks:** behind a run the queue is looked at every ten seconds and
//!   the machine not at all; at the head a busy machine is asked again after
//!   half a minute, then every minute. Where the run stands is said when what
//!   holds it changes, and every minute.
//! - **A machine that cannot be read:** the first look is the run's answer and
//!   ends it; once the wait is on it is said and the wait goes on, and a wait
//!   that runs out with the machine unread ends with that error.
//!
//! Kluris adds two things ConsensFlow's queue has no use for, outside the
//! protocol: a machine that is not there ([`Look::Gone`]: zeelin not running)
//! ends the wait with 75 at any look, never waited on; and the machine's own
//! lock refusing a run at the head ([`Turn::refused_there`]) is a busy look,
//! after which the run keeps its place and waits on.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a run waits for its turn unless `--wait` says another, in minutes.
pub const DEFAULT_WAIT_MINUTES: u32 = 60;

/// The longest a run may be told to wait, in minutes: a day.
pub const MAX_WAIT_MINUTES: u32 = 24 * 60;

/// How often a run touches its ticket while it lives.
const BEAT: Duration = Duration::from_secs(5);

/// How long a ticket may go untouched and be a run's: six beats.
const DEAD_AFTER_MS: i64 = 30_000;

/// How far a ticket's touch may be ahead of the clock and be a run's.
const AHEAD_MS: i64 = 10_000;

/// How often a waiting run looks at the queue, in milliseconds.
const LOOK_MS: u64 = 10_000;

/// How often a run at the head looks at a busy machine: the first time, and
/// from then on, in milliseconds.
const MACHINE_LOOK_MS: [u64; 2] = [30_000, 60_000];

/// How often a waiting run says where it stands, in milliseconds.
const SAY_EVERY_MS: i64 = 60_000;

/// The milliseconds since the epoch, by the system's clock.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}

/// How many runs are queued for a machine, as the default choice and a run
/// told not to wait say it (ConsensFlow's words).
pub fn queued_words(queued: usize) -> String {
    let (runs, are) = if queued == 1 {
        ("run", "is")
    } else {
        ("runs", "are")
    };
    format!("{queued} {runs} {are} queued for it")
}

/// What a ticket says of its run, for the runs behind it to say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Who {
    /// The tree the run is from: its folder's name.
    pub tree: String,
    /// The command, as it was typed, cut short.
    pub command: String,
}

impl Who {
    /// The run of `command` from the checkout `root`.
    pub fn of(root: &Path, command: &str) -> Self {
        let tree = root.file_name().map_or_else(
            || root.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let command = if command.chars().count() > 80 {
            format!("{}…", command.chars().take(79).collect::<String>())
        } else {
            command.to_owned()
        };
        Self { tree, command }
    }
}

/// One run ahead of another in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ahead {
    pub who: Who,
    /// How long ago it came, in milliseconds.
    pub since_ms: i64,
}

/// The folder every runner on this Mac keeps its queues in.
#[derive(Debug, Clone)]
pub struct Queues {
    root: PathBuf,
    beat: Duration,
}

impl Queues {
    /// The folder `CF_WINDOWS_QUEUES` names, else `.cf-windows-queues` in the
    /// home folder; none where neither is known (the run then waits for the
    /// machine without a place in a queue).
    pub fn of_env() -> Option<Self> {
        let named = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let root = named("CF_WINDOWS_QUEUES").map(PathBuf::from).or_else(|| {
            named("HOME")
                .or_else(|| named("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".cf-windows-queues"))
        })?;
        Some(Self { root, beat: BEAT })
    }

    /// The queues in `root`, touched every `beat`.
    #[cfg(test)]
    pub fn at(root: &Path, beat: Duration) -> Self {
        Self {
            root: root.to_path_buf(),
            beat,
        }
    }

    fn folder(&self, machine: &str) -> PathBuf {
        self.root.join(machine)
    }

    /// How many runs are in `machine`'s queue now (alive), the one that goes
    /// on there included.
    pub fn depth(&self, machine: &str, now_ms: i64) -> usize {
        tickets(&self.folder(machine))
            .into_iter()
            .filter(|ticket| ticket.alive(now_ms))
            .count()
    }

    /// Takes a ticket for `machine` for the run `who`, which came `now_ms`.
    pub fn join(&self, machine: &str, who: &Who, now_ms: i64) -> io::Result<Ticket> {
        let folder = self.folder(machine);
        fs::create_dir_all(&folder)?;
        // Named for the moment it came, and after the newest run that is
        // beating: a clock set back must not put a run that comes now ahead of
        // the runs that were there.
        let newest = tickets(&folder)
            .iter()
            .filter(|ticket| ticket.alive(now_ms))
            .map(|ticket| ticket.came_ms)
            .max();
        let mut at = newest.map_or(now_ms, |newest| now_ms.max(newest + 1));
        let text = format!("{}\n{}\n", who.tree, who.command);
        let path = loop {
            let path = folder.join(ticket_name(at, std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    io::Write::write_all(&mut file, text.as_bytes())?;
                    break path;
                }
                // A second run of this process in the same millisecond: after the first.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => at += 1,
                Err(error) => return Err(error),
            }
        };
        touch(&path, &text, now_ms)?;
        let (stop, stopped) = mpsc::channel::<()>();
        let beating = {
            let (path, text, beat) = (path.clone(), text.clone(), self.beat);
            thread::spawn(move || {
                while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(beat) {
                    // A ticket that is gone (taken for a dead one while this Mac
                    // slept) is made again under its name, which is its place.
                    let _ = touch(&path, &text, self::now_ms());
                }
            })
        };
        Ok(Ticket {
            path,
            text,
            stop: Some(stop),
            beating: Some(beating),
        })
    }
}

/// A ticket's name: `<came, 15 digits>-<process, 10 digits>.ticket`.
pub fn ticket_name(came_ms: i64, process: u32) -> String {
    format!("{came_ms:015}-{process:010}.ticket")
}

/// Touches the ticket `path`, writing it again with `text` if it is gone.
fn touch(path: &Path, text: &str, now_ms: i64) -> io::Result<()> {
    if !path.exists() {
        fs::write(path, text)?;
    }
    let file = fs::OpenOptions::new().write(true).open(path)?;
    let since = u64::try_from(now_ms).unwrap_or(0);
    file.set_modified(UNIX_EPOCH + Duration::from_millis(since))
}

/// A ticket as it is on disk.
struct Entry {
    path: PathBuf,
    /// When the run came, from its name.
    came_ms: i64,
    /// When it was last touched.
    touched_ms: Option<i64>,
}

impl Entry {
    fn alive(&self, now_ms: i64) -> bool {
        self.touched_ms.is_some_and(|touched| {
            touched - now_ms <= AHEAD_MS && now_ms - touched <= DEAD_AFTER_MS
        })
    }

    fn who(&self) -> Who {
        let text = fs::read_to_string(&self.path).unwrap_or_default();
        let mut lines = text.lines();
        Who {
            tree: lines.next().unwrap_or("a run").to_owned(),
            command: lines.next().unwrap_or("").to_owned(),
        }
    }
}

/// The tickets in `folder`, in the order the runs came.
fn tickets(folder: &Path) -> Vec<Entry> {
    let Ok(listing) = fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<Entry> = listing
        .filter_map(Result::ok)
        .filter_map(|file| {
            let name = file.file_name().to_string_lossy().into_owned();
            let came_ms = name
                .strip_suffix(".ticket")?
                .split('-')
                .next()?
                .parse()
                .ok()?;
            let touched_ms = file
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .and_then(|since| i64::try_from(since.as_millis()).ok());
            Some(Entry {
                path: file.path(),
                came_ms,
                touched_ms,
            })
        })
        .collect();
    found.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    found
}

/// A run's place in a machine's queue: its file, kept alive by a thread that
/// touches it, and taken away when this is dropped, however the run ended.
pub struct Ticket {
    path: PathBuf,
    text: String,
    stop: Option<mpsc::Sender<()>>,
    beating: Option<JoinHandle<()>>,
}

impl Ticket {
    /// The live runs that came before this one, oldest first. A ticket not
    /// touched for [`DEAD_AFTER_MS`] is a run that was killed, and is taken
    /// away; this one's own, if it was taken so, is made again.
    pub fn ahead(&self, now_ms: i64) -> Vec<Ahead> {
        if !self.path.exists() {
            let _ = touch(&self.path, &self.text, now_ms);
        }
        let Some(folder) = self.path.parent() else {
            return Vec::new();
        };
        let mine = self.path.file_name();
        let mut ahead = Vec::new();
        for entry in tickets(folder) {
            if entry.path.file_name() >= mine {
                break;
            }
            if entry.alive(now_ms) {
                ahead.push(Ahead {
                    who: entry.who(),
                    since_ms: now_ms - entry.came_ms,
                });
            } else {
                let _ = fs::remove_file(&entry.path);
            }
        }
        ahead
    }

    /// The ticket's file.
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(beating) = self.beating.take() {
            let _ = beating.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// What a wait is paced by: the time, and a pause that ends early on a stop.
pub trait Pace {
    fn now_ms(&mut self) -> i64;
    /// How long a waiting run pauses between two looks, in milliseconds: at
    /// the queue (`None`), or at a machine that was busy (`Some(n)`, the looks
    /// of this kind before this one).
    fn look_ms(&self, machine_looks: Option<usize>) -> u64 {
        match machine_looks {
            None => LOOK_MS,
            Some(before) => MACHINE_LOOK_MS[before.min(1)],
        }
    }
    /// Waits `ms`; false if a stop was heard.
    fn pause(&mut self, ms: u64) -> bool;
    /// Whether a stop was heard, after a look that failed as a stop would make
    /// it fail (a Ctrl-C ends the probes too: they are in the terminal's group).
    fn stopped_after_failure(&mut self) -> bool;
}

/// The system's clock, and a stop heard by `stop`.
pub struct Real<'a> {
    stop: &'a dyn Fn() -> bool,
}

impl<'a> Real<'a> {
    pub fn new(stop: &'a dyn Fn() -> bool) -> Self {
        Self { stop }
    }
}

impl Pace for Real<'_> {
    fn now_ms(&mut self) -> i64 {
        now_ms()
    }

    fn pause(&mut self, ms: u64) -> bool {
        let until = now_ms() + i64::try_from(ms).unwrap_or(i64::MAX);
        while now_ms() < until {
            if (self.stop)() {
                return false;
            }
            thread::sleep(Duration::from_millis(50.min(ms)));
        }
        !(self.stop)()
    }

    fn stopped_after_failure(&mut self) -> bool {
        // The probe can end on the Ctrl-C a moment before this process's own
        // handler has set the flag.
        if (self.stop)() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
        (self.stop)()
    }
}

/// What a machine says when the run at the head of its queue asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Look {
    /// It may go, with what is to be said of the machine.
    Go(Vec<String>),
    /// Not now, and why: the run waits.
    Busy(String),
    /// The machine is not there (zeelin not running), and why: the run ends
    /// with 75 at once, never waiting on it.
    Gone(String),
}

/// How a wait for a machine's turn ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Waited {
    /// It is this run's turn and the machine is ready: what is to be said of
    /// the machine.
    Go(Vec<String>),
    /// It is not, and why: the run ends with 75.
    Refused(String),
    /// A stop was heard.
    Stopped,
}

/// The queue a run waits in, and what it is called in the run's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place<'a> {
    /// The machine's folder in the queues: `zeewin`, `zeewin2` (a Linux run's
    /// too: zeelin is the same laptop).
    pub queue: &'a str,
    /// What the run waits for, as its lines say it: `zeewin2`, `zeelin`.
    pub machine: &'a str,
}

impl<'a> Place<'a> {
    /// A machine with a queue of its own.
    pub fn of(machine: &'a str) -> Self {
        Self {
            queue: machine,
            machine,
        }
    }

    /// "queued for it", and where the queue is another machine's, whose.
    fn queued(&self, queued: usize) -> String {
        let words = queued_words(queued);
        if self.queue == self.machine {
            words
        } else {
            format!("{words} ({}'s queue: the same laptop)", self.queue)
        }
    }
}

/// A run's wait for its turn: its ticket (none with `--wait 0`, or where no
/// queue could be kept), held until this is dropped, and where the wait
/// stands, so that it can go on after the machine's own lock refused the run.
pub struct Turn<'a> {
    place: Place<'a>,
    minutes: u32,
    began: i64,
    ticket: Option<Ticket>,
    /// What was last said of what holds the run, and when.
    said: Option<(String, i64)>,
    machine_looks: usize,
    looked: bool,
    /// The machine's own lock refused the run: the next look is this.
    refused: Option<String>,
}

impl<'a> Turn<'a> {
    /// The run `who` comes to `place` and will wait up to `minutes`. A queue
    /// that cannot be kept (a folder that cannot be made, a full disk) does not
    /// stop the run: it is said, and the run waits for the machine alone.
    pub fn take(
        queues: Option<&Queues>,
        place: Place<'a>,
        who: &Who,
        minutes: u32,
        pace: &mut dyn Pace,
        say: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<(Self, Option<String>), String> {
        let began = pace.now_ms();
        let mut refusal = None;
        if minutes == 0 {
            let queued = queues.map_or(0, |queues| queues.depth(place.queue, began));
            if queued > 0 {
                refusal = Some(format!(
                    "{} is busy: {}",
                    place.machine,
                    place.queued(queued)
                ));
            }
        }
        let ticket = match queues.filter(|_| minutes > 0) {
            Some(queues) => match queues.join(place.queue, who, began) {
                Ok(ticket) => Some(ticket),
                Err(cause) => {
                    say(&format!(
                        "no place could be kept in the queue for {} ({cause}): waiting for the machine alone",
                        place.queue
                    ))?;
                    None
                }
            },
            None => None,
        };
        let turn = Self {
            place,
            minutes,
            began,
            ticket,
            said: None,
            machine_looks: 0,
            looked: false,
            refused: None,
        };
        Ok((turn, refusal))
    }

    /// The machine's own lock refused the run although it was at the head
    /// (`why`, what the bootstrap said): the run keeps its place, and its next
    /// look is this refusal.
    pub fn refused_there(&mut self, why: String) {
        self.refused = Some(why);
        self.machine_looks = 0;
    }

    /// Waits for this run's turn, and for the machine to be ready (`ready`),
    /// until the minutes it was given have passed since it came. `say` is told
    /// where the run stands, when it changes and every minute.
    pub fn wait(
        &mut self,
        ready: &mut dyn FnMut() -> Result<Look, String>,
        pace: &mut dyn Pace,
        say: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<Waited, String> {
        let machine = self.place.machine;
        let minutes = self.minutes;
        let limit_ms = i64::from(minutes) * 60_000;
        loop {
            let now = pace.now_ms();
            let ahead = self
                .ticket
                .as_ref()
                .map(|ticket| ticket.ahead(now))
                .unwrap_or_default();
            let (blocked, elder_for, unread, pause) = if let Some(first) = ahead.first() {
                self.machine_looks = 0;
                let place = ordinal(ahead.len() + 1);
                (
                    format!(
                        "{place} in the queue for {}; the run ahead is {} ({})",
                        self.place.queue, first.who.tree, first.who.command
                    ),
                    Some(first.since_ms),
                    None,
                    pace.look_ms(None),
                )
            } else {
                let asked = match self.refused.take() {
                    Some(why) => Ok(Look::Busy(why)),
                    None => ready(),
                };
                if !matches!(asked, Ok(Look::Go(_))) && pace.stopped_after_failure() {
                    return Ok(Waited::Stopped);
                }
                let pause = pace.look_ms(Some(self.machine_looks));
                self.machine_looks += 1;
                match asked {
                    Ok(Look::Go(lines)) => return Ok(Waited::Go(lines)),
                    Ok(Look::Gone(why)) => return Ok(Waited::Refused(why)),
                    Ok(Look::Busy(why)) => (why, None, None, pause),
                    Err(why) if !self.looked || minutes == 0 => return Err(why),
                    Err(why) => (
                        format!("{machine} could not be asked: {why}"),
                        None,
                        Some(why),
                        pause,
                    ),
                }
            };
            self.looked = true;
            let shown = match elder_for {
                Some(elder) => format!("{blocked}, here for {}", minutes_of(elder)),
                None => blocked.clone(),
            };
            if minutes == 0 {
                return Ok(Waited::Refused(shown));
            }
            let waited = now - self.began;
            if waited >= limit_ms {
                if let Some(why) = unread {
                    return Err(format!(
                        "{machine} could not be asked in {} of waiting (--wait {minutes}): {why}",
                        minutes_of(waited)
                    ));
                }
                return Ok(Waited::Refused(format!(
                    "{machine} is still busy after waiting {} (as far as --wait {minutes} goes): {shown}",
                    minutes_of(waited)
                )));
            }
            // Said when what holds the run changes, and every minute: the
            // seconds the run ahead has been there are no change.
            let changed = self.said.as_ref().is_none_or(|(was, _)| *was != blocked);
            let due = self
                .said
                .as_ref()
                .is_none_or(|(_, at)| now - at >= SAY_EVERY_MS);
            if changed || due {
                say(&format!(
                    "waiting for {machine}, {} of up to {minutes} min: {shown}",
                    minutes_of(waited)
                ))?;
                self.said = Some((blocked, now));
            }
            if !pace.pause(pause) {
                return Ok(Waited::Stopped);
            }
        }
    }

    /// The run's ticket, if it holds one.
    #[cfg(test)]
    pub fn ticket(&self) -> Option<&Ticket> {
        self.ticket.as_ref()
    }
}

/// `n` as "1st", "2nd", "3rd", "4th".
fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// `ms` as whole minutes, or seconds under one.
fn minutes_of(ms: i64) -> String {
    if ms < 60_000 {
        format!("{} s", ms.max(0) / 1000)
    } else {
        format!("{} min", ms / 60_000)
    }
}

#[cfg(test)]
mod tests;
