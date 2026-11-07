//! Golden tests of what `kluris doctor` says of each brain's sync, in text
//! and in `--json`: doctor asks every brain's remote each time, and says
//! each brain's sync once, on its check's line. Three brains with git:
//! - `pulled`: a teammate pushed a commit, which doctor pulls (PASS);
//! - `silent`: its remote does not answer within doctor's 3 s (an
//!   upload-pack that sleeps first), so doctor says so, and how long reads
//!   now skip it (WARN);
//! - `unreachable`: its ssh says at once that the host refused the
//!   connection (a script, so the line is the same on every OS), and
//!   doctor gives that line (WARN).

mod common;

use std::fs;
use std::path::Path;

use common::PINNED_GIT_DATE;
use common::upgrade::World;

const BRAINS: [&str; 3] = ["pulled", "silent", "unreachable"];

/// What the `unreachable` brain's ssh says.
const CONNECTION_REFUSED: &str = "ssh: connect to host fakehost port 22: Connection refused";

/// A path as git's shell takes it (Git for Windows' sh takes `/`).
fn sh_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// The world: each brain committed and pushed to its own bare remote in
/// `work/remotes/`, then set up as the module docs say.
fn world() -> World {
    let world = World::with_brains(&BRAINS);
    let git = |cwd: &Path, args: &[&str]| world.git(cwd, args, PINNED_GIT_DATE);
    let remotes = world.work.join("remotes");
    fs::create_dir_all(&remotes).expect("remotes");
    let mut config = Vec::new();
    for name in BRAINS {
        let brain = world.brain(name);
        fs::create_dir_all(&brain).expect("brain");
        fs::write(
            brain.join("kluris.yml"),
            format!("name: {name}\ndescription: d\n"),
        )
        .expect("kluris.yml");
        fs::write(brain.join("brain.md"), format!("# {name}\n")).expect("brain.md");
        world.init(&brain);
        world.commit(&brain, "init", PINNED_GIT_DATE);
        let bare = remotes.join(format!("{name}.git"));
        let bare_arg = bare.display().to_string();
        git(&remotes, &["init", "-q", "--bare", "-b", "main", &bare_arg]);
        git(&brain, &["remote", "add", "origin", &bare_arg]);
        git(&brain, &["push", "-q", "-u", "origin", "main"]);
        config.push((name, brain));
    }
    // A teammate's commit waits on `pulled`'s remote.
    let mate = world.work.join("mate");
    let mate_arg = mate.display().to_string();
    let bare = remotes.join("pulled.git").display().to_string();
    git(&world.work, &["clone", "-q", &bare, &mate_arg]);
    fs::write(mate.join("notes.md"), "# Notes\n").expect("notes.md");
    world.commit(&mate, "notes", PINNED_GIT_DATE);
    git(&mate, &["push", "-q"]);
    // `silent`'s remote sleeps before it answers.
    let script = world.work.join("sleeping-upload-pack.sh");
    fs::write(
        &script,
        "#!/bin/sh\nsleep 10\nexec git upload-pack \"$@\"\n",
    )
    .expect("script");
    let pack = format!("sh '{}'", sh_path(&script));
    git(
        &world.brain("silent"),
        &["config", "remote.origin.uploadpack", &pack],
    );
    // `unreachable`'s ssh says the host refused the connection.
    let script = world.work.join("ssh-down.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho '{CONNECTION_REFUSED}' >&2\nexit 255\n"),
    )
    .expect("script");
    let ssh = format!("sh '{}'", sh_path(&script));
    let unreachable = world.brain("unreachable");
    git(&unreachable, &["config", "core.sshCommand", &ssh]);
    git(&unreachable, &["config", "ssh.variant", "ssh"]);
    git(
        &unreachable,
        &["remote", "set-url", "origin", "fakehost:brain.git"],
    );
    let config: Vec<(&str, &Path, &str)> = config
        .iter()
        .map(|(name, brain)| (*name, brain.as_path(), ""))
        .collect();
    world.write_config(&config);
    world
}

#[test]
fn doctor_says_each_brains_sync_on_its_line() {
    let world = world();
    let out = world.kluris(&["doctor", "--no-refresh"], &[]);
    insta::assert_snapshot!("doctor_sync_text", world.render(&out));
}

#[test]
fn doctor_json_says_each_brains_sync_in_its_check() {
    let world = world();
    let out = world.kluris(&["doctor", "--no-refresh", "--json"], &[]);
    insta::assert_snapshot!("doctor_sync_json", world.render(&out));
}
