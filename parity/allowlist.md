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
