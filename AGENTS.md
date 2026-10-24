# AGENTS.md

## Project: kluris

CLI tool that turns AI agents into team subject matter experts with shared, human-curated knowledge.
Published to PyPI as `kluris`. Source: `ngvoicu/kluris`.

Read and write to the **ngvoicu-sme** brain through kluris (never edit brain files by hand). Use the `/kluris-ngvoicu-sme` skill — search, learn, remember, create.

## Quick Reference

```bash
pip install -e ".[dev]"          # dev install (pytest + the pack server deps)
pytest tests/ -v                 # 1016 tests collected (the Docker e2e skips itself)
pytest tests/ --cov=kluris --cov=kluris_runtime -q   # 92% coverage (CI measures only kluris)
pytest tests/ -m "not docker_network" -q   # skip the opt-in Docker e2e
```

## Source Layout

All CLI commands are in `src/kluris/cli.py` (single file).
CLI-side logic is in `src/kluris/core/` (13 modules); read-only behaviour (frontmatter reading, neuron walking, search, wake-up, deprecation, excerpts) lives in `src/kluris_runtime/` and `kluris.core.*` re-exports it.
The `kluris pack` chat server (FastAPI + LiteLLM) is `src/kluris/pack/`; its Docker templates are `packaging/*.template`, duplicated byte-for-byte in `src/kluris/_packaging/`.
Specmint companion playbooks ship in `src/kluris/vendored/` as four skill folders (`SKILL.md` + `references/`, plus `assets/` for the `-html` pair), re-vendored from Specmint v3.0.0 by `scripts/sync-specmint.py`.
Agent skill and workflow templates are inline strings in `src/kluris/core/agents.py`.
No Jinja2 in the CLI (only the pack's chat UI uses it).

## Key Files

- `src/kluris/cli.py` -- all 13 Click commands (incl. `register`, `pack`, and `companion`), wizard logic, KlurisGroup error handler, `search` + `wake-up` commands and collectors, `_resolve_brains` (picker + non-TTY guard + `--brain all`), `_do_install` (per-destination atomic stage-then-rename), `_sync_brain_state` (batch git log + `update_frontmatter(preloaded=...)`)
- `src/kluris/core/agents.py` -- AGENT_REGISTRY (9 agents, including Hermes), per-brain SKILL.md renderer (`render_skill(*, skill_name, brain_name, brain_path, has_git, brain_description, companions=None, companion_home=None)`). Every brain gets a named `kluris-<name>` skill, even when only one brain is registered. SKILL_BODY contains a single-brain header, Bootstrap, Query first, Brain vs current project, How the brain is structured, Intent detection (uses `kluris search` as the primary search path), Writing rules, and CLI commands sections. Brain paths are emitted in POSIX form via `_posix_path()` so bash on Windows handles them correctly.
- `src/kluris/core/brain.py` -- BRAIN_TYPES, scaffold_brain(), _generate_readme(), validate_brain_name() (rejects reserved word `all`, max 48 chars, must start with a letter)
- `src/kluris/core/config.py` -- Pydantic models (`BrainEntry`, `GlobalConfig`, `AgentsConfig`, `StructureNode`, `BrainConfig`), config read/write, register/unregister.
- `src/kluris/core/maps.py` -- generate_brain_md(), generate_map_md()
- `src/kluris/core/linker.py` -- synapse validation, bidirectional checks, orphan detection, type-aware check_frontmatter(), the `fix_*` helpers; `detect_deprecation_issues()` and the neuron-walk helpers are re-exports of `kluris_runtime`
- `src/kluris/core/search.py` -- re-export shim over `kluris_runtime.search` (`search_brain` plus the private `_collect_searchable`, `_score_hit`, `_matched_fields`, `_extract_snippet` names); powers `kluris search`. The implementation (occurrence-count ranking, UTF-8-safe snippets, neurons + glossary + brain.md) is in the runtime.
- `src/kluris/core/wake_up.py`, `src/kluris/core/neuron_excerpt.py` -- re-export shims (`build_payload`, `extract`) over the runtime
- `src/kluris/core/frontmatter.py` -- write_frontmatter, `update_frontmatter(path, patch, *, preloaded=(meta, body)?)` — the preloaded form skips the second disk read, used by dream's hot path; `read_frontmatter` is the runtime's
- `src/kluris/core/mri.py` -- graph building, standalone HTML generation: path-based C4-style drill-down canvas, file tree on the left, search + recent on the right, neuron modal. `_MRI_JS` is a raw string (`r"""`) interpolated into an f-string; keep it raw and use single-backslash JS escapes.
- `src/kluris/core/git.py` -- subprocess git wrapper; `git_log_file_dates()` returns `(latest_by_path, created_by_path)` from one walk — replaces the per-file helpers entirely; `git_commit()` supplies a fallback identity when git config has none
- `src/kluris/core/companions.py` -- the four known Specmint companions (`KNOWN`), install/refresh/uninstall of whole companion folders under `~/.kluris/companions/<name>/` (allowlist `SKILL.md`, `references/`, `assets/`; stage-then-rename)
- `src/kluris/core/pack.py` -- `stage_pack()`: copies the brain, `src/kluris/pack/` (as `app/`) and `src/kluris_runtime/` into a Docker context and renders the `packaging/*.template` files
- `src/kluris_runtime/` -- the read-only runtime (frontmatter, neuron_index, deprecation, search, search_fts, snapshot, wake_up, neuron_excerpt). No Click/Rich/git/`kluris.*` imports, no write APIs; `kluris.core.*` re-exports it
- `src/kluris/pack/` -- the chat server baked into packs: `main.py` (`create_app`), `config.py`, `agent.py` (tool loop), `providers/litellm_provider.py`, `routes/chat.py`, `tools/` (8 read-only model tools plus `files_tool`, which only backs `GET /api/brain/files`), `history.py` (SQLite sessions), `search_index.py`, `system_prompt.py`, `templates/` + `static/`
- `src/kluris/vendored/`, `src/kluris/_packaging/` -- vendored companion folders (`SKILL.md` + `references/` + `assets/`); package-data copy of the pack templates
- `scripts/sync-specmint.py` -- re-vendors all four companions from the Specmint repos (default `../specmint/<name>`): allowlist copy, `SKILL.md`-only sanitizing where every rule must match exactly once (a renamed upstream heading aborts the run), `--check` (exit 1 when stale), `--diff`

## Agent Bootstrap Protocol

On the first `/<skill>` call of a session, the agent runs
`kluris wake-up --brain <name> --json` via Bash and caches the output.
Subsequent calls reuse the cache until one of these
mutating commands fires: `/<skill> remember`, `/<skill> learn`,
`kluris dream --brain <name>`, or direct brain-file edits. The instruction
is baked into SKILL_BODY's Bootstrap section.

## Deprecation Frontmatter

Neurons may opt into deprecation with `status: deprecated`, `deprecated_at: YYYY-MM-DD`,
and `replaced_by: ./path/to/new.md`. `linker.detect_deprecation_issues()` reports
four kinds: `active_links_to_deprecated`, `deprecated_without_replacement`,
`replaced_by_missing`, `replaced_by_not_active` (replaced_by points to a
deprecated neuron or a non-neuron file). `kluris dream` surfaces them as
non-blocking warnings (text + `--json`). `kluris wake-up --brain <name> --json`
exposes a `deprecation_count` summary for agent bootstrap.

## Constraints

- All file I/O must use `encoding="utf-8"` (Windows compatibility)
- All paths must use `pathlib.Path` (cross-platform)
- Global config at `~/.kluris/config.yml` (override: KLURIS_CONFIG env var)
- `kluris.yml` in brains is gitignored -- local config only
- Brain types (product-group, personal, product, research, blank) are scaffold-only
- brain.md is lightweight (root lobes only, no neuron index)
- Agents navigate hierarchically: wake-up snapshot -> brain.md -> map.md -> neurons
- Slash command: one named skill per registered brain: `/kluris-<name>` from the first brain onward. Each handles search, learn, remember, and create -- dream is CLI-only. The search intent tells the agent to call `kluris search "<query>" --brain <name> --json` via Bash as the fast path and fall back to manual brain.md → map.md navigation only if the CLI returns zero results. The universal `~/.agents/skills/` slot is written on every install; OpenCode has no registry entry because it already loads `~/.claude/skills/` and `~/.agents/skills/` (https://opencode.ai/docs/skills/), so these skills work there.
- `_do_install()` is called by five command paths: `create`, `register`, `remove`, `companion add/remove`, and `doctor`. `doctor` is the post-`pipx upgrade kluris` refresh path -- run it once after every kluris version bump to refresh the skill files baked into `~/.<agent>/skills/kluris*` plus Hermes profile skill dirs. Pass `--no-refresh` to keep doctor read-only.
- Companions are embedded specmint playbooks shipped as folders. Per-brain opt-ins live in `kluris.yml` under `companions: []`. Runtime copies live under `~/.kluris/companions/<name>/` (`SKILL.md` + `references/` + `assets/`; no other entries, no symlinks or hidden files), are refreshed unconditionally by `doctor` (which also upgrades an old single-file install to the folder), and are referenced by the generated layer-1 Kluris SKILL.md, which tells agents that `references/`/`assets/` paths resolve against the companion folder and `.specs/...` paths are project paths.
- `register` -- registers a brain directory already on disk (in-place, no copy). Use `git clone <url> <path>` first if the brain is hosted on git, then `kluris register <path>`. Identity comes from `brain.md` H1 via `_read_brain_identity`. Calls `_do_install()` on success. Zip support was removed in 2.16.0; `.zip` paths error with a clear hint pointing at `unzip` + `git clone`.
- Sync, commit, and branch: kluris brains are plain git repos. Use `git push` / `git pull` / `git checkout` directly. The wrapper commands (`kluris clone`, `kluris push`, `kluris pull`, `kluris branch`) were removed in 2.16.0.
- **Version pinned in THREE files (must all match before tagging):** `pyproject.toml`, `src/kluris/__init__.py`, AND `tests/test_config.py::test_kluris_importable` (asserts the literal version string). PyPI rejects re-uploads, so the bump is one atomic step. v2.18.3 shipped with the third file forgotten -- treat all three as one change.
- End-user upgrade flow: `pipx upgrade kluris && kluris doctor`. `doctor` reruns `_do_install` to refresh installed agent skills with the new SKILL.md content; pass `--no-refresh` to keep doctor read-only.
- Read-only brain behaviour lives in `src/kluris_runtime/`; `kluris.core.*` re-exports it (never a copy). Enforced by `tests/test_runtime_delegation.py` (`is`-identity) and `tests/test_runtime_isolation.py` (no Click/Rich/git/`kluris.*` imports, no write APIs). Change read-side behaviour in the runtime.
- `kluris pack` (Docker chat server, source in `src/kluris/pack/`) is auth-free on purpose since v2.28.5 (trusted host; log redaction and read-only brain kept; LLM credentials still needed for chat), brain-agnostic (per-brain retrieval tuning goes in the deployer's `/data/config/system_prompt.md`, never in pack code), and has one LLM layer, LiteLLM (TLS: `litellm.ssl_verify` takes the CA-bundle path string, `litellm.aclient_session` the `SSLContext`, both needed; `store=False` only to OpenAI proper via `_is_openai_proper`; LiteLLM also attaches its own `uvicorn.access` redaction filter at import). A failed boot smoke test disables chat (brain-only mode, `POST /chat` returns 503) and the process keeps running. Search is two engines on purpose: substring (`kluris_runtime/search.py`, CLI) and BM25/FTS5 (`search_fts.py`, pack, substring fallback). Pack code uses relative imports only. Generated packs are not the source.
- Pack templates exist in two byte-identical copies, `packaging/*.template` and `src/kluris/_packaging/*.template`; edit both (`test_packaged_templates_match_source_templates`). The image exact-pins its deps in `packaging/Dockerfile.template` (`litellm==1.105.4`); the `dev` extra in `pyproject.toml` uses floors kept in lockstep (`litellm>=1.105.4,<1.106`; 1.106.x is excluded because tools calls need the `[mcp]` extra, BerriAI/litellm#46045). `KLURIS_BRAIN_DIR` / `KLURIS_DATA_DIR` are internal, never documented as deployer knobs.
- Test fixtures: `tests/conftest.py` has `cli_runner`, `temp_config`, `temp_home`, `temp_brain`, `counting_git_run` (counts git subprocess invocations to enforce subprocess-count assertions on dream/mri batch behavior); `tests/pack/conftest.py` adds the pack fixtures (`fixture_brain`, `api_key_config`, `stub_provider`, ...) and an autouse boot-cache reset.
- Tests must pass before pushing: `pytest tests/ -q` (the Docker e2e, marker `docker_network`, skips itself unless a daemon is up AND `KLURIS_RUN_DOCKER_E2E=1`, via a module-level `pytestmark`: a default run is 1015 passed + 1 skipped, and CI skips it; deselect with `-m "not docker_network"`)
- CI runs on PR to `main` only (ubuntu, macos, windows x Python 3.10-3.13)
- Remotes: `origin` is a private NAS, `upstream` is GitHub. Only a `v*` tag pushed to `upstream` publishes to PyPI (`.github/workflows/publish.yml`: `hatch build` + trusted publishing). Release: `git tag vX.Y.Z && git push origin main vX.Y.Z && git push upstream main vX.Y.Z`. A failed publish job is re-run (`gh run rerun <id> -R ngvoicu/kluris --failed`), never fixed with a new version.
