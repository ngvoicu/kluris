//! The bash zeelin is sent, as text, so the tests can read it and run it. Each
//! goes on ssh's command line as `bash -c '<script>'` (zee's login shell is
//! bash, and Linux takes a command line of megabytes).
//!
//! - The probe ([`probe`]) says, before anything is sent, whether a run may go:
//!   it looks at Kluris's own one-run lock (`flock -n` on it, held for no
//!   longer than that), poker-lab's `~/.compile-lock/owner` and zeewin2's run
//!   record seen from the Linux side, and refuses with 75 naming the holder.
//! - The bootstrap ([`bootstrap`]) runs in a session of its own (`setsid`, so
//!   the stop can end all of it): it takes Kluris's lock with `flock -n` for
//!   the whole run, asks the probe's questions again under it, writes the
//!   run's record, unpacks the tarball and sources the run script that came in
//!   it. Its `trap` ends every other process of the session, removes the
//!   tarball and the record, however the run ends; the lock goes with its
//!   descriptor.
//! - The run script ([`run_script`]) rides in the tarball: the task cap
//!   (`ulimit -u`), old runs swept, then each step under `timeout`.
//! - The stop ([`stop_script`]) ends a run that was stopped on this side or
//!   whose connection was lost: a session without a terminal is not ended by
//!   sshd when its connection goes.
//!
//! The probe and the bootstrap end their output with `== exit N`.

use crate::ssh_ci::outcome::{EXIT_MARK, STEP_MARK, TREE_MARK};
use crate::ssh_ci::plan::{
    self, CARGO_TEST, CI_DIR, PARITY, RELEASE_BUILD, TOOLCHAIN, TREE_FILE, UV_VERSION, fill,
    tarball_name,
};

/// The ssh host: the laptop's Ubuntu under WSL2.
pub const HOST: &str = "zeelin";
/// `CARGO_BUILD_JOBS` there: the laptop's 16 threads are poker-lab's and the
/// owner's too, so a run takes half, as zeewin2's runs do.
pub const JOBS: u32 = 8;
/// The most tasks (processes and threads: Linux counts both, for all of the
/// user's processes) while a run goes. Cargo's jobs, rustc's threads, the test
/// harness's threads and the processes the tests start stay well within it; a
/// run that forks without end stops there.
pub const TASK_LIMIT: u32 = 1024;

/// The run script, at the root of the tarball.
pub const RUN_FILE: &str = ".kluris-run.sh";
/// Kluris's one-run lock on zeelin, under [`CI_DIR`], held with `flock`.
pub const LOCK: &str = ".linux-run.lock";
/// The holder's run id and process (its session), two lines, beside the lock.
pub const OWNER: &str = ".linux-run.id";
/// Where tarballs are sent, under [`CI_DIR`].
pub const INCOMING: &str = "incoming";

/// uv's Linux tarball ([`UV_VERSION`]), fetched into Kluris's tools folder
/// when the machine has none there; its hash is GitHub's `.sha256` of it.
pub const UV_SHA256: &str = "2e2b37e9811e17675a9e70bed5e1a58fc8c0388be63d751d72cc735188c149ff";
pub const UV_URL: &str =
    "https://github.com/astral-sh/uv/releases/download/0.12.8/uv-x86_64-unknown-linux-gnu.tar.gz";

/// What zeelin's neighbours leave where a run looks before it goes.
#[derive(Debug, Clone)]
pub struct Layout {
    /// poker-lab's lock, from the home: a run goes only while it is not there.
    pub poker_lab: String,
    /// The Windows user's profile, as WSL mounts it: where zeewin2's record is.
    pub windows_home: String,
    /// The record of the run holding zeewin2 (ConsensFlow's or Kluris's
    /// `cargo xtask windows`, two lines with CRLF: the run id, the process).
    pub windows_owner: String,
}

impl Layout {
    pub fn zeelin() -> Self {
        Self {
            poker_lab: ".compile-lock/owner".into(),
            windows_home: "/mnt/c/Users/AGV".into(),
            windows_owner: "/mnt/c/Users/AGV/consensflow-build/.windows-run.id".into(),
        }
    }
}

