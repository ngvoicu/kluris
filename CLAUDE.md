# Kluris

Kluris turns AI agents into team subject matter experts by giving them shared,
human-curated knowledge stored in git-backed **brains**. It is a Rust CLI,
`kluris`: it creates and registers brains, installs an agent skill per brain
(with the Specmint companion playbooks a brain opts into), wakes up and
searches a brain for an agent, keeps its maps and links healthy (`dream`,
`mri`), commits approved files, and syncs every brain it opens with its
remote through one session (fetch, merge, push; `sync`, `resolve`, `commit`).
`main` holds the released Kluris; work goes on short-lived branches,
fast-forwarded into `main` at release.

## Knowledge base

Read and write to the **ngvoicu-sme** brain through kluris (never edit brain
files by hand). Use the `/kluris-ngvoicu-sme` skill — search, learn, remember,
create.

Topics relevant to this repo: kluris-cli overview, architecture, multi-brain resolver, agent skills, wake-up protocol, testing, CI/CD, deprecation.

## Layout

```
Cargo.toml            # workspace: crates/kluris-core, crates/kluris-cli, xtask (edition 2024)
rust-toolchain.toml   # pinned toolchain (rustfmt, clippy)
.cargo/config.toml    # `cargo xtask ...` alias
crates/kluris-core/   # library: every operation, typed inputs and outputs, no printing
crates/kluris-cli/    # package `kluris`, binary `kluris`: parses arguments, calls core, prints
xtask/                # dev tasks (cargo xtask; windows and linux runners share ssh_ci/)
vendored/             # the four Specmint companion folders (cargo xtask sync-specmint)
dist-workspace.toml   # dist 0.33.1: targets, installers, archives (see Release)
about.toml, about.hbs # cargo-about config and template of THIRD_PARTY_LICENSES.md
scripts/release/      # release tooling: pinned tools, licence report, local build, installer smoke, release notes
.github/workflows/    # ci.yml, release.yml (dist's own), release-notes.yml
```

Rules: synchronous code, no async runtime; no `unwrap()`/`expect()` on user
data, errors are values; cross-platform (`std::path`, Unix-only APIs behind
`cfg`).

## Build & Test

