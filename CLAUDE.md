# CLAUDE.md

## What This Is

Kluris turns AI agents into team subject matter experts by giving them shared, human-curated knowledge stored in git-backed **brains**.

**Kluris = the tool. A brain = the git repo it creates.**

## Knowledge base

Read and write to the **ngvoicu-sme** brain through kluris (never edit brain
files by hand). Use the `/kluris-ngvoicu-sme` skill — search, learn, remember,
create.

Topics relevant to this repo: kluris-cli overview, architecture, multi-brain resolver, agent skills, kluris-pack, wake-up protocol, testing, CI/CD, deprecation.

## Build & Test

```bash
source .venv/bin/activate        # or: pipx install -e .
pip install -e ".[dev]"          # dev install with pytest
pytest tests/ -v                 # run all tests (482 tests)
pytest tests/ --cov=kluris -q    # with coverage (90%+)
pytest tests/test_create.py -v   # single test file
```

## Architecture

```
src/kluris/
  cli.py              # Click CLI -- all 13 commands in one file (incl. search, wake-up, pack, register, companion)
  core/
    config.py          # Pydantic models (GlobalConfig, BrainConfig, BrainEntry)
    brain.py           # BRAIN_TYPES, scaffold_brain(), validate_brain_name()
    maps.py            # generate_brain_md(), generate_map_md() -- auto-generated files
    frontmatter.py     # read_frontmatter(), write_frontmatter(),
                       # update_frontmatter(path, patch, *, preloaded=(meta, body)?)
    linker.py          # validate_synapses(), validate_bidirectional(), detect_orphans(),
                       # detect_deprecation_issues(), check_frontmatter() with type validation
    search.py          # _collect_searchable(), _score_hit(), _matched_fields(),
                       # _extract_snippet(), search_brain() -- backs `kluris search`
    mri.py             # build_graph(), generate_mri_html() -- standalone HTML viz
    git.py             # git_init(), git_add(), git_commit(), git_log(),
                       # git_log_file_dates() batch helper (one subprocess replaces 2N)
    agents.py          # AGENT_REGISTRY (8 agents), per-brain SKILL.md renderer
                       # (always kluris-<name>, even for one brain);
                       # brain_path emitted in POSIX form (C:/ on Windows) for bash
```

## Key Design Decisions

- **All commands in one cli.py** -- not split into separate files. Works fine at current size.
- **No Jinja2** -- templates are inline Python strings in brain.py and agents.py. Jinja2 was removed from dependencies.
- **kluris.yml is gitignored** -- local config only (agents, companions). Not shared between team members.
- **Brain types are scaffold-only** -- after creation, type is irrelevant. Same commands everywhere.
- **brain.md is lightweight** -- root lobes + glossary link only. No neuron index. Agents navigate through map.md hierarchy.
- **Agent skill/workflow templates are inline** in agents.py, not .j2 template files.
- **MRI uses inline canvas JS** -- no vendored Cytoscape.js. Standalone HTML with search, inspector, and interactive graph navigation.
- **Cross-platform** -- all file I/O uses `encoding="utf-8"`, all paths use `pathlib.Path`.
- **wake-up bootstrap protocol** -- SKILL.md instructs the agent to run `kluris wake-up --brain <name> --json` on the first `/<skill>` of a session (via Bash), cache the snapshot, and refresh only after brain-mutating commands. This replaces walking brain.md -> map.md -> neurons on every turn.
- **Per-brain skill scheme** -- every registered brain installs as one `kluris-<name>` skill, even when it is the only brain, so project pointers stay stable as more brains are added. `kluris doctor` sweeps the entire `kluris*` artifact set before writing the named layout, so legacy bare `kluris` installs migrate automatically. Per-destination atomic via stage-then-rename so a partial-write failure leaves the OLD skill in place.
- **No default brain** -- resolution order is: explicit `--brain NAME` → exactly 1 brain registered → interactive picker (TTY only). Non-TTY / `--json` / `KLURIS_NO_PROMPT=1` all force the resolver to error out instead of prompting.
- **`--brain all`** -- accepted only on fan-out commands (dream, status, mri, companion add/remove). Other commands reject it with a clear error. `all` is also a reserved brain name to prevent collision.
- **Sticky-selection tradeoff** -- `kluris use` was removed deliberately. Repeated commands in a terminal session each trigger the picker (or take `--brain`). The compensating affordances are `KLURIS_NO_PROMPT`, `--brain all`, the integer-pick UX, and the per-brain slash command names that make ambiguity disappear in agent workflows.
- **Deprecation frontmatter is opt-in** -- neurons may set `status: deprecated` + `deprecated_at` + `replaced_by`. Absence of `status` means active. `linker.detect_deprecation_issues()` surfaces 4 kinds of warnings (`active_links_to_deprecated`, `deprecated_without_replacement`, `replaced_by_missing`, `replaced_by_not_active`) through `kluris dream`; they are non-blocking (do not break `healthy`). `kluris wake-up` exposes a `deprecation_count` summary.
- **KlurisGroup detects --json via ctx args** -- scans `ctx.protected_args + ctx.args` in addition to `sys.argv` so JSON error output works under CliRunner (tests) as well as shell.
- **Companions are embedded, not installed as separate skills** -- specmint-core/tdd ship inside the kluris package and are copied to `~/.kluris/companions/<name>/SKILL.md` on opt-in. Layer-1 SKILL.md references their paths; they are NOT auto-loaded as agent skills.
- **BrainConfig.companions** -- per-brain opt-in list; missing from older kluris.yml defaults to `[]`.
- **Companion refresh on doctor** -- `kluris doctor` calls `companions.refresh()` for the union of referenced and installed known companions so pipx-upgraded kluris auto-updates bundled playbooks without version comparison.

