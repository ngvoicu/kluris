use std::time::Duration;

use super::*;

/// Queues in a folder of their own (never `~/.cf-windows-queues`), whose
/// tickets are not beaten (an hour), so the times a case names are the only
/// times there are.
fn queues() -> (tempfile::TempDir, Queues) {
    let dir = tempfile::tempdir().unwrap();
    let queues = Queues::at(dir.path(), Duration::from_secs(3600));
    (dir, queues)
}

fn who(tree: &str) -> Who {
    Who {
        tree: tree.to_owned(),
        command: format!("cargo xtask windows in {tree}"),
    }
}

fn files(dir: &Path) -> Vec<String> {
    let Ok(listing) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = listing
        .map(|file| file.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A moment a case counts from, in milliseconds.
const DAY: i64 = 1_800_000_000_000;

/// A pace whose time moves only as the wait pauses, and that hears a stop
/// after `stop_after` pauses if it is given one.
struct Slow {
    now: i64,
    pauses: Vec<u64>,
    stop_after: Option<usize>,
    on_pause: Box<dyn FnMut(usize, i64)>,
}

impl Slow {
    fn at(now: i64) -> Self {
        Self {
            now,
            pauses: Vec::new(),
            stop_after: None,
            on_pause: Box::new(|_, _| {}),
        }
    }
}

impl Pace for Slow {
    fn now_ms(&mut self) -> i64 {
        self.now
    }

    fn pause(&mut self, ms: u64) -> bool {
        self.pauses.push(ms);
        self.now += i64::try_from(ms).unwrap();
        (self.on_pause)(self.pauses.len(), self.now);
        self.stop_after.is_none_or(|n| self.pauses.len() < n)
    }

    fn stopped_after_failure(&mut self) -> bool {
        false
    }
}

const ZEEWIN2: Place<'static> = Place {
    queue: "zeewin2",
    machine: "zeewin2",
};

const ZEELIN: Place<'static> = Place {
    queue: "zeewin2",
    machine: "zeelin",
};

/// How a wait of the run `mine` at `place` ended and what it said:
/// `Go(lines)`, `Refused(why)`, `Stopped` or `Error(why)`.
fn waited(
    queues: Option<&Queues>,
    place: Place,
    minutes: u32,
    answers: &mut dyn Iterator<Item = Result<Look, String>>,
    pace: &mut Slow,
) -> (String, Vec<String>) {
    let mut said = Vec::new();
    let mut say = |line: &str| {
        said.push(line.to_owned());
        Ok(())
    };
    let (mut turn, refused) =
        Turn::take(queues, place, &who("mine"), minutes, pace, &mut say).expect("a turn is taken");
    let ended = match refused {
        Some(why) => Ok(Waited::Refused(why)),
        None => turn.wait(
            &mut || {
                answers
                    .next()
                    .expect("the machine was asked more often than the case planned")
            },
            pace,
            &mut say,
        ),
    };
    let ended = match ended {
        Ok(Waited::Go(lines)) => format!("Go({lines:?}, ticket {})", turn.ticket().is_some()),
        Ok(Waited::Refused(why)) => format!("Refused({why})"),
        Ok(Waited::Stopped) => "Stopped".to_owned(),
        Err(error) => format!("Error({error})"),
    };
    drop(turn);
    (ended, said)
}

fn busy(why: &str) -> Result<Look, String> {
    Ok(Look::Busy(why.to_owned()))
}

fn go() -> Result<Look, String> {
    Ok(Look::Go(vec!["on it".to_owned()]))
}

mod tickets {
    use super::*;

    #[test]
    fn runs_are_ahead_of_each_other_in_the_order_they_came() {
        let (_dir, queues) = queues();
        let first = queues.join("zeewin2", &who("a"), DAY).unwrap();
        let second = queues.join("zeewin2", &who("b"), DAY + 5_000).unwrap();
        let third = queues.join("zeewin2", &who("c"), DAY + 6_000).unwrap();
        let now = DAY + 10_000;
        assert_eq!(first.ahead(now), []);
        assert_eq!(
            second.ahead(now),
            [Ahead {
                who: who("a"),
                since_ms: 10_000
            }]
        );
        let trees: Vec<String> = third.ahead(now).into_iter().map(|r| r.who.tree).collect();
        assert_eq!(trees, ["a", "b"]);
        // Another machine's queue is its own.
        let other = queues.join("zeewin", &who("d"), DAY + 1).unwrap();
        assert_eq!(other.ahead(now), []);
        assert_eq!(queues.depth("zeewin2", now), 3);
        assert_eq!(queues.depth("zeewin", now), 1);
        assert_eq!(queues.depth("nowhere", now), 0);
    }