```bash
cargo build --release
INSTA_UPDATE=no cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

`INSTA_UPDATE=no` makes a missing or changed golden snapshot fail and
writes nothing (see "Golden tests").

## Windows machines (`cargo xtask windows`)

Kluris's tests run on the household Windows machine zeewin2 over ssh, the
way ConsensFlow's runner does (`xtask/src/windows.rs` has the whole design):

```bash
cargo xtask windows                               # cargo test --workspace --no-fail-fast
cargo xtask windows -- -p kluris-core             # extra cargo test args after --
cargo xtask windows --wait 0 --clippy             # ask once: 75 at once if busy or a run is queued
cargo xtask windows --installer-smoke --wrapper ../sites/kluris-site/install.ps1   # then the life cycle (see Release)
cargo xtask windows --installer-smoke --published 3.0.0-rc.1   # a GitHub release, live wrapper
cargo xtask windows --installer-smoke --published 3.0.0 --unpinned   # GitHub's latest, which must be 3.0.0
cargo xtask windows --clippy                      # cargo clippy --workspace --all-targets -- -D warnings, no tests
```

- **The machine.** `zeewin2` (the owner's ASUS laptop, `ssh zeewin2`, 8
  jobs), the one Windows machine: every run goes there, waiting in its
  queue. `--machine zeewin2` is accepted and changes nothing; any other
  `--machine` is a usage error (2). Facts: zeespirevpn-sme
  (`devices/agvlaptop.md`) and consensflow-sme (`operations/test-machines.md`).
- **What goes.** `git archive` of HEAD, or of a `git stash create` commit when
  tracked files have uncommitted changes (untracked files do not go), without
  `target/`, with the run script beside the files (and the site wrapper
  under `--wrapper`), sent by scp to `%USERPROFILE%\<run id>.tgz`.
- **The queue, shared with ConsensFlow.** A busy machine is waited for, first
  come first served, in the queue every runner on this Mac keeps: a folder
  per machine in `~/.cf-windows-queues/` (or `CF_WINDOWS_QUEUES`), a ticket
  per waiting or running run, `<arrival ms, 15 digits>-<pid, 10
  digits>.ticket`, holding two lines (the tree's folder name, the command).
  ConsensFlow's `cargo xtask windows` uses the same folder, names and rules
  (its `tools/xtask/src/windows/queue.rs` is the protocol;
  `xtask/src/ssh_ci/queue.rs` keeps it to the letter), so the two projects'
  runs are served in the order they came. A ticket is alive while its run
  touches it every 5 s; one untouched for 30 s (or touched more than 10 s
  ahead of the clock) is a killed run, and whoever looks removes it. A live
  ticket is removed only by its own run, when the run ends, however it ends
  (success, failure, the bound, Ctrl-C, a lost connection). A run goes when
  no older live ticket is there and the machine is ready, and holds its
  ticket while it runs. `--wait MINUTES` (60 unless given, at most 1440)
  bounds the wait; while waiting the run says its place and what holds the
  machine, each minute or when that changes, and a wait that runs out says
  how long it waited and what for, and ends with **75**. `--wait 0` asks
  once and takes no ticket: 75 at once if the machine is busy, and before
  asking it while a live run is queued for it.
- **One run per machine, shared with ConsensFlow.** A run takes ConsensFlow's
  lock, `%USERPROFILE%\consensflow-build\.windows-run.lock` (opened for the run
  alone, ConsensFlow's `.windows-run.pid`/`.windows-run.id` records beside it),
  before anything else. A held lock refuses the run with 75, naming the
  holder (a Kluris or a ConsensFlow run, its id and process); the bootstrap
  never waits on, takes or breaks it. The lock is the last word: a run it
  refuses at the head of the queue goes back to its place and waits on (with
  `--wait 0` the command ends with 75).
- **zeewin2 is busy** (asked only at the head of its queue) while poker-lab's
  `~/.compile-lock/owner` exists on `zeelin` (a zeelin that does not answer
  is an Ubuntu that is not running, and the check passes), while a Kluris
  Linux run holds zeelin's one-run lock `~/.kluris-ci/.linux-run.lock` (the
  same laptop; asked with `flock -n` as `cargo xtask linux` asks it, the
  holder named from `.linux-run.id`), and while C: has under 50 GiB free.
  Those locks are only looked at: never held or broken, and never waited on
  but through the queue.
- **Nothing on the machine changes.** No Windows settings, Defender, firewall,
  registry or PATH (user or machine) changes, no logins, no system-wide
  installs. Besides the run's tarball and the lock records in
  `%USERPROFILE%\consensflow-build\`, everything Kluris adds lives in
  `%USERPROFILE%\.kluris-ci\`:
  `tools\rustup` (`RUSTUP_HOME`: the tree's pinned toolchain, installed by the
  machine's own `~\.cargo\bin\rustup.exe` called by full path), `tools\cargo`
  (`CARGO_HOME`), `cache\target` (`CARGO_TARGET_DIR`: registry deps stay
  built; the workspace crates rebuild in each run's new folder), for
  `--installer-smoke` without `--published` `tools\uv-0.12.8\uv.exe`
  (fetched once, SHA-256 checked), `tools\python` (uv's managed CPython,
  which runs `scripts\release\serve.py`) and `cache\uv`, and `runs\<run
  id>\` (the unpacked tree, its `tmp\` as TEMP and TMP). PATH is prepended
  for the run's own process only. A machine without rustup (the user's
  `~\.cargo\bin\rustup.exe` and `cargo.exe`) is reported, not fixed; a
  missing VS Build Tools linker shows as a cargo failure. The one exception is
  `--installer-smoke`, whose test is the installer's user PATH edit: the
  smoke saves `HKCU\Environment\Path` (bytes and registry type,
  `scripts\release\user-path.ps1`, into `runs\<id>\user-path.saved`),
  writes it back however it ends (the run does again after a step ended past
  its bound, and so does the stop) and compares it byte for byte; the system
  PATH is only read. kluris.exe and the receipt go to a sandbox profile.
- **Bounds and cleanup.** The run is in a job object: at most 64 processes,
  all ended when the run ends. Each step has its own bound (toolchain 30
  min, cargo-test 90, clippy 60, python 10, release-dist 60, installer-smoke
  20, release-versions 60, lifecycle 20, published-smoke 20; a step past it
  is ended, status 124), the whole run their sum plus 20 minutes (this side
  kills ssh and sends the stop), and every probe and cleanup ssh its own
  bound of seconds (the C: free-space probe 45 s, the zeelin lock probe 15
  s). The bootstrap's `finally` always removes the run's tarball and its
  lock records and closes the lock (after a failed upload, a 30-second ssh
  with no lock removes that run's own tarball and says whether it could; the
  command ends with 1); the next run, under the lock, sweeps run folders
  older than a day and every folder in `cache\` that no run uses (all but
  `target` and `uv`), each said in a `== sweep:` line, and removes stray
  `kluris-*.tgz` older than a day without a line. Ctrl-C ends the run there
  too (130).
- **The answer.** The output streams back; the status is the machine's last
  line `== exit N` (ssh to PowerShell answers 1 for every failure), then a
  summary line on stderr, `windows: zeewin2 run <id> tree <commit, 12
  characters>: <step> <status> (<time>), ...; exit N`. A run that ends
  without an `== exit` line answers 1 (or ssh's own failure) and sends the
  stop. Each step's end line is
  `== step NAME exit CODE SECONDSs <run id>`, and the summary counts only the
  lines that end with this run's id, so step lines inside a step's output (a
  failing test printing what it captured) never reach it.
- **Goldens on Windows.** The run cannot set `INSTA_UPDATE` or bring files
  back, so a Windows variant is taken from insta's output in the log (see
  "Golden tests").

## Linux machine (`cargo xtask linux`)

The same tests run on `zeelin`, the household laptop's
Ubuntu 26.04 under WSL2 (the same laptop as zeewin2), over ssh
(`xtask/src/linux.rs` has the whole design; the upload, the streamed run and
the stop are shared with `windows` in `xtask/src/ssh_ci/`):

```bash
cargo xtask linux                                 # cargo test --workspace --no-fail-fast
cargo xtask linux -- -p kluris-core               # extra cargo test args after --
cargo xtask linux --installer-smoke --wrapper ../sites/kluris-site/install.sh   # then the life cycle (see Release)
cargo xtask linux --installer-smoke --published 3.0.0-rc.1   # a GitHub release, live wrapper
cargo xtask linux --installer-smoke --published 3.0.0 --unpinned   # GitHub's latest, which must be 3.0.0
cargo xtask linux --clippy                        # clippy -D warnings on every target, no tests
cargo xtask linux --wait 0 --clippy               # ask once: 75 at once if busy or a run is queued
```

- **The machine.** `ssh zeelin` (10.6.0.6:22, user `zee`, bash; 16 threads,
  27 GiB). It answers only while WSL runs (a scheduled task keeps it up while
  the owner is logged in). It is poker-lab's compute box first. Facts:
  zeespirevpn-sme (`devices/agvlaptop.md`) and consensflow-sme
  (`operations/test-machines.md`). Builds run with 8 jobs.
- **The queue: zeewin2's.** zeelin and zeewin2 are one laptop, so a Linux run
  waits its turn in zeewin2's folder of the queue the Windows runs and
  ConsensFlow's runner keep (`~/.cf-windows-queues/zeewin2`, or
  `$CF_WINDOWS_QUEUES/zeewin2`; the rules and `--wait` as in "Windows
  machines" above): it waits behind the zeewin2 runs of either project that
  came first, and theirs wait behind it. While it waits it says its place and
  what holds the laptop, each minute or when that changes; a wait that runs
  out ends with 75, and `--wait 0` asks once (75 at once while zeelin is busy
  or a run is queued for zeewin2).
- **Asked at the head of the queue, nothing sent until it may go**, by a
  bounded probe (5 s to connect, 30 s in all) and again by the bootstrap
  under its lock. zeelin not answering ends the command with **75** at once
  ("zeelin not running": it is never started from here, nor waited on; an
  ssh that connected but could not say is an error, 1, on the first look,
  and later is reported while the run waits on, ending with 1 if the wait
  runs out that way). Busy, waited for,
  each refusal naming the holder: poker-lab's `~/.compile-lock/owner`
  present, a run holding zeewin2 (its record
  `/mnt/c/Users/AGV/consensflow-build/.windows-run.id`, written by ConsensFlow's
  and Kluris's `cargo xtask windows`, or `/mnt/c/Users/AGV` out of sight), and
  Kluris's own one-run lock `~/.kluris-ci/.linux-run.lock` held (`flock -n`,
  the holder's id and process in `.linux-run.id` beside it). The bootstrap's
  75 is the last word: the run goes back to its place and waits on. The
  other projects' locks are only looked at: never taken or broken, and never
  waited on but through the queue. They are checked when the run starts: a
  poker-lab job that starts later runs beside it. A Windows run killed hard
  leaves its `.windows-run.id` behind, and Linux runs stay refused until a
  Windows run or a person removes it.
- **What goes.** The same tarball as for Windows (HEAD, or a `git stash
  create` commit with the uncommitted tracked changes; not `target/`), by
  scp to
  `~/.kluris-ci/incoming/<run id>.tgz`.
- **Nothing outside `~/.kluris-ci/` changes.** No `sudo`, no system packages,
  no shell profiles, nothing of poker-lab's. `tools/rustup` (`RUSTUP_HOME`:
  the tree's pinned toolchain, installed by the user's own
  `~/.cargo/bin/rustup` called by full path; `~/.cargo/env` is never read),
  `tools/cargo` (`CARGO_HOME`), `cache/target` (`CARGO_TARGET_DIR`),
  `cache/xdg` (`XDG_CACHE_HOME`), for the smokes (`--published` too)
  `tools/uv-0.12.8/uv` (fetched once, SHA-256 checked), `tools/python` (uv's
  managed CPython, which runs `scripts/release/serve.py` and the smokes'
  JSON checks) and `cache/uv`, and `runs/<run id>/` (the unpacked tree, its
  `tmp/` as `TMPDIR`). PATH changes for the run's own process only. A
  zeelin without rustup is reported, not fixed.
- **Bounds and cleanup.** The bootstrap runs in a session of its own
  (`setsid`) under `set -euo pipefail`; its `trap` ends every other process of
  the session and removes the tarball and the record however the run ends,
  and the lock goes with its descriptor. `ulimit -u 1024` caps the user's
  tasks while the run goes (Linux counts threads and every process of `zee`,
  so it is a ceiling against runaway forks, not the Windows job's 64). Each
  step has the Windows bounds (toolchain 30 min, cargo-test 90) under
  `timeout` (TERM, then KILL 30 s later; status 124), and
  the whole run their sum plus 20 minutes. Ctrl-C (130), the bound (124)
  and a lost connection send the stop, which ends the run's session only if
  the record names that run. The next run, under the lock, sweeps run
  folders older than a day and every folder in `cache/` that no run uses
  (all but `target`, `uv` and `xdg`), each said in a `== sweep:` line, and
  removes tarballs older than a day without a line.
- **The answer.** As on Windows: the output streams back, the status is the
  run's last line `== exit N`, then a summary line on stderr `linux: zeelin
  run <id> tree <commit, 12 characters>: <step> <status> (<time>), ...; exit
  N`, from the step lines that end with the run's id.
- **zeelin is WSL2, not native Linux.** Kluris's WSL code paths run
  there (`mri` prints `file://wsl.localhost/<distro>/...` links, built with
  `wslpath -w`; `golden_dream_mri.rs` expects that form on a WSL host, its
  `@wsl` snapshot), so
  behaviour only native Linux shows is first covered by GitHub's Ubuntu
  runners at release.

