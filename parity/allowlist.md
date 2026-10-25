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

None yet.