/// What a run is asked for.
#[derive(Debug, Clone, Copy)]
pub struct Spec<'a> {
    /// The tree's `rust-toolchain.toml` channel.
    pub channel: &'a str,
    /// `CARGO_BUILD_JOBS` there.
    pub jobs: u32,
    /// `ulimit -u` for the run.
    pub tasks: u32,
    pub parity: bool,
    /// The `parity/*.py` files `--parity-files` named (already checked); empty:
    /// the whole suite.
    pub parity_files: &'a [String],
    /// Appended to `cargo test --workspace --no-fail-fast`.
    pub cargo_args: &'a [String],
}

/// `text` as bash takes it between single quotes.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn quote_all(words: &[String]) -> String {
    words
        .iter()
        .map(|word| quote(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The run's tarball there, from the home, as scp names it.
pub fn remote_tarball(id: &str) -> String {
    format!("{CI_DIR}/{INCOMING}/{}", tarball_name(id))
}

pub fn probe_line(layout: &Layout) -> String {
    format!("bash -c {}", quote(&probe(layout)))
}

/// The bootstrap in a session of its own, named after the run (the stop
/// checks the name before it ends the session).
pub fn bootstrap_line(id: &str, layout: &Layout) -> String {
    format!(
        "exec setsid -w bash -c {} {id}",
        quote(&bootstrap(id, layout))
    )
}

pub fn stop_line(id: &str) -> String {
    format!("bash -c {}", quote(&stop_script(id)))
}

pub fn cleanup_line(id: &str) -> String {
    format!("bash -c {}", quote(&cleanup_script(id)))
}

fn prelude() -> String {
    fill(
        PRELUDE,
        &[
            ("exit", EXIT_MARK),
            ("ci", CI_DIR),
            ("lock", LOCK),
            ("owner", OWNER),
        ],
    )
}

fn checks(layout: &Layout) -> String {
    fill(
        CHECKS,
        &[
            ("poker_lab", &layout.poker_lab),
            ("poker_lab_q", &quote(&layout.poker_lab)),
            ("windows_home_q", &quote(&layout.windows_home)),
            ("windows_owner_q", &quote(&layout.windows_owner)),
        ],
    )
}

pub fn probe(layout: &Layout) -> String {
    fill(
        PROBE,
        &[
            ("prelude", &prelude()),
            ("incoming", INCOMING),
            ("checks", &checks(layout)),
        ],
    )
}

pub fn bootstrap(id: &str, layout: &Layout) -> String {
    fill(
        BOOTSTRAP,
        &[
            ("prelude", &prelude()),
            ("id", &quote(id)),
            ("incoming", INCOMING),
            ("tarball", &tarball_name(id)),
            ("checks", &checks(layout)),
            ("tree_mark", TREE_MARK),
            ("tree_file", TREE_FILE),
            ("run_file", RUN_FILE),
        ],
    )
}

pub fn run_script(spec: &Spec) -> String {
    let toolchain: Vec<String> = [
        spec.channel,
        "--profile",
        "minimal",
        "--component",
        "clippy",
        "--component",
        "rustfmt",
        "--no-self-update",
    ]
    .map(str::to_owned)
    .to_vec();
    // Every test binary runs, so one failing binary does not hide the others'.
    let mut test: Vec<String> = ["test", "--workspace", "--no-fail-fast"]
        .map(str::to_owned)
        .to_vec();
    test.extend(spec.cargo_args.iter().cloned());
    let parity = if spec.parity {
        fill(
            PARITY_TEMPLATE,
            &[
                ("uv_version", UV_VERSION),
                ("uv_url", UV_URL),
                ("uv_sha256", UV_SHA256),
                ("release", &RELEASE_BUILD.minutes.to_string()),
                ("parity", &PARITY.minutes.to_string()),
                ("pytest", &quote_all(&plan::pytest_args(spec.parity_files))),
            ],
        )
    } else {
        String::new()
    };
    fill(
        RUN_TEMPLATE,
        &[
            ("tasks", &spec.tasks.to_string()),
            ("ci", CI_DIR),
            ("incoming", INCOMING),
            ("jobs", &spec.jobs.to_string()),
            ("step_mark", STEP_MARK),
            ("toolchain_minutes", &TOOLCHAIN.minutes.to_string()),
            ("toolchain", &quote_all(&toolchain)),
            ("test_minutes", &CARGO_TEST.minutes.to_string()),
            ("test", &quote_all(&test)),
            ("parity_steps", &parity),
        ],
    )
}

pub fn stop_script(id: &str) -> String {
    fill(
        STOP,
        &[
            ("ci", CI_DIR),
            ("owner", OWNER),
            ("id", &quote(id)),
            ("incoming", INCOMING),
            ("tarball", &tarball_name(id)),
        ],
    )
}

/// Removes the run's own tarball after a failed upload, and says `removed`,
/// `absent` or `failed: <why>`. The name is the run's alone, so no lock is taken.
pub fn cleanup_script(id: &str) -> String {
    fill(
        CLEANUP,
        &[
            ("ci", CI_DIR),
            ("incoming", INCOMING),
            ("tarball", &tarball_name(id)),
        ],
    )
}

/// What the probe and the bootstrap begin with: the exit line, the refusal,
/// Kluris's folder and lock, and the lock's holder as its record names it.
const PRELUDE: &str = r#"set -euo pipefail
exit_run() { printf '\n«exit»%s\n' "$1"; exit "$1"; }
refuse() { printf '%s\n' "$1"; exit_run 75; }
ci="${HOME:?}/«ci»"
lock="$ci/«lock»"
owner="$ci/«owner»"
holder() {
  if [ -f "$owner" ]; then
    printf 'a Kluris run (%s, process %s)' "$(sed -n 1p "$owner")" "$(sed -n 2p "$owner")"
  else
    printf 'a run that left no record'
  fi
}"#;

/// poker-lab's lock and zeewin2's record: only looked at, never taken, broken
/// or waited on. A Windows side out of sight could be running a build, so it
/// refuses too.
const CHECKS: &str = r#"poker_lab=«poker_lab_q»
windows_home=«windows_home_q»
windows_owner=«windows_owner_q»
if [ -e "${HOME:?}/$poker_lab" ]; then
  refuse "zeelin is busy: poker-lab's job holds it (~/«poker_lab»), and no run goes while it does: not waiting"
fi
if [ ! -d "$windows_home" ]; then
  refuse "zeelin cannot see zeewin2's side ($windows_home is not there), so a build there cannot be ruled out: not going"
fi
if [ -e "$windows_owner" ]; then
  of_id=$(sed -n 1p "$windows_owner" 2>/dev/null | tr -d '\r') || of_id=''
  of_pid=$(sed -n 2p "$windows_owner" 2>/dev/null | tr -d '\r') || of_pid=''
  case $of_id in
    kluris-*) who="a Kluris run ($of_id, process $of_pid)" ;;
    '') who='a run that left no record' ;;
    *) who="a ConsensFlow run ($of_id, process $of_pid)" ;;
  esac
  refuse "the laptop is busy: $who holds zeewin2 ($windows_owner), and the laptop runs one build at a time: not waiting"
