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
crates/kluris-cli/    # binary `kluris`: parses arguments, calls core, prints
xtask/                # dev tasks (cargo xtask)
vendored/             # the four Specmint companion folders (unchanged from v2)
parity/               # uv project: the v2.28.11 oracle vs the v3 binary
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
cargo xtask windows --machine zeewin --parity     # cargo test --workspace, then parity
cargo xtask windows --machine zeewin2 -- -p kluris-core   # extra cargo test args after --
cargo xtask windows --parity                      # zeewin2 if free and allowed, else zeewin
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
  VS Build Tools linker) is reported, not fixed.
- **Bounds and cleanup.** The run is in a job object: at most 64 processes,
  all ended when the run ends. Each step has its own bound (toolchain 30 min,
  cargo-test 90, release-build 60, parity 90; a step past it is ended, status
  124), the whole run one more (this side kills ssh and sends the stop), and
  every probe a few seconds. The bootstrap's `finally` always removes the
  run's tarball and its lock records and closes the lock; run folders older
  than a day and stray `kluris-*.tgz` are swept by the next run, under the lock.
  Ctrl-C ends the run there too (130).
- **The answer.** The output streams back; the status is the machine's last
  line `== exit N` (ssh to PowerShell answers 1 for every failure), then a
  summary line with each step's status and time.
- **Parity on Windows.** v2.28.11 runs there as the oracle (the hash-pinned
  `parity/oracle.lock` has the `win_amd64` wheels) against the release
  `kluris.exe` (`KLURIS_V3_BIN`). The harness reads CRLF in stdout/stderr as
  LF (PAR-004).

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