    #[test]
    fn a_ticket_goes_with_its_run() {
        let (dir, queues) = queues();
        let ticket = queues.join("zeewin2", &who("a"), DAY).unwrap();
        assert_eq!(files(&dir.path().join("zeewin2")).len(), 1);
        drop(ticket);
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn a_dead_ticket_is_passed_and_removed_and_a_live_one_never() {
        let (dir, queues) = queues();
        let killed = queues.join("zeewin2", &who("killed"), DAY).unwrap();
        let alive = queues.join("zeewin2", &who("alive"), DAY + 2_000).unwrap();
        let waiting = queues
            .join("zeewin2", &who("waiting"), DAY + 3_000)
            .unwrap();
        // 31 s after the first was touched: six beats gone; the second's 29 s
        // are within its time.
        let later = DAY + 31_000;
        let ahead: Vec<String> = waiting
            .ahead(later)
            .into_iter()
            .map(|r| r.who.tree)
            .collect();
        assert_eq!(ahead, ["alive"]);
        assert!(!killed.path().exists(), "the dead ticket was taken away");
        assert!(alive.path().exists(), "the live one is left");
        assert_eq!(files(&dir.path().join("zeewin2")).len(), 2);
        drop(killed);
    }

    #[test]
    fn a_ticket_touched_ahead_of_the_clock_holds_no_one_back() {
        let (_dir, queues) = queues();
        // Came before this run, touched a minute ahead of the clock: a run
        // killed before the clock was set back.
        let killed = queues.join("zeewin2", &who("killed"), DAY - 5_000).unwrap();
        touch(killed.path(), "killed\n", DAY + 60_000).unwrap();
        let mine = queues.join("zeewin2", &who("mine"), DAY).unwrap();
        assert!(mine.path().file_name() > killed.path().file_name());
        assert_eq!(mine.ahead(DAY), []);
        assert_eq!(queues.depth("zeewin2", DAY), 1);
        // 10 s ahead is two processes' clocks a little apart: alive.
        let near = queues.join("zeewin", &who("near"), DAY - 5_000).unwrap();
        touch(near.path(), "near\n", DAY + 10_000).unwrap();
        let after = queues.join("zeewin", &who("after"), DAY).unwrap();
        assert_eq!(after.ahead(DAY).len(), 1);
    }

    #[test]
    fn a_run_that_comes_after_the_clock_was_set_back_is_still_behind() {
        let (_dir, queues) = queues();
        let there = queues.join("zeewin2", &who("there"), DAY).unwrap();
        // The clock goes back five seconds; the new run is named after the one there.
        let late = queues.join("zeewin2", &who("late"), DAY - 5_000).unwrap();
        let ahead: Vec<String> = late
            .ahead(DAY - 4_000)
            .into_iter()
            .map(|r| r.who.tree)
            .collect();
        assert_eq!(ahead, ["there"]);
        drop(there);
    }

    #[test]
    fn a_ticket_taken_away_while_its_run_lives_is_made_again_in_its_place() {
        let (_dir, queues) = queues();
        let first = queues.join("zeewin2", &who("a"), DAY).unwrap();
        let second = queues.join("zeewin2", &who("b"), DAY + 1_000).unwrap();
        fs::remove_file(second.path()).unwrap();
        assert_eq!(second.ahead(DAY + 2_000).len(), 1, "a still goes first");
        assert!(second.path().exists(), "b's ticket is back under its name");
        drop(first);
    }

    #[test]
    fn a_run_keeps_its_ticket_alive_by_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let queues = Queues::at(dir.path(), Duration::from_millis(20));
        let ticket = queues
            .join("zeewin2", &who("a"), now_ms() - 3_600_000)
            .unwrap();
        let until = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let touched = fs::metadata(ticket.path()).unwrap().modified().unwrap();
            let touched_ms =
                i64::try_from(touched.duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap();
            if now_ms() - touched_ms < 5_000 {
                break;
            }
            assert!(std::time::Instant::now() < until, "never touched");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn what_a_ticket_says_is_its_tree_and_its_command_cut_short() {
        let who = Who::of(
            Path::new("/Users/x/Projects/kluris-queue"),
            "cargo xtask linux --clippy",
        );
        assert_eq!(who.tree, "kluris-queue");
        assert_eq!(who.command, "cargo xtask linux --clippy");
        let long = Who::of(Path::new("/k"), &"x".repeat(100));
        assert_eq!(long.command.chars().count(), 80);
        assert!(long.command.ends_with('…'));
    }
}

/// A ticket as ConsensFlow's runner writes it, and a Kluris ticket as
/// ConsensFlow's runner reads it: `tools/xtask/src/windows/queue.rs` in the
/// ConsensFlow repository at commit adc8fb3d24e92766e051310035867c82b5287088
/// (`Queues::join`, `touch`, `tickets`, `Entry::who`).
mod consensflow {
    use super::*;

    /// ConsensFlow's `Queues::join` at adc8fb3: the folder
    /// `<queues>/<machine>`, the name `format!("{at:015}-{:010}.ticket",
    /// std::process::id())`, the text `format!("{}\n{}\n", who.tree,
    /// who.command)` written with `create_new`, then its mtime set to the
    /// moment (`touch`). Written by hand here, not with Kluris's code.
    fn consensflow_ticket(root: &Path, machine: &str, at: i64, pid: u32) -> PathBuf {
        let folder = root.join(machine);
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{at:015}-{pid:010}.ticket"));
        fs::write(
            &path,
            "consensflow\ncargo xtask windows --machine zeewin2 -- cargo test -p cf-store\n",
        )
        .unwrap();
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_millis(u64::try_from(at).unwrap()))
            .unwrap();
        path
    }

    #[test]
    fn a_consensflow_ticket_holds_a_kluris_run_back() {
        let (dir, queues) = queues();
        let theirs = consensflow_ticket(dir.path(), "zeewin2", DAY, 31_456);
        let mut pace = Slow::at(DAY + 1_000);
        // The ConsensFlow run beats for 25 s, then its ticket goes as its run ends.
        pace.on_pause = Box::new(move |n, now| {
            if n < 3 {
                let file = fs::OpenOptions::new().write(true).open(&theirs).unwrap();
                file.set_modified(UNIX_EPOCH + Duration::from_millis(u64::try_from(now).unwrap()))
                    .unwrap();
            } else {
                let _ = fs::remove_file(&theirs);
            }
        });
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut [go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket true)"#);
        assert_eq!(pace.pauses, [10_000, 10_000, 10_000]);
        assert_eq!(
            said,
            [
                "waiting for zeewin2, 0 s of up to 60 min: 2nd in the queue for zeewin2; \
              the run ahead is consensflow (cargo xtask windows --machine zeewin2 -- cargo test -p cf-store), here for 1 s"
            ]
        );
    }

    #[test]
    fn a_kluris_ticket_reads_under_consensflows_rules() {
        let (dir, queues) = queues();
        let older = consensflow_ticket(dir.path(), "zeewin2", DAY, 7);
        let ours = queues
            .join(
                "zeewin2",
                &Who::of(Path::new("/p/kluris-queue"), "cargo xtask linux --clippy"),
                DAY + 1,
            )
            .unwrap();
        let name = ours
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // `<15 digits>-<10 digits>.ticket`: what `tickets` parses (the part
        // before the first `-`) and sorts by name.
        let (came, rest) = name.split_once('-').unwrap();
        let process = rest.strip_suffix(".ticket").unwrap();
        assert_eq!((came.len(), process.len()), (15, 10), "{name}");
        assert!(
            came.bytes()
                .chain(process.bytes())
                .all(|b| b.is_ascii_digit())
        );
        assert_eq!(came.parse::<i64>().unwrap(), DAY + 1);
        assert_eq!(process.parse::<u32>().unwrap(), std::process::id());
        let mut names = files(&dir.path().join("zeewin2"));
        names.sort();
        assert_eq!(
            names,
            [
                older.file_name().unwrap().to_string_lossy().into_owned(),
                name.clone()
            ]
        );
        // Two lines, read by `Entry::who` as the tree and the command.
        assert_eq!(
            fs::read_to_string(ours.path()).unwrap(),
            "kluris-queue\ncargo xtask linux --clippy\n"
        );
        // Its touch is the moment it came: alive to a ConsensFlow reader.
        let touched = fs::metadata(ours.path()).unwrap().modified().unwrap();
        assert_eq!(
            touched.duration_since(UNIX_EPOCH).unwrap().as_millis(),
            u128::try_from(DAY + 1).unwrap()
        );
    }
}

mod waits {
    use super::*;

