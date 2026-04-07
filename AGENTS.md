# AGENTS.md

## Project: kluris-cli

CLI tool that turns AI agents into team subject matter experts with shared, human-curated knowledge.
Published to PyPI as `kluris`. Source: `ngvoicu/kluris-cli`.

## Quick Reference

```bash
pip install -e ".[dev]"          # dev install
pytest tests/ -v                 # 255 tests
pytest tests/ --cov=kluris -q    # 90%+ coverage
```

## Source Layout

All CLI commands are in `src/kluris/cli.py` (single file).
Core logic is in `src/kluris/core/` (8 modules).
Agent skill and workflow templates are inline strings in `src/kluris/core/agents.py`.
No Jinja2 templates -- dependency was removed.

## Key Files

- `src/kluris/cli.py` -- all 17 Click commands, wizard logic, KlurisGroup error handler, `wake-up` command and collectors
- `src/kluris/core/agents.py` -- AGENT_REGISTRY (8 agents), single `kluris` skill renderer. SKILL_BODY contains Bootstrap / Query first / Brain selection sections.
- `src/kluris/core/brain.py` -- BRAIN_TYPES, NEURON_TEMPLATES, scaffold_brain(), _generate_readme()
- `src/kluris/core/config.py` -- Pydantic models, config read/write, register/unregister
- `src/kluris/core/maps.py` -- generate_brain_md(), generate_map_md()
- `src/kluris/core/linker.py` -- synapse validation, bidirectional checks, orphan detection, **detect_deprecation_issues()**
- `src/kluris/core/mri.py` -- graph building, standalone HTML generation
- `src/kluris/core/git.py` -- subprocess git wrapper

## Agent Bootstrap Protocol

On the first `/kluris` call of a session, the agent runs `kluris wake-up --json`
via Bash and caches the output. Subsequent calls reuse the cache until one of
these mutating commands fires: `/kluris remember`, `/kluris learn`,
`kluris neuron`, `kluris lobe`, `kluris dream`, `kluris push`. The instruction
is baked into SKILL_BODY's Bootstrap section.

## Deprecation Frontmatter

Neurons may opt into deprecation with `status: deprecated`, `deprecated_at: YYYY-MM-DD`,
and `replaced_by: ./path/to/new.md`. `linker.detect_deprecation_issues()` reports
three kinds: `active_links_to_deprecated`, `deprecated_without_replacement`,
`replaced_by_missing`. `kluris dream` surfaces them as non-blocking warnings
(text + `--json`).

## Constraints

- All file I/O must use `encoding="utf-8"` (Windows compatibility)
- All paths must use `pathlib.Path` (cross-platform)
- Global config at `~/.kluris/config.yml` (override: KLURIS_CONFIG env var)
- `kluris.yml` in brains is gitignored -- local config only
- Brain types (product-group, personal, product, research, blank) are scaffold-only
- NEURON_TEMPLATES (decision, incident, runbook) are available to all brains
- brain.md is lightweight (root lobes only, no neuron index)
- Agents navigate hierarchically: wake-up snapshot -> brain.md -> map.md -> neurons
- Slash command: 1 (/kluris handles search, learn, remember, and create -- push and dream are CLI-only)
- Version must be updated in both pyproject.toml and src/kluris/__init__.py
- Tests must pass before pushing: `pytest tests/ -q`
- CI runs on PR only (ubuntu, macos, windows x Python 3.10-3.13)
- Tags trigger PyPI publish: `git tag v0.X.Y && git push origin v0.X.Y`