fi"#;

const PROBE: &str = r#"«prelude»
mkdir -p "$ci/«incoming»"
if [ -e "$lock" ] && ! flock -n "$lock" true; then
  refuse "zeelin's one-run lock is held by $(holder): one run at a time, not waiting"
fi
«checks»
exit_run 0
"#;

/// The bootstrap, with `$$` its session (`setsid`) and the record's process.
const BOOTSTRAP: &str = r#"«prelude»
id=«id»
tarball="$ci/«incoming»/«tarball»"
run="$ci/runs/$id"
held=0
cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  for pid in $(pgrep -s "$$" 2>/dev/null || true); do
    if [ "$pid" != "$$" ]; then kill -KILL "$pid" 2>/dev/null || true; fi
  done
  rm -f -- "${tarball:?}"
  if [ "$held" = 1 ]; then rm -f -- "${owner:?}"; fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -p "$ci/runs"
exec 9>>"$lock"
if ! flock -n 9; then
  refuse "zeelin's one-run lock is held by $(holder): one run at a time, not waiting"
fi
«checks»
printf '%s\n%s\n' "$id" "$$" > "$owner"
held=1
mkdir -p "$run"
tar -xzmf "$tarball" -C "$run" || exit_run "$?"
printf '«tree_mark»%s\n' "$(head -n 1 "$run/«tree_file»")"
cd "$run"
. "$run/«run_file»"
printf 'the run script ended without its exit line\n'
exit_run 1
"#;

