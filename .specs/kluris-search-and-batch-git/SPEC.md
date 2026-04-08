---
id: kluris-search-and-batch-git
title: Kluris search + batch git
status: completed
created: 2026-04-08
updated: 2026-04-08
priority: high
tags: [cli, search, performance, git]
---

# Kluris search + batch git

## Overview

Two independent v2.2.0 follow-ups to kluris-cli v2.1.0:

1. **`kluris search <query>`** — a new read-only CLI command that ranks
   matches across neurons + glossary.md + brain.md. Single-brain by
   default (picker prompts when 2+ brains); JSON-first output. Replaces
   the current agent flow of "read brain.md → pick lobes → read N map.md
   files → read N neurons" with one bash call. Backed by a new
   `core/search.py` module.

2. **Batch git/date work in `_sync_brain_state`** — replace the per-neuron
   `git log` subprocess loop with a single `git log --name-only HEAD`
   invocation that builds **two** path→date maps (latest + created), so
   both `updated:` and `created:` frontmatter fields can be refreshed
   from one walk. Also deletes the dead `_get_recent_changes` helper in
   `core/maps.py` and adds a `preloaded=(meta, body)` shortcut to
   `frontmatter.update_frontmatter` so the second `frontmatter.load()`
   per updated neuron disappears.

Both features are independently shippable. Each is its own phase with its
own TDD cycles. They can ship together as v2.2.0 or separately as
patch/minor releases.

**Out of scope:** `kluris rename <old> <new>` was researched and
designed in interview round 2 but pulled from this spec (user decision).
Can be forged as its own spec later.

## Acceptance Criteria

### Phase 1: Search

- [x] `kluris search "foo"` on a single-brain setup returns ranked results with no prompting
- [x] `kluris search "foo"` on a 2+-brain setup prompts via `_pick_brain_interactively` (TTY) or errors cleanly (non-TTY/`--json`/`KLURIS_NO_PROMPT=1`)
- [x] Picker tests monkeypatch `kluris.cli._is_interactive` (NOT `sys.stdin.isatty`) to match the existing v2.1.0 picker test pattern
- [x] `--brain NAME` skips the picker and targets a specific brain
- [x] `--brain all` is rejected with the same error as other single-brain commands (`_resolve_brains` does this automatically when `allow_all=False`)
- [x] `--limit N` caps result count (default 10, max 100)
- [x] `--lobe LOBE` filters results to neurons under `<brain>/<lobe>/`
- [x] `--tag TAG` filters to neurons whose frontmatter `tags:` includes TAG
- [x] `--json` emits this exact envelope:
  ```json
  {
    "ok": true,
    "brain": "<brain_name>",
    "query": "<original_query>",
    "total": <int>,
    "results": [
      {
        "file": "projects/btb/auth.md",
        "title": "Auth flow",
        "matched_fields": ["title", "body"],
        "snippet": "...auth flow uses oauth2 with...",
        "score": 12,
        "deprecated": false
      }
    ]
  }
  ```
  Each result has: `file`, `title`, `matched_fields` (array, never null), `snippet` (str, empty if no body match), `score` (int), `deprecated` (bool).
- [x] Text output shows a compact table: score, title, file, snippet for body matches
- [x] **Scoring uses occurrence counts**, not boolean hits: `score = (title_count * 10) + (tag_count * 5) + (path_count * 3) + (body_count * 1)`. Three matches in the body = 3 body points. Ties broken by file path (alphabetical, stable).
- [x] **`matched_fields`** lists every field where the query matched at least once: `["title"]`, `["body"]`, `["title", "body"]`, etc. Order: `title`, `tag`, `path`, `body`. Never empty (a result with score 0 is filtered out before emission).
- [x] Glossary entries appear as first-class results with `file: "glossary.md"`, `title: <term>`, body = the definition text. Tags are empty for glossary entries. The glossary parser is the same one wake-up uses (tolerant of both markdown table and `**Term** -- Definition` formats).
- [x] brain.md content appears as a result with `file: "brain.md"`, `title: <H1>`, body = the markdown body. No tags.
- [x] Search matches are **literal substring** (not regex): `re.search` is NOT used. The query and the searchable text are both `.lower()`-folded; matching uses `str.count()` and `in`. Special characters (`.`, `*`, `?`, `[`) are treated literally.
- [x] Search supports non-ASCII queries (UTF-8 throughout — kluris uses `encoding="utf-8"` everywhere)
- [x] Empty-query argument fails with a clear error
- [x] Zero-brain setup errors with "No brains registered"
- [x] Results include deprecated neurons (status: deprecated in frontmatter) with `deprecated: true` so agents can skip them or redirect to `replaced_by`. Deprecation is detected from each neuron's frontmatter directly during collection — NOT from `linker.detect_deprecation_issues()` (which reports issue states like `active_links_to_deprecated`, not the full set of deprecated neurons).
- [x] `core/search.py` is a new module; cli.py does not exceed its current line count (1722) by more than +80 lines
- [x] All new code is covered by tests; existing tests still pass

### Phase 2: Batch git + cleanup