## Config Paths

- **Global config:** `~/.kluris/config.yml` (override: `KLURIS_CONFIG` env var)
- **Brain config:** `<brain>/kluris.yml` (gitignored, local only)
- **Installed skills:** `~/.claude/skills/`, `~/.cursor/skills/`, `~/.copilot/skills/`, etc.

## Agent Registry (8 agents)

| Agent | Dir | Format |
|-------|-----|--------|
| claude | ~/.claude/skills/kluris-&lt;name&gt;/ | SKILL.md |
| cursor | ~/.cursor/skills/kluris-&lt;name&gt;/ | SKILL.md |
| windsurf | ~/.codeium/windsurf/skills/kluris-&lt;name&gt;/ | SKILL.md + workflow |
| copilot | ~/.copilot/skills/kluris-&lt;name&gt;/ | SKILL.md |
| codex | ~/.codex/skills/kluris-&lt;name&gt;/ | SKILL.md |
| gemini | ~/.gemini/skills/kluris-&lt;name&gt;/ | SKILL.md |
| kilocode | ~/.kilo/skills/kluris-&lt;name&gt;/ | SKILL.md |
| junie | ~/.junie/skills/kluris-&lt;name&gt;/ | SKILL.md |

The universal `~/.agents/skills/` slot mirrors the same per-brain layout for tools that scan it.

## Agent Skill

Every brain registers as one `kluris-<name>` skill. Each rendered SKILL.md is bound to exactly one brain — there is no runtime brain picker inside the skill body. Windsurf also gets one `kluris-<name>.md` workflow file per brain.

The skill body contains six load-bearing sections (in order):

1. **Brain header** -- single-brain block at the top: `# Brain: {name}` with the absolute path and git label, plus a per-skill `--brain <name>` instruction when the skill name is `kluris-<X>`.
2. **Bootstrap** -- tells the agent to call `kluris wake-up{brain_flag} --json` on the first `/<skill>` of a session, cache the result, and refresh only after mutating commands.
3. **Query first -- never guess** -- enforces "check the brain before answering" and "never fabricate brain content".
4. **How the brain is structured** -- top-down navigation rules.
5. **Intent detection** -- search/think/learn/remember/create patterns with the right brain-flag interpolation.
6. **Writing rules + CLI commands** -- frontmatter contract and the mechanical CLI examples.

## Multi-brain CLI behavior

- **0 brains** → resolver errors with a hint to run `kluris create`.
- **1 brain** → auto-resolves for terminal CLI convenience, but the installed skill is still `kluris-<name>` and passes `--brain <name>`.
- **2+ brains + TTY** → interactive picker `[1] foo [2] bar [3] all` for fan-out commands (dream/status/mri/companion add/remove); `[1] foo [2] bar` (no `all`) for single-brain commands (wake-up/search).
- **2+ brains + non-interactive** (`--json`, no TTY, or `KLURIS_NO_PROMPT=1`) → resolver errors with the available brains listed and a hint to pass `--brain NAME` or `--brain all`.
- **Stale brain paths** → annotated `(missing)` in the picker; resolver raises `ClickException` if the user actually tries to use one.

