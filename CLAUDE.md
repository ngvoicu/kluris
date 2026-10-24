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
pip install -e ".[dev]"          # dev install: pytest + the pack server deps (fastapi, litellm, ...)
pytest tests/ -v                 # run all tests (1016 collected; the Docker e2e skips itself)
pytest tests/ --cov=kluris --cov=kluris_runtime -q   # with coverage (92%; CI measures only kluris)
pytest tests/test_create.py -v   # single test file
pytest tests/ -m "not docker_network" -q   # skip the opt-in Docker e2e
```

`tests/pack/test_e2e_docker.py` (marker `docker_network`) is the opt-in Docker e2e. A module-level `pytestmark` skipif gates it on a Docker daemon AND `KLURIS_RUN_DOCKER_E2E=1`, so a plain `pytest tests/` run (and CI) reports 1015 passed + 1 skipped and builds no image. To run it: `KLURIS_RUN_DOCKER_E2E=1 pytest tests/pack/test_e2e_docker.py -v` (builds the pack image; needs `127.0.0.1:8765` free).

## Architecture

```
src/kluris/
  cli.py              # Click CLI -- all 13 commands in one file (incl. search, wake-up, pack, register, companion)
  core/               # CLI-side logic, 13 modules; read-only behaviour is re-exported from kluris_runtime
    config.py          # Pydantic models (GlobalConfig, BrainEntry, BrainConfig, AgentsConfig, StructureNode)
    brain.py           # BRAIN_TYPES, scaffold_brain(), validate_brain_name()
    maps.py            # generate_brain_md(), generate_map_md() -- auto-generated files
    frontmatter.py     # write_frontmatter(), update_frontmatter(path, patch, *, preloaded=(meta, body)?);
                       # read_frontmatter() + YAML_SUFFIXES are re-exports of kluris_runtime
    linker.py          # validate_synapses(), validate_bidirectional(), detect_orphans(),
                       # check_frontmatter() with type validation, fix_* helpers;
                       # neuron-walk helpers + detect_deprecation_issues() are runtime re-exports
    search.py          # re-export shim over kluris_runtime.search: search_brain() + the private names
                       # (_collect_searchable, _score_hit, ...) -- backs `kluris search`
    wake_up.py         # re-export shim: build_payload() -- backs `kluris wake-up`
    neuron_excerpt.py  # re-export shim: extract() -- title + first-line excerpt for the MRI
    mri.py             # build_graph(), generate_mri_html() -- standalone HTML viz
    git.py             # git_init(), git_add(), git_commit(), git_log(), is_git_repo(), git_status(),
                       # git_log_file_dates() batch helper (one subprocess replaces 2N)
    agents.py          # AGENT_REGISTRY (9 agents), per-brain SKILL.md renderer
                       # (always kluris-<name>, even for one brain);
                       # brain_path emitted in POSIX form (C:/ on Windows) for bash
    companions.py      # KNOWN companions; install/refresh/uninstall companion FOLDERS into ~/.kluris/companions/<name>/
    pack.py            # stage_pack() -- builds the `kluris pack` Docker context
  pack/               # the chat server baked into packs (FastAPI + LiteLLM); SOURCE of every generated pack
    main.py            # create_app() factory, /healthz, brain-only fallback when LLM config is bad or the boot smoke fails
    config.py          # env-driven Config (exactly one auth shape: API key OR OAuth client_credentials)
    agent.py           # provider-agnostic tool-calling loop (round cap, tool-call cap, synthesis fallback)
    system_prompt.py   # default playbook + /data/config/system_prompt.md (re-read per request)
    history.py         # SQLite sessions/messages at /data/sessions.db
    search_index.py    # boot-time persistent FTS5 index written into /data (the runtime itself never writes)
    readonly.py        # boot check that the bundled brain is not writable
    middleware.py      # log secret-redaction filter (no auth/CSRF/rate-limit)
    streaming.py       # SSE encoding of agent events
    providers/         # base.py (LLMProvider interface), litellm_provider.py (the one provider)
    routes/chat.py     # /, /chat (SSE), /chat/new, /api/brain/*, /api/sessions*
    tools/             # brain.py (8 read-only model tools + files_tool, which only backs GET /api/brain/files),
                       # schemas.py (OpenAI-format schemas for the 8)
    templates/ static/ # Jinja2 chat UI (base.html, chat.html); brain-explorer.js, sse.js, style.css
  vendored/           # the four Specmint companions as folders (specmint-core/-tdd and -html variants):
                      # SKILL.md + references/ (+ assets/ for -html), from Specmint v3.0.0
  _packaging/         # package-data copy of packaging/*.template (see Pack)