## Release (dist)

Kluris ships from GitHub Releases only (no PyPI, crates.io, Homebrew or
npm; both crates are `publish = false`), built by dist 0.33.1 from
`dist-workspace.toml`: eight targets (macOS arm64/x86_64, Linux x86_64 and
aarch64 as gnu and static musl, Windows x86_64/arm64), tar.gz/zip archives
with a `.sha256` each and one `sha256.sum`, GitHub attestations, and
`kluris-installer.sh` / `kluris-installer.ps1` into `~/.kluris/bin` (no
updater: `kluris self-update`). dist names everything after the Cargo
package, hence the package `kluris`.

- **Archive layouts** (what `kluris_core::dist::binary_in_archive` and
  self-update follow): a `.tar.gz` holds everything in a top-level
  `kluris-<target>/` folder; a Windows `.zip` is flat, `kluris.exe` at its
  root. `--version` is the package's version (clap's `version`), so a
  release prints its own.
- **What `self-update` picks:** a candidate is a release that is not a
  prerelease and carries this target's exact archive (`kluris-<target>.tar.gz`
  / `.zip`), so a release without binaries or built without this platform
  never is; `latest` is the newest candidate when it is newer than the running
  version, else the current version (`update_available` false). A prerelease
  installs only when named with `--version`.