## CLI Commands (13)

create, register, list, status, search, wake-up, companion, dream, pack, mri, remove, doctor, help

- **`register`** -- register a brain directory already on disk (in-place, no copy). Use `git clone <url> <path>` first if the brain is hosted on git, then `kluris register <path>`. Identity comes from `brain.md` H1. Calls `_do_install()` on success so agent skills refresh to include the new brain. Passing a `.zip` errors with a hint to `unzip` first (zip support was removed in 2.16.0).

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
- **validate_brain_name()** -- lowercase alphanumeric + hyphens only; max 48 chars; rejects reserved word `all`
- **git_init() sets user.email/name** -- for CI/test environments without global git config
- **_do_install() per-destination atomic** -- stages new SKILL.md files into sibling temp dirs, sweeps `kluris*` artifacts only after successful staging, then atomically renames staged dirs into place. Partial-write failures leave the OLD skill in place for that destination.
- **_run_dream_on_brain()** -- called after scaffold/import paths to regenerate maps
- **KlurisGroup** -- custom Click group that outputs JSON errors when --json is in args (scans ctx args + sys.argv)
- **All commands support --json** -- structured output for scripting
- **Deprecation frontmatter** -- optional `status`, `deprecated_at`, `replaced_by` on neurons; dream reports warnings, doesn't break healthy
- **wake-up output schema** -- `{ok, name, path, description, brain_md, lobes[{name, neurons}], total_neurons, recent[{path, updated}], glossary[{term, definition}], deprecation_count, deprecation[]}`
- **search output schema** -- `{ok, brain, query, total, results[{file, title, matched_fields[], snippet, score, deprecated}]}`
- **dream uses batch git** -- `_sync_brain_state` calls `is_git_repo()` once, then `git_log_file_dates()` once to fetch `(latest_by_path, created_by_path)`. For a 100-neuron brain that's exactly 2 subprocess calls (was ~200). Uses `%aI` (author date).
- **mri output schema (unified)** -- always `{ok, brains: [{name, output_path, preflight_fixes, nodes, edges}, ...]}` regardless of brain count.
- **`_do_install` callers** -- five command paths rewrite installed SKILL.md files: `create`, `register`, `remove`, `companion add/remove`, and `doctor`. `doctor` is the muscle-memory refresh path after `pipx upgrade kluris` -- it runs prerequisite checks, refreshes companions, AND `_do_install`. Pass `--no-refresh` to skip the refresh.
- **`_is_interactive()`** helper wraps `sys.stdin.isatty()` so tests can monkeypatch it (CliRunner replaces sys.stdin during invoke and `monkeypatch.setattr("sys.stdin.isatty", ...)` does not survive the swap)

## Testing

- conftest.py has 5 fixtures: `cli_runner`, `temp_config`, `temp_home`, `temp_brain`, `bare_remote` (bare git remote with `HEAD = refs/heads/main` for CI compat)
- Tests use monkeypatch for KLURIS_CONFIG and HOME env vars
- Git tests use real git in tmp_path (not mocked)
- Picker tests monkeypatch `kluris.cli._is_interactive`, NOT `sys.stdin.isatty` (CliRunner stdin swap)

## CI/CD

- `.github/workflows/ci.yml` -- tests on PR only (ubuntu, macos, windows x Python 3.10-3.13)
- `.github/workflows/publish.yml` -- publish to PyPI on tag v*
- **Version pinned in THREE files (must all match):** `pyproject.toml`, `src/kluris/__init__.py`, AND `tests/test_config.py::test_kluris_importable` (the test asserts the literal version string). The third file was added between v2.18.1 and v2.18.2; v2.18.3 shipped without bumping it and CI broke. Treat the three-file bump as one atomic step before tagging — PyPI rejects re-uploads of any existing version.
- **End-user upgrade:** `pipx upgrade kluris && kluris doctor`. `doctor` is the muscle-memory refresh path -- it runs prerequisite checks AND `_do_install` to rewrite installed agent skills with the new SKILL.md content. Pass `--no-refresh` to keep doctor read-only.