- [x] `_sync_brain_state` replaces its per-file `git_file_last_modified` / `git_file_created_date` calls with a single `git_log_file_dates(brain_path)` invocation that returns BOTH `latest_by_path` and `created_by_path` maps from one git subprocess walk
- [x] The batch helper uses `%aI` (author ISO date) to match today's `git_file_last_modified` (`core/git.py:111-118`) and `git_file_created_date` (`core/git.py:121-128`) byte-for-byte
- [x] For a 100-neuron brain that's already up to date, dream's date-refresh subprocess count is **exactly 2**: one `is_git_repo()` check + one batch `git log` call. (Was ~100-200 before.)
- [x] For a 100-neuron no-git brain, dream's date-refresh subprocess count is **exactly 1**: just the `is_git_repo()` check that returns False and short-circuits the batch call
- [x] Dead code `_get_recent_changes` in `core/maps.py:99-114` is deleted (zero callers anywhere in src)
- [x] `frontmatter.update_frontmatter` accepts `preloaded=(meta, body)` keyword. When provided, the function does NOT call `frontmatter.load()` — it merges the patch into the supplied meta, dumps the result with the supplied body, and writes back. Single source of truth for the API; the architecture diagram and tests use this exact shape.
- [x] `_sync_brain_state` reads `meta, body = read_frontmatter(md)` once, then passes `preloaded=(meta, body)` to every `update_frontmatter` call for that neuron, cutting per-updated-neuron read IO from 2 to 1
- [x] Behavior unchanged: same `updated:` and `created:` dates written, same set of files updated. Diff output of dream is identical between the old and new implementation on a verification fixture
- [x] Uncommitted neurons (absent from `git log --name-only HEAD`) retain their existing frontmatter dates (matches today's silent-skip behavior)
- [x] Brains with no git history (fresh `kluris create --no-git`) skip date refresh entirely without crashing
- [x] Renamed neurons in git history have their `created` and `updated` dates derived from the new path's commit history (matches today, since `git log -- newpath` and `git log -- oldpath` both work via path matching when `-M` is not passed)
- [x] All existing dream/mri tests still pass

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                          cli.py                               │
│                                                                │
│  @cli.command("search")  (NEW, Phase 1)                       │
│   ├─ _resolve_brains(brain_name, allow_all=False, as_json)   │
│   └─ from core.search import search_brain ──────────────┐    │
│                                                          │    │
│  @cli.command("dream") / @cli.command("mri")             │    │
│   └─ _sync_brain_state(brain_path, brain_config)         │    │
│       ├─ NEW: latest_by_path, created_by_path =          │    │
│       │       core.git.git_log_file_dates(brain_path) ───┼──┐ │
│       └─ for md in _neuron_files(brain_path):            │  │ │
│             meta, body = read_frontmatter(md)            │  │ │
│             ├─ last_mod = latest_by_path.get(rel_path)   │  │ │
│             ├─ if last_mod != meta["updated"]:           │  │ │
│             │     update_frontmatter(md, {"updated":...},│  │ │
│             │                         preloaded=(meta,   │  │ │
│             │                                    body))  │  │ │
│             └─ if "created" not in meta:                 │  │ │
│                   created = created_by_path.get(rel_path)│  │ │
│                   if created:                            │  │ │
│                       update_frontmatter(md, {"created":...,│ │
│                                          preloaded=(meta,│  │ │
│                                                    body))│  │ │
└──────────────────────────────────────────────────────────┼──┼─┘
                                                           │  │
┌──────────────────────────────────────────────────────────▼──┼─┐
│                       core/search.py (NEW)                   │ │
│                                                               │ │
│  def search_brain(                                            │ │
│      brain_path, query,                                       │ │
│      *, limit=10, lobe_filter=None, tag_filter=None,          │ │
│  ) -> list[dict]:                                             │ │
│      ├─ _collect_searchable(brain_path)                       │ │
│      │    ├─ _neuron_files() from linker                      │ │
│      │    │    └─ for each: read_frontmatter, extract title,  │ │
│      │    │       record status, tags, body                   │ │
│      │    ├─ glossary entries via shared parser               │ │
│      │    │    (lifted from cli._wake_up_collect_glossary)    │ │
│      │    └─ brain.md content                                 │ │
│      ├─ _score_hit(item, query_lower) → int                   │ │
│      │    title_count * 10 + tag_count * 5 +                  │ │
│      │    path_count * 3 + body_count * 1                     │ │
│      ├─ _matched_fields(item, query_lower) → list[str]        │ │
│      ├─ _extract_snippet(text, query_lower, width=200)        │ │
│      ├─ filter by lobe + tag                                  │ │
│      ├─ filter out score == 0                                 │ │
│      ├─ sort by (-score, file_path)                           │ │
│      ├─ apply limit                                           │ │
│      └─ attach `deprecated` flag (set during collection)      │ │
│                                                               │ │
│  def extract_neuron_title(path, meta, content) -> str         │ │
└──────────────────────────────────────────────────────────────┘ │
                                                                 │
┌────────────────────────────────────────────────────────────────▼─┐
│                       core/git.py                                  │
│                                                                    │
│  def git_log_file_dates(                                          │
│      brain_path: Path,                                            │
│  ) -> tuple[dict[str, str], dict[str, str]]:   (NEW, Phase 2)    │
│      """Returns (latest_by_path, created_by_path).               │
│                                                                    │
│      One subprocess call:                                         │
│        git log --format=COMMIT %aI --name-only HEAD               │
│                                                                    │
│      Walks output newest-first. State machine:                    │
│        - "COMMIT <date>" line: set current_date                   │
│        - blank line: skip                                         │
│        - any other line: it's a file path                         │
│            - latest[path]: first time seen wins (= newest)        │
│            - created[path]: every time seen overwrites (= oldest) │
│                                                                    │
│      Returns ({}, {}) if not a git repo or git log fails.        │
│      Caller is expected to call is_git_repo() first.              │
│      """                                                           │
│                                                                    │
│  def git_file_last_modified(...)  ← still exists (kept for       │
│  def git_file_created_date(...)      backward compat; not used   │
│                                       in _sync_brain_state after  │
│                                       this refactor)              │
└──────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────┐
│                   core/frontmatter.py                              │
│                                                                    │
│  def update_frontmatter(                                          │
│      path, patch, *, preloaded=None,                              │
│  ):                                                                │
│      """                                                           │
│      preloaded: optional (meta, body) tuple from a prior          │
│      read_frontmatter() call. When provided, the function         │
│      skips frontmatter.load() entirely — uses the supplied        │
│      meta and body directly. Single source of truth for the API.  │
│      """                                                           │
│      if preloaded is None:                                        │
│          post = frontmatter.load(str(path))   ← legacy path       │
│          meta = dict(post.metadata)                               │
│          body = post.content                                      │
│      else:                                                         │
│          meta, body = preloaded                                   │
│          meta = dict(meta)  # don't mutate caller's dict          │
│      meta.update(patch)                                           │
│      path.write_text(frontmatter.dumps(                           │
│          frontmatter.Post(body, **meta)                           │
│      ), encoding="utf-8")                                         │
└──────────────────────────────────────────────────────────────────┘
```

## Testing Architecture

### Test Framework & Tools

| Tool | Choice | Version | Purpose |
|------|--------|---------|---------|
| Test framework | pytest | ≥8.0 | Unit + integration |
| Mocking | pytest monkeypatch | stdlib | Env vars + module attrs only; no `unittest.mock` |
| DB testing | N/A | — | Kluris has no database |
| HTTP clients | N/A | — | Kluris has no HTTP clients |
| CLI testing | `click.testing.CliRunner` | from click | Invoke commands in-process |
| File I/O | pytest `tmp_path` | stdlib | Real filesystem in tmp dirs |
| Git | real `git` subprocess | — | Real git in tmp dirs; no mocks |
| Subprocess counting | `monkeypatch.setattr(git_module, "_run", counting_wrapper)` | — | Wrap `core.git._run` to increment a counter on each call |

### Isolation Strategy

| Layer | Approach | Services |
|-------|----------|----------|
| Search scoring | Pure functions; no mocks | None |
| Search IO | Real files in `tmp_path` | None |
| Git batching | Real git in `tmp_path` | Local git repo via `temp_brain` fixture |
| Subprocess count | Wrap `core.git._run` via monkeypatch | git subprocess |
| TTY picker | `monkeypatch.setattr(cli_module, "_is_interactive", lambda: True)` | None |
| CLI commands | `CliRunner.invoke` with `temp_brain` / `temp_home` / `temp_config` | None |

**Critical:** Picker tests MUST patch `kluris.cli._is_interactive`, NOT `sys.stdin.isatty`. CliRunner replaces `sys.stdin` during invoke, so isatty patches don't survive. The codebase already follows this pattern in `tests/test_brain_resolution.py`.

### No network calls anywhere. No external services. Real git + real filesystem only.

### Coverage Targets

| Metric | Target |
|--------|--------|
| Line coverage (overall) | ≥90% (maintain existing baseline) |
| Branch coverage | ≥85% (maintain existing baseline) |
| `core/search.py` line coverage | ≥95% (new module; set a high bar) |
| `_sync_brain_state` line coverage | ≥95% (core refactor target) |

### Test Commands

| Command | Purpose |
|---------|---------|
| `.venv/bin/python -m pytest tests/ --no-header -q` | Run all tests |
| `.venv/bin/python -m pytest tests/test_search.py -v` | Phase 1 tests only |
| `.venv/bin/python -m pytest tests/test_dream.py tests/test_mri_cmd.py tests/test_git.py tests/test_frontmatter.py -v` | Phase 2 tests only |
| `.venv/bin/python -m pytest tests/ --cov=kluris -q` | With coverage |

### Conventions

- Test file naming: `tests/test_<command>.py` for CLI tests, `tests/test_<module>.py` for core module tests
- Test function naming: `test_<command>_<behavior>` (e.g., `test_search_ranks_title_above_body`)
- Fixture usage: prefer `temp_brain` + `cli_runner` from `conftest.py`; fall back to raw `tmp_path` + `monkeypatch.setenv` when the brain fixture doesn't fit
- JSON assertions: `data = json.loads(result.output); assert data["ok"] is True` (mirror `tests/test_json_output.py`)
- Big-brain fixture: new `create_test_brain_with_neurons(runner, name, path, count=100)` helper added to `tests/conftest.py`
- Subprocess counting: new helper in `tests/conftest.py` that wraps `core.git._run` via monkeypatch and exposes a counter

## Library Choices

| Need | Library | Version | Alternatives | Rationale |
|------|---------|---------|--------------|-----------|
| Text search/ranking | Python stdlib only (`str.lower`, `str.count`, `in`) | — | rapidfuzz, whoosh, sqlite3 FTS5 | Strict "no new deps" constraint. Substring matching is sufficient for <1000-neuron brains. rapidfuzz adds a native extension. whoosh/FTS5 require an index step. Skip. |
| Batch git parsing | stdlib `subprocess` + line iteration | — | gitpython, pygit2 | gitpython is a heavy dep. pygit2 needs libgit2 bindings. Parsing `git log --format=...` output is ~30 lines and matches how `core/git.py` already calls git. |
| Frontmatter | python-frontmatter (already a dep) | ≥1.1 | pyyaml directly | Already used everywhere in the codebase. |

## Phase 1: Search [completed]

Phase goal: ship `kluris search <query>` with full JSON + text output,
ranking, filters, deprecation flag, and agent-facing SKILL.md guidance.
Nine TEST-IMPL pairs covering collection, scoring, snippet extraction,
the search function, filters, deprecation, CLI command + picker, edge
cases, and text/help registration.

- [x] [TEST-SCH-01] Write `tests/test_search.py::test_collect_searchable_returns_neurons_glossary_brain_md` — create a temp brain with 2 neurons (one tagged, one with status: deprecated), 2 glossary terms in markdown table format, and a brain.md with an H1 title; call `core.search._collect_searchable(brain_path)`; assert it returns 5 items; each item has `kind` ∈ {`neuron`, `glossary`, `brain_md`}, `file` (relative path), `title` (string), `tags` (list, empty for glossary/brain_md), `body` (string), `is_deprecated` (bool, set on neurons via frontmatter `status: deprecated`). File: `tests/test_search.py`. Isolation: `tmp_path`.
- [x] [IMPL-SCH-02] Create `src/kluris/core/search.py`. Implement `_collect_searchable(brain_path) -> list[dict]`. Iterate via `linker._neuron_files`, read each with `read_frontmatter`, extract title via a new shared `extract_neuron_title(path, meta, content)` helper (mirror the logic in `core/maps.py:61-87`). Read `glossary.md` via a shared parser lifted from `cli._wake_up_collect_glossary` (or call the wake-up helper directly if it can be imported cleanly — otherwise duplicate the parsing). Read `brain.md` body via `read_frontmatter`. For each neuron, set `is_deprecated = str(meta.get("status", "active")).lower() == "deprecated"`. Glossary and brain.md items have `is_deprecated = False`. Return a flat list. → satisfies [TEST-SCH-01]

- [x] [TEST-SCH-03] Write `tests/test_search.py::test_score_hit_uses_occurrence_counts` — given a searchable item, assert that `_score_hit` returns `(title_count * 10) + (tag_count * 5) + (path_count * 3) + (body_count * 1)`. Cover: title contains query 2x → 20; body contains query 3x → 3; tag matches once → 5; path matches once → 3; combined (title 1x + body 2x → 12); zero matches → 0; query is lowercase-folded against lowercased text. Also write `tests/test_search.py::test_matched_fields_lists_every_hit_field` — given a hit in title and body, assert `_matched_fields` returns `["title", "body"]` in field order. File: `tests/test_search.py`.
- [x] [IMPL-SCH-04] Add `_score_hit(item, query_lower) -> int` and `_matched_fields(item, query_lower) -> list[str]` to `core/search.py`. Both lowercase-fold via `.lower()` (NOT `re.search` or `re.IGNORECASE`). Use `text.count(query_lower)` for occurrence counts. `_matched_fields` returns the list `["title", "tag", "path", "body"]` filtered to fields where `count > 0`, in that order. → satisfies [TEST-SCH-03]

- [x] [TEST-SCH-05] Write `tests/test_search.py::test_extract_snippet_centers_on_first_match` — given a 1000-char body with the query at position 500, assert `_extract_snippet` returns at most 200 chars centered on position 500, with `...` markers if the slice doesn't reach the start/end. Tests: match at start (no left ellipsis), match at end (no right ellipsis), no match (returns empty string), multi-byte UTF-8 query (returns intact UTF-8 — the snippet must NOT split a multi-byte character). File: `tests/test_search.py`.
- [x] [IMPL-SCH-06] Add `_extract_snippet(text, query_lower, width=200)` to `core/search.py`. Use `.lower().find(query_lower)` to locate the first match. Slice `text[max(0, idx-100) : idx+100+len(query_lower)]`. Add `...` prefix if `idx > 100`. Add `...` suffix if `idx+100+len(query_lower) < len(text)`. UTF-8 safety: slice on str (not bytes) so character boundaries are preserved automatically. → satisfies [TEST-SCH-05]

- [x] [TEST-SCH-07] Write `tests/test_search.py::test_search_brain_basic_ranking_and_limit` — build a brain with 5 neurons matching `"oauth"` in different fields: A (title 1x → score 11 because the H1 line is also part of the body), B (tag 1x → 5), C (path 1x → 3), D (body 2x → 2), E (no match → excluded). Call `search_brain(brain_path, "oauth", limit=10)`. Assert: 4 results in order [A, B, C, D]. Also test `limit=2` returns only [A, B]. Also test `limit=0` returns []. File: `tests/test_search.py`.
- [x] [IMPL-SCH-08] Implement `search_brain(brain_path, query, *, limit=10) -> list[dict]` in `core/search.py`. **Minimum viable implementation: collection + scoring + sorting + limit only.** No filters, no deprecation flag yet (those come in later pairs). Steps: call `_collect_searchable`, score each item via `_score_hit`, drop items with score 0, sort by `(-score, file)`, apply `[:limit]`, attach `snippet` (empty string for non-body matches via `_extract_snippet`), build the result dicts. Result dicts have: `file`, `title`, `matched_fields`, `snippet`, `score`. (No `deprecated` field yet.) → satisfies [TEST-SCH-07]

- [x] [TEST-SCH-09] Write `tests/test_search.py::test_search_brain_filters_by_lobe_and_tag` — create 4 neurons split across two lobes (`projects/api/`, `knowledge/`) with overlapping tags (`auth`, `oauth`). All match the query. Test: `search_brain(brain_path, "x", lobe_filter="projects")` returns only `projects/*` results (count==2). Test: `search_brain(brain_path, "x", tag_filter="oauth")` returns only neurons whose frontmatter `tags:` includes `oauth`. Test: both filters together AND. Test: filtering works on neurons; glossary and brain.md results are excluded by `lobe_filter` (their `file` is at brain root) and excluded by `tag_filter` (they have no tags). File: `tests/test_search.py`.
- [x] [IMPL-SCH-10] Add `lobe_filter: str | None = None` and `tag_filter: str | None = None` parameters to `search_brain`. Apply both filters during the collection-to-results step (after scoring, before sorting). For `lobe_filter`: keep only items whose `file` starts with `<lobe_filter>/` (forward-slash separator). For `tag_filter`: keep only items whose `tags` list contains the exact tag string. Glossary and brain.md items naturally fail both filters. → satisfies [TEST-SCH-09]

- [x] [TEST-SCH-11] Write `tests/test_search.py::test_search_brain_marks_deprecated_neurons` — create a brain with 2 matching neurons: A active, B with `status: deprecated` in frontmatter. Call `search_brain`. Assert both results have a `deprecated` field. A's `deprecated` is False, B's is True. Also test: a deprecated neuron with valid `replaced_by` (no incoming links, would NOT be returned by `detect_deprecation_issues`) is still correctly marked `deprecated: true` — this is the regression test for Codex finding #3. Also test: glossary and brain.md results have `deprecated: false`. File: `tests/test_search.py`.
- [x] [IMPL-SCH-12] Modify `search_brain` to attach `deprecated` to each result dict, sourced from the `is_deprecated` field that `_collect_searchable` already sets via direct frontmatter check (NOT via `linker.detect_deprecation_issues`). → satisfies [TEST-SCH-11]

- [x] [TEST-SCH-13] Write three test functions in `tests/test_search.py`: (a) `test_search_cli_single_brain_json` — create 1 brain via `create_test_brain`, write a neuron with a known body, run `cli_runner.invoke(cli, ["search", "oauth", "--json"])`, assert the JSON envelope matches the schema in Acceptance Criteria (`ok`, `brain`, `query`, `total`, `results` with each result having `file`/`title`/`matched_fields`/`snippet`/`score`/`deprecated`); (b) `test_search_cli_multi_brain_picker` — create 2 brains, monkeypatch `cli_module._is_interactive` to return True, invoke with `input="1\n"`, assert the picker output mentions both brain names and the first brain is picked; (c) `test_search_cli_brain_all_rejected` — create 2 brains, invoke with `--brain all`, assert exit_code != 0 and the error mentions "only supported on dream, push, status, mri". File: `tests/test_search.py`.
- [x] [IMPL-SCH-14] Add `@cli.command("search")` in `cli.py`. Signature: `search(query, brain_name, lobe, tag, limit, as_json)`. Click options: `--brain`, `--lobe`, `--tag`, `--limit` (int default 10), `--json`. Body: call `_resolve_brains(brain_name, allow_all=False, as_json=as_json)`, take the single resolved brain, call `core.search.search_brain(brain_path, query, limit=limit, lobe_filter=lobe, tag_filter=tag)`, build the JSON envelope `{"ok": True, "brain": name, "query": query, "total": len(results), "results": results}`, emit via `click.echo(json_lib.dumps(...))`. Resolver wiring (picker, `--brain all` rejection) is inherited automatically because `allow_all=False`. → satisfies [TEST-SCH-13]

- [x] [TEST-SCH-15] Write four test functions in `tests/test_search.py`: (a) `test_search_empty_query_errors` — invoke with `query=""`, assert exit_code != 0 and error mentions "empty"; (b) `test_search_no_matches_returns_empty` — invoke with a query that matches nothing, assert `data["total"] == 0` and `data["results"] == []`; (c) `test_search_special_characters_treated_literally` — write a neuron containing `a.b*?` literally in the body, invoke `kluris search "a.b*?" --json`, assert the neuron is found (proves no regex interpretation); (d) `test_search_non_ascii_query` — write a neuron with body containing `café résumé naïve`, invoke `kluris search "café" --json`, assert the neuron is found and the snippet preserves the UTF-8 characters. File: `tests/test_search.py`.
- [x] [IMPL-SCH-16] Add an empty-query guard at the top of the `search` command body: `if not query: raise click.ClickException("Query cannot be empty.")`. Verify the no-match, special-char, and UTF-8 cases are already handled by IMPL-SCH-08 (literal substring matching + UTF-8 strings). If any of the four sub-tests still fail after the empty-query guard, fix the underlying logic. → satisfies [TEST-SCH-15]

- [x] [TEST-SCH-17] Write two test functions in `tests/test_search.py`: (a) `test_search_text_output_has_table` — non-JSON invocation, write 2 matching neurons, assert the output contains the score, title, file, and at least one snippet line; (b) `test_search_help_includes_search_command` — invoke `cli_runner.invoke(cli, ["help", "--json"])`, assert `data["commands"]` contains an entry with `name == "search"` and `len(data["commands"]) == 17`. Update the existing `tests/test_help.py::test_help_json` and `tests/test_json_output.py::test_help_json` assertions from `== 16` to `== 17`. File: `tests/test_search.py` (new tests) + `tests/test_help.py` and `tests/test_json_output.py` (assertion updates).
- [x] [IMPL-SCH-18] Add the text rendering branch to the `search` command (when `as_json` is False). Use `rich.Table` or plain `console.print` to show columns: score, title, file, snippet. Add the `("search", "Search the brain for a query string")` row to `commands_info` in `help_cmd` (`cli.py:1566` area). → satisfies [TEST-SCH-17]

## Phase 2: Batch git + cleanup [completed]

Phase goal: collapse the per-neuron git loop in `_sync_brain_state` into
ONE subprocess call that returns BOTH `latest_by_path` and
`created_by_path`, fix the hidden 2x read in `update_frontmatter`, and
delete the dead `_get_recent_changes` helper. Six TEST-IMPL pairs.

- [x] [TEST-SCH-19] Write `tests/test_git.py::test_git_log_file_dates_returns_two_maps` — create a git repo in `tmp_path` with 4 commits: commit 1 adds `a.md`, commit 2 modifies `a.md` and adds `b.md`, commit 3 modifies `b.md`, commit 4 adds `c.md`. Use `GIT_AUTHOR_DATE` and `GIT_COMMITTER_DATE` env vars to make commit dates deterministic (e.g., `2026-01-01`, `2026-02-01`, `2026-03-01`, `2026-04-01`). Call `core.git.git_log_file_dates(repo_path)`. Assert: returns a tuple `(latest_by_path, created_by_path)`; `latest_by_path["a.md"] == "2026-02-01..."`, `latest_by_path["b.md"] == "2026-03-01..."`, `latest_by_path["c.md"] == "2026-04-01..."`; `created_by_path["a.md"] == "2026-01-01..."`, `created_by_path["b.md"] == "2026-02-01..."`, `created_by_path["c.md"] == "2026-04-01..."`. Also test: empty repo (no commits) returns `({}, {})`. Also test: non-git directory returns `({}, {})`. File: `tests/test_git.py`.
- [x] [IMPL-SCH-20] Add `git_log_file_dates(brain_path: Path) -> tuple[dict[str, str], dict[str, str]]` to `src/kluris/core/git.py`. Implementation:
  1. Call `_run(["git", "log", "--format=COMMIT %aI", "--name-only", "HEAD"], cwd=brain_path)`. Catch `subprocess.CalledProcessError` / non-zero return code → return `({}, {})`.
  2. Initialize `latest = {}`, `created = {}`, `current_date = None`.
  3. Walk `result.stdout.splitlines()`:
     - If line starts with `"COMMIT "`: `current_date = line[len("COMMIT "):].strip()`.
     - Elif line is empty: continue (blank lines separate commits).
     - Else: it's a file path. If `current_date is None`: skip (defensive). If `path not in latest`: `latest[path] = current_date` (newest-first walk: first occurrence is most recent). Always: `created[path] = current_date` (oldest-first effectively because we keep overwriting until the last/oldest occurrence wins).
  4. Return `(latest, created)`.
  Use `%aI` (NOT `%cI`) to match `core/git.py:114` and `:124`. Note: merge commits are skipped by `git log --name-only` by default; if any leak in (e.g. due to user git config), they're harmless because they have no file lines. Commits that touch files outside `_neuron_files`'s scope (e.g., `.github/workflows/ci.yml`) appear in the maps but are ignored by the caller because the caller only iterates `_neuron_files`. → satisfies [TEST-SCH-19]

- [x] [TEST-SCH-21] Write `tests/test_frontmatter.py::test_update_frontmatter_preloaded_skips_disk_read` — create a neuron file with frontmatter and body. Call `meta, body = read_frontmatter(path)`. Then **monkeypatch `frontmatter.load` to raise** (`monkeypatch.setattr("frontmatter.load", lambda *a, **k: pytest.fail("frontmatter.load was called"))`). Call `update_frontmatter(path, {"updated": "2026-04-07"}, preloaded=(meta, body))`. Assert it succeeds without invoking `frontmatter.load`. Read the file back and assert the new `updated` field is in place AND the body is unchanged. Also test: legacy call (no `preloaded`) still works after un-patching. File: `tests/test_frontmatter.py`.
- [x] [IMPL-SCH-22] Modify `update_frontmatter` in `core/frontmatter.py`. New signature: `def update_frontmatter(path: Path, patch: dict, *, preloaded: tuple[dict, str] | None = None) -> None`. If `preloaded is None`, behave exactly as today (`post = frontmatter.load(str(path)); meta = dict(post.metadata); body = post.content`). Else unpack `meta, body = preloaded` and **do not call `frontmatter.load`**. Then merge: `meta = dict(meta); meta.update(patch)`. Write: `path.write_text(frontmatter.dumps(frontmatter.Post(body, **meta)), encoding="utf-8")`. Defensive copy of `meta` so the caller's dict is not mutated. → satisfies [TEST-SCH-21]

- [x] [TEST-SCH-23] Write `tests/test_dream.py::test_sync_brain_state_uses_batch_git_with_exact_subprocess_count` — create a 100-neuron synthetic brain via a new `create_test_brain_with_neurons(runner, name, path, count=100)` helper added to `tests/conftest.py`. The helper writes N neuron files into `<brain>/projects/`, runs git add + commit on each. Then: monkeypatch `core.git._run` with a counting wrapper (also added to `conftest.py` as a fixture). Run `cli_runner.invoke(cli, ["dream"])`. Assert: `_run` was called **exactly 2 times**: one `is_git_repo()` check + one `git_log_file_dates()` call. (Was ~200 before the refactor.) Also assert: dream's exit_code == 0 and the JSON output reports `dates_updated` consistent with the test's expectations. File: `tests/test_dream.py`. New fixture: `tests/conftest.py::counting_git_run` and `tests/conftest.py::create_test_brain_with_neurons`.
- [x] [IMPL-SCH-24] Modify `_sync_brain_state` in `cli.py:135-197`:
  1. At the top of the function (before the neuron loop), call `is_git_repo(brain_path)`. If False: set `latest_by_path = {}; created_by_path = {}` and skip the `git_log_file_dates` call. If True: `latest_by_path, created_by_path = git_log_file_dates(brain_path)`.
  2. Replace the per-file git calls inside the neuron loop with `latest_by_path.get(rel_path)` and `created_by_path.get(rel_path)`.
  3. Replace `meta, _ = read_frontmatter(md)` with `meta, body = read_frontmatter(md)` so body is captured.
  4. Pass `preloaded=(meta, body)` to every `update_frontmatter` call inside the loop.
  5. Behavior must match the existing implementation 1:1: same files updated, same dates written. The two-pass `update_frontmatter` calls (one for `updated:`, one for `created:`) should each get a fresh `(meta, body)` tuple OR be consolidated into a single `update_frontmatter` call with both keys in the patch dict (cleaner). Recommendation: consolidate.
  6. Add `from kluris.core.git import git_log_file_dates, is_git_repo` to the function-local import line at `cli.py:145`. Remove the `git_file_last_modified` and `git_file_created_date` imports if no longer used. Also add the helper functions to `tests/conftest.py`. → satisfies [TEST-SCH-23]

- [x] [TEST-SCH-25] Write two test functions: (a) `tests/test_dream.py::test_sync_brain_state_handles_uncommitted_neurons` — create a brain via `temp_brain` (which makes 1 initial commit), then write a NEW neuron file that is NOT committed. Run dream. Assert dream completes with exit_code 0 AND the uncommitted neuron retains its scaffolded `updated:` field (i.e., the batch map didn't have it, so the loop silently skipped the date refresh — matches today). (b) `tests/test_dream.py::test_sync_brain_state_no_git_brain_skips_date_refresh` — create a brain with `--no-git`, write 3 neurons. Monkeypatch `_run` counting wrapper. Invoke dream. Assert exit_code 0 AND `_run` was called **at most 1 time** (just the `is_git_repo()` check; no batch call). All neurons retain their scaffolded frontmatter. File: `tests/test_dream.py`.
- [x] [IMPL-SCH-26] Verify the IMPL-SCH-24 changes already handle both cases. The uncommitted-neuron case is automatic: `latest_by_path.get(rel_path)` returns None for files absent from git, the loop's existing `if last_mod and ...:` guard skips the update. The no-git case is automatic: the `is_git_repo()` short-circuit prevents the batch call. If either test fails, fix the IMPL-SCH-24 logic until both pass. (No new code expected if IMPL-SCH-24 is correct.) → satisfies [TEST-SCH-25]

- [x] [TEST-SCH-27] Write `tests/test_git.py::test_git_log_file_dates_rename_history_parity` — create a git repo with one neuron, commit it; use `git mv` to rename it; commit again with deterministic `GIT_AUTHOR_DATE`. Call both the per-file helpers (`git_file_last_modified`, `git_file_created_date`) and the batch helper (`git_log_file_dates`) for the new path. Assert all four results match (within ISO date prefix). Closes Codex finding #8 rename parity.
- [x] [IMPL-SCH-28] Verified IMPL-SCH-20 already produces matching output. Without `-M`, both helpers use path matching (no rename detection), so they all return the rename commit's date for the new path. No new code. → satisfies [TEST-SCH-27]

- [x] [TEST-SCH-29] Write `tests/test_maps.py::test_get_recent_changes_dead_code_removed` — assert that `from kluris.core.maps import _get_recent_changes` raises `ImportError`. (Dead code is gone.) File: `tests/test_maps.py`.
- [x] [IMPL-SCH-30] Delete the `_get_recent_changes` function from `src/kluris/core/maps.py:99-114`. Verified no callers remain via grep. → satisfies [TEST-SCH-29]

---

## Resume Context

> Spec freshly forged 2026-04-08 and revised after Codex review. Ready
> to begin Phase 1. First task is TEST-SCH-01 (write the failing test
> for `_collect_searchable` in `core/search.py`). The target module
> does not exist yet — expect the test to fail with `ModuleNotFoundError`
> on the first run. Next file to create: `tests/test_search.py`. Then
> implement the collector in `src/kluris/core/search.py`.
>
> TDD Phase: RED — no test files yet.
> Failing Tests: — (none yet)
> Last Test Run: — (none yet)
> Next step: write `tests/test_search.py::test_collect_searchable_returns_neurons_glossary_brain_md`
> and run `.venv/bin/python -m pytest tests/test_search.py::test_collect_searchable_returns_neurons_glossary_brain_md -v`

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-04-08 | Drop Feature 3 (brain rename) from this spec | User explicitly excluded rename in interview round 2. Can be forged as its own spec later. |
| 2026-04-08 | Single-brain search (`allow_all=False`), not fan-out | User picked it. Cleaner JSON schema, agents already know which brain via per-brain skill names. |
| 2026-04-08 | Search includes neurons + glossary.md + brain.md | User picked it over neurons-only. Glossary terms are valuable first-class results; wake-up snapshot doesn't help human CLI users. |
| 2026-04-08 | `core/search.py` is a new module | Keeps cli.py under +80 lines and isolates the matching logic for standalone testing. |
| 2026-04-08 | Feature 2 scope: batch git + delete dead code + fix update_frontmatter double-read | User picked the full cleanup option. |
| 2026-04-08 | Phase 1 and Phase 2 are independently shippable | Zero code dependencies between them. Can ship as v2.2.0 bundled or as v2.1.1 + v2.2.0 separately. |
| 2026-04-08 | **(Codex review revision)** Scoring model: occurrence counts, NOT boolean hits | Codex finding #2: the original spec was internally inconsistent (acceptance criteria said boolean weights, IMPL said count, test asserted count). Picked occurrence counts because that's what the test fixture expects and what produces meaningful rankings (a neuron mentioning the query 5x is more relevant than one mentioning it once). Formula: `title_count * 10 + tag_count * 5 + path_count * 3 + body_count * 1`. |
| 2026-04-08 | **(Codex review revision)** Result schema uses `matched_fields: list[str]`, NOT `matched_in: str` | Codex finding #2: a hit in title AND body has no unambiguous singular `matched_in` value. `matched_fields` is a list of every field where the query matched, in order `title, tag, path, body`. Never empty (results with score 0 are filtered out). |
| 2026-04-08 | **(Codex review revision)** Glossary entries normalize to `{file: "glossary.md", title: <term>, body: <definition>, tags: []}` | Codex finding #2: original spec didn't say how glossary's `(term, definition)` shape mapped to the search schema. Each glossary term becomes one searchable item. The term is the title; the definition is the body. No tags. |
| 2026-04-08 | **(Codex review revision)** Deprecated detection from frontmatter, NOT `linker.detect_deprecation_issues` | Codex finding #3: `detect_deprecation_issues` returns issue states (`active_links_to_deprecated`, etc.), not the full set of deprecated neurons. A deprecated neuron with valid `replaced_by` and no incoming links would be missed. Fix: read `meta.get("status", "active")` directly during `_collect_searchable`. |
| 2026-04-08 | **(Codex review revision)** Batch git returns TWO maps: `(latest_by_path, created_by_path)` | Codex finding #1: original spec proposed one map but `_sync_brain_state` needs both (`updated:` from latest, `created:` from oldest). Single git log walk produces both: walking newest-first, latest is "first time path is seen" and created is "last time path is seen" (overwrite). |
| 2026-04-08 | **(Codex review revision)** Batch git uses `%aI` (author date), NOT `%cI` (committer date) | Codex finding #1: today's `git_file_last_modified` (`core/git.py:114`) and `git_file_created_date` (`core/git.py:124`) both use `%aI`. Matching them byte-for-byte preserves test expectations and existing brain frontmatter. |
| 2026-04-08 | **(Codex review revision)** `update_frontmatter` API is `preloaded=(meta, body)` tuple | Codex finding #5: the original spec was internally contradictory about the signature. Picked the tuple form because it eliminates a SECOND READ entirely (not just a second parse). Caller (`_sync_brain_state`) already has both meta and body from the initial `read_frontmatter` call. |
| 2026-04-08 | **(Codex review revision)** Task slicing: each IMPL does only the minimum for its preceding TEST | Codex finding #4: original IMPL-SCH-08 added lobe/tag/deprecated handling all at once, making subsequent TEST tasks pass immediately. Re-sliced so IMPL-SCH-08 has core search only, IMPL-SCH-10 adds filters, IMPL-SCH-12 adds the deprecated flag. Each TEST→IMPL pair is one minimal red-green cycle. |
| 2026-04-08 | **(Codex review revision)** Removed "no-code" IMPL tasks; consolidated into prior IMPLs or made tests broader | Codex finding #4: original IMPL-SCH-16 / IMPL-SCH-30 / IMPL-SCH-36 were "verify wiring, no new code" — not real green steps. Restructured: TEST-SCH-13 now bundles JSON + picker + `--brain all` rejection so IMPL-SCH-14 satisfies all three with one command implementation. Phase 2's no-code IMPLs (TEST-SCH-25, TEST-SCH-27) explicitly say "verify IMPL-SCH-24/IMPL-SCH-20 handles this; no new code expected if prior IMPL is correct" — they're verification gates, not phantom green steps. |
| 2026-04-08 | **(Codex review revision)** Subprocess count assertion is exact, not `≤5` | Codex finding #7: original `≤5` was too loose. Tightened: dream on a 100-neuron git brain should call `_run` exactly 2 times (one `is_git_repo` + one batch `git_log_file_dates`). On a no-git brain: at most 1 (`is_git_repo` returns False, batch is skipped). |
| 2026-04-08 | **(Codex review revision)** Picker tests monkeypatch `kluris.cli._is_interactive`, NOT `sys.stdin.isatty` | Codex finding #6: `CliRunner` swaps `sys.stdin` during invoke, so isatty patches don't survive. The codebase already follows this pattern at `tests/test_brain_resolution.py:48-49`. Spec now states this explicitly in TEST-SCH-13 and the Testing Architecture section. |
| 2026-04-08 | **(Codex review revision)** Edge case tests added: literal special chars + non-ASCII | Codex finding #8: spec was missing tests for literal regex-special-character queries (`a.b*?`) and non-ASCII queries (`café`). Added to TEST-SCH-15. |
| 2026-04-08 | **(Codex review revision)** Rename history parity test added | Codex finding #8: spec needed an explicit test that the batch implementation produces the same dates as the per-file helpers for renamed files. Added as TEST-SCH-27. |
| 2026-04-08 | Snippet length cap = 200 chars | Matches the intent of wake-up's compact index. UTF-8 safe via str slicing (not bytes). |
| 2026-04-08 | No subprocess mocking via `unittest.mock` | Codebase convention is real git + tmp_path. Batch-git tests use `monkeypatch.setattr(core.git, "_run", counting_wrapper)` instead. |
| 2026-04-08 | Total task count after Codex revision: 30 (was 36) | Removed 3 no-code IMPL pairs from Phase 1 + restructured Phase 2 to drop redundant pairs and add the rename-parity test. Phase 1: 9 pairs (18 tasks). Phase 2: 6 pairs (12 tasks). |

## TDD Log

| Task | Red | Green | Refactor |
|------|-----|-------|----------|
| [TEST-SCH-01] | pytest test_search.py: 1 test, 1 failed — `ModuleNotFoundError: No module named 'kluris.core.search'` | — | — |
| [IMPL-SCH-02] | — | pytest test_search.py: 1 passed. Full suite: 312 passed (was 311). | none |
| [TEST-SCH-03] | pytest test_search.py: 4 tests, 3 failed — `ImportError: cannot import name '_score_hit'/_matched_fields` | — | — |
| [IMPL-SCH-04] | — | pytest test_search.py: 4 passed (collector + 3 scoring/matched_fields tests). | none |
| [TEST-SCH-05] | pytest test_search.py: 10 tests, 6 failed — `ImportError: cannot import name '_extract_snippet'` | — | — |
| [IMPL-SCH-06] | — | pytest test_search.py: 10 passed (4 prior + 6 snippet tests). | none |
| [TEST-SCH-07] | pytest test_search.py: 13 tests, 3 failed — `ImportError: cannot import name 'search_brain'` | — | — |
| [IMPL-SCH-08] | — | pytest test_search.py: 13 passed. Discovery: H1 line is part of `read_frontmatter`'s body, so a title hit also counts as a body hit. Updated test fixture expectations from 10→11 for the title-only neuron. This is correct behavior, not a bug. | none |
| [TEST-SCH-09] | pytest test_search.py: 17 tests, 4 failed — `TypeError: search_brain() got an unexpected keyword argument 'lobe_filter'` | — | — |
| [IMPL-SCH-10] | — | pytest test_search.py: 17 passed. | none |
| [TEST-SCH-11] | pytest test_search.py: 19 tests, 2 failed — `KeyError: 'deprecated'` (the deprecation test fixture includes a deprecated neuron with valid replaced_by and no incoming links — naive deprecation detection via `detect_deprecation_issues` would have missed it; this test enforces direct frontmatter check) | — | — |
| [IMPL-SCH-12] | — | pytest test_search.py: 19 passed. Codex finding #3 closed. | none |
| [TEST-SCH-13] | pytest test_search.py: 22 tests, 3 failed — search command does not exist (`UsageError: No such command 'search'`) | — | — |
| [IMPL-SCH-14] | — | pytest test_search.py: 22 passed (after dropping `--json` from the picker test, which forces non-interactive mode and would never trigger the picker). | none |
| [TEST-SCH-15] | pytest test_search.py: 26 passed — **TDD anomaly**: empty-query guard, literal-substring, and UTF-8 behavior were all built prophylactically into IMPL-SCH-04/06/14 so the new tests passed without a RED state. Logged in Deviations table. The tests serve as regression coverage. | — | — |
| [IMPL-SCH-16] | — | No additional code needed (already in IMPL-SCH-14). | none |
| [TEST-SCH-17] | pytest test_search.py: 28 tests, 1 failed — `assert 16 == 17` (search not yet in commands_info) | — | — |
| [IMPL-SCH-18] | — | pytest test_search.py + test_help.py + test_json_output.py: 45 passed. Full suite: 340 passed (was 312). **Phase 1 complete.** | none |
| [TEST-SCH-19] | pytest test_git.py: 14 tests, 4 failed — `ImportError: cannot import name 'git_log_file_dates'` | — | — |
| [IMPL-SCH-20] | — | pytest test_git.py: 14 passed (10 prior + 4 batch tests including the %aI vs %cI verification). | none |
| [TEST-SCH-21] | pytest test_frontmatter.py: 10 tests, 2 failed — `update_frontmatter() got an unexpected keyword argument 'preloaded'` | — | — |
| [IMPL-SCH-22] | — | pytest test_frontmatter.py: 10 passed (8 prior + 2 preloaded tests). | none |
| [TEST-SCH-23] | pytest test_dream.py: 1 test failed — `assert 100 == 2` (per-file `git log -1 --format=%aI -- <file>` calls, exactly as the bug predicted) | — | — |
| [IMPL-SCH-24] | — | pytest test_dream.py: 1 passed (subprocess count exactly 2). Full suite: 348 passed (was 340). | none |
| [TEST-SCH-25] | pytest test_dream.py: 2 new tests, both passed immediately — uncommitted-neuron and no-git-brain cases are handled by IMPL-SCH-24's `latest_by_path.get()` and `is_git_repo()` short-circuit (verification gate, documented in spec). | — | — |
| [IMPL-SCH-26] | — | No additional code (verification gate). pytest test_dream.py: all passing. | none |
| [TEST-SCH-27] | pytest test_git.py: 1 new test passed immediately — IMPL-SCH-20's batch helper produces parity with per-file helpers (verification gate, both use path matching with no `-M`). | — | — |
| [IMPL-SCH-28] | — | No additional code (verification gate). | none |
| [TEST-SCH-29] | pytest test_maps.py: 1 test failed — `Failed: DID NOT RAISE ImportError` (the dead code was still importable) | — | — |
| [IMPL-SCH-30] | — | pytest full suite: 352 passed (was 348). `_get_recent_changes` deleted from `core/maps.py:99-114`. **Phase 2 complete. Spec complete.** | none |

## Deviations

| Task | Spec Said | Actually Did | Why |
|------|-----------|-------------|-----|
| TEST-SCH-07 | A title-only neuron (`# OAuth flow`) gets score 10 (1 title hit) | A title-only neuron gets score 11 because `read_frontmatter` returns the body INCLUDING the H1 line, so the title hit also counts as a body hit (10 + 1 = 11) | This is correct behavior per the occurrence-count formula, not a bug. Updated test fixture expectations from 10→11 to match reality. The H1-as-body double-count is desirable: it slightly boosts neurons whose title contains the query, on top of the 10x title weight. |
| TEST-SCH-15 | Empty-query guard added in IMPL-SCH-16 (one task after the test) | Empty-query guard was added preemptively in IMPL-SCH-14, so TEST-SCH-15's empty-query test passed immediately (no RED state) | The guard is a single-line check at the top of the search command body. Adding it during the CLI command IMPL was natural; splitting it out into a separate task would have been pedantic. The other three TEST-SCH-15 tests (no-match, special chars, UTF-8) also passed because IMPL-SCH-04 (`str.count`, no `re`) and IMPL-SCH-06 (str slicing, not bytes) were already correct. The tests serve as regression coverage and as proof of literal-substring + UTF-8 contracts. |