    #[test]
    fn a_run_with_no_one_ahead_and_a_ready_machine_goes_at_once_holding_its_place() {
        let (dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut [go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket true)"#);
        assert!(said.is_empty() && pace.pauses.is_empty());
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn two_runs_go_first_come_first_served() {
        let (dir, queues) = queues();
        let mut say = |_: &str| Ok(());
        let mut first_pace = Slow::at(DAY);
        let (mut first, _) = Turn::take(
            Some(&queues),
            ZEEWIN2,
            &who("first"),
            60,
            &mut first_pace,
            &mut say,
        )
        .unwrap();
        let mut second_pace = Slow::at(DAY + 1_000);
        let (mut second, _) = Turn::take(
            Some(&queues),
            ZEEWIN2,
            &who("second"),
            60,
            &mut second_pace,
            &mut say,
        )
        .unwrap();
        // The second is behind the first, and the machine is not asked.
        second_pace.stop_after = Some(1);
        let mut said = Vec::new();
        let waited = second
            .wait(
                &mut || panic!("the machine is asked only at the head"),
                &mut second_pace,
                &mut |line| {
                    said.push(line.to_owned());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(waited, Waited::Stopped);
        assert!(said[0].contains("2nd in the queue for zeewin2; the run ahead is first"));
        // The first goes, and holds its place while it runs.
        assert_eq!(
            first.wait(&mut go, &mut first_pace, &mut say).unwrap(),
            Waited::Go(vec!["on it".into()])
        );
        assert_eq!(files(&dir.path().join("zeewin2")).len(), 2);
        drop(first);
        second_pace.stop_after = None;
        assert_eq!(
            second.wait(&mut go, &mut second_pace, &mut say).unwrap(),
            Waited::Go(vec!["on it".into()])
        );
        drop(second);
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn a_wait_that_runs_out_ends_saying_how_long_and_what_for() {
        let (dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            3,
            &mut std::iter::repeat_with(|| busy("zeewin2 is busy: poker-lab's job holds it")),
            &mut pace,
        );
        assert_eq!(
            ended,
            "Refused(zeewin2 is still busy after waiting 3 min (as far as --wait 3 goes): \
             zeewin2 is busy: poker-lab's job holds it)"
        );
        // Asked after 30 s, then every minute; said at the start and each minute.
        assert_eq!(pace.pauses, [30_000, 60_000, 60_000, 60_000]);
        assert_eq!(said.len(), 3, "{said:?}");
        assert_eq!(
            said[0],
            "waiting for zeewin2, 0 s of up to 3 min: zeewin2 is busy: poker-lab's job holds it"
        );
        assert!(said[1].starts_with("waiting for zeewin2, 1 min of up to 3 min: "));
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn what_holds_the_run_is_said_when_it_changes() {
        let (_dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut [busy("poker-lab"), busy("short of disk"), go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket true)"#);
        assert_eq!(
            said,
            [
                "waiting for zeewin2, 0 s of up to 60 min: poker-lab",
                "waiting for zeewin2, 30 s of up to 60 min: short of disk",
            ]
        );
    }

    #[test]
    fn wait_0_asks_once_takes_no_ticket_and_refuses_as_before() {
        let (dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            0,
            &mut [busy("zeewin2 is short of disk")].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, "Refused(zeewin2 is short of disk)");
        assert!(said.is_empty() && pace.pauses.is_empty());
        assert!(!dir.path().join("zeewin2").exists(), "no ticket was taken");
        let (ended, _) = waited(
            Some(&queues),
            ZEEWIN2,
            0,
            &mut [go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket false)"#);
    }

    #[test]
    fn wait_0_is_refused_behind_a_live_ticket_and_never_asks() {
        let (_dir, queues) = queues();
        let queued = queues.join("zeewin2", &who("there"), DAY).unwrap();
        let mut pace = Slow::at(DAY + 1_000);
        let (ended, _) = waited(
            Some(&queues),
            ZEEWIN2,
            0,
            &mut std::iter::empty(),
            &mut pace,
        );
        assert_eq!(ended, "Refused(zeewin2 is busy: 1 run is queued for it)");
        // A Linux run says whose queue it is.
        let (ended, _) = waited(Some(&queues), ZEELIN, 0, &mut std::iter::empty(), &mut pace);
        assert_eq!(
            ended,
            "Refused(zeelin is busy: 1 run is queued for it (zeewin2's queue: the same laptop))"
        );
        // A dead one does not count.
        let mut later = Slow::at(DAY + 31_000);
        let (ended, _) = waited(
            Some(&queues),
            ZEEWIN2,
            0,
            &mut [go()].into_iter(),
            &mut later,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket false)"#);
        drop(queued);
    }

    #[test]
    fn a_machine_that_is_gone_ends_the_wait_at_once_and_is_never_waited_on() {
        let (dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            Some(&queues),
            ZEELIN,
            60,
            &mut [Ok(Look::Gone("zeelin not running".into()))].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, "Refused(zeelin not running)");
        assert!(said.is_empty() && pace.pauses.is_empty());
        // Nor later in the wait.
        let (ended, said) = waited(
            Some(&queues),
            ZEELIN,
            60,
            &mut [
                busy("poker-lab"),
                Ok(Look::Gone("zeelin not running".into())),
            ]
            .into_iter(),
            &mut pace,
        );
        assert_eq!(ended, "Refused(zeelin not running)");
        assert_eq!(said.len(), 1);
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn a_linux_run_queues_in_zeewin2s_folder_behind_a_windows_run() {
        let (dir, queues) = queues();
        let windows = queues.join("zeewin2", &who("consensflow"), DAY).unwrap();
        let mut pace = Slow::at(DAY);
        pace.stop_after = Some(1);
        let folder = dir.path().join("zeewin2");
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen_in = std::rc::Rc::clone(&seen);
        pace.on_pause = Box::new(move |_, _| seen_in.borrow_mut().push(files(&folder).len()));
        let (ended, said) = waited(
            Some(&queues),
            ZEELIN,
            60,
            &mut std::iter::empty(),
            &mut pace,
        );
        assert_eq!(ended, "Stopped");
        assert_eq!(
            said,
            [
                "waiting for zeelin, 0 s of up to 60 min: 2nd in the queue for zeewin2; \
              the run ahead is consensflow (cargo xtask windows in consensflow), here for 0 s"
            ]
        );
        // Both tickets were in zeewin2's folder while it waited; its own went
        // with it, the Windows run's stays.
        assert_eq!(*seen.borrow(), [2]);
        assert_eq!(files(&dir.path().join("zeewin2")).len(), 1);
        assert!(windows.path().exists());
        assert!(!dir.path().join("zeelin").exists());
    }

    #[test]
    fn a_refusal_by_the_machines_lock_keeps_the_place_and_waits_on() {
        let (dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let mut said = Vec::new();
        let mut say = |line: &str| {
            said.push(line.to_owned());
            Ok(())
        };
        let (mut turn, _) = Turn::take(
            Some(&queues),
            ZEEWIN2,
            &who("mine"),
            60,
            &mut pace,
            &mut say,
        )
        .unwrap();
        let mut answers = [go(), go()].into_iter();
        let mut ready = || answers.next().unwrap();
        assert_eq!(
            turn.wait(&mut ready, &mut pace, &mut say).unwrap(),
            Waited::Go(vec!["on it".into()])
        );
        let ticket = turn.ticket().unwrap().path().to_path_buf();
        turn.refused_there("the lock is held by a ConsensFlow run".into());
        assert_eq!(
            turn.wait(&mut ready, &mut pace, &mut say).unwrap(),
            Waited::Go(vec!["on it".into()])
        );
        assert_eq!(pace.pauses, [30_000]);
        assert!(ticket.exists(), "the same ticket, its place kept");
        drop(turn);
        assert_eq!(
            said,
            ["waiting for zeewin2, 0 s of up to 60 min: the lock is held by a ConsensFlow run"]
        );
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn refusals_by_the_machines_lock_back_off_to_a_minute() {
        let (_dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let mut say = |_: &str| Ok(());
        let (mut turn, _) = Turn::take(
            Some(&queues),
            ZEEWIN2,
            &who("mine"),
            60,
            &mut pace,
            &mut say,
        )
        .unwrap();
        let mut ready = || go();
        assert_eq!(
            turn.wait(&mut ready, &mut pace, &mut say).unwrap(),
            Waited::Go(vec!["on it".into()])
        );
        // Each refusal sends the tree again at the next look: after the first,
        // those looks are a minute apart, as at a busy machine.
        for _ in 0..3 {
            turn.refused_there("the lock is held by a ConsensFlow run".into());
            assert_eq!(
                turn.wait(&mut ready, &mut pace, &mut say).unwrap(),
                Waited::Go(vec!["on it".into()])
            );
        }
        assert_eq!(pace.pauses, [30_000, 60_000, 60_000]);
    }

    #[test]
    fn a_stop_ends_the_wait_and_the_ticket_goes() {
        let (dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        pace.stop_after = Some(2);
        let (ended, _) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut std::iter::repeat_with(|| busy("busy")),
            &mut pace,
        );
        assert_eq!(ended, "Stopped");
        assert_eq!(files(&dir.path().join("zeewin2")), Vec::<String>::new());
    }

    #[test]
    fn a_machine_that_cannot_be_asked_first_is_the_error_and_later_is_said() {
        let (_dir, queues) = queues();
        let mut pace = Slow::at(DAY);
        let (ended, _) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut [Err("no answer in 45s".to_owned())].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, "Error(no answer in 45s)");
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut [busy("busy"), Err("no answer in 45s".to_owned()), go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket true)"#);
        assert_eq!(
            said[1],
            "waiting for zeewin2, 30 s of up to 60 min: zeewin2 could not be asked: no answer in 45s"
        );
        let (ended, _) = waited(
            Some(&queues),
            ZEEWIN2,
            1,
            &mut [
                busy("busy"),
                Err("no answer".to_owned()),
                Err("no answer".to_owned()),
            ]
            .into_iter(),
            &mut pace,
        );
        assert_eq!(
            ended,
            "Error(zeewin2 could not be asked in 1 min of waiting (--wait 1): no answer)"
        );
    }

    #[test]
    fn with_no_queue_a_run_waits_for_the_machine_alone() {
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            None,
            ZEEWIN2,
            60,
            &mut [busy("busy"), go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket false)"#);
        assert_eq!(said.len(), 1);
    }

    #[test]
    fn a_queue_that_cannot_be_kept_is_said_and_the_run_waits_for_the_machine_alone() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the queues' folder should be.
        let blocked = dir.path().join("queues");
        fs::write(&blocked, "").unwrap();
        let queues = Queues::at(&blocked, Duration::from_secs(3600));
        let mut pace = Slow::at(DAY);
        let (ended, said) = waited(
            Some(&queues),
            ZEEWIN2,
            60,
            &mut [go()].into_iter(),
            &mut pace,
        );
        assert_eq!(ended, r#"Go(["on it"], ticket false)"#);
        assert!(
            said[0].starts_with("no place could be kept in the queue for zeewin2 ("),
            "{said:?}"
        );
    }

    #[test]
    fn ordinals_and_times() {
        let words: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101].map(ordinal).to_vec();
        assert_eq!(
            words,
            [
                "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st"
            ]
        );
        assert_eq!(minutes_of(59_999), "59 s");
        assert_eq!(minutes_of(60_000), "1 min");
        assert_eq!(minutes_of(-5), "0 s");
        assert_eq!(queued_words(2), "2 runs are queued for it");
    }
}
