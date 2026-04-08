# Research Notes — kluris search + batch git + rename

**Date:** 2026-04-08
**Researcher:** specmint-tdd:researcher agent
**Spec:** kluris-search-batch-rename

## Stack baseline

- Python 3.10+, pyproject.toml v2.1.0 (`pyproject.toml:7`), `__init__.py:3`
- Deps: click>=8.1, rich>=13.0, pyyaml>=6.0, python-frontmatter>=1.1, pydantic>=2.0 (`pyproject.toml:12-18`)
- Installed venv: Python 3.14
- 28 test files in `tests/`, conftest has 5 fixtures + `create_test_brain` helper (`tests/conftest.py:13-111`)
- cli.py is **1722 lines** (not 1500), one file
- core/ has 10 modules

---

## Feature 1 — `kluris search <query>`

### Architecture findings

**Command shape to mirror:**

- `wake-up` (`cli.py:797-864`) — single-brain read-only, resolves one brain via `_resolve_brains(brain_name, allow_all=False, as_json=as_json)` at cli.py:814. Closest template.
- `list` (`cli.py:628-655`) — simplest read-only; JSON-first pattern.
- `status` (`cli.py:867-906`) — fan-out read-only example; shows `allow_all=True`.

**`_resolve_brains` signature confirmed** (`cli.py:295-368`):
```python
def _resolve_brains(
    brain_name: str | None,
    *,
    allow_all: bool = False,
    as_json: bool = False,
) -> list[tuple[str, dict]]:
```
Resolution: `all` → explicit → 0-brains error → 1 auto → multi TTY picker → multi non-TTY error. `KLURIS_NO_PROMPT=1` + `as_json` + `not _is_interactive()` all force non-interactive.

**Frontmatter reader** — `core/frontmatter.py:22-25`:
```python
def read_frontmatter(path: Path) -> tuple[dict, str]:
    post = frontmatter.load(str(path))
    return _normalize_metadata(dict(post.metadata)), post.content
```
`frontmatter.load()` reads full file + YAML parse. **No lazy-body API.** Every call = one full file read. No "just the title line" helper.

**Title extraction pattern exists inline** in `maps._get_neurons` at `core/maps.py:61-87`:
```python
title = item.stem.replace("-", " ").title()
# ... read_frontmatter ...
for line in content.split("\n"):
    if line.startswith("# "):
        title = line[2:].strip()
        break
tags = meta.get("tags", [])
```
Private helper, not exposed. Search would need to extract a shared helper or reimplement.

**`_wake_up_collect_recent` (`cli.py:679-699`) does NOT surface titles** — only `{path, updated}`. So search is new territory for title-in-JSON.

**Neuron walk cost:** `linker._neuron_files` → `rglob("*.md")` + filter via `SKIP_DIRS`/`SKIP_FILES`. For N neurons: N × `frontmatter.load()` = N full reads. 100 neurons = ~1 second, scales linearly.

**JSON test pattern** — mirror `tests/test_json_output.py:1-164` + `tests/test_wake_up.py:37-61`.

**`core/search.py` does not exist** — greenfield module, no collision.

### Ranking and scoring

Zero precedent in the codebase. Design decision for the spec:
- Plain substring count (simplest, deterministic)
- TF-IDF (more accurate, more code)
- Pure Python scoring (no new deps) vs external (new dep)

No existing helpers.

### Risk assessment

- **Performance cliff on big brains.** O(N × filesize) IO per search call. A 500-neuron brain × 20KB avg = 10MB IO/call.
- **Title extraction fragile.** Hand-written neurons that start with content before the heading are not tested.
- **`--brain all` fan-out semantics.** Would require results grouped per brain (like `status` at cli.py:888-893).
- **JSON output wrapping.** Existing commands wrap in `{"ok": True, ...}`. `mri` has backward-compat branch at cli.py:1257-1262.
- **Regex DoS** if query is unsanitized regex. Substring match avoids.
- **Snippet size.** Wake-up brain_md cap is 4000 bytes — reasonable precedent.

### Open questions — Feature 1

1. Single-brain (`allow_all=False`) or fan-out (`allow_all=True`)?
2. Ranking algorithm — substring count, term-frequency, or fuzzier?
3. Should results include titles (requires extracting the title helper)?
4. Snippet length cap — 200 chars? 500? 4000 (match wake-up)?
5. Search what fields — frontmatter (tags, title, related), body, filename, map.md, or all?
6. Case sensitivity / tokenization?
7. Should wake-up snapshot expose a query helper for agents that already have it cached, or is `search` always a separate subprocess call?
8. Filter deprecated neurons? Surface with a flag?

---

## Feature 2 — Batch git/date work in `_sync_brain_state`

