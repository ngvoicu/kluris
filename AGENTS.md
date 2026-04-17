# AGENTS.md

## Project: kluris-cli

CLI tool that turns AI agents into team subject matter experts with shared, human-curated knowledge.
Published to PyPI as `kluris`. Source: `ngvoicu/kluris-cli`.

## Quick Reference

```bash
pip install -e ".[dev]"          # dev install
pytest tests/ -v                 # 473 tests
pytest tests/ --cov=kluris -q    # 90%+ coverage
```

## Source Layout

All CLI commands are in `src/kluris/cli.py` (single file).
Core logic is in `src/kluris/core/` (8 modules).
Agent skill and workflow templates are inline strings in `src/kluris/core/agents.py`.
No Jinja2 templates -- dependency was removed.

## Key Files

- `src/kluris/cli.py` -- all 20 Click commands (incl. `register` added in v2.9.0), wizard logic, KlurisGroup error handler, `search` + `wake-up` commands and collectors, `_resolve_brains` (picker + non-TTY guard + `--brain all`), `_do_install` (per-destination atomic stage-then-rename), `_sync_brain_state` (batch git log + `update_frontmatter(preloaded=...)`), `_locate_brain_root` + `_get_git_origin_url` helpers used by register
- `src/kluris/core/agents.py` -- AGENT_REGISTRY (8 agents), per-brain SKILL.md renderer (`render_skill(skill_name, brain_name, brain_path, has_git, brain_description)`). With 1 brain registered the skill is named `kluris`; with N brains each gets `kluris-<name>`. SKILL_BODY contains a single-brain header, Bootstrap, Query first, Intent detection (uses `kluris search` as the primary search path), Writing rules, and CLI commands sections. Brain paths are emitted in POSIX form via `_posix_path()` so bash on Windows handles them correctly.
- `src/kluris/core/brain.py` -- BRAIN_TYPES, NEURON_TEMPLATES, scaffold_brain(), _generate_readme(), validate_brain_name() (rejects reserved word `all`, max 48 chars)
- `src/kluris/core/config.py` -- Pydantic models, config read/write (with legacy `default_brain` shim), register/unregister
- `src/kluris/core/maps.py` -- generate_brain_md(), generate_map_md()
- `src/kluris/core/linker.py` -- synapse validation, bidirectional checks, orphan detection, **detect_deprecation_issues()**, type-aware check_frontmatter()
- `src/kluris/core/search.py` -- powers `kluris search`. `_collect_searchable` walks neurons + glossary + brain.md, `_score_hit` uses occurrence-count ranking, `_extract_snippet` is UTF-8 safe, `search_brain` is the public entry point.
- `src/kluris/core/frontmatter.py` -- read_frontmatter, write_frontmatter, `update_frontmatter(path, patch, *, preloaded=(meta, body)?)` — the preloaded form skips the second disk read, used by dream's hot path
- `src/kluris/core/mri.py` -- graph building, standalone HTML generation with file-browser tree sidebar in the expand modal
- `src/kluris/core/git.py` -- subprocess git wrapper; `git_log_file_dates()` returns `(latest_by_path, created_by_path)` from one walk — replaces the per-file helpers entirely

## Agent Bootstrap Protocol

On the first `/<skill>` call of a session, the agent runs `kluris wake-up --json`
(or `kluris wake-up --brain <name> --json` for per-brain skills) via Bash and
caches the output. Subsequent calls reuse the cache until one of these
mutating commands fires: `/<skill> remember`, `/<skill> learn`, `kluris neuron`,
`kluris lobe`, `kluris dream`, `kluris push`. The instruction is baked into
SKILL_BODY's Bootstrap section.

## Deprecation Frontmatter

Neurons may opt into deprecation with `status: deprecated`, `deprecated_at: YYYY-MM-DD`,
and `replaced_by: ./path/to/new.md`. `linker.detect_deprecation_issues()` reports
four kinds: `active_links_to_deprecated`, `deprecated_without_replacement`,
`replaced_by_missing`, `replaced_by_not_active` (replaced_by points to a
deprecated neuron or a non-neuron file). `kluris dream` surfaces them as
non-blocking warnings (text + `--json`). `kluris wake-up --json` exposes a
`deprecation_count` summary for agent bootstrap.

## Constraints

- All file I/O must use `encoding="utf-8"` (Windows compatibility)
- All paths must use `pathlib.Path` (cross-platform)
- Global config at `~/.kluris/config.yml` (override: KLURIS_CONFIG env var)
- `kluris.yml` in brains is gitignored -- local config only
- Brain types (product-group, personal, product, research, blank) are scaffold-only
- NEURON_TEMPLATES (decision, incident, runbook) are available to all brains
- brain.md is lightweight (root lobes only, no neuron index)
- Agents navigate hierarchically: wake-up snapshot -> brain.md -> map.md -> neurons
- Slash command: one per registered brain. With 1 brain → `/kluris`. With 2+ brains → `/kluris-<name>` per brain. Each handles search, learn, remember, and create -- push and dream are CLI-only. The search intent tells the agent to call `kluris search "<query>" --brain <name> --json` via Bash as the fast path and fall back to manual brain.md → map.md navigation only if the CLI returns zero results.
- `_do_install()` is called by six commands: `create`, `clone`, `register` (v2.9.0), `remove`, `install-skills`, and `doctor` (added in v2.2.2). `doctor` is the post-`pipx upgrade kluris` refresh path -- run it once after every kluris version bump to refresh the skill files baked into `~/.<agent>/skills/kluris*`. Pass `--no-refresh` to keep doctor read-only.
- `register` (v2.9.0) is the sibling of `clone` for non-git sources. Takes a directory path (registers in-place, no copy) or a `.zip` file (extracts to `~/<basename>` or `--dest`, then registers). Identity comes from `brain.md` H1 via `_read_brain_identity`. Auto-detects `git remote get-url origin` via `_get_git_origin_url` to populate `repo`. Zip handling validates each member's resolved path stays inside the extraction root (zip-slip defense; Python < 3.12's `extractall` does not do this). Cleanup: zip source → remove extracted dir on failure; directory source → NEVER delete.
- Version must be updated in both pyproject.toml and src/kluris/__init__.py
- Tests must pass before pushing: `pytest tests/ -q`
- CI runs on PR only (ubuntu, macos, windows x Python 3.10-3.13)
- Tags trigger PyPI publish: `git tag v0.X.Y && git push origin v0.X.Y`
