# CLAUDE.md

## What This Is

Kluris turns AI agents into team subject matter experts by giving them shared, human-curated knowledge stored in git-backed **brains**.

**Kluris = the tool. A brain = the git repo it creates.**

## Build & Test

```bash
cd kluris-cli
source .venv/bin/activate        # or: pipx install -e .
pip install -e ".[dev]"          # dev install with pytest
pytest tests/ -v                 # run all tests (255 tests)
pytest tests/ --cov=kluris -q    # with coverage (90%+)
pytest tests/test_create.py -v   # single test file
```

## Architecture

```
src/kluris/
  cli.py              # Click CLI -- all 17 commands in one file (incl. wake-up)
  core/
    config.py          # Pydantic models (GlobalConfig, BrainConfig, BrainEntry)
    brain.py           # BRAIN_TYPES, NEURON_TEMPLATES, scaffold_brain(), validate_brain_name()
    maps.py            # generate_brain_md(), generate_map_md() -- auto-generated files
    frontmatter.py     # read_frontmatter(), write_frontmatter(), update_frontmatter()
    linker.py          # validate_synapses(), validate_bidirectional(), detect_orphans(),
                       # detect_deprecation_issues()
    mri.py             # build_graph(), generate_mri_html() -- standalone HTML viz
    git.py             # git_init(), git_add(), git_commit(), git_log(), git_push(), etc.
    agents.py          # AGENT_REGISTRY (8 agents), single `kluris` skill/workflow renderer
```

## Key Design Decisions

- **All commands in one cli.py** -- not split into separate files. Works fine at current size.
- **No Jinja2** -- templates are inline Python strings in brain.py and agents.py. Jinja2 was removed from dependencies.
- **kluris.yml is gitignored** -- local config only (agents, git branch). Not shared between team members.
- **Brain types are scaffold-only** -- after creation, type is irrelevant. All templates available everywhere.
- **NEURON_TEMPLATES are global** -- decision, incident, runbook available to every brain regardless of type.
- **brain.md is lightweight** -- root lobes + glossary link only. No neuron index. Agents navigate through map.md hierarchy.
- **Agent skill/workflow templates are inline** in agents.py, not .j2 template files.
- **MRI uses inline canvas JS** -- no vendored Cytoscape.js. Standalone HTML with search, inspector, and interactive graph navigation.
- **Cross-platform** -- all file I/O uses `encoding="utf-8"`, all paths use `pathlib.Path`.
- **wake-up bootstrap protocol** -- SKILL.md instructs the agent to run `kluris wake-up --json` on the first `/kluris` of a session (via Bash), cache the snapshot, and refresh only after brain-mutating commands. This replaces walking brain.md -> map.md -> neurons on every turn.
- **Deprecation frontmatter is opt-in** -- neurons may set `status: deprecated` + `deprecated_at` + `replaced_by`. Absence of `status` means active. `linker.detect_deprecation_issues()` surfaces 3 kinds of warnings through `kluris dream`; they are non-blocking (do not break `healthy`).
- **KlurisGroup detects --json via ctx args** -- scans `ctx.protected_args + ctx.args` in addition to `sys.argv` so JSON error output works under CliRunner (tests) as well as shell.

## Config Paths

- **Global config:** `~/.kluris/config.yml` (override: `KLURIS_CONFIG` env var)
- **Brain config:** `<brain>/kluris.yml` (gitignored, local only)
- **Installed skills:** `~/.claude/skills/`, `~/.cursor/skills/`, `~/.copilot/skills/`, etc.

## Agent Registry (8 agents)

| Agent | Dir | Format |
|-------|-----|--------|
| claude | ~/.claude/skills/kluris/ | SKILL.md |
| cursor | ~/.cursor/skills/kluris/ | SKILL.md |
| windsurf | ~/.codeium/windsurf/skills/kluris/ | SKILL.md + workflow |
| copilot | ~/.copilot/skills/kluris/ | SKILL.md |
| codex | ~/.codex/skills/kluris/ | SKILL.md |
| gemini | ~/.gemini/skills/kluris/ | SKILL.md |
| kilocode | ~/.kilo/skills/kluris/ | SKILL.md |
| junie | ~/.junie/skills/kluris/ | SKILL.md |

## Agent Skill

One `kluris` skill is installed across supported agents.
Windsurf also gets a `kluris.md` workflow for manual invocation.
Search and guided documentation happen through `/kluris` in the agent, not a separate CLI search command.

The skill body contains four load-bearing sections (in order):

1. **`{brain_info}` block** -- rendered per install with every registered brain and its absolute path; one is marked `(default)`.
2. **Bootstrap** -- tells the agent to call `kluris wake-up --json` on the first `/kluris` of a session, cache the result, and refresh only after mutating commands.
3. **Query first -- never guess** -- enforces "check the brain before answering" and "never fabricate brain content".
4. **Brain selection** -- three-tier rule for picking a brain when multiple are registered: exact name > cwd path hint > default.

## CLI Commands (17)

create, clone, list, status, wake-up, neuron, lobe, dream, push, mri, templates, install-skills, uninstall-skills, remove, doctor, help, use

## Brain File Structure

```
<brain>/
  kluris.yml      # local config (gitignored)
  brain.md        # root lobes directory (auto-generated by dream)
  glossary.md     # domain terms (hand-edited)
  README.md       # usage guide (generated once, then hand-editable)
  .gitignore      # secrets, kluris.yml, brain-mri.html
  <lobe>/
    map.md        # lobe contents (auto-generated by dream)
    <neuron>.md   # knowledge files
    <sub-lobe>/
      map.md      # nested lobe contents
```

## Key Conventions

- **encoding="utf-8"** on every write_text() and read_text() call -- Windows compat
- **validate_brain_name()** -- lowercase alphanumeric + hyphens only
- **git_init() sets user.email/name** -- for CI/test environments without global git config
- **_do_install() does clean-slate** -- deletes existing kluris* files before writing new ones
- **_run_dream_on_brain()** -- called after neuron/lobe creation to regenerate maps
- **KlurisGroup** -- custom Click group that outputs JSON errors when --json is in args (scans ctx args + sys.argv)
- **All commands support --json** -- structured output for scripting
- **Deprecation frontmatter** -- optional `status`, `deprecated_at`, `replaced_by` on neurons; dream reports warnings, doesn't break healthy
- **wake-up output schema** -- `{ok, name, path, is_default, description, lobes[{name, neurons}], total_neurons, recent[{path, updated}]}`

## Testing

- 255 tests across 28 test files
- conftest.py has 5 fixtures: cli_runner, temp_config, temp_home, temp_brain, bare_remote
- Tests use monkeypatch for KLURIS_CONFIG and HOME env vars
- Git tests use real git in tmp_path (not mocked)
- bare_remote fixture sets HEAD to refs/heads/main (CI compat)

## CI/CD

- `.github/workflows/ci.yml` -- tests on PR only (ubuntu, macos, windows x Python 3.10-3.13)
- `.github/workflows/publish.yml` -- publish to PyPI on tag v*
- Version in pyproject.toml AND src/kluris/__init__.py (must match)