/// The run script, sourced by the bootstrap with `$run` its folder and `$id`
/// its name.
///
/// - `ulimit -u` caps the tasks while the run goes; each step runs under
///   `timeout` for its minutes (then TERM, and KILL 30 s later), its status 124
///   when it was ended.
/// - The tools are the user's rustup by its full path, with Kluris's own
///   `RUSTUP_HOME`, `CARGO_HOME`, target, cache and temp folders; PATH changes
///   for this process only, and no profile is read or written.
/// - A machine without rustup is told about and not fixed.
const RUN_TEMPLATE: &str = r#"if ! ulimit -u «tasks» 2>/dev/null; then
  printf '== no task cap (ulimit -u «tasks» failed): no run starts without its bound\n'
  exit_run 1
fi
printf '== limits: at most «tasks» tasks (processes and threads) of this user while the run goes; each step bounded in time\n'
ci="${HOME:?}/«ci»"
tools="$ci/tools"
cache="$ci/cache"
tmp="$run/tmp"
mkdir -p "$tools" "$cache" "$tmp"
while IFS= read -r old; do
  rm -rf -- "${old:?}"
  if [ -e "$old" ]; then
    printf '== sweep: could not remove runs/%s\n' "${old##*/}"
  else
    printf '== sweep: removed runs/%s\n' "${old##*/}"
  fi
done < <(find "$ci/runs" -mindepth 1 -maxdepth 1 -type d ! -name "$id" -mmin +1440)
find "$ci/«incoming»" -mindepth 1 -maxdepth 1 -type f -name 'kluris-*.tgz' -mmin +1440 -exec rm -f -- {} + || true
rustup="${HOME:?}/.cargo/bin/rustup"
cargo="${HOME:?}/.cargo/bin/cargo"
if [ ! -x "$rustup" ] || [ ! -x "$cargo" ]; then
  printf 'Missing on zeelin: rustup for this user (%s and %s). Kluris installs no Rust there: install rustup at the machine, then run again.\n' "$rustup" "$cargo"
  exit_run 1
fi
export RUSTUP_HOME="$tools/rustup"
export CARGO_HOME="$tools/cargo"
export CARGO_TARGET_DIR="$cache/target"
export CARGO_BUILD_JOBS='«jobs»'
export CARGO_TERM_COLOR=never
export TMPDIR="$tmp"
export XDG_CACHE_HOME="$cache/xdg"
export PATH="${HOME:?}/.cargo/bin:$PATH"
failed=0
last=0
step() {
  local name=$1 minutes=$2 started=$SECONDS code=0
  shift 2
  printf '«step_mark»%s: %s (at most %s min)\n' "$name" "$*" "$minutes"
  timeout --kill-after=30s "${minutes}m" "$@" </dev/null || code=$?
  if [ "$code" -eq 124 ]; then
    printf '«step_mark»%s: still running after %s min, ended\n' "$name" "$minutes"
  fi
  last=$code
  if [ "$code" -ne 0 ] && [ "$failed" -eq 0 ]; then failed=$code; fi
  printf '«step_mark»%s exit %s %ss\n' "$name" "$code" "$((SECONDS - started))"
}
step toolchain «toolchain_minutes» "$rustup" toolchain install «toolchain»
if [ "$failed" -ne 0 ]; then exit_run "$failed"; fi
step cargo-test «test_minutes» "$cargo" «test»
«parity_steps»exit_run "$failed"
"#;