### Architecture findings

**`_sync_brain_state` is at `cli.py:135-197`.** The relevant loop (`cli.py:147-167`):
```python
for md in brain_path.rglob("*.md"):
    if md.name in {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}:
        continue
    if ".git" in md.parts:
        continue
    try:
        meta, _ = read_frontmatter(md)
        changed = False
        last_mod = git_file_last_modified(brain_path, str(md.relative_to(brain_path)))
        if last_mod and str(meta.get("updated", "")) != last_mod[:10]:
            update_frontmatter(md, {"updated": last_mod[:10]})
            changed = True
        if "created" not in meta:
            created = git_file_created_date(brain_path, str(md.relative_to(brain_path)))
            if created:
                update_frontmatter(md, {"created": created[:10]})
                changed = True
```

**Per-neuron cost:**
1. `read_frontmatter(md)` = 1 full file read
2. `git_file_last_modified()` at `core/git.py:111-118` = 1 `subprocess.run(["git", "log", "-1", "--format=%aI", "--", filename])`. Always runs.
3. `git_file_created_date()` at `core/git.py:121-128` = 1 more subprocess call, gated on missing `created`.
4. `update_frontmatter(md)` at `core/frontmatter.py:34-39` re-runs `frontmatter.load()` — **hidden 2x read cost per updated neuron.**

**For clean 100-neuron brain:** ~100 subprocess spawns × ~20-50ms = 2-5 seconds of pure subprocess overhead.

**For fresh brain with N neurons missing both fields:** 2N subprocess spawns + N file writes (each = 2 reads + 1 write).

**Callers of `_sync_brain_state`:**
- `dream` command at cli.py:1055-1058
- `mri` command at cli.py:1236-1238

**`git_log` signature** at `core/git.py:76-91`:
```python
def git_log(path: Path, limit: int = 10) -> list[dict]:
    result = _run(["git", "log", f"-{limit}", "--format=%H|%s|%aI"], cwd=path)
```
Single-repo, no path filter. No batch variant exists.

**Dead code:** `_get_recent_changes` at `core/maps.py:99-114` has **zero callers** anywhere in src. Safe to delete.

**No existing `git log --name-only` or `--name-status` usage.** Greenfield.

**Batch pattern that would work:**
```bash
git log --format="COMMIT %aI" --name-only HEAD
```
Walks every commit, parses line-by-line, records newest date per path. **1 subprocess spawn instead of 2N.**

### Test infrastructure findings

**No subprocess mocking exists** — searched `tests/` for `mock`, `patch`, `MagicMock`, `subprocess.run`. Every git test uses real git in `tmp_path` (`tests/test_git.py:22-146`). Only `monkeypatch.setattr` usage:
- `cli_module._is_interactive` (6 tests in `test_brain_resolution.py`)
- `cli_module.render_commands` (1 test in `test_install.py:288`)

**No big-brain fixture.** Grep found no 100-neuron synthetic brain. Closest is `test_wake_up.py:78-92` with 10 neurons.

**`temp_brain` at `conftest.py:48-100`** already makes a real git commit via subprocess.run. Wraps subprocess directly, bypasses `core/git.py` helpers. Established pattern.

**No test today asserts "updated field reflects git last-modified date".** Feature 2 can introduce such assertions without collision.

### Risk assessment

- **Ordering semantics.** `git log --name-only HEAD` walks reverse chronological; first entry per path = newest. `--all` pulls in feature branches — safer to use HEAD.
- **Rename detection.** Don't use `-M` unless explicitly needed.
- **Uncommitted neurons.** Must match today's behavior: file absent from walk → skip silently.
- **Frontmatter date drift.** Today's loop updates `updated:` every dream call. Batch must produce same output to avoid breaking tests.
- **Windows subprocess perf.** The batch benefit is largest on Windows where git spawn is slow. Verify shell escaping for `--format` flag.
- **`update_frontmatter` double-read.** Today each update = 1 read + 1 write per neuron. Batch refactor could pass pre-parsed metadata to avoid the second load.
- **Determinism in tests.** Real git commits include current timestamp — tests that assert specific dates will be flaky unless they use `GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE`. No existing tests set these.
- **Failure isolation.** Current per-file logic silently swallows exceptions. Batch should match: if whole-brain git call fails, empty map + graceful degradation.

### Open questions — Feature 2

1. Delete `_get_recent_changes` dead code in same spec, or leave it?
2. `--all` or `HEAD` for the batch log?
3. Rename tracking (`-M`)?
4. Batch `update_frontmatter` calls too, or leave that for later?
5. Perf benchmark in tests — assert subprocess count or wall-clock time?
6. MRI preflight: share the same batch implementation as dream?
7. Warning when git history is missing?