- **Archives** hold `kluris`, `LICENSE`, `README.md`,
  `THIRD_PARTY_NOTICES.md` (ported code) and `THIRD_PARTY_LICENSES.md` (the
  crates built into `kluris`, from cargo-about). Auto-includes are off: dist
  resolves the crate's `license-file = "../../LICENSE"` from the wrong folder.
  After a dependency change run `scripts/release/licenses.sh` (CI runs it
  with `--check`; a crate under a license not in `about.toml` fails it).
- **What the installers do** (0.33.1, read from the generated scripts): the
  shell one checks the archive's SHA-256, writes `~/.config/kluris/env.sh`
  and `env.fish` beside the receipt `~/.config/kluris/kluris-receipt.json`,
  and adds `. "$HOME/.config/kluris/env.sh"` to `~/.profile` and `~/.zshrc`
  (or the existing bash/zsh rc files) plus fish's `conf.d/kluris.env.fish`;
  the PowerShell one checks no checksum itself (the site's `install.ps1`
  does it first, see "Checksums"), prepends to `HKCU\Environment\Path`
  as REG_EXPAND_SZ and writes the receipt to `%LOCALAPPDATA%\kluris\`
  (one line, CRLF, no BOM; `binaries` `["kluris.exe"]`, `install_prefix` in
  backslashes, the same text it puts first in PATH). It rebuilds the user
  PATH as `-split ';' -ne ''`, so empty entries (a trailing `;`) are gone and
  a REG_SZ value becomes REG_EXPAND_SZ; `kluris self-uninstall` then removes
  only its own entry and cannot bring those back.
- **release.yml is generated:** change `dist-workspace.toml`, then
  `dist generate`; `dist plan` fails while it is stale.
- **The release page leads with the site's install commands**
  (`post-announce-jobs = ["./release-notes"]` in `dist-workspace.toml`):
  after dist announces the release, `release.yml` calls
  `.github/workflows/release-notes.yml`, which runs
  `scripts/release/release_notes.py` on dist's plan and puts the result on
  the release with `gh release edit --notes-file`: dist's `## Install`
  section is swapped for the site's two short commands (and their
  `KLURIS_VERSION` forms), the download table and the attestation notes
  staying as dist wrote them. A body not laid out as dist 0.33.1 writes it
  makes the script exit 1 — the release then keeps dist's body. Tests:
  `scripts/release/test_release_notes.py` (fixture
  `fixtures/plan-3.0.0-rc.1.json`), run in ci.yml.
