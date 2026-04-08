# Interview 01

**Date:** 2026-04-08

## Round 1 answers

1. **Feature 1 search scope:** Single brain only (`allow_all=False`), mirror `wake-up`. Picker prompts when 2+ brains, `--brain NAME` to skip.
2. **Feature 1 ranking:** Weighted substring hits — `title*10 + tag*5 + path*3 + body*1`. Deterministic, no deps, no DoS risk.
3. **Feature 3 rename strategy:** Originally picked "rename everything including the directory" but subsequently decided to SKIP Feature 3 entirely. Removed from scope.
4. **Feature 3 README handling:** N/A (skipped)

## Round 2 answers

1. **Feature 3 rename error handling:** Full rollback picked, but superseded by "skip Feature 3 entirely" in Q4. Feature dropped.
2. **Feature 2 scope:** FULL CLEANUP — batch git log + delete dead `_get_recent_changes` + fix `update_frontmatter` double-read. Takes on all three sub-improvements in one phase.
3. **Feature 1 search fields:** Also include glossary.md + brain.md (not just neurons). The wake-up snapshot already caches glossary for the agent, but CLI search returns them as first-class results so human users benefit too.
4. **Feature 3 dirty brain check:** N/A (feature skipped)

## Decisions applied as sensible defaults (no explicit answer yet)

1. **Batch git `HEAD` vs `--all`:** Use `HEAD` only. Feature branches shouldn't leak into the date map. Documented in Decision Log.
2. **Search snippet length cap:** 200 characters centered on the first match (100 before, 100 after). Matches the intent of being "compact index for agents".
3. **Search deprecated filter:** Show deprecated results but annotate them with `deprecated: true` in the JSON envelope, so agents can decide to skip or redirect to `replaced_by`. No `--include-deprecated` flag needed.
4. **Phase release strategy:** Two independent phases (search, batch git). Each shippable on its own. If they ship together, it's v2.2.0. If separately, both can be patch releases.

## Scope change

- Dropped from the spec: **Feature 3 (brain rename)**
- Renamed spec dir: `kluris-search-batch-rename` → `kluris-search-and-batch-git`
- Spec ID: `kluris-search-and-batch-git`