---

## Feature 3 — `kluris rename <old-name> <new-name>`

### Where the brain name lives

1. **`~/.kluris/config.yml` (registry)** — dict key in `GlobalConfig.brains: dict[str, BrainEntry]` (`core/config.py:22`). Rename = pop old key + insert new.

2. **`<brain>/kluris.yml` (local config)** — `BrainConfig.name: str` (required) at `core/config.py:53`. Read via `read_brain_config`, written via `write_brain_config`. Confirmed by `tests/test_brain.py:55-62`.

3. **`<brain>/brain.md`** — rendered with name in H1. Template at `core/maps.py:141-146`:
```python
content = (
    f"# {name}\n\n{description}\n\n"
    f"## Lobes\n\n{lobe_links}\n\n"
    f"## Reference\n\n"
    f"- [glossary.md](./glossary.md) — ..."
)
```
**Regenerated by `_sync_brain_state` at `cli.py:183`** — every dream overwrites brain.md from scratch. So rename + dream auto-refreshes brain.md.

4. **`<brain>/README.md`** — rendered via `scaffold_brain` at `core/brain.py:245-246`, **never regenerated**. Template `_generate_readme` at `core/brain.py:252-497` embeds `{name}` **41 times** (slash-command examples, CLI examples, title). **README is the biggest rename footprint.** Marked hand-editable per CLAUDE.md ("generated once, then hand-editable").

5. **`<brain>/brain.md`** — see #3.

6. **Agent skill files (SKILL.md + Windsurf workflows)** — every SKILL.md has the brain name in multiple places. Re-rendered on every `_do_install` call. **Rename + `_do_install()` auto-refreshes all skills.** Skill path naming (`cli.py:1301-1314`):
```python
if len(brains) == 1:
    return [("kluris", only_name, only_entry)]   # single: "kluris"
return [(f"kluris-{n}", n, e) for n, e in brains.items()]  # multi: "kluris-<name>"
```
Windsurf workflow (`core/agents.py:412-434`): `workflow_dir / f"{skill_name}.md"` — confirmed `kluris.md` single, `kluris-<name>.md` multi.

7. **MRI HTML file** — `mri.py:226` uses `brain_name = brain_path.name` (DIRECTORY NAME, not registered name). **Decoupled from registry.** Rename-in-registry leaves MRI showing the old directory name unless the directory is also renamed.

8. **`brain_path.name` elsewhere** — only `mri.py:226`. No other references.

9. **Neuron files** — **brain-name-free.** `generate_neuron_content` at `core/brain.py:111-140` produces frontmatter with only `parent: ./map.md`, `template`, `related`, `tags`, `created`, `updated`. Neurons are portable.

10. **map.md files** — **brain-name-free.** `generate_map_md` at `core/maps.py:152-199` uses only lobe/sibling names.

11. **glossary.md** — brain-name-free.

**Hidden references:** Only `mri.py:226`. No caches, no separate lookup-by-name.

### Install idempotency after rename

After `rename_brain(old, new) + _do_install()`:
- `_compute_skills_to_render(config.brains)` with new name keys
- `_sweep_kluris(base, ...)` globs `kluris*` and deletes everything (`cli.py:1291-1298`)
- Atomic stage → rename for all brains (`cli.py:1386-1394`)

**The existing sweep-then-rename pattern handles rename perfectly.** Same as 1→N and N→1 transitions.

### `register_brain` / `unregister_brain` atomicity

`core/config.py:113-124`:
```python
def register_brain(name: str, entry: BrainEntry) -> None:
    config = read_global_config()
    config.brains[name] = entry
    write_global_config(config)

def unregister_brain(name: str) -> None:
    config = read_global_config()
    config.brains.pop(name, None)
    write_global_config(config)
```
**Neither is atomic.** `write_global_config` writes directly via `path.write_text(...)` at `config.py:85-96` — **no temp-file + rename pattern.** Process kill mid-write = empty/half-written config.

For rename (conceptually = unregister + register), naive sequential calls create a race where the brain is registered under neither name. **Need a single-operation `rename_brain(old, new)` in `core/config.py`** that does one read + one atomic write.

### Directory rename — three options

**A. Rename registry only, leave directory alone.**
- Pros: simple, safe, no git repo path changes, no stale editor references
- Cons: `BrainEntry.path` still points at `/old-name/`, MRI HTML still shows old name, README still has old name
- Mismatch visible: `kluris list` shows new-name at path /old-name/

**B. Rename registry AND move the directory on disk.**
- Pros: everything stays in sync
- Cons: breaks IDE paths, breaks `cd` in terminals, breaks any script referencing the old path. Git history preserved but other tools may not handle rename gracefully.