- **LF checkouts everywhere** (`.gitattributes` `* text=auto eol=lf`, ahead of
  the `-text` rules for byte-exact fixtures, templates and MRI assets).
  GitHub's Windows runners check out with `core.autocrlf=true`; the rule
  gives every checkout the committed bytes, so `build-versions.ps1` reads
  `Cargo.toml`'s version line and the Windows release embeds the same LF
  Specmint companions (`vendored/`, `include_dir!`) as every other target:
  the same tree the zeewin2/zeelin runs' `git archive` sends.
- **A release job that never got a runner is re-run, never re-tagged.**
  GitHub's capacity for some labels (macOS arm64 above all) can leave a
  build queued until it is cancelled ("The job was not acquired by Runner
  of type hosted even after multiple attempts"). `gh run rerun <id> -R
  ngvoicu/kluris --failed` runs it and the jobs after it again in the same
  run. Deleting and re-pushing a tag is for a broken release only.
- **Its tag trigger is dist's own** (`'**[0-9]+.[0-9]+.[0-9]+*'`): dist
  cannot generate a narrower glob (`tag-namespace = "v3"` makes one that
  misses `v3.0.0`), and a hand edit would turn off its drift check.
- **Tools** (`scripts/release/tool.sh dist|cargo-about|actionlint`): the
  pinned prebuilt binary, its SHA-256 checked, in `target/tools/`; nothing is
  installed globally.
- **Checks** (ci.yml runs each): `dist plan --output-format=json` through
  `scripts/release/check_dist_plan.py`; `scripts/release/build-local.sh`
  (`.ps1` on Windows) builds this machine's release in release.yml's order
  (the archive's manifest beside it, so the shell installer embeds its
  SHA-256) into `target/distrib`; `scripts/release/installer-smoke.sh
  [--wrapper <site install.sh>] target/distrib` (`.ps1` on Windows) installs
  it from a local server in a sandbox HOME, and checks the rc lines, a fresh
  login shell, `kluris --version` and `kluris doctor --json`.
- **A published release, for real** (`cargo xtask windows|linux
  --installer-smoke --published <version> [--unpinned]`, which builds
  nothing; on the Mac `sh scripts/release/published-smoke.sh [--unpinned]
  <version>`): `scripts/release/published-smoke.sh` (`.ps1` on Windows)
  installs that GitHub release through the live site wrapper
  (`KLURIS_VERSION=<version>`; with `--unpinned`/`-Unpinned` no
  `KLURIS_VERSION`, so GitHub's latest, which must be `<version>`)
  in a sandbox HOME or profile, checks `kluris --version`, `kluris doctor
  --json` and `kluris self-update --check --json` (no update), then
  `kluris self-uninstall --yes`: the installer's files go and the sandbox's
  own rc files come back byte for byte. On Windows the user PATH is saved,
  always written back and compared byte for byte, as for the smoke.
- **The installed life cycle** (`cargo xtask windows|linux --installer-smoke`
  runs it after the installer smoke): `scripts/release/build-versions.sh
  target/releases 3.0.0 3.0.1` (`.ps1`: `-Versions 3.0.0,3.0.1`, one word)
  builds the tree as two releases that differ only in version, and
  `scripts/release/lifecycle-smoke.sh target/releases 3.0.0 3.0.1` (`.ps1`
  on Windows) serves them with a fake GitHub release API (`serve.py`; plain
  http is allowed to loopback in `KLURIS_GITHUB_API_URL`), installs 3.0.0
  with its own installer, makes a brain, a companion and skills, then
  checks `self-update --check --json`, `self-update --json` (the binary is
  the new archive's, only the receipt's `version` changes), `--version`,
  and `self-uninstall --yes --json` (exactly the installer's files and PATH
  entry go; every other file, the seeded rc files included, is
  byte-identical); then a KLURIS_NO_MODIFY_PATH=1 install over the user's
  own env.sh and profile line (Windows: the user PATH), and on Unix the
  uninstalls the pre-flight must block (an rc file it cannot rewrite, a
  read-only folder in a purged ~/.kluris, a read-only or unsearchable
  fish conf.d, on macOS an immutable receipt): exit 1, every file under
  the sandbox HOME byte-identical, then a rerun that finishes. ci.yml's
  installer-smoke job runs it on the three OSes (`check_workflows.sh` fails
  if that goes).
- **A receipt authorises nothing unless it is Kluris's**:
  `Receipt::read` (so `self-update` and `self-uninstall` alike) accepts only
  dist 0.33.1's receipt for Kluris: `source` kluris/kluris/ngvoicu/github,
  `provider.source` cargo-dist, `binaries` exactly `["kluris"]`
  (`["kluris.exe"]`), no cdylibs, cstaticlibs or aliases, `install_layout`
  flat, an absolute `install_prefix`, a boolean `modify_path`; anything
  else is "The receipt at <path> is not a Kluris install receipt; nothing
  was changed (<why>)", exit 1. The one file removed or replaced is
  `<install_prefix>/kluris(.exe)`.
- **`self-uninstall` checks everything before the first change**
  (`dist/preflight.rs`): the whole plan (rc edits, the fish file, env
  scripts, under `--purge` every generated skill and ~/.kluris, the
  binary, the receipt, their emptied folders) is built from reads alone,
  where only NotFound means "not there"; `preflight()` then checks it
  without changing anything: rc files readable and writable and their
  real folder (a link's target's) writable, every folder an entry leaves
  writable and searchable, every folder of a removed tree readable,
  writable and searchable (`faccessat` with `AT_EACCESS`, no probe
  writes), no `uchg`/`schg`/append flag on macOS, no read-only attribute on
  a Windows file. Anything in the way: exit 1, nothing changed, each
  blocker in `blocked` (`{path, problem, fix}` in the JSON). Not checked:
  Windows ACLs and Linux `chattr` flags.
- **The user's folder links stay**: a folder the uninstall would
  remove (the bin folder, the receipt's folder, under `--purge` ~/.kluris)
  that is a link (or a Windows junction) is never removed, nor is the
  folder it points to; what Kluris put in it goes, and the link is in
  `kept`. A purge through a linked ~/.kluris checks and removes each entry
  of its folder, nested links not followed; a link inside a plain
  ~/.kluris goes with it, never its target. The life cycles install into
  linked folders (Unix: ~/.config/kluris, ~/.kluris/bin, a linked
  ~/.kluris purged; Windows: junctioned `.kluris\bin` and
  `LOCALAPPDATA\kluris`).
- **`self-uninstall`'s order keeps what a retry needs**: with `modify_path`
  false (KLURIS_NO_MODIFY_PATH=1) no rc file, env script, fish file or PATH
  entry is touched. Otherwise: the rc lines first (a failure stops
  everything), then the fish conf.d line, env.sh/env.fish and the Windows
  PATH entry (a failure stops before the binary), then the binary, the
  receipt last. env.sh/env.fish go only when they are byte for byte what
  dist writes (`write_env_script_sh`/`_fish` around `$HOME/.kluris/bin`);
  one the user changed is kept, with the lines sourcing it, and listed in
  `kept`. A failure the pre-flight could not see stops before the binary
  and a rerun finishes; once the binary is gone (by then no shell file
  sources a removed script and the PATH entry is gone) the rest still
  goes, exit 1, and `left` says what to remove by hand ("remove the file
  <receipt>", "remove the empty folder ~/.kluris/bin").
- **Checksums.** dist's shell installer verifies the archive with
  `sha256sum` only and silently skips it without one (macOS 14 and older;
  Apple added `sha256sum` in macOS 15). The site's `install.sh` then puts
  a temp `sha256sum` that is `exec shasum -a 256 "$@"` first on PATH for
  the installer alone, and refuses with neither command;
  `installer-smoke.sh --wrapper` tests it with `sha256sum` off PATH and a
  corrupted archive. dist's PowerShell installer checks no checksum at
  all, so the site's `install.ps1` checks the archive before it runs it:
  it reads the installer's own release from its `$app_version` line, picks
  this machine's `kluris-<target>.zip` as the installer does (x86_64 or
  arm64 from `OSArchitecture`), downloads it and its `.zip.sha256` from
  where the installer would (`KLURIS_DOWNLOAD_URL`,
  `INSTALLER_DOWNLOAD_URL`, the `KLURIS_INSTALLER_GHE_BASE_URL` /
  `_GITHUB_BASE_URL` forms, else `<KLURIS_RELEASES_URL>/download/v<release>`),
  compares their SHA-256, and only then runs the installer with
  `KLURIS_DOWNLOAD_URL` set to the `file://` URL of the temp folder that
  holds the checked zip (dist's supported override; it is given back as it
  was afterwards). A mismatch ("kluris: <zip> does not match its published
  SHA-256, so nothing was installed") or a `.sha256` it cannot download
  throws before the installer runs: nothing installed, user PATH
  untouched. `installer-smoke.ps1 -Wrapper` tests both refusals (no
  `.kluris`, no receipt, the user PATH byte for byte as before) and that
  each install fetches exactly the installer, the `.sha256` and the zip.
  `kluris self-update` checks `sha256.sum` itself.
- **Site wrappers** live in `../sites/kluris-site` (`install.sh`,
  `install.ps1`): thin pass-throughs to `releases/latest/download/` or
  `releases/download/v$KLURIS_VERSION/` (`install.ps1` checks the archive
  before the installer runs, see "Checksums"); a push there deploys the site.
- **The site says what the binary does**: `scripts/release/check_site.sh
  <site dir>` (the binary from `KLURIS_BIN`, else this tree's release build)
  fails, naming each hit, on a retired word on a page (`*.html`,
  `README.md`: the watch mode, the sync service, a major version after the
  product's name or as a `v` word, Python packaging), on the site's
  `AGENTS.md` differing from its `CLAUDE.md`, and on a `kluris <command>
  --flag` in a page's code that `kluris help --json` or that command's
  `--help` does not know; it prints what its parse misses. Tests:
  `scripts/release/test_check_site.py`. ci.yml's Linux installer-smoke job
  runs it on the `site` checkout, kluris-site's default branch, with the
  dist build it made: a push of `main` checks the published site, so a
  release pushes the site before `main`.

## Format contract

What Kluris writes and prints for programs and agents is stable: `--json`
output, exit codes, files written into brains, `~/.kluris/config.yml`, a
brain's `kluris.yml` and SKILL.md files. The golden tests are their spec. A
change to a golden is a decision, made on purpose and said in the release
notes.

## Golden tests

Kluris's own output is pinned by golden tests: `insta` snapshots
(`crates/*/tests/snapshots/`) of `--json` output, exit codes, both streams,
files written into brains, `~/.kluris/config.yml`, brain `kluris.yml` and
SKILL.md files, beside frozen corpora read row by row
(`crates/kluris-core/tests/fixtures/corpus/` for the YAML and float formats,
`xtask/fixtures/sync-specmint/`). A snapshot holds the same bytes on every
machine and every day: the helpers in `crates/kluris-cli/tests/common/`
replace only the temp root (`<ROOT>`), the binary's version and today's date
with markers, and dump files so that every byte shows (a CR, a BOM, a byte
that is not UTF-8): each entry is a `== PATH (KIND, N bytes)` header and its
escaped body, no body line can read as a header or marker, and an `<end of
dump>` line closes the dump, so two different trees never dump alike. A
run's snapshot is framed the same way: each stream a `== stdout (N bytes)` /
`== stderr (N bytes)` section with its body escaped as a file's, then `==
exit N` and an `<end of run>` line, so no stream reads as another stream,
the exit or a marker. A test that asserts nothing changed compares the byte
maps (`tree_files`). A path Kluris prints or writes in the OS's own form (a
brain's, the config's) is checked whole in that form on the raw output, read
in the one form the text has (printed text as is, a JSON document's parsed
strings, a dumped YAML or JSON file's parsed values), and shown `/`-joined
(`native_paths`): such a path written partly with `/` or with doubled
backslashes fails. The temp root is read the same way (`normalize` takes the
text's form): it becomes `<ROOT>` as the OS writes it or `/`-joined, in that
form's own spelling only; any other spelling fails where the OS writes the
root with `\`, and elsewhere stays as it is in the snapshot. One snapshot
then holds on every OS and a separator change still fails on Windows. A
snapshot has an `@windows` (or `@wsl`) variant only for a real difference of
the OS: Windows lists a folder without regard to case, a test makes no
symlinks there, the OS words its own errors.

- **Checking.** `INSTA_UPDATE=no cargo test --workspace` fails on a missing
  or changed snapshot and writes nothing; CI runs the tests the same way
  (insta writes nothing where `CI` is set). A normalization is never widened
  to make a snapshot pass.
- **Changing output.** A change to what Kluris prints or writes is a change
  to its snapshots, made on purpose: write them with `INSTA_UPDATE=always
  cargo test -p <crate> --test <file>` (cargo-insta is not used), read every
  changed `.snap` before committing, and say in the commit what changed and
  why; the release notes say it too. The review reads the `.snap` diff as
  the behaviour change; a snapshot that changed without a reason in the
  commit is a bug.
- **On Windows and Linux.** `cargo xtask windows|linux` runs the goldens
  there with `INSTA_UPDATE` unset (insta writes only `.snap.new` files, which
  stay on the machine) and brings back only the log: a Windows variant is the
  new snapshot insta prints there, written as the `@windows` file, then run
  again until every golden passes on that machine.

## Design decisions

They live in the spec at
`/Users/gabrielvoicu/Projects/ngvoicu/kluris/.specs/kluris-3-2-sync/` (local
to the `main` checkout, not in git): `SPEC.md`.
