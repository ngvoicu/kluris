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

### PAR-003: `windsurf` agent is `devin`
- Command: every command that reads a brain's `kluris.yml` and every command that rewrites it (`create`, `register`, `companion add`, `companion remove`, `doctor`)
- Differs: an agent name `windsurf` in `agents.commands_for` reads as `devin`, so when v3 writes a `kluris.yml` the line `  - windsurf` is `  - devin` (the default list is v2's `DEFAULT_AGENT_COMMANDS` in v2's order with `devin` in Windsurf's place, also for v2's pre-Hermes legacy set, which still migrates to the default). Every other byte is v2's. Unknown agent names are kept, as in v2; v2.28.11 reads a file with `devin` or `antigravity` without error (it skips names outside its registry).
- Decision: Windsurf became Devin (SPEC decision log 2026-10-24, research-04, interview-01 #15-16; SPEC acceptance "a `windsurf` entry in an existing `kluris.yml` is treated as `devin`"; T-53 brief)
- Tests: `crates/kluris-core/tests/config.rs::{v2_brain_config_round_trips_with_windsurf_written_as_devin, windsurf_is_read_as_devin_and_unknown_agents_are_kept, pre_hermes_default_list_migrates_to_the_default_like_v2}`, `parity/test_config_roundtrip.py::test_v2_reads_v3_agent_names` (no v3 command writes `kluris.yml` yet)