/// The parity steps: uv fetched (its hash checked) into the tools folder when
/// it is not there, a release `kluris`, then the harness, with the oracle's
/// venv, uv's cache and Python kept in Kluris's folder.
const PARITY_TEMPLATE: &str = r#"uvdir="$tools/uv-«uv_version»"
uv="$uvdir/uv"
if [ ! -x "$uv" ]; then
  started=$SECONDS
  part="$tools/uv-«uv_version».tar.gz"
  unpacked="$tools/uv-«uv_version».part"
  rm -rf -- "${uvdir:?}" "${unpacked:?}"
  if ! curl --fail --silent --show-error --location --max-time 300 --output "$part" '«uv_url»'; then
    printf '== step uv: could not fetch uv «uv_version»\n'
  else
    sha=$(sha256sum "$part" | cut -d ' ' -f 1)
    if [ "$sha" != '«uv_sha256»' ]; then
      printf '== step uv: the tarball hashes to %s, not «uv_sha256»\n' "$sha"
    elif mkdir -p "$unpacked" && tar -xzf "$part" -C "$unpacked" --strip-components=1 && [ -x "$unpacked/uv" ]; then
      mv -- "$unpacked" "$uvdir"
    else
      printf '== step uv: could not unpack uv «uv_version»\n'
    fi
  fi
  rm -rf -- "${unpacked:?}" "${part:?}"
  code=0
  if [ ! -x "$uv" ]; then code=1; fi
  last=$code
  if [ "$code" -ne 0 ] && [ "$failed" -eq 0 ]; then failed=$code; fi
  printf '== step uv exit %s %ss\n' "$code" "$((SECONDS - started))"
fi
if [ -x "$uv" ]; then
  export UV_CACHE_DIR="$cache/uv"
  export UV_PYTHON_INSTALL_DIR="$tools/python"
  export UV_PYTHON_BIN_DIR="$tools/python-bin"
  export UV_PYTHON_PREFERENCE=only-managed
  export UV_PROJECT_ENVIRONMENT="$cache/parity-venv"
  export KLURIS_PARITY_ORACLE_DIR="$cache/oracle"
  export KLURIS_V3_BIN="$CARGO_TARGET_DIR/release/kluris"
  export PATH="$uvdir:$PATH"
  step release-build «release» "$cargo" 'build' '--release' '--package' 'kluris-cli'
  if [ "$last" -eq 0 ]; then step parity «parity» "$uv" «pytest»; fi
fi
"#;

/// The stop: the run's session ended only if the record names this run and
/// that process is this run's bootstrap, then the record and the tarball
/// removed. It ends with 0 whether or not there was anything left to end.
const STOP: &str = r#"set -euo pipefail
ci="${HOME:?}/«ci»"
owner="$ci/«owner»"
if [ -f "$owner" ] && [ "$(sed -n 1p "$owner")" = «id» ]; then
  sid=$(sed -n 2p "$owner")
  case $sid in
    ''|*[!0-9]*) ;;
    *)
      if tr '\0' ' ' 2>/dev/null < "/proc/$sid/cmdline" | grep -qF -- «id»; then
        pkill -KILL -s "$sid" || true
      fi
      ;;
  esac
  rm -f -- "${owner:?}"
fi
rm -f -- "$ci/«incoming»/«tarball»"
exit 0
"#;

/// The cleanup of a failed upload: the run's tarball and nothing else.
const CLEANUP: &str = r#"set -euo pipefail
tarball="${HOME:?}/«ci»/«incoming»/«tarball»"
if [ ! -e "$tarball" ]; then echo absent; exit 0; fi
if why=$(rm -f -- "$tarball" 2>&1); then echo removed; else echo "failed: $why"; fi
exit 0
"#;