**C. Rename registry + update kluris.yml + regenerate brain.md, leave directory + README untouched.**
- Registry name is a pure label. `kluris.yml` updated (source-of-truth for dream). brain.md regenerated via dream. README NOT regenerated (preserves user edits), stays stale with old name. User told to manually fix README if they care.
- Least destructive option.

### Risk assessment

- **`kluris.yml` is gitignored** (confirmed at `core/brain.py:91`). Rename is local-only — does NOT propagate to teammates via git. Each teammate renames independently.
- **Collisions.** Rename A→B when B already exists must fail. Mirror `cli.py:453-458`.
- **Name validation.** New name must pass `validate_brain_name` (reserved `all`, length ≤48, lowercase).
- **Stale skills if install fails.** Existing partial-failure guard helps but some agents might end up in new state, others in old.
- **README staleness** — 41 references to old name. Re-render = clobber user edits. Don't re-render = stale README.
- **MRI HTML staleness** — `brain-mri.html` is gitignored, local artifact. Shows old directory name until user re-runs `kluris mri`.
- **Dream overwrites brain.md.** Rename must update `kluris.yml` BEFORE dream runs, otherwise dream reverts brain.md to old name.
- **Config atomicity** — existing non-atomic `write_global_config` is a pre-existing risk.
- **BrainEntry.path** — if Option A/C, unchanged. If Option B, must be updated; `_check_brain_paths` at `cli.py:241-257` catches stale paths.

### Test infrastructure findings

- `create_test_brain` helper (`conftest.py:13-23`) for multiple brains
- `tests/test_install.py:137-169` for install assertion patterns
- `tests/test_brain.py:55-62` for kluris.yml assertion
- `tests/test_remove.py:16-17` for registry assertion
- `tests/test_dream.py:48-58` for brain.md regeneration assertion
- `tests/test_install.py:266-299` for partial-failure simulation (monkeypatch render_commands)

### Open questions — Feature 3

1. Directory rename strategy — A, B, or C?
2. README re-render or leave stale?
3. Auto-run dream after rename to refresh brain.md?
4. Auto-run `_do_install` after rename? (almost certainly yes)
5. Atomicity — rollback registry on install failure?
6. Upgrade `write_global_config` to temp+rename in-scope or deferred?
7. Warn on uncommitted changes like `kluris remove` does?
8. Emit guide: "run kluris push to commit updated brain.md"?
9. Order of operations: kluris.yml → dream → _do_install?

---

## Test infrastructure summary

### Current setup
- **Framework:** pytest ≥8.0
- **Runner:** `.venv/bin/python -m pytest tests/`
- **Mocking library:** none. Uses `monkeypatch` for env vars + module attrs only. No subprocess mocking.
- **Docker/testcontainers:** neither
- **CI:** ubuntu/macos/windows × Python 3.10-3.13 on PR only
- **311 tests passing**

### Quality assessment
- Naming: `test_<command>_<behavior>` — good
- Isolation: every test sets `KLURIS_CONFIG` + `HOME` to `tmp_path` — good
- Edge case coverage: strong for picker/partial-failure/stale-paths. Gaps: no perf tests, no big-brain tests, no batch-git tests.

### Recommended additions
- `create_test_brain_with_neurons(runner, name, path, count)` helper in conftest for Feature 1/2 big-brain tests
- Fixture for injecting git commit timestamps via `GIT_AUTHOR_DATE` env var for deterministic batch-git assertions
- No need for testcontainers, mutation testing, or property-based testing for this spec

---

## Headline findings

1. **Feature 1 — search:** no existing title-from-body helper beyond private `maps._get_neurons`. O(N) full file reads unavoidable without a cache. Mirror `wake-up` shape + `test_wake_up.py` JSON assertion style. `core/search.py` is greenfield.

2. **Feature 2 — batch git:** per-neuron loop at `cli.py:147-167` fires ~2N subprocess calls. Single `git log --format=... --name-only HEAD` can replace all of them. No subprocess mocking exists; tests use real git. Need a big-brain fixture. Dead code `_get_recent_changes` at `maps.py:99-114` can be deleted in same spec. Hidden 2x read cost in `update_frontmatter`.

3. **Feature 3 — rename:** brain name lives in registry dict key + kluris.yml `name:` + brain.md H1 + README.md (41 refs) + 8 agent SKILLs + universal slot + Windsurf + `mri.py:226`. Scaffolded neurons/maps are name-free. `kluris.yml` is gitignored — per-teammate. The sweep-then-rename install pattern already handles label changes. `write_global_config` is NOT atomic. README re-render vs stale is the biggest UX tension.
