# Parity allowlist

Every deliberate difference between Kluris v2.28.11 (the oracle) and Kluris 3
that the parity harness tolerates is listed here. A difference that is not
listed is a bug in Kluris 3.

Normalizations every comparison applies (they are not entries): the temp root
becomes `<ROOT>` (raw, JSON-escaped and `C:/` forms), `kluris <version>`
becomes `kluris <VERSION>`, an `updated:` value equal to today at either end of
a run, and human-readable output, whose wording is checked by substring only.

## Format

One entry per difference, under its own heading:

```
### PAR-NNN: <short title>
- Command: the command and flags it applies to (`kluris dream --json`)
- Differs: what v3 does instead of v2, precisely enough to write the assertion
- Decision: the spec decision or interview answer it comes from (link or id)
- Tests: the parity test(s) that encode it
```

## Entries

### PAR-001: JSON/TOML frontmatter ignored
- Command: every command that reads markdown neuron frontmatter (`dream`, `search`, `wake-up`, `mri`, `status`)
- Differs: a markdown file whose (whitespace-stripped) text opens with a line that is exactly `{` is not read as JSON frontmatter: v3 sees no frontmatter (empty metadata, the whole text as body), where v2 parses the `{` ... `}` block with `json.loads` through python-frontmatter's `JSONHandler`. `+++` TOML frontmatter is no frontmatter in both (v2's python-frontmatter has no `toml` package installed), so only JSON differs in practice.
- Decision: accidental v2 behaviour via python-frontmatter, decided 2026-10-25 (T-51 brief, Kluris 3 Phase 1)
- Tests: `crates/kluris-core/tests/frontmatter_read.rs::parse_json_and_toml_frontmatter_are_not_frontmatter` (no parity scenario yet)

### PAR-002: first body line keeps its indentation
- Command: every command that rewrites a markdown neuron's frontmatter (`dream`'s date, parent and synapse fixes)
- Differs: when the body's first non-blank line is indented (`    code first`), v3 writes it with its indentation; v2 writes it without (python-frontmatter strips the whole body on read). Leading whitespace-only lines and all trailing whitespace are still removed, as in v2.
- Decision: Kluris 3 decision (T-52 brief, Phase 1, IMPL-K3-06; research-01 §1 "Body handling")
- Tests: `crates/kluris-core/tests/yaml_emit.rs::markdown_framing_matches_python_frontmatter` (rows "indented first line", "blank lines then indented line"; no parity scenario yet)

### PAR-003: `windsurf` agent is `devin`; `antigravity` joins the default
- Command: every command that reads a brain's `kluris.yml` and every command that rewrites it (`create`, `register`, `companion add`, `companion remove`, `doctor`)
- Differs:
  - An agent name `windsurf` in `agents.commands_for` reads as `devin`, so a `kluris.yml` written by v3 has `  - devin` where v2 has `  - windsurf`.
  - The default `commands_for` (what `create` and `register` write, and what an empty selection means) is v2's `DEFAULT_AGENT_COMMANDS` in v2's order with `devin` in place of `windsurf`, then `antigravity`: `claude, cursor, devin, copilot, codex, gemini, hermes, kilocode, junie, antigravity`. v2 writes its 9 names with `windsurf`.
  - A `commands_for` equal as a set to v2's pre-Hermes 8 names (v2 migrates these too, to its own 9) or to v2.28's 9 names (with `windsurf` or with `devin`) reads as the v3 default, so a brain created by v2 gets `antigravity` without editing its file; v2 keeps the 9 names. Any other list is kept as written (only `windsurf` -> `devin`), unknown names included, as in v2.
  - Every other byte of `kluris.yml` is v2's. v2.28.11 reads a file with `devin` or `antigravity` without error (it skips names outside its registry).
- Decision: Windsurf became Devin (SPEC decision log 2026-10-24, research-04, interview-01 #15-16; SPEC acceptance "a `windsurf` entry in an existing `kluris.yml` is treated as `devin`"; T-53 brief); default list with `antigravity` and the v2-default migration decided by the chief on T-53 (m-4244), 2026-10-25
- Tests: `crates/kluris-core/tests/config.rs::{defaults_are_v2_defaults_with_devin_then_antigravity, v2_default_lists_migrate_to_the_v3_default, v2_brain_config_round_trips_with_the_v3_default_agents, windsurf_is_read_as_devin_and_unknown_agents_are_kept}`, `parity/test_config_roundtrip.py::test_v2_reads_v3_agent_names` (no v3 command writes `kluris.yml` yet)

### PAR-004: CRLF in stdout and stderr reads as LF
- Command: every command the harness runs, on Windows
- Differs: on Windows v2 (Python text streams) ends each line of stdout and stderr with CRLF where v3 (Rust `println!`) writes LF. The harness decodes both streams of both binaries with every `\r\n` read as `\n` before comparing; a lone `\r` is kept. Files written into brains and configs are compared as bytes, without this.
- Decision: Kluris 3 decision, chief's brief for T-56 (Phase 1, TEST-K3-09 / IMPL-K3-10), 2026-10-25
- Tests: `parity/test_harness.py::test_run_turns_crlf_into_lf`; every parity test on Windows (`cargo xtask windows --parity`)