src/kluris_runtime/   # read-only runtime, shipped in the wheel AND copied into pack images (see below)
  frontmatter.py neuron_index.py deprecation.py search.py search_fts.py
  snapshot.py wake_up.py neuron_excerpt.py
packaging/            # pack templates: Dockerfile, docker-compose.yml, .env.example, README, ignore files
scripts/              # sync-specmint.py: re-vendors all four companions from the Specmint repos (see Key Design Decisions)
```

## Read-only runtime (`kluris_runtime`)

`src/kluris_runtime/` owns every read-only brain behaviour: frontmatter reading, neuron walking and the path sandbox, deprecation diagnostics, lexical search, wake-up payloads, neuron excerpts, plus the pack-only `search_fts.py` (BM25) and `snapshot.py` (one-walk boot snapshot). It exists so the pack image can ship it alone, without Click, Rich, git or `kluris.core`.

- `kluris.core.*` **re-exports** runtime symbols (never a copy). `tests/test_runtime_delegation.py` asserts `is`-identity; a redefinition fails it. Write-side code (`write_frontmatter`, `update_frontmatter`, maps, linker fixes) stays in `kluris.core`.
- `tests/test_runtime_isolation.py` asserts the runtime imports no Click/Rich/git/`kluris.*` and contains no filesystem-write APIs (regex over `src/kluris_runtime/`).
- Change read-side behaviour in the runtime, never in the `kluris.core` shim.

## Pack (`kluris pack`)

`core/pack.py:stage_pack()` writes a Docker context: the brain COPY'd into `brain/`, `src/kluris/pack/` as `app/`, `src/kluris_runtime/` as `kluris_runtime/`, plus the rendered `packaging/` templates. The image runs the chat server as a non-root user with `/app` (brain included) `chmod -R a-w`; `/data` is the only writable path (sessions.db, FTS index, `config/system_prompt.md`). The output dir is never allowed inside any registered brain; `--force` rebuilds but preserves the deployer's `.env`, `.env.local`, `.env.production` and `.env.staging` (`.env.example` is regenerated).

Standing decisions (deliberate; do not reverse without asking):

- **Auth-free on purpose (since v2.28.5).** The access token, rate limit and compose hardening were removed; the port binds all interfaces and the pack is meant for a trusted host/network. Kept: log secret-redaction and read-only brain enforcement. LLM credentials are still required in `.env` for chat (exactly one of API key or OAuth client_credentials); without them the app boots brain-only.
- **Brain-agnostic.** Per-brain retrieval strategy belongs in the deployer's `/data/config/system_prompt.md` (re-read per request; `KLURIS_LOCK_SYSTEM_PROMPT` pins it as read on the first chat request), never in pack code or the default prompt.
- **One LLM layer: LiteLLM.** `Config.litellm_model` picks the route: Anthropic -> `anthropic/<m>`, OpenAI-proper -> `openai/responses/<m>`, gateway/OAuth -> `openai/<m>` (base gets an idempotent `/v1`). `drop_params=True` is load-bearing. TLS needs BOTH globals, in different forms: `litellm.ssl_verify` gets the CA bundle PATH string (LiteLLM ignores an `SSLContext` placed in that global and builds a certifi context, so a private CA would not be trusted on the Anthropic and Responses routes), `litellm.aclient_session` keeps the `SSLContext` (read only by the OpenAI handler). `store=False` goes only to OpenAI proper (`_is_openai_proper` in `litellm_provider.py`), never to gateways/OAuth/Anthropic: the explicit gate keeps gateway request bodies unchanged across LiteLLM upgrades.
- **Two search engines, on purpose.** `kluris_runtime/search.py` (literal substring) backs the CLI; `search_fts.py` (BM25 over SQLite FTS5) backs the pack, with automatic substring fallback when FTS5 is missing or a query errors.
- **Two byte-identical template copies:** `packaging/*.template` and `src/kluris/_packaging/*.template`. `_read_template()` prefers the checkout copy and falls back to the installed package data, so edit both; `test_packaged_templates_match_source_templates` enforces it.
- **Exact pins in the image, floors in `dev`.** `packaging/Dockerfile.template` pins every dep (e.g. `litellm==1.105.4`); the `dev` extra in `pyproject.toml` uses floors kept in lockstep (`litellm>=1.105.4,<1.106`). 1.106.x is excluded on purpose: on every 1.106.x release a tools call raises `No module named 'mcp'` unless the `[mcp]` extra is installed (BerriAI/litellm#46045), and the pack sends tools on every call. The dev cap comes off when the image moves past it. The CLI's own dependencies stay permissive.
- **LiteLLM adds its own log filter.** At import it attaches `AccessLogPathFilter` and `AccessLogRedactionFilter` to `uvicorn.access`, which rewrites `?token=` to `REDACTED` before Kluris's `RedactingLogFilter` runs. The placeholder text is therefore not a contract; the contract is that a registered secret never reaches the line and the record's args stay a 5-tuple with the int status after every filter on that logger (uvicorn's `AccessFormatter` unpacks them): `test_uvicorn_access_logger_carries_redaction_filter`.
- **A failed boot smoke test disables chat, not the process.** The app runs brain-only: `POST /chat` returns 503, `/healthz` stays 200, the brain explorer keeps working, and the deployer fixes `.env` and restarts.
- **`KLURIS_BRAIN_DIR` / `KLURIS_DATA_DIR` are internal.** The Dockerfile sets them; never document them as deployer knobs (setting them in `.env` overrides the baked paths and breaks the container).
- **Generated packs are not the source.** `src/kluris/pack/` is; edits to a generated pack are reverted by the next `kluris pack --force`.
- **Relative imports only** under `src/kluris/pack/` (it runs as `kluris.pack` in tests and `app` in the image): `test_pack_source_has_no_absolute_self_imports`. Nothing in `pack/tools/` (the eight model-facing tools plus `files_tool`) calls write APIs: `test_readonly_enforcement.py`.

## Key Design Decisions

- **All commands in one cli.py** -- not split into separate files. Works fine at current size.
- **No Jinja2 in the CLI** -- templates are inline Python strings in brain.py and agents.py; Jinja2 is not a CLI dependency. Only the pack chat UI (`pack/routes/chat.py`, `pack/templates/`) uses it (a `dev` extra and an image pin).
- **kluris.yml is gitignored** -- local config only (agents, companions). Not shared between team members.
- **Brain types are scaffold-only** -- after creation, type is irrelevant. Same commands everywhere.
- **brain.md is lightweight** -- root lobes + glossary link only. No neuron index. Agents navigate through map.md hierarchy.
- **Agent skill/workflow templates are inline** in agents.py, not .j2 template files.
- **MRI uses inline canvas JS** -- no vendored Cytoscape.js. One standalone HTML file (`<brain>/brain-mri.html`, gitignored): C4-style boxes with path-based drill-down to any depth, breadcrumb + back/forward, file tree on the left, search and recently-updated on the right, neuron detail in a modal. The mode-based views and the force-directed "Expert" graph were removed in v2.18.0, and the right-sidebar inspector in v2.15.9. `kluris mri` runs the dream preflight, then prints a clickable `file://` link; it does not open a browser.
- **`_MRI_JS` must stay a raw string** (`_MRI_JS = r"""..."""` in core/mri.py). It is interpolated as a value into the f-string in `_render_mri_html`, so write JS escapes with single backslashes (`/\r?\n/`, `\b`). The earlier non-raw string needed doubled escapes (`\\r`, `\\n`, `\\b`), shipped a broken `brain-mri.html` when one was missed (fixed in v2.15.6), and went raw in v2.17.0.
- **YAML neurons are opt-in** -- a `.yml`/`.yaml` file is a neuron only if it opens with a complete `#---` ... `#---` comment block (`neuron_index.has_yaml_opt_in_block`); `kluris.yml` is never one. `check_frontmatter()` requires only `updated` for them (markdown needs `parent`, `created`, `updated`).
- **Cross-platform** -- all file I/O uses `encoding="utf-8"`, all paths use `pathlib.Path`.
- **wake-up bootstrap protocol** -- SKILL.md instructs the agent to run `kluris wake-up --brain <name> --json` on the first `/<skill>` of a session (via Bash), cache the snapshot, and refresh only after brain-mutating commands. This replaces walking brain.md -> map.md -> neurons on every turn.
- **Per-brain skill scheme** -- every registered brain installs as one `kluris-<name>` skill, even when it is the only brain, so project pointers stay stable as more brains are added. `kluris doctor` sweeps the entire `kluris*` artifact set before writing the named layout, so legacy bare `kluris` installs migrate automatically. Per-destination atomic via stage-then-rename so a partial-write failure leaves the OLD skill in place.
- **No default brain** -- resolution order is: explicit `--brain NAME` → exactly 1 brain registered → interactive picker (TTY only). Non-TTY / `--json` / `KLURIS_NO_PROMPT=1` all force the resolver to error out instead of prompting.
- **`--brain all`** -- accepted only on fan-out commands (dream, status, mri, companion commands). Other commands reject it with a clear error. `all` is also a reserved brain name to prevent collision.
- **Sticky-selection tradeoff** -- `kluris use` was removed deliberately. Repeated commands in a terminal session each trigger the picker (or take `--brain`). The compensating affordances are `KLURIS_NO_PROMPT`, `--brain all`, the integer-pick UX, and the per-brain slash command names that make ambiguity disappear in agent workflows.
- **Deprecation frontmatter is opt-in** -- neurons may set `status: deprecated` + `deprecated_at` + `replaced_by`. Absence of `status` means active. `detect_deprecation_issues()` (runtime, re-exported by `linker`) surfaces 4 kinds of warnings (`active_links_to_deprecated`, `deprecated_without_replacement`, `replaced_by_missing`, `replaced_by_not_active`) through `kluris dream`; they are non-blocking (do not break `healthy`). `kluris wake-up` exposes a `deprecation_count` summary.
- **KlurisGroup detects --json via ctx args** -- scans `ctx.protected_args + ctx.args` in addition to `sys.argv` so JSON error output works under CliRunner (tests) as well as shell.
- **Companions are embedded, not installed as separate skills** -- specmint-core, specmint-tdd, specmint-core-html, and specmint-tdd-html ship inside the kluris package and are copied to `~/.kluris/companions/<name>/` on opt-in. Layer-1 SKILL.md references their `SKILL.md` paths; they are NOT auto-loaded as agent skills.
- **Companions ship and install as folders** -- `SKILL.md` + `references/` (+ `assets/` for the `-html` pair), because the v3 playbooks point at those files. `companions._verify_runtime_dir` allowlists exactly `SKILL.md`, `references/`, `assets/` (no other top-level entry, no symlinks, no hidden files; the copy keeps links as links so the check rejects them). `install`/`refresh` copy into `.<name>.tmp`, verify, move the old dir to `.<name>.old`, then rename in; any failure leaves the old copy, and a stray file or symlink at the companion path is replaced instead of wedging every later refresh. `kluris doctor` upgrades an old single-file install to the folder layout.
- **`scripts/sync-specmint.py` re-vendors from Specmint** (v3.0.0; sources expected beside the repo at `../specmint/<name>`, overridable with `--core/--tdd/--core-html/--tdd-html`). It copies only the allowlist (`SKILL.md`, `references/`, `assets/`) verbatim and sanitizes `SKILL.md` alone (the `npx skills add` install section becomes a Kluris compatibility section; the html `~/.claude/skills/<name>/assets/` hint becomes the Kluris path). Every sanitizer rule must match exactly once, so a renamed heading upstream aborts the sync instead of silently shipping plugin text; the sanitized `SKILL.md` must not mention plugin/sidecar strings, and every `references/...`/`assets/...` path a shipped markdown file mentions must exist. All four are built in memory before anything is written. `--check` writes nothing and exits 1 when the vendored copy is stale; `--diff` prints full unified diffs. Running it twice is a no-op. The skill's companion block tells the agent that paths starting with `references/` or `assets/` resolve against the companion's own folder and `.specs/...` paths are project paths.
- **BrainConfig.companions** -- per-brain opt-in list; missing from older kluris.yml defaults to `[]`.
- **Companion refresh on doctor** -- `kluris doctor` calls `companions.refresh()` for the union of referenced and installed known companions so pipx-upgraded kluris auto-updates bundled playbooks (whole folders) without version comparison.

## Config Paths

- **Global config:** `~/.kluris/config.yml` (override: `KLURIS_CONFIG` env var)
- **Brain config:** `<brain>/kluris.yml` (gitignored, local only)
- **Installed skills:** `~/.claude/skills/`, `~/.cursor/skills/`, `~/.copilot/skills/`, etc.

## Agent Registry (9 agents)

| Agent | Dir | Format |
|-------|-----|--------|
| claude | ~/.claude/skills/kluris-&lt;name&gt;/ | SKILL.md |
| cursor | ~/.cursor/skills/kluris-&lt;name&gt;/ | SKILL.md |
| windsurf | ~/.codeium/windsurf/skills/kluris-&lt;name&gt;/ | SKILL.md + workflow (`~/.codeium/windsurf/global_workflows/kluris-<name>.md`) |
| copilot | ~/.copilot/skills/kluris-&lt;name&gt;/ | SKILL.md |
| codex | ~/.codex/skills/kluris-&lt;name&gt;/ | SKILL.md |
| gemini | ~/.gemini/skills/kluris-&lt;name&gt;/ | SKILL.md |
| hermes | ~/.hermes/skills/kluris-&lt;name&gt;/ and ~/.hermes/profiles/*/skills/kluris-&lt;name&gt;/ | SKILL.md |
| kilocode | ~/.kilo/skills/kluris-&lt;name&gt;/ | SKILL.md |
| junie | ~/.junie/skills/kluris-&lt;name&gt;/ | SKILL.md |

