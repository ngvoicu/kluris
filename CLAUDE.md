# Kluris 3 (branch `v3`)

Branch `v3` is the Kluris 3 Rust rewrite. Kluris 2 (Python, PyPI `kluris`) lives
on `main`; its v2.28.11 tag is the frozen reference for everything here.

Kluris turns AI agents into team subject matter experts by giving them shared,
human-curated knowledge stored in git-backed **brains**. Kluris 3 keeps every v2
command except `pack` and reads and writes existing brains, configs and agent
skills exactly as v2 does.

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
vendored/             # the four Specmint companion folders (unchanged from v2)
parity/               # uv project: the v2.28.11 oracle vs the v3 binary
dist-workspace.toml   # dist 0.33.1: targets, installers, archives (see Release)
about.toml, about.hbs # cargo-about config and template of THIRD_PARTY_LICENSES.md
scripts/release/      # release tooling: pinned tools, licence report, local build, installer smoke
.github/workflows/    # ci.yml, parity.yml, release.yml (dist's own)
```

Rules: synchronous code, no async runtime; no `unwrap()`/`expect()` on user
data, errors are values; cross-platform (`std::path`, Unix-only APIs behind
`cfg`).

## Build & Test

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
KLURIS_V3_BIN=target/release/kluris uv run --project parity pytest parity -q
```

The parity harness builds the oracle venv (Python 3.11, `parity/oracle.lock`,
hash-pinned) once into `parity/.oracle/` and reuses it while the lock is
unchanged. Without `KLURIS_V3_BIN` every parity test fails (it never skips).

## Windows machines (`cargo xtask windows`)

Kluris 3's tests and parity harness run on the two household Windows
machines over ssh, the way ConsensFlow's runner does (`xtask/src/windows.rs`
has the whole design):

```bash
cargo xtask windows --machine zeewin --parity     # cargo test --workspace --no-fail-fast, then parity
cargo xtask windows --machine zeewin2 -- -p kluris-core   # extra cargo test args after --
cargo xtask windows --parity                      # zeewin2 if free and allowed, else zeewin
cargo xtask windows --parity --parity-files parity/test_smoke.py   # only that parity file
cargo xtask windows --machine zeewin --installer-smoke --wrapper ../sites/kluris-site/install.ps1
```

- **Machines.** `zeewin` (AGV-COMPUTE1, `ssh zeewin`, short of memory: builds
  run with 2 jobs) and `zeewin2` (the owner's ASUS laptop, `ssh zeewin2`, 8
  jobs). Facts: zeespirevpn-sme (`devices/agv-compute1.md`,
  `devices/agvlaptop.md`) and consensflow-sme (`operations/test-machines.md`).
- **What goes.** `git archive` of HEAD, or of a `git stash create` commit when
  tracked files have uncommitted changes (untracked files do not go), without
  `target/`, `parity/.oracle/`, `parity/.venv/`.
