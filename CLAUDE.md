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