The universal `~/.agents/skills/` slot mirrors the same per-brain layout for tools that scan it; it is written on every install.

OpenCode has no entry of its own: it already loads skills from `~/.claude/skills/` and `~/.agents/skills/` (https://opencode.ai/docs/skills/), so the `kluris-<name>` skills work there without a registry agent.

## Agent Skill

Every brain registers as one `kluris-<name>` skill. Each rendered SKILL.md is bound to exactly one brain — there is no runtime brain picker inside the skill body. Windsurf also gets one `kluris-<name>.md` workflow file per brain.

The skill body (`SKILL_BODY` in agents.py) contains eight load-bearing sections (in order):

1. **Brain header** -- single-brain block at the top: `# Brain: {name}` with the absolute path and git label, plus a per-skill `--brain <name>` instruction when the skill name is `kluris-<X>`. The opted-in companion playbooks (Specmint block, which also says their `references/`/`assets/` paths resolve against the companion folder) follow it.
2. **Bootstrap** -- tells the agent to call `kluris wake-up{brain_flag} --json` on the first `/<skill>` of a session, cache the result, and refresh only after mutating commands.
3. **Query first -- never guess** -- enforces "check the brain before answering" and "never fabricate brain content"; its "When NOT to check the brain" subsection lists the trivial-work exceptions.
4. **Brain vs current project** -- the brain is a separate git repo; never create `brain.md`/`map.md`/`kluris.yml` in the project; no brain write without explicit human approval.
5. **How the brain is structured** -- top-down navigation rules.
6. **Intent detection** -- search/think/learn/remember/create/capture/fix-synapses/review/setup-pointers patterns with the right brain-flag interpolation.
7. **Writing rules** -- frontmatter contract and the approval protocol (show full content, verify `related:` paths, wait for approval, run `kluris dream --json` after writing).
8. **CLI commands** -- the mechanical terminal commands (search, wake-up, dream, status, mri); git operations go through plain `git`.

## Multi-brain CLI behavior

- **0 brains** → resolver errors with a hint to run `kluris create`.
- **1 brain** → auto-resolves for terminal CLI convenience, but the installed skill is still `kluris-<name>` and passes `--brain <name>`.
- **2+ brains + TTY** → interactive picker `[1] foo [2] bar [3] all` for fan-out commands (dream/status/mri/companion commands); `[1] foo [2] bar` (no `all`) for single-brain commands (wake-up/search/pack).
- **2+ brains + non-interactive** (`--json`, no TTY, or `KLURIS_NO_PROMPT=1`) → resolver errors with the available brains listed and a hint to pass `--brain NAME` or `--brain all`.
- **Stale brain paths** → annotated `(missing)` in the picker; resolver raises `ClickException` if the user actually tries to use one.

## CLI Commands (13)

create, register, list, status, search, wake-up, companion, dream, pack, mri, remove, doctor, help

- **`register`** -- register a brain directory already on disk (in-place, no copy). Use `git clone <url> <path>` first if the brain is hosted on git, then `kluris register <path>`. Identity comes from `brain.md` H1. Calls `_do_install()` on success so agent skills refresh to include the new brain. Passing a `.zip` errors with a hint to `unzip` first (zip support was removed in 2.16.0).
- **`companion`** -- a group: `companion list|add|remove`, all accept `--brain all`; `add` and `remove` rewrite the installed skills via `_do_install()`.
- **`pack`** -- `kluris pack [--brain NAME] [--output DIR] [--exclude GLOB]... [--force] [--json]`; single-brain (no `--brain all`). See the Pack section.
- **`mri`** -- `--output` is rejected with `--brain all`; default output is `<brain>/brain-mri.html`.

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
- **validate_brain_name()** -- lowercase alphanumeric + hyphens only, must start with a letter; max 48 chars; rejects reserved word `all`
- **git_commit() injects an identity** -- `_commit_env()` sets `GIT_AUTHOR_*`/`GIT_COMMITTER_*` (`kluris` / `kluris@local`) only when git config has no `user.name` + `user.email`, so CI/test environments without global git config work. `git_init()` sets no identity.
- **_do_install() per-destination atomic** -- stages new SKILL.md files into sibling temp dirs, sweeps `kluris*` artifacts only after successful staging, then atomically renames staged dirs into place. Partial-write failures leave the OLD skill in place for that destination.
- **_run_dream_on_brain()** -- called by `create` after scaffolding to regenerate maps and brain.md before the first commit (`register` does not call it)
- **KlurisGroup** -- custom Click group that outputs JSON errors when --json is in args (scans ctx args + sys.argv)
- **All commands support --json** -- structured output for scripting
- **Deprecation frontmatter** -- optional `status`, `deprecated_at`, `replaced_by` on neurons; dream reports warnings, doesn't break healthy
- **wake-up output schema** -- `{ok, name, path, description, brain_md, lobes[{name, description, neurons, yaml_count}], total_neurons, total_yaml_neurons, recent[{path, updated, file_type}], glossary[{term, definition}], deprecation_count, deprecation[], parse_errors}` (built by `kluris_runtime.wake_up.build_payload`). Given a boot snapshot (the pack only) each lobe also carries `top_tags` and `parse_errors` is a real count; the CLI never has one (`parse_errors` is 0).
- **search output schema** -- `{ok, brain, query, total, results[{file, file_type, title, matched_fields[], snippet, score, deprecated}]}`; `total` is `len(results)` after `--limit`, not the full match count (the runtime's paged variant, used by the pack, returns the full count).
- **dream uses batch git** -- `_sync_brain_state` calls `is_git_repo()` once, then `git_log_file_dates()` once to fetch `(latest_by_path, created_by_path)`. For a 100-neuron brain that's exactly 2 subprocess calls (was ~200). Uses `%aI` (author date).
- **mri output schema (unified)** -- always `{ok, brains: [{name, output_path, preflight_fixes, nodes, edges}, ...]}` regardless of brain count.
- **`_do_install` callers** -- five command paths rewrite installed SKILL.md files: `create`, `register`, `remove`, `companion add/remove`, and `doctor`. `doctor` is the muscle-memory refresh path after `pipx upgrade kluris` -- it runs prerequisite checks, refreshes companions, AND `_do_install`. Pass `--no-refresh` to skip the refresh.
- **`_is_interactive()`** helper wraps `sys.stdin.isatty()` so tests can monkeypatch it (CliRunner replaces sys.stdin during invoke and `monkeypatch.setattr("sys.stdin.isatty", ...)` does not survive the swap)

## Testing

- `tests/conftest.py` has 5 fixtures: `cli_runner`, `temp_config`, `temp_home`, `temp_brain`, `counting_git_run` (counts git subprocess invocations to enforce subprocess-count assertions on dream/mri batch behavior), plus the helpers `create_test_brain()` / `create_test_brain_with_neurons()`
- `tests/pack/conftest.py` adds the pack fixtures (`fixture_brain`, `api_key_env`, `oauth_env`, `api_key_config`, `stub_provider`) and an autouse `_reset_boot_caches` that clears the FTS-index, snapshot, wake-up, secret and pinned-prompt registries after every test. `tests/pack/evals/` holds the scripted-provider answer-quality evals.
- Architecture guards: `test_runtime_delegation.py` (identity), `test_runtime_isolation.py` (runtime purity), `tests/pack/test_pack_stager.py` (template drift, no absolute `kluris.pack` imports), `tests/pack/test_readonly_enforcement.py` (no write APIs in tools/runtime)
- Companion guards: `tests/test_companions.py` (folder install, allowlist, stage-then-rename, refresh over a single-file install, stray file/symlink at the path, vendored folders match the allowlist and every mentioned `references/`/`assets/` path exists), `tests/test_sync_specmint.py` (sanitizer rules fail loudly, allowlist copy, write path then `--check` is a no-op), `tests/test_doctor.py` (doctor upgrades a single-file install)
- Tests use monkeypatch for KLURIS_CONFIG and HOME env vars
- Git tests use real git in tmp_path (not mocked)
- Picker tests monkeypatch `kluris.cli._is_interactive`, NOT `sys.stdin.isatty` (CliRunner stdin swap)

## CI/CD

- `.github/workflows/ci.yml` -- tests on PR to `main` only (ubuntu, macos, windows x Python 3.10-3.13 = 12 jobs): `pip install -e ".[dev]"`, then `pytest tests/ -v --cov=kluris --cov-report=term-missing`. CI sets no `KLURIS_RUN_DOCKER_E2E`, so the Docker e2e is skipped on all 12 jobs. No test workflow runs on push.
- `.github/workflows/publish.yml` -- on push of a `v*` tag: `hatch build`, then `pypa/gh-action-pypi-publish` (trusted publishing, `id-token: write`, Python 3.12). It runs no tests.
- **Remotes:** `origin` is a private NAS (backup, publishes nothing); `upstream` is GitHub. Only a `v*` tag pushed to `upstream` publishes to PyPI. Release: `git push origin main vX.Y.Z && git push upstream main vX.Y.Z`.
- **A failed publish job is re-run, never fixed with a new version:** `gh run rerun <id> -R ngvoicu/kluris --failed` (usual causes are transient: image pull timeout, upload/attestation dropped mid-transfer). Verify with `https://pypi.org/pypi/kluris/<version>/json` returning 200.
- **Version pinned in THREE files (must all match):** `pyproject.toml`, `src/kluris/__init__.py`, AND `tests/test_config.py::test_kluris_importable` (the test asserts the literal version string). The third file was added between v2.18.1 and v2.18.2; v2.18.3 shipped without bumping it and CI broke. Treat the three-file bump as one atomic step before tagging — PyPI rejects re-uploads of any existing version.
- **End-user upgrade:** `pipx upgrade kluris && kluris doctor`. `doctor` is the muscle-memory refresh path -- it runs prerequisite checks AND `_do_install` to rewrite installed agent skills with the new SKILL.md content. Pass `--no-refresh` to keep doctor read-only.