- **One run per machine, shared with ConsensFlow.** A run takes ConsensFlow's
  lock, `%USERPROFILE%\consensflow-build\.windows-run.lock` (opened for the run
  alone, ConsensFlow's `.windows-run.pid`/`.windows-run.id` records beside it),
  before anything else. A held lock ends the command at once with **75**,
  naming the holder (a Kluris or a ConsensFlow run, its id and process): it
  never waits, takes or breaks it.
- **zeewin2 is refused (75)** while poker-lab's `~/.compile-lock/owner` exists
  on `zeelin` (a zeelin that does not answer is an Ubuntu that is not running,
  and the check passes) and while C: has under 50 GiB free. That lock is only
  looked at: never taken, broken or waited on. With no `--machine`, a refused
  or busy zeewin2 sends the run to zeewin.
- **Nothing on the machine changes.** No Windows settings, Defender, firewall,
  registry or PATH (user or machine) changes, no logins, no system-wide
  installs. Everything Kluris adds lives in `%USERPROFILE%\.kluris-ci\`:
  `tools\rustup` (`RUSTUP_HOME`: the tree's pinned toolchain, installed by the
  machine's own `~\.cargo\bin\rustup.exe` called by full path), `tools\cargo`
  (`CARGO_HOME`), `tools\uv-0.12.8\uv.exe` (fetched once, SHA-256 checked),
  `tools\python` (uv's managed CPython), `cache\target` (`CARGO_TARGET_DIR`:
  registry deps stay built; the workspace crates rebuild in each run's new
  folder), `cache\oracle` (`KLURIS_PARITY_ORACLE_DIR`), `cache\parity-venv`,
  `cache\uv`, and `runs\<run id>\` (the unpacked tree, its `tmp\`). PATH is
  prepended for the run's own process only. A machine without rustup (or the
  VS Build Tools linker) is reported, not fixed. The one exception is
  `--installer-smoke`, whose test is the installer's user PATH edit: the
  smoke saves `HKCU\Environment\Path` (bytes and registry type,
  `scripts\release\user-path.ps1`, into `runs\<id>\user-path.saved`),
  writes it back however it ends (the run does again after a step ended past
  its bound, and so does the stop) and compares it byte for byte; the system
  PATH is only read. kluris.exe and the receipt go to a sandbox profile.
- **Bounds and cleanup.** The run is in a job object: at most 64 processes,
  all ended when the run ends. Each step has its own bound (toolchain 30 min,
  cargo-test 90, release-build 60, parity 90; a step past it is ended, status
  124), the whole run one more (this side kills ssh and sends the stop), and
  every probe a few seconds. The bootstrap's `finally` always removes the
  run's tarball and its lock records and closes the lock (after a failed
  upload, a 30-second ssh with no lock removes that run's own tarball and says
  whether it could); run folders older
  than a day and stray `kluris-*.tgz` are swept by the next run, under the lock.
  Ctrl-C ends the run there too (130).
- **The answer.** The output streams back; the status is the machine's last
  line `== exit N` (ssh to PowerShell answers 1 for every failure), then a
  summary line with each step's status and time.
- **Parity on Windows.** v2.28.11 runs there as the oracle (the hash-pinned
  `parity/oracle.lock` has the `win_amd64` wheels) against the release
  `kluris.exe` (`KLURIS_V3_BIN`). The harness reads CRLF in stdout/stderr as
  LF (PAR-004). `--parity-files parity/test_x.py[,parity/test_y.py]`
  (repeatable, needs `--parity`) runs `pytest` on those files alone instead
  of the whole suite: each must be a `parity/*.py` file in this worktree,
  checked before anything is sent.

## Linux machine (`cargo xtask linux`)

The same tests and parity harness run on `zeelin`, the household laptop's
Ubuntu 26.04 under WSL2 (the same laptop as zeewin2), over ssh
(`xtask/src/linux.rs` has the whole design; the upload, the streamed run and
the stop are shared with `windows` in `xtask/src/ssh_ci/`):

```bash
cargo xtask linux --parity                        # cargo test --workspace --no-fail-fast, then parity
cargo xtask linux -- -p kluris-core               # extra cargo test args after --
cargo xtask linux --parity --parity-files parity/test_smoke.py   # only that parity file
cargo xtask linux --installer-smoke --wrapper ../sites/kluris-site/install.sh
```

- **The machine.** `ssh zeelin` (10.6.0.6:22, user `zee`, bash; 16 threads,
  27 GiB). It answers only while WSL runs (a scheduled task keeps it up while
  the owner is logged in). It is poker-lab's compute box first. Facts:
  zeespirevpn-sme (`devices/agvlaptop.md`) and consensflow-sme
  (`operations/test-machines.md`). Builds run with 8 jobs.
- **Refused with 75, nothing sent**, by a bounded probe (5 s to connect, 30 s
  in all) and again by the bootstrap under its lock: zeelin not answering
  ("zeelin not running": it is never started from here; an ssh that connected
  but could not say is an error, 1), poker-lab's `~/.compile-lock/owner`
  present, a run holding zeewin2 (its record
  `/mnt/c/Users/AGV/consensflow-build/.windows-run.id`, written by ConsensFlow's
  and Kluris's `cargo xtask windows`, or `/mnt/c/Users/AGV` out of sight), and
  Kluris's own one-run lock `~/.kluris-ci/.linux-run.lock` held (`flock -n`,
  the holder's id and process in `.linux-run.id` beside it). Each refusal
  names the holder. The other projects' locks are only looked at: never
  taken, broken or waited on. They are checked when the run starts: a
  poker-lab job or a Windows run that starts later runs beside it.
- **What goes.** The same tarball as for Windows (HEAD, or a `git stash
  create` commit with the uncommitted tracked changes; not `target/`,
  `parity/.oracle/`, `parity/.venv/`), by scp to
  `~/.kluris-ci/incoming/<run id>.tgz`.
- **Nothing outside `~/.kluris-ci/` changes.** No `sudo`, no system packages,
  no shell profiles, nothing of poker-lab's. `tools/rustup` (`RUSTUP_HOME`:
  the tree's pinned toolchain, installed by the user's own
  `~/.cargo/bin/rustup` called by full path; `~/.cargo/env` is never read),
  `tools/cargo` (`CARGO_HOME`), `tools/uv-0.12.8/uv` (fetched once, SHA-256
  checked), `tools/python` (uv's managed CPython), `cache/target`
  (`CARGO_TARGET_DIR`), `cache/oracle` (`KLURIS_PARITY_ORACLE_DIR`),
  `cache/parity-venv`, `cache/uv`, `cache/xdg`, and `runs/<run id>/` (the
  unpacked tree, its `tmp/` as `TMPDIR`). PATH changes for the run's own
  process only. A zeelin without rustup is reported, not fixed.
- **Bounds and cleanup.** The bootstrap runs in a session of its own
  (`setsid`) under `set -euo pipefail`; its `trap` ends every other process of
  the session and removes the tarball and the record however the run ends,
  and the lock goes with its descriptor. `ulimit -u 1024` caps the user's
  tasks while the run goes (Linux counts threads and every process of `zee`,
  so it is a ceiling against runaway forks, not the Windows job's 64). Each
  step has the Windows bounds (toolchain 30 min, cargo-test 90, release-build
  60, parity 90) under `timeout` (TERM, then KILL 30 s later; status 124), and
  the whole run one more. Ctrl-C (130), the bound (124) and a lost
  connection send the stop, which ends the run's session only if the record
  names that run. Run folders and tarballs older than a day are swept by the
  next run, under the lock.
- **The answer.** As on Windows: the output streams back, the status is the
  run's last line `== exit N`, then a summary line `linux: zeelin run <id>
  tree <commit>: <step> <status> (<time>), ...; exit N`.
- **Parity on Linux.** v2.28.11 runs there as the oracle (the hash-pinned
  universal `parity/oracle.lock`) against the release `kluris`
  (`KLURIS_V3_BIN`); `--parity-files` as for Windows.
- **zeelin is WSL2, not native Linux.** v2's and v3's WSL code paths run
  there (`mri` prints `file://wsl.localhost/<distro>/...` links, built with
  `wslpath -w`; `parity/test_mri.py` expects that form on a WSL host), so
  behaviour only native Linux shows is first covered by GitHub's Ubuntu
  runners at release.

## Release (dist)

Kluris 3 ships from GitHub Releases only (no PyPI, crates.io, Homebrew or
npm; both crates are `publish = false`), built by dist 0.33.1 from
`dist-workspace.toml`: eight targets (macOS arm64/x86_64, Linux x86_64 and
aarch64 as gnu and static musl, Windows x86_64/arm64), tar.gz/zip archives
with a `.sha256` each and one `sha256.sum`, GitHub attestations, and
`kluris-installer.sh` / `kluris-installer.ps1` into `~/.kluris/bin` (no
updater: `kluris self-update`). dist names everything after the Cargo
package, hence the package `kluris`.

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
  the PowerShell one checks no checksum, prepends to `HKCU\Environment\Path`
  as REG_EXPAND_SZ and writes the receipt to `%LOCALAPPDATA%\kluris\`.
- **release.yml is generated:** change `dist-workspace.toml`, then
  `dist generate`; `dist plan` fails while it is stale.
- **Its tag trigger is dist's own** (`'**[0-9]+.[0-9]+.[0-9]+*'`, decided
  2026-10-26), not `v3.*`: dist cannot generate `v3.*` (`tag-namespace =
  "v3"` makes a glob that misses `v3.0.0`), and a hand edit would turn off
  its drift check. v2 and v3 stay apart by tree instead, as GitHub runs the
  workflows of the tagged commit: v2.29.0 is tagged on a Python commit
  before `main` becomes Rust and later v2 fixes on branch `v2`, which has no
  release.yml, so they run v2's publish.yml; this tree has no publish.yml,
  so a v3 tag can never reach PyPI; a v2 tag pushed onto a v3 commit makes
  `dist plan` fail ("nothing to release"), never a release.
  `scripts/release/check_workflows.sh` (ci.yml) fails if publish.yml, or any
  workflow publishing to PyPI, comes back.
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
- **Site wrappers** live in `../sites/kluris-site` (`install.sh`,
  `install.ps1`): thin pass-throughs to `releases/latest/download/` or
  `releases/download/v$KLURIS_VERSION/`; a push there deploys the site.

## Parity rule

Byte parity with v2.28.11 for `--json` output, exit codes, files written into
brains, `~/.kluris/config.yml`, brain `kluris.yml` and SKILL.md files.
Human-readable (non-JSON) output keeps v2's wording, not its exact layout.
Every deliberate difference goes into `parity/allowlist.md` with the decision
it comes from; an unlisted difference is a bug.

## Design decisions

They live in the spec at
`/Users/gabrielvoicu/Projects/ngvoicu/kluris/.specs/kluris-v3-rust/` (local to
the `main` checkout, not in git): `SPEC.md` plus the research files
(`research-03-v2-contract.md` is the v2 command contract with file:line
references into v2.28.11). The v2 source on `main` is the read-only reference.
