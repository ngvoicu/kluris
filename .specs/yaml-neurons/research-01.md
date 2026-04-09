I have a complete picture of the test style, call graph, and fixture patterns. I have all the research material needed. Let me produce the final research findings as the assistant message (per the system prompt: "Return findings directly as your final assistant message — the parent agent reads your text output, not files you create").

---

# Research Notes — yaml-neurons

## Date: 2026-04-09
## Researcher: specmint-tdd:researcher agent
## Spec ID: yaml-neurons
## Project: `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli`

---

## 1. Project Architecture

### Tech Stack (verified from pyproject.toml)
- **Language:** Python >= 3.10
- **Package:** `kluris` v2.2.9 (published to PyPI)
- **Build backend:** hatchling
- **Runtime deps:** `click>=8.1`, `rich>=13.0`, `pyyaml>=6.0`, `python-frontmatter>=1.1`, `pydantic>=2.0`
- **Dev deps:** `pytest>=8.0`, `pytest-cov>=5.0`
- **Entry point:** `kluris = "kluris.cli:cli"`

### Source Layout
```
src/kluris/
  __init__.py             (version string)
  cli.py                  (1822 lines — all 17 Click commands, plus wake-up collectors,
                           _sync_brain_state, _resolve_brains, _do_install)
  core/
    config.py             (Pydantic models, read_global_config, write_global_config)
    brain.py              (509 lines — BRAIN_TYPES, NEURON_TEMPLATES, scaffold_brain,
                           generate_neuron_content, GITIGNORE_CONTENT, validate_brain_name)
    frontmatter.py        (64 lines — read/write/update wrapping python-frontmatter)
    maps.py               (181 lines — generate_brain_md, generate_map_md,
                           _get_neurons, _get_lobes, _get_sub_lobes)
    linker.py             (368 lines — _all_md_files, _neuron_files,
                           validate_synapses, validate_bidirectional, detect_orphans,
                           check_frontmatter, detect_deprecation_issues,
                           fix_bidirectional_synapses, fix_missing_frontmatter)
    search.py             (358 lines — _collect_searchable, _score_hit,
                           _matched_fields, _extract_snippet, search_brain)
    mri.py                (2537 lines — _all_md_files, _extract_title_and_excerpt,
                           _build_content_preview, build_graph, generate_mri_html
                           with ~2300 lines of inline HTML/CSS/JS)
    git.py                (git_log_file_dates, git_commit, git_push, is_git_repo)
    agents.py             (467 lines — AGENT_REGISTRY (8 agents), SKILL_BODY,
                           render_skill, _posix_path)
```

### Test Infrastructure
- **Framework:** pytest 8.0+, pytest-cov 5.0+
- **28 test files** under `tests/`, 290 tests currently passing
- **Fixtures** (conftest.py, 167 lines): `cli_runner`, `temp_config`, `temp_home`, `temp_brain`, `bare_remote`, `counting_git_run`
- **Two key helpers:** `create_test_brain(runner, name, path, **extra_flags)` — invokes `cli` with `--json` to skip prompts; `create_test_brain_with_neurons(runner, name, path, count=100)` — scaffolds + adds N committed neurons.
- **Style:** `tmp_path` for disk, `monkeypatch` for `KLURIS_CONFIG`/`HOME` env vars, real git subprocess in temp dirs (never mocked), CliRunner for command invocation. Many low-level tests bypass CliRunner entirely and call `build_graph(...)` / `generate_mri_html(...)` / `read_frontmatter(...)` directly on a tmp_path brain — see `tests/test_mri.py`, `tests/test_linker.py`, `tests/test_maps.py`.

### Deployment / CI
- **CI:** `.github/workflows/ci.yml` runs tests on PR only across ubuntu/macos/windows × Python 3.10–3.13
- **Publish:** tag `v*` triggers PyPI publish via `.github/workflows/publish.yml`
- **Version parity required:** `pyproject.toml` and `src/kluris/__init__.py` must match.

---

## 2. Current State — The `.md`-only Assumption Audit

### 2.1 Complete list of hardcoded-markdown chokepoints

Every one of these must be revisited for the yaml-neurons spec. Line numbers verified against the current HEAD source.

| # | File | Line(s) | Symbol / Context | What it does | Consumers |
|---|------|---------|------------------|--------------|-----------|
| 1 | `src/kluris/core/linker.py` | 16 | `SKIP_FILES = {"brain.md", "index.md", "glossary.md", "README.md", ".gitignore"}` | Filename-based skip set used by `_all_md_files` / `_neuron_files` | All validators |
| 2 | `src/kluris/core/linker.py` | 21-34 | `_all_md_files(brain_path)` — `for item in brain_path.rglob("*.md")` | **Primary chokepoint for all validators.** Walks brain collecting markdown only | `validate_synapses`, `detect_orphans`, `_neuron_files` |
| 3 | `src/kluris/core/linker.py` | 37-44 | `_neuron_files(brain_path)` — filters out SKIP_FILES and `map.md` | **Central chokepoint used by every non-MRI consumer that needs "the neurons".** | `search.py::_collect_searchable`, `cli.py::status`, `cli.py::dream`, `linker.py::validate_synapses/fix_bidirectional_synapses/check_frontmatter/detect_deprecation_issues` |
| 4 | `src/kluris/core/linker.py` | 138 | `elif target.exists() and target.suffix == ".md":` in `validate_bidirectional` | Bidirectional synapse check rejects non-.md targets | `validate_bidirectional` |
| 5 | `src/kluris/core/linker.py` | 200 | `if md_file.name == "map.md" or md_file.name == "brain.md":` in `detect_orphans` | Only map.md and brain.md references count | `detect_orphans` |
| 6 | `src/kluris/core/linker.py` | 365 | `update_frontmatter(neuron, {"parent": "./map.md"})` in `fix_missing_frontmatter` | Default parent is `./map.md` — fine for yaml neurons too | `fix_missing_frontmatter` |
| 7 | `src/kluris/core/maps.py` | 10 | `SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", ".gitignore"}` | Duplicate of linker's SKIP_FILES (diverged historically) | `generate_map_md` |
| 8 | `src/kluris/core/maps.py` | 65 | `if item.is_file() and item.suffix == ".md" and item.name not in SKIP_FILES:` in `_get_neurons` | **The contents section of every map.md is built from .md files only.** Yaml neurons will not appear in any lobe's contents listing until this is fixed. | `generate_map_md` |
| 9 | `src/kluris/core/maps.py` | 151-156 | `contents_lines.append(f"- [{n['name']}](./{n['name']}) — {n['title']}")` | Link format for each neuron row — works for any filename | `generate_map_md` |
| 10 | `src/kluris/core/mri.py` | 12 | `SKIP_FILES = {".gitignore", "README.md"}` — **narrower** than linker's | MRI scans more files than linker (includes glossary.md and index.md as nodes) | `_all_md_files` |
| 11 | `src/kluris/core/mri.py` | 15-23 | `_all_md_files(brain_path)` — `for item in brain_path.rglob("*.md")` | MRI's own brain walker. **Does not share linker's helper** (different skip policy). | `build_graph` |
| 12 | `src/kluris/core/mri.py` | 106-115 | Type dispatch by filename: `brain.md` → brain, `index.md` → index, `glossary.md` → glossary, `map.md` → map, else → neuron | **Where node "type" is decided.** Need a new branch for yaml files. | `build_graph` |
| 13 | `src/kluris/core/mri.py` | 120 | `meta, content = read_frontmatter(f)` | **Calls python-frontmatter on whatever file it finds.** Will raise on raw yaml. | `build_graph` |
| 14 | `src/kluris/core/mri.py` | 1178 | `const lobePalette = ['#7bf7ff','#ff8bd8','#f8c76d','#7df7b4','#9ea9ff','#ffa06f','#b8f0c1','#f2a8ff'];` | JS lobe palette — no per-type distinction needed if yaml uses the lobe color | `colorForNode` |
| 15 | `src/kluris/core/mri.py` | 1243-1250 | `function colorForNode(node)` — dispatches by `node.type` | **This is where a new `type: 'yaml'` would wire in a distinct color.** | inline JS |
| 16 | `src/kluris/core/mri.py` | 1252-1261 | `function nodeRadius(node)` — dispatches by `node.type` | Yaml neurons should probably share the neuron radius | inline JS |
| 17 | `src/kluris/core/mri.py` | 1364 | `const total = nodes.filter(n => n.type === 'neuron' || n.type === 'glossary' || n.type === 'index').length;` | "Showing N of M neurons" counter | inline JS |
| 18 | `src/kluris/core/mri.py` | 1597-1599 | `const targetPath = isLast ? partPath : partPath + '/map.md'; const label = part.replace('.md', '');` | Breadcrumb path builder (strips .md) | inline JS |
| 19 | `src/kluris/core/mri.py` | 1652 | `.filter(n => n.type === 'neuron' && n.path)` in `buildFileTree` | File browser tree only shows neurons; yaml would need inclusion | inline JS |
| 20 | `src/kluris/core/mri.py` | 1737 | `const breadcrumb = node.path.split('/').map(p => p.replace('.md', ''))...` | Modal header breadcrumb strips `.md` only | inline JS |
| 21 | `src/kluris/core/mri.py` | 1746 | `const linkRe = /\[([^\]]+)\]\(([^)]+\.md)\)/g;` | **Modal content link regex only matches `.md` targets.** Yaml files will not be clickable link targets from inside a neuron's modal view. | `openModal` |
| 22 | `src/kluris/core/mri.py` | 1786 | `.filter(n => n && n.type === 'neuron')` in navigation | "Connected nodes" list in modal excludes non-neuron types | `openModal` |
| 23 | `src/kluris/core/mri.py` | 2290 | Comment: `// Top-level: path is "lobe/map.md" (2 parts). Sub-lobe: "lobe/sub/map.md" (3+ parts)` | Comment-only but the surrounding logic infers lobe/sublobe from map.md path structure | lobe focus/zoom logic |
| 24 | `src/kluris/cli.py` | 153 | `for md in brain_path.rglob("*.md"):` in `_sync_brain_state` | **Dream's hot path. Updates frontmatter dates for every .md under the brain.** | `dream` |
| 25 | `src/kluris/cli.py` | 154 | `if md.name in {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}:` | Dream's filename skip set — yet another copy | `dream` |
| 26 | `src/kluris/cli.py` | 666 | `_WAKE_UP_SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}` | **Fourth copy of the skip set.** | `wake-up` |
| 27 | `src/kluris/cli.py` | 677-681 | `neurons = [md for md in child.rglob("*.md") ...]` in `_wake_up_collect_lobes` | **Wake-up's per-lobe neuron counter.** Yaml neurons silently missing from `lobes[].neurons`. | `wake-up` |
| 28 | `src/kluris/cli.py` | 689 | `for md in brain_path.rglob("*.md"):` in `_wake_up_collect_recent` | **Wake-up's "5 most recently updated neurons" collector.** Yaml neurons will never surface in the recent list. | `wake-up` |
| 29 | `src/kluris/cli.py` | 1027 | `parent_map = "./map.md"` in the `neuron` command | Default parent used when creating new neurons. Reusable. | `neuron` |

### 2.2 Fan-in diagram: who calls `_neuron_files`?

```
_neuron_files(brain_path)              [linker.py:37-44]
  ├── search.py::_collect_searchable   [line 207] — "for neuron in _neuron_files(brain_path)"
  │     └── search_brain()             [called by `kluris search`]
  ├── cli.py::status                   [line 955]
  ├── linker.py::validate_synapses     [line 79] — builds `neurons` set
  ├── linker.py::validate_bidirectional [line 115]
  ├── linker.py::fix_bidirectional_synapses [line 157]
  ├── linker.py::check_frontmatter     [line 230]
  ├── linker.py::detect_orphans        [line 195]
  └── linker.py::detect_deprecation_issues [used similarly]
```

**Key insight:** `search.py` inherits its `.md`-only behavior entirely from `_neuron_files`. A single edit to `_neuron_files` (or a new sibling `_yaml_neuron_files`) propagates to search, status, and every validator in one move.

### 2.3 Callers that do NOT go through `_neuron_files`

These bypass the central helper and have their own rglob/suffix logic — **each is its own edit site**:

- `mri.py::_all_md_files` — MRI is independent; no dependency on linker. It has its own skip policy (narrower: keeps glossary.md / index.md as nodes).
- `cli.py::_sync_brain_state` — dream's date-updater. Its own rglob + skip set.
- `cli.py::_wake_up_collect_lobes` — per-lobe rglob.
- `cli.py::_wake_up_collect_recent` — brain-wide rglob.
- `maps.py::_get_neurons` — **per-lobe, `iterdir()` only, not rglob.** Used to build map.md contents. Not recursive because each lobe's map.md only lists its immediate neuron children (sub-lobes get their own map.md).

### 2.4 What the scanners ALREADY skip that yaml-neurons must NOT accidentally pick up

- `kluris.yml` — **CRITICAL.** Lives at every brain root; is gitignored but still on disk. Scanners walk with `rglob("*.md")` today, so it's invisible; but if we change to `rglob("*")` or add `.yml` matching, we must exclude this file. It is stored at `<brain>/kluris.yml` (confirmed at `brain.py:217`, referenced at `cli.py:473, 495, 497, 584, 604, 606, 1392, 1396`, and gitignored at `brain.py:91`).
- `.git/*`, `.github/*`, `.vscode/*`, `.idea/*`, `node_modules/*`, `__pycache__/*` — already in SKIP_DIRS.
- `.gitignore`, `README.md` — already in SKIP_FILES.
- `brain.md`, `map.md`, `index.md`, `glossary.md` — auto-generated; must stay excluded.
- Hidden dirs (starting with `.`) — skipped in `_all_md_files` via `any(part.startswith(".") for part in item.parts[:-1])`.

**There is NO existing test covering the "yaml file at brain root is not a neuron" invariant** because yaml files at the brain root (kluris.yml is the only one) are currently invisible to every scanner. This becomes a regression risk as soon as we add yaml scanning.

### 2.5 Divergent skip sets (the consistency smell)

These four sets all mean "auto-generated files kluris manages" but are not unified:

```python
# linker.py:16
SKIP_FILES = {"brain.md", "index.md", "glossary.md", "README.md", ".gitignore"}
# maps.py:10
SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", ".gitignore"}
# mri.py:12
SKIP_FILES = {".gitignore", "README.md"}   # NARROWER — keeps glossary/index/map as nodes
# cli.py:154 and cli.py:666
{"map.md", "brain.md", "index.md", "glossary.md", "README.md"}
```

The yaml-neurons spec will need to decide whether to (a) unify these while touching them, or (b) leave the inconsistency and just add `.yml`/`.yaml` support to each. The spec body should explicitly call out this decision.

---

## 3. Library & Stack Constraints

### 3.1 `python-frontmatter` (v1.1+) — what it can and cannot do

From the library's [default_handlers.py](https://github.com/eyeseast/python-frontmatter/blob/main/frontmatter/default_handlers.py):

```python
class YAMLHandler(BaseHandler):
    FM_BOUNDARY = re.compile(r"^-{3,}\s*$", re.MULTILINE)
    START_DELIMITER = END_DELIMITER = "---"
```

**Key facts:**
- The default handler splits on `^---$` lines (three or more dashes).
- `START_DELIMITER` / `END_DELIMITER` are **customizable at handler init**.
- **A file containing only YAML body and no `---` delimiters will NOT be parsed.** The `split()` method returns the whole file as body with no metadata.
- Multiple handler classes ship out of the box: `YAMLHandler`, `JSONHandler`, `TOMLHandler` — none of them handle "raw YAML body with no delimiters".

**Implication for kluris:** `kluris.core.frontmatter.read_frontmatter` today assumes `---` delimiters wrapping a YAML metadata block. Calling it on a raw `openapi.yml` file (which has no `---` delimiters, just `openapi: 3.1.0\ninfo:\n  title: ...`) will:
- Return `({}, <full file body>)` — the entire file is treated as "content" with no metadata.
- Not raise, but also not surface `info.title`, `tags`, `parent`, etc.

This is **actually a fortunate soft-fail**: a raw yaml neuron will be readable as `(empty_meta, raw_yaml_text)` today, which means scanners that already wrap `read_frontmatter` in `try: ... except: pass` (see `mri.py:119-122`, `linker.py:83`) will keep working. The question is how to give yaml neurons proper titles, tags, and frontmatter fields.

### 3.2 OpenAPI 3.1 extension mechanism (per spec)

From the [OpenAPI 3.1 spec](https://spec.openapis.org/oas/v3.1.0.html) and [Swagger extensions docs](https://swagger.io/docs/specification/v3_0/openapi-extensions/):

- Extension fields always start with `x-`.
- Allowed in specific object types (info, paths, operations, schemas, etc.) — **not everywhere**, but they ARE allowed on the root OpenAPI Object and on the `info` object.
- The extension value can be a primitive, array, object, or null. Sub-properties don't need the `x-` prefix.
- Tool vendors typically prefix their own extensions: `x-speakeasy-*`, `x-amazon-*`, etc.

**A vendor extension for kluris frontmatter is 100% spec-compliant.** The most natural home is either the root or `info:`. A proposal like:

```yaml
openapi: 3.1.0
info:
  title: Payments API
  version: 1.0.0
x-kluris:
  parent: ./map.md
  related:
    - ../../knowledge/use-raw-sql.md
  tags: [api, payments]
  created: 2026-04-09
  updated: 2026-04-09
```

...validates cleanly in every major OpenAPI linter (Redocly, Spectral, Swagger Editor).

### 3.3 Frontmatter-format library (jlevy) — hash-style YAML prior art

There IS a published convention for YAML metadata on non-markdown files: [frontmatter-format](https://github.com/jlevy/frontmatter-format). It defines six styles:

1. **YAML style** (`---` / `---`) — markdown, plain text
2. **HTML style** (`<!---` / `--->`) — HTML, XML
3. **Hash style** (`#---` / `#---`) — Python, CSV
4. **Slash style** (`//---` / `//---`) — Rust, C++
5. **Slash-star style** (`/*---` / `---*/`) — JS, TS, CSS, C
6. **Dash style** (`----` / `----`) — SQL

For YAML files, **the obvious style is hash-style** (`#---` / `#---`) because `#` is a valid YAML comment, so the file remains valid YAML with or without a tool aware of the metadata block. Example:

```yaml
#---
# parent: ./map.md
# related: []
# tags: [api]
# created: 2026-04-09
# updated: 2026-04-09
#---
openapi: 3.1.0
info:
  title: Payments API
```

**But:** this is NOT supported out-of-the-box by `python-frontmatter`. It would require either:
- Pulling in `frontmatter-format` as a new dep, or
- Writing a 30-line parser in `kluris.core.frontmatter` that strips the `#` prefix and reuses the YAMLHandler logic.

### 3.4 Obsidian precedent — the limit of the non-markdown approach

From the Obsidian forum discussion on [backlinks to non-markdown files](https://forum.obsidian.md/t/showing-backlinks-to-non-markdown-files/18776) and [plugin API limits](https://forum.obsidian.md/t/is-it-possible-to-register-file-links-from-a-non-markdown-file/78883): Obsidian lets you LINK TO non-markdown files (so they show in Graph View as terminal nodes) but backlinks FROM non-markdown files do not work because the Obsidian API doesn't expose a link registration hook for non-markdown content.

This is exactly the asymmetry kluris will hit: easy to make yaml files a link *target*; harder to parse links *out of* yaml files. And unlike Obsidian, **we should not care about scraping links out of yaml body content** — OpenAPI/JSON Schema yaml will have thousands of `$ref:` entries that are structural references to API schemas, not synapses. Only the `related:` array in the `x-kluris` frontmatter block should count.

Sources:
- [OpenAPI Extensions — Swagger](https://swagger.io/docs/specification/v3_0/openapi-extensions/)
- [OpenAPI 3.1.0 Spec](https://spec.openapis.org/oas/v3.1.0.html)
- [python-frontmatter handlers docs](https://python-frontmatter.readthedocs.io/en/latest/handlers.html)
- [frontmatter-format (jlevy)](https://github.com/jlevy/frontmatter-format)
- [YAML Frontmatter Explained — Nat Thompson](https://natthompson.com/blog/yaml-frontmatter-explained)

---

## 4. Option Analyses

### 4.1 Frontmatter feasibility — 4 options

#### Option A — YAML comment block (hash-style `#---` / `#---`)

```yaml
#---
# parent: ./map.md
# related: []
# tags: [api, openapi]
# created: 2026-04-09
# updated: 2026-04-09
#---
openapi: 3.1.0
info:
  title: Payments API
```

**Pros:**
- File is valid YAML (hash lines are comments).
- Frontmatter is visually delimited, easy for humans to author.
- No coupling to OpenAPI-specific semantics — works on any `.yml` file (k8s manifests, CI configs, etc.).
- Parsing is simple: strip leading `# ` from each line inside the `#---` block, pass to YAML parser.
- Compatible with the "frontmatter convention" from [jlevy/frontmatter-format](https://github.com/jlevy/frontmatter-format).

**Cons:**
- Requires new parser code in `kluris.core.frontmatter` — **python-frontmatter does not support hash-style delimiters out of the box**.
- If the human forgets to add the block, the file has no frontmatter at all (versus Option C where the block IS the file).
- `write_frontmatter` needs a dedicated writer branch that knows the target file's suffix.
- Every `.yml` neuron must start with `#---` — **an authoring cost**. The `learn` / `remember` wizard in agents.py must be taught to emit this wrapper.

**Effort:** Medium. ~60 lines of new parser code + ~40 lines of writer code + tests.

#### Option B — Side-car `.meta` file

```
brain/
  projects/
    payments-api/
      openapi.yml          (untouched, pure OpenAPI)
      openapi.yml.meta     (YAML metadata)
```

`openapi.yml.meta`:
```yaml
parent: ./map.md
related: []
tags: [api, openapi]
created: 2026-04-09
updated: 2026-04-09
```

**Pros:**
- **Zero impact on the yaml file itself.** A team's yaml file validates against any OpenAPI tool without kluris-specific cruft.
- Simple read path: if yaml file exists, also look for `<file>.meta`.
- Write path is trivial — it's just a regular YAML dump.
- Doesn't require touching `python-frontmatter` at all.

**Cons:**
- **Two files per neuron** — noise in git diffs, confusing `ls` output.
- Orphaned `.meta` files (yaml deleted but meta left behind) become a new cleanup concern for `dream`.
- The CLI has to teach users: "add openapi.yml → also creates openapi.yml.meta".
- `validate_synapses` / `detect_orphans` need to know both files exist as a *pair*.
- Feels un-idiomatic for a git-backed knowledge repo where the whole file IS the unit of knowledge.

**Effort:** Low-medium. Simple to implement but adds UX friction.

#### Option C — OpenAPI `x-kluris:` extension key (only works for OpenAPI files)

```yaml
openapi: 3.1.0
info:
  title: Payments API
  version: 1.0.0
x-kluris:
  parent: ./map.md
  related:
    - ../../knowledge/use-raw-sql.md
  tags: [api, payments]
  created: 2026-04-09
  updated: 2026-04-09
```

**Pros:**
- **100% OpenAPI-spec-compliant.** Validates in Redocly, Spectral, Swagger Editor without any warnings.
- The metadata is a **first-class part of the document**, not a separate block. Anyone reading the yaml sees it.
- Easy to parse: `yaml.safe_load(path) → data.get("x-kluris", {})`.
- Easy to write: `data["x-kluris"] = patch; yaml.safe_dump(data, path)`.
- Naturally scoped to OpenAPI-like schemas — won't accidentally be picked up in unrelated yaml files.

**Cons:**
- **Only works for documents with a top-level object structure** that allows arbitrary keys. OpenAPI 3.1 does; JSON Schema does; k8s manifests technically do; but **a yaml list or a bare scalar file would not support it.**
- Locks the feature into OpenAPI-shape yaml. If the team wants to capture a `ci.yml` or a `prometheus.yml`, those have their own schemas where `x-kluris:` may not be valid.
- Re-writing a yaml file with PyYAML loses comments and preserves only values — **a serious loss of human-authored context.** Mitigation: use `ruamel.yaml` (new dep) for round-trip preservation. But that's another dep.
- Filters into the yaml file's body section: when the search command builds a body snippet, should it include the `x-kluris:` block? Need a filter.

**Effort:** Medium. Read path is easy. Write path is the hard part — need round-trip yaml to preserve comments, or accept comment loss.

#### Option D — No frontmatter; derive from path + optional `info:` block

```yaml
# openapi.yml in brain/projects/payments-api/
openapi: 3.1.0
info:
  title: Payments API
  version: 1.0.0
  description: Core payments backend
paths:
  /charge:
    post: ...
```

**Pros:**
- **Zero authoring cost.** Any `openapi.yml` file dropped into a lobe is instantly a kluris-known neuron.
- Title comes from `info.title`, description from `info.description`, path from the filesystem.
- `parent` is always `./map.md` (the containing lobe) — inferred, not stated.
- `created` / `updated` come from git (same batch helper dream already uses).
- `related:` is the one thing we lose — yaml neurons can't declare bidirectional synapses. **But**: other neurons can still link TO the yaml file via markdown `[charges](./openapi.yml)` syntax. So the yaml file is a valid link target even without declaring back-links.
- `tags:` can be inferred from the containing directory (`payments-api/openapi.yml` → `tags: [payments-api, api]`).

**Cons:**
- Can't express `related:` from a yaml neuron to other neurons — creates asymmetric graphs.
- Can't express `deprecated` / `replaced_by` on a yaml file — the deprecation feature doesn't work for yaml.
- Can't add `tags:` for cross-cutting concerns beyond what the path implies.
- `check_frontmatter()` currently requires `parent`, `created`, `updated` on every neuron. If yaml neurons genuinely have none of these, they either need a bypass in the validator or a virtual-frontmatter synthesizer.

**Effort:** Low. No new parsing, no writer changes. But the "no related links from yaml" limitation may be a dealbreaker.

#### Frontmatter option comparison

| Criterion | A (hash block) | B (side-car) | C (x-kluris) | D (path-derived) |
|-----------|----------------|--------------|--------------|------------------|
| Human authoring cost | Medium (learn convention) | Medium (two files) | Low (familiar OpenAPI) | **Zero** |
| Works on any .yml file | **Yes** | **Yes** | OpenAPI-shape only | **Yes** |
| Preserves yaml validity | **Yes** (hash = comment) | **Yes** | **Yes** (ext is spec) | **Yes** |
| `related:` / synapses | **Yes** | **Yes** | **Yes** | No |
| `deprecated` support | **Yes** | **Yes** | **Yes** | No |
| Comment preservation on write | **Yes** (string-based) | **Yes** (meta is separate) | No (PyYAML drops comments) | **N/A** |
| Requires new dep | No | No | No (or ruamel for roundtrip) | No |
| Parser complexity | Medium | Low | Low | Low |
| Writer complexity | Medium | Low | Medium (or high w/ comments) | N/A |
| Accidental kluris.yml pickup risk | Low (no `#---` block) | High (if kluris.yml had a .meta) | **Medium (kluris.yml has no x-kluris but the walker must still skip it)** | High (kluris.yml has no path rule) |
| Fits kluris mental model | Medium | Low | High (if OpenAPI) | High (but limited) |

**Recommendation:** **Option A** or a **hybrid A+D**.
- Pure A is the cleanest general-purpose solution: an author can write any yaml with an optional `#---` frontmatter block, and kluris treats the presence of that block as consent to be a neuron.
- A+D hybrid: if a yaml file has a `#---` block, use it. If it has `info.title`/`info.description`, use those as fallbacks. If it has neither, use the filename. This gives the agent the lightest possible authoring path (for OpenAPI files, just drop the file) while preserving the explicit `related:`/`deprecated:` workflow when needed.
- Option C is tempting for OpenAPI-specific use cases but breaks the "any yaml file" invariant and loses comments on write.
- Option B should be rejected — the two-file model is too alien for the kluris mental model and creates a new orphan class.

### 4.2 Link detection FROM a `.yml` file

**Answer: do NOT parse yaml body content for links.** OpenAPI and similar yaml files contain thousands of `$ref:` structural references (to schemas and components, not other neurons). Trying to treat those as synapses would explode the graph and match the wrong things.

**Instead, use a layered approach:**

1. **Frontmatter `related:`** (if Option A or C is used) — exactly like markdown neurons. Parses an explicit list.
2. **Frontmatter `parent:`** — same as markdown; always `./map.md`.
3. **No body-level link extraction.** `LINK_PATTERN` (markdown `[text](url)` regex) does not apply to yaml files at all. Don't attempt it.
4. **Inline-link edges in MRI (`mri.py:192-208`)** — these apply only to files where `read_frontmatter` returns a body with markdown syntax. For yaml files, the body will be yaml, so `LINK_PATTERN.finditer(content)` will match nothing (no `[text](url)` in a yaml file). **This naturally no-ops** — no edit required. The inline-link pass silently skips yaml files, which is correct.

**What about outgoing edges from markdown neurons TO yaml neurons?** This ALREADY works if we do nothing special:
- A markdown neuron can write `[API spec](./openapi.yml)` in its body.
- `mri.py:192-208` runs `LINK_PATTERN.finditer(content)` which matches any `[text](path)` regardless of suffix.
- The resolution logic at line 197 resolves `./openapi.yml` as a brain-relative path and looks it up in `node_ids`. **If the yaml file is already a node, the edge will be added.**
- Line 198 filter `t_resolved != rel` still holds.
- **But:** line 1746 in the JS modal regex is `/\[([^\]]+)\]\(([^)]+\.md)\)/g` — it only matches `.md` targets. So the modal view won't turn `[API spec](./openapi.yml)` into a clickable link. That regex must be broadened to `/\[([^\]]+)\]\(([^)]+\.(md|yml|yaml))\)/g`.

**What about orphan detection?** `detect_orphans` at `linker.py:193-212` walks every map.md and brain.md, collects referenced links, and checks each neuron against the set. For yaml neurons, we need each yaml neuron to appear in the map.md of its containing lobe — which happens automatically once `maps.py::_get_neurons` (line 61-87) is broadened to include `.yml`/`.yaml` files.

### 4.3 Node-type vs file-type design

The decision is: should yaml neurons have `type: 'yaml'` (a new enum value) or `type: 'neuron'` with an added `file_type: 'yaml'` field?

#### Option X — New node type: `type: 'yaml'`

```python
# mri.py build_graph
elif f.suffix in {".yml", ".yaml"}:
    ntype = "yaml"
else:
    ntype = "neuron"
```

**Pros:**
- Discoverable in the graph JSON without string-parsing the path.
- JS `colorForNode(node)` can add `if (node.type === 'yaml') return '#f8c76d';` cleanly.
- Filter logic (`.filter(n => n.type === 'neuron')`) is intentional: if yaml files shouldn't show in the file browser tree, leave them out. If they should, add `|| n.type === 'yaml'` explicitly.
- Matches the existing pattern (`brain`, `index`, `glossary`, `map`, `neuron`).

**Cons:**
- **~18 locations across mri.py need updating** — not just `colorForNode`, but every JS filter that mentions `n.type === 'neuron'`:
  - Line 1364 — total count
  - Line 1652 — file tree filter
  - Line 1786 — modal navigation filter
  - Plus any future filter will need the same awareness.
- Creates an artificial distinction for `_neuron_files()` — is yaml a "neuron" or not? The current helper returns a flat list; splitting yaml off as a new type means a second helper or a union result.
- Breaks the "neuron = unit of knowledge" mental model where a yaml OpenAPI spec is just as much a unit of knowledge as a markdown decision record.

#### Option Y — Keep `type: 'neuron'`, add `file_type: 'yaml' | 'markdown'`

```python
# mri.py build_graph
if f.name == "brain.md":
    ntype = "brain"
elif ...
else:
    ntype = "neuron"  # unchanged for both md and yml

# Add file_type discriminator
nodes.append({
    ...
    "type": ntype,
    "file_type": "yaml" if f.suffix in {".yml", ".yaml"} else "markdown",
    ...
})
```

**Pros:**
- **Zero JS filter rewrites** — every `.type === 'neuron'` filter Just Works for yaml too.
- `_neuron_files()` stays conceptually unified: "all neurons, regardless of file type".
- Color / icon logic can discriminate via `file_type`: `colorForNode` becomes `if (node.type === 'neuron' && node.file_type === 'yaml') return '...';`
- Matches the "neuron = unit of knowledge" mental model.
- Easier to extend later (e.g., a future `json` neuron type).

**Cons:**
- Two dimensions where one used to suffice — slight cognitive load.
- Existing consumers that switch on `type` (rare — mostly JS filters) need to know about the second dimension if they want to distinguish.

**Recommendation:** **Option Y** (keep `type: 'neuron'`, add `file_type`).
- Fewer touch points in the 2537-line mri.py JS code.
- Preserves the existing filter idioms.
- Better matches the "yaml is just another flavor of neuron" conceptual framing.
- The single point that distinguishes yaml is `colorForNode`, which can take the new discriminator and emit a clearly different hue (e.g., `#9ea9ff` vs the current `#7bf7ff` lobe color, or a fixed yaml-green).

### 4.4 Wake-up / dream / search schema additions

These are JSON outputs consumed by the agent skill. Breaking changes are unacceptable. The yaml-neurons spec must extend them *additively*.

#### Current wake-up schema (cli.py:869-937 + collectors at 669-801)

```json
{
  "ok": true,
  "name": "my-brain",
  "path": "/absolute/path",
  "description": "...",
  "brain_md": "<body>",
  "lobes": [{"name": "projects", "neurons": 42}],
  "total_neurons": 42,
  "recent": [{"path": "projects/auth.md", "updated": "2026-04-08"}],
  "glossary": [{"term": "OAuth", "definition": "..."}],
  "deprecation_count": 0,
  "deprecation": []
}
```

**Required changes:**
- `lobes[].neurons` — today it's one count. Either keep it as "all neurons regardless of file type" (simplest, no breaking change) or split to `{markdown: N, yaml: M, total: N+M}`. **Recommendation: keep as total, add `lobes[].neurons_by_type: {markdown: N, yaml: M}` as a new optional field.**
- `total_neurons` — same as above. Keep as total; add `total_yaml_neurons: N` as a new field.
- `recent[]` — works as-is for yaml. The `path` field will show `projects/openapi.yml`. Optionally add `file_type: 'yaml' | 'markdown'` per entry so the agent knows what it's looking at.

#### Current search schema (cli.py:804-866)

```json
{
  "ok": true,
  "brain": "my-brain",
  "query": "payments",
  "total": 3,
  "results": [
    {
      "file": "projects/payments.md",
      "title": "Payments service",
      "matched_fields": ["title", "body"],
      "snippet": "...payments...",
      "score": 18,
      "deprecated": false
    }
  ]
}
```

**Required changes:**
- Add `results[].file_type: 'yaml' | 'markdown'` — tells the agent what it's looking at so it can render an appropriate "snippet" presentation.
- `snippet` extraction needs a yaml-aware variant: for a yaml file, the "body" is the whole yaml document (minus frontmatter). Showing the first N lines is probably more useful than the markdown prose-based snippet logic. **Recommendation: the snippet extractor lives in `search.py::_extract_snippet`; add a branch for file_type == yaml that returns "first occurrence of query in yaml body, with 100-char context" — same behavior, just without the markdown-specific heuristics.**
- Scoring: `_score_hit` uses `title*10 + tag*5 + path*3 + body*1`. For yaml neurons, what's the title? **Answer: (a) `x-kluris.title` frontmatter field if present, (b) `info.title` if it's an OpenAPI file, (c) filename stem as Title Case.** This is the same fallback ladder as markdown neurons.

#### Current dream schema (test_dream.py:33-36, cli.py dream command)

```json
{
  "ok": true,
  "healthy": false,
  "broken_synapses": 0,
  "one_way_synapses": 0,
  "orphans": 0,
  "frontmatter_issues": 0,
  "deprecation_issues": 0,
  "fixes": {
    "dates_updated": 3,
    "parents_inferred": 1,
    "reverse_synapses_added": 2,
    "orphan_references_added": 0,
    "total": 6
  },
  "maps_regenerated": ["projects"],
  "lobes_discovered": ["projects", "infrastructure"]
}
```

**Required changes:**
- `fixes.dates_updated` should include yaml neurons whose `updated:` field was bumped.
- `frontmatter_issues` — if Option A (hash block) is used and a yaml neuron has the block but is missing `parent`, that's an issue to report.
- No new top-level fields needed. The existing counters are totals; they can absorb yaml neuron counts transparently.

### 4.5 Color palette for MRI visualization

The current lobe palette at `mri.py:1178`:
```javascript
const lobePalette = ['#7bf7ff','#ff8bd8','#f8c76d','#7df7b4','#9ea9ff','#ffa06f','#b8f0c1','#f2a8ff'];
```

And the current type-color dispatch at `mri.py:1243-1250`:
```javascript
function colorForNode(node) {
  if (node.type === 'brain') return '#ffffff';
  if (node.type === 'glossary') return '#ffc6f4';
  if (node.type === 'index') return '#ffd28e';
  if (node.type === 'map') return lobeColor(node.lobe);
  // neuron: desaturated lobe color
  return desaturate(lobeColor(node.lobe), 0.3);
}
```

Neurons today get the lobe's base color desaturated by 0.3. Yaml neurons need a visually distinct treatment — they're structurally different (schemas, contracts, configs) and should be instantly identifiable in the graph.

**Three palette options:**

#### Option 1 — Single fixed yaml color (most distinct)
`#9ea9ff` (pale periwinkle, distinct from all 8 lobe colors). Reads as "structured/configuration" to the eye. Pros: instantly identifiable. Cons: loses lobe-association.

#### Option 2 — Lobe color with different saturation (minimal visual change)
Use the lobe's base color at 0.6 desaturation (lighter than markdown neurons at 0.3) and a different node shape (if we can render non-circles). Pros: preserves lobe grouping visually. Cons: too subtle to distinguish at a glance.

#### Option 3 — Lobe color with yaml-distinct border / outline
Keep fill as desaturated lobe color but add a bright yellow stroke around yaml node circles. Pros: best of both worlds — lobe grouping + yaml recognition. Cons: requires touching the node drawing code (not just `colorForNode`), slightly more work.

**Recommendation:** **Option 1** with `#f8c76d` (the existing accent amber, which is already the glossary/index color family) or a new `#ffd28e`. It's the lowest-risk, smallest-diff change. Two lines in `colorForNode`:
```javascript
function colorForNode(node) {
  if (node.type === 'brain') return '#ffffff';
  if (node.type === 'glossary') return '#ffc6f4';
  if (node.type === 'index') return '#ffd28e';
  if (node.type === 'map') return lobeColor(node.lobe);
  if (node.file_type === 'yaml') return '#f8c76d';  // NEW
  return desaturate(lobeColor(node.lobe), 0.3);
}
```

If the spec goes with Option X (new type), substitute `node.type === 'yaml'` for `node.file_type === 'yaml'`.

### 4.6 Agent template update (agents.py SKILL.md)

Current state (agents.py:180):
```
If user asks for OpenAPI: generate `openapi.yml` (OpenAPI 3.1), not markdown.
```

This one line is the ONLY hint the agent currently has about yaml files. And today, nothing in kluris scans them. **This is the exact mismatch yaml-neurons fixes.**

**Required SKILL.md updates (in SKILL_BODY):**

1. **"How the brain is structured" section** (lines 116-138):
   - Add a line: "Neurons can be markdown (`.md`) OR structured yaml (`.yml`/`.yaml`). Yaml neurons are for machine-readable contracts like OpenAPI specs, JSON schemas, or config references. They sit alongside markdown neurons in lobes and show in `map.md` the same way."

2. **Intent detection → "Learn from project"** (line 180):
   - Replace the current single line with: "If user asks for OpenAPI: generate `openapi.yml` (OpenAPI 3.1) with a `#---` frontmatter block at the top containing `parent`, `related`, `tags`, `created`, `updated`. Example: [insert template]. Kluris will index it as a yaml neuron."
   - OR (for Option C): "... with an `x-kluris:` extension on the root containing `parent`, `related`, `tags`, `created`, `updated`."

3. **Writing rules** (lines 244-281):
   - Add: "Yaml neurons carry the same frontmatter contract as markdown neurons (`parent`, `related`, `tags`, `created`, `updated`), delivered via [hash-style block | x-kluris extension | side-car file] depending on the chosen approach."
   - Extend the example to show a yaml frontmatter block next to the markdown example.

4. **Search intent** (line 144-162):
   - Add: "Search results may include yaml neurons (look for `file_type: 'yaml'` in `--json` output). Their `snippet` shows raw yaml content; their `title` comes from the frontmatter `title` field, `info.title` (OpenAPI), or the filename stem."

5. **CLI commands list** (lines 283-292):
   - `kluris neuron` can now accept `.yml` / `.yaml` filenames. Update docs accordingly — though check whether `neuron` command should even support yaml creation or leave that to the agent's `Write` tool (since yaml neurons are typically not templated the way decisions/incidents are).

**Testing implication:** `tests/test_agents.py` (7642 bytes) exercises `render_skill(...)` output. Adding yaml-awareness to SKILL_BODY means new tests that assert the yaml-related strings are present in the rendered skill.

---

## 5. Test Infrastructure Analysis

### Current Setup
- **Test framework:** pytest 8.0+ (detected from `pyproject.toml` dev deps and `tests/` directory).
- **Test runner:** `pytest tests/ -v` or `pytest tests/ --cov=kluris -q` (documented in AGENTS.md and CLAUDE.md).
- **Test directory convention:** Separate `tests/` tree (not colocated). 28 test files.
- **Test file count:** 28 unit/integration test files, **290 tests**, 90%+ coverage.
- **Mocking library:** `monkeypatch` (pytest built-in) for env vars, `CliRunner` (click.testing) for command invocation. **No jest-style mocks.** Real filesystem via `tmp_path`, real git subprocess in temp dirs.
- **Coverage tool:** `pytest-cov` 5.0+.
- **Mutation testing:** Not configured.
- **Docker:** Not used in testing.
- **Testcontainers:** Not used.
- **CI test gates:** `.github/workflows/ci.yml` runs `pytest tests/` on PR across ubuntu/macos/windows × Python 3.10–3.13. No coverage threshold gate that I could find.
- **Version gate:** `pyproject.toml` and `src/kluris/__init__.py` versions must match.
- **Existing test count:** 290 tests across 28 files.

### Test Quality Assessment

**Naming convention:** Descriptive `test_<behavior>_<condition>` (e.g. `test_graph_nodes_include_metadata`, `test_focus_on_node_zooms_sublobe_members_only`, `test_dream_fixes_one_way_synapse`). Very readable; tests document the behavior precisely.

**Isolation approach:** Each test is fully independent. Every test scaffolds its own brain in `tmp_path` (either via the CliRunner + `create_test_brain` helper, or via the `_make_brain_with_neurons(tmp_path)` low-level helper pattern seen in `test_mri.py`, `test_linker.py`, `test_maps.py`, `test_search.py`). No shared mutable fixtures beyond `tmp_path` and `monkeypatch`. The `temp_brain` fixture in conftest.py scaffolds a brain with 3 lobes, git init, and registers it in `temp_config` — but it's parametrized per-test.

**Edge case coverage:** Strong. Examples:
- `test_graph_ignores_invalid_frontmatter_links` (test_mri.py:65) — out-of-brain paths in frontmatter.
- `test_html_under_500kb` (test_mri.py:366) — performance gate on generated HTML size.
- `test_search_chips_removed` (test_mri.py:350) — regression test for deleted feature.
- `test_sidebars_are_collapsible_and_long_names_dont_overflow` (test_mri.py:320) — UX detail tests that assert specific CSS/JS classes.

**Anti-patterns found:**
- **None significant.** Tests are clean, focused, and avoid the common anti-patterns.
- The one smell: `test_mri.py` has many tests that grep raw HTML output for specific class names and JS function names (e.g. `assert ".lobe-card" in html`, `assert "function renderLobes" in html`). This couples tests to implementation details. For yaml-neurons, **follow the same pattern** — the existing style is to assert on HTML/JS strings, and deviating would be inconsistent.

**Strengths:**
- Tests exercise real filesystem I/O via `tmp_path`. No mocked disk.
- Tests exercise real git subprocess (not mocked) when testing date-sync or commit behavior.
- CliRunner tests verify the full JSON output contracts — breaking changes get caught immediately.
- Per-feature test files (`test_mri.py`, `test_mri_cmd.py`, `test_wake_up.py`, `test_search.py`) keep concerns separated.
- `counting_git_run` fixture lets tests assert "dream should make exactly 2 subprocess calls" — performance regression gate.

### Recommended Test Strategy for yaml-neurons

Since the spec is TDD-first, every new behavior needs a failing test first. Here's the test matrix:

| Test file | New tests | Behavior |
|-----------|-----------|----------|
| `tests/test_yaml_neurons.py` (NEW) | `test_read_yaml_neuron_with_hash_frontmatter` | Parser extracts `parent`, `related`, `tags`, `created`, `updated` from a `#---` block |
| `tests/test_yaml_neurons.py` | `test_read_yaml_neuron_without_frontmatter` | Raw yaml (no block) is readable, metadata is empty dict, body is whole file |
| `tests/test_yaml_neurons.py` | `test_read_yaml_neuron_malformed_frontmatter` | Hash block with bad yaml is handled gracefully (no exception) |
| `tests/test_yaml_neurons.py` | `test_write_yaml_neuron_preserves_body` | Writing frontmatter to a yaml file leaves the yaml body intact |
| `tests/test_frontmatter.py` (extend) | `test_update_frontmatter_yaml_file` | `update_frontmatter(path, patch)` works on yaml files |
| `tests/test_linker.py` (extend) | `test_neuron_files_includes_yaml_files` | `_neuron_files()` returns both .md and .yml entries |
| `tests/test_linker.py` | `test_neuron_files_excludes_kluris_yml` | **Critical regression guard.** `kluris.yml` at brain root must never appear |
| `tests/test_linker.py` | `test_validate_synapses_on_yaml_neuron_related_list` | Broken synapse detection works when `related:` is in a yaml neuron |
| `tests/test_linker.py` | `test_fix_bidirectional_synapses_yaml_to_md` | Reverse-link from a yaml neuron points at an md neuron and vice versa |
| `tests/test_linker.py` | `test_detect_orphans_yaml_neuron_not_in_map` | Orphan detection catches a yaml neuron not linked from its lobe's map.md |
| `tests/test_linker.py` | `test_check_frontmatter_yaml_missing_parent` | Frontmatter validator reports missing `parent` on yaml neuron |
| `tests/test_linker.py` | `test_deprecation_on_yaml_neuron` | Deprecation frontmatter works on yaml neurons (replaced_by, etc.) |
| `tests/test_maps.py` (extend) | `test_get_neurons_includes_yaml_files` | `_get_neurons(lobe_path)` returns `.yml`/`.yaml` neurons |
| `tests/test_maps.py` | `test_generate_map_md_lists_yaml_neurons` | Generated map.md contains links to yaml files |
| `tests/test_search.py` (extend) | `test_collect_searchable_includes_yaml_neurons` | `_collect_searchable()` returns yaml neuron entries |
| `tests/test_search.py` | `test_search_ranks_yaml_neuron_title_match` | Scoring works the same for yaml neurons (title > tag > path > body) |
| `tests/test_search.py` | `test_search_snippet_yaml_body` | Snippet extraction for yaml body returns lines around match |
| `tests/test_search.py` | `test_search_excludes_kluris_yml` | **Regression guard.** `kluris.yml` never appears in search results |
| `tests/test_wake_up.py` (extend) | `test_wake_up_counts_yaml_neurons_in_lobes` | `lobes[].neurons` includes yaml count |
| `tests/test_wake_up.py` | `test_wake_up_total_neurons_includes_yaml` | `total_neurons` includes yaml count |
| `tests/test_wake_up.py` | `test_wake_up_recent_includes_yaml_updates` | Recently updated yaml neurons appear in `recent[]` |
| `tests/test_wake_up.py` | `test_wake_up_excludes_kluris_yml` | **Regression guard.** kluris.yml never in wake-up output |
| `tests/test_dream.py` (extend) | `test_dream_updates_yaml_neuron_dates` | `_sync_brain_state` updates `updated:` on yaml neurons from git log |
| `tests/test_dream.py` | `test_dream_regenerates_map_with_yaml_entries` | After adding a yaml neuron, dream updates map.md to include it |
| `tests/test_dream.py` | `test_dream_excludes_kluris_yml_from_sync` | **Regression guard.** kluris.yml never touched by dream |
| `tests/test_mri.py` (extend) | `test_graph_includes_yaml_neurons_as_nodes` | `build_graph()` adds yaml files as nodes |
| `tests/test_mri.py` | `test_graph_yaml_neuron_has_file_type_yaml` | Node has `file_type: 'yaml'` (or `type: 'yaml'`) |
| `tests/test_mri.py` | `test_graph_markdown_to_yaml_link_creates_edge` | Markdown neuron linking to yaml file creates an inline edge |
| `tests/test_mri.py` | `test_graph_yaml_frontmatter_related_creates_edge` | yaml neuron with `related:` field creates edges |
| `tests/test_mri.py` | `test_html_colors_yaml_neurons_distinctly` | Generated HTML has the new yaml color constant in JS |
| `tests/test_mri.py` | `test_html_modal_link_regex_matches_yaml` | Modal link regex broadened to match .yml/.yaml targets |
| `tests/test_mri.py` | `test_html_file_tree_includes_yaml_files` | File browser tree in modal shows yaml neurons |
| `tests/test_mri.py` | `test_graph_excludes_kluris_yml` | **Regression guard.** kluris.yml never a graph node |
| `tests/test_neuron.py` (extend) | `test_create_yaml_neuron_via_cli` | `kluris neuron openapi.yml --lobe projects` creates a valid yaml neuron |
| `tests/test_agents.py` (extend) | `test_skill_body_mentions_yaml_neurons` | Rendered SKILL.md tells the agent about yaml neurons |
| `tests/test_brain.py` (extend) | `test_validate_brain_name_doesnt_affect_yaml_files` | Brain name validator only affects brain names, not neuron filenames |

**Approximate new test count:** ~35 new tests across 10 existing test files + 1 new test file.

### Fixtures to add

Following the existing pattern of per-file `_make_brain_with_X(tmp_path)` helpers:

```python
# In tests/test_linker.py (or a new tests/helpers/yaml_brain.py if shared):
def _make_brain_with_yaml_neurons(tmp_path):
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")
    (brain / "kluris.yml").write_text("name: brain\n", encoding="utf-8")  # CRITICAL: must be excluded
    
    projects = brain / "projects"
    projects.mkdir()
    (projects / "map.md").write_text("---\nauto_generated: true\nparent: ../brain.md\n---\n# Projects\n", encoding="utf-8")
    
    # md neuron
    (projects / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated: [./openapi.yml]\ntags: [auth]\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    
    # yaml neuron with hash-style frontmatter (Option A)
    (projects / "openapi.yml").write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# related: [./auth.md]\n"
        "# tags: [api, openapi]\n"
        "# created: 2026-04-01\n"
        "# updated: 2026-04-01\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
        "paths: {}\n",
        encoding="utf-8",
    )
    return brain
```

**Test pattern to use throughout (based on existing style):** Low-level helpers for direct core module tests (`build_graph(brain)`, `read_frontmatter(path)`) and CliRunner for command-level tests (`runner.invoke(cli, ["search", "..."])`).

**CRITICAL constraint from user memory (`feedback_kluris_no_live_disk.md`):** Tests must use `tmp_path` fixtures and must NEVER shell out to the `kluris` command or touch `~/.kluris`, `~/.claude/skills/`, etc. Every test in the existing suite already follows this — new yaml-neurons tests must too. Use `monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))` and `monkeypatch.setenv("HOME", str(tmp_path))` at the top of every CliRunner-based test.

---

## 6. Risk Assessment

### 6.1 Breaking changes

| Risk | Severity | Mitigation |
|------|----------|------------|
| Agent skills cached from older `kluris wake-up` runs see unexpected `file_type` field in `recent[]` | **Low** | Additive fields don't break consumers. Agent code should ignore unknown keys. |
| JSON output contract change on `kluris search --json` | **Medium** | `file_type` field is additive. `total` may increase (yaml neurons now count), but that's the intended behavior. |
| Existing tests that assert exact neuron counts fail when fixtures now include yaml files | **Medium** | None of the existing fixtures include yaml files (verified via grep — no `.yml` or `.yaml` in `tests/` except `KLURIS_CONFIG` env and `kluris.yml` in fixtures). Existing tests will not see yaml neurons unless they add them explicitly. |
| `detect_orphans` flags yaml neurons as orphans because map.md didn't use to list them | **Low** | Once `maps.py::_get_neurons` is updated, dream regenerates map.md with yaml entries on first run. Transient issue. |
| `check_frontmatter` starts reporting new issues on yaml neurons that lack a hash block | **Medium** | If Option A is chosen and existing yaml files in test brains don't have `#---` blocks, the validator will flag them. **Solution:** `check_frontmatter` treats missing frontmatter on yaml files as a soft warning, not a hard error. Or: only enforce frontmatter on `.md` files and use a lighter contract for `.yml` (no required `parent`, since filesystem position already encodes it). |
| `python-frontmatter`'s YAMLHandler raising on `kluris.yml` if a walker accidentally picks it up | **HIGH** | **kluris.yml** lives at every brain root. It's a Pydantic-serialized yaml with fields like `name`, `description`, `type`, `git`, `agents`. If any walker picks it up as a yaml neuron: (a) search will surface brain config in results, (b) wake-up recent will include `kluris.yml`, (c) mri will render it as a node, (d) dream may try to update its `updated:` field. **The spec MUST explicitly add `kluris.yml` to every walker's SKIP_FILES set — in ALL SEVEN locations.** |

### 6.2 Performance

| Risk | Severity | Mitigation |
|------|----------|------------|
| Larger brains now have more files to walk (yaml neurons added) | Low | Marginal — yaml neurons are typically few compared to markdown neurons. |
| `_sync_brain_state` in dream makes one `read_frontmatter` call per yaml file | Low | Same cost as markdown neurons. Dream already batches git log calls. |
| `build_graph` allocates node objects for yaml neurons | Low | Linear with neuron count; no graph-quadratic regression. |
| MRI HTML size grows because yaml nodes are added to the JSON payload | **Medium** | `test_html_under_500kb` gate at `test_mri.py:366` will catch regressions. Yaml neurons with large `content_full` bodies (e.g. a full 500-line OpenAPI spec) could inflate HTML noticeably. **Solution:** cap `content_full` for yaml neurons at a lower threshold than markdown, or strip the yaml body entirely and only keep `content_preview` for yaml. |
| `_extract_snippet` on a large yaml body is slower than on markdown prose | Low | Linear scan is fast even on multi-MB files. |

### 6.3 Security

| Risk | Severity | Mitigation |
|------|----------|------------|
| Malicious yaml neuron with YAML anchors / `!!python/object` tags | **Medium** | Always use `yaml.safe_load`. Confirm existing code does this — **verified**: `src/kluris/core/config.py:100` uses `yaml.safe_load`. The new yaml-neurons parser must do the same. |
| Path-traversal via `parent:` or `related:` in a yaml neuron | Low | Existing `_is_within_brain` check in linker.py:47-57 applies to ALL frontmatter, including yaml. Already solved. |
| Large yaml file (DoS) consumed by `read_frontmatter` | Low | File size caps can be added if needed. Not currently enforced for markdown, so consistency says don't add for yaml either. |

### 6.4 Scalability

| Risk | Severity | Mitigation |
|------|----------|------------|
| A brain with hundreds of OpenAPI files from a large microservices project | Low | Dream's batch git helper handles arbitrary file counts. The existing `conftest.py::create_test_brain_with_neurons(count=100)` already tests the 100-neuron path. Adding yaml neurons is proportional. |
| MRI graph with yaml node explosion | Low | Same as above — linear cost. |

### 6.5 Migration

| Risk | Severity | Mitigation |
|------|----------|------------|
| Existing brains have NO yaml neurons today, so there's nothing to migrate | None | **This is a purely additive feature.** No migration path needed. Existing brains work unchanged; users who want yaml neurons just start adding them. |
| Brains created BEFORE this spec that hand-authored yaml files outside of kluris's awareness are now suddenly picked up | **Medium** | On first `kluris dream` after upgrade, these files become visible. Dream will try to update their dates, possibly flag missing frontmatter. **Solution:** make the first dream run idempotent and non-destructive — it should add missing fields, not error out. If a yaml file has no frontmatter at all, treat it as opt-out (don't flag it, but also don't index it). This requires an "opt-in marker" — e.g. the presence of the `#---` block says "this is a kluris yaml neuron; without it, skip". |
| Team brains shared via git where one dev upgrades to the yaml-neurons version and another doesn't | Low | Additive feature. The old kluris version continues to ignore yaml files, the new one indexes them. No git conflicts. |

### 6.6 The single highest-risk item

**`kluris.yml` accidental pickup.** This file:
- Lives at `<brain>/kluris.yml` (brain root).
- Contains Pydantic-serialized config (brain name, description, git settings, agent opt-ins).
- Is gitignored — so doesn't leak via `kluris push` — but IS on disk.
- Has a yaml extension — matches any naive `*.yml` scanner.
- Is NOT currently in any SKIP_FILES set (because scanners only looked at `.md` before).

**Mitigation checklist** — the spec must add `kluris.yml` to:
1. `src/kluris/core/linker.py:16` — `SKIP_FILES`
2. `src/kluris/core/maps.py:10` — `SKIP_FILES`
3. `src/kluris/core/mri.py:12` — `SKIP_FILES`
4. `src/kluris/cli.py:154` — dream's inline skip set
5. `src/kluris/cli.py:666` — `_WAKE_UP_SKIP_FILES`

**And every scanner needs a regression test:** `assert "kluris.yml" not in result`.

Better yet: create a single shared `SKIP_FILES_ALL` constant in one module and import it everywhere. This is the point where the divergent skip sets become a bug waiting to happen.

---

## 7. Open Questions for Interview

The following need user input before the spec can be finalized:

1. **Frontmatter format: which of Options A (hash block), C (x-kluris extension), or D (derive from path) is preferred?**
   - Option A is most general (works on any yaml file) but requires agent authoring discipline.
   - Option C is OpenAPI-idiomatic but locks the feature to OpenAPI-shape yaml and loses comments on write.
   - Option D is zero-friction for dropping in yaml files but can't express synapses or deprecation.
   - Hybrid A+D (block if present, derive otherwise) is also feasible — is the added complexity worth it?

2. **Scope: which yaml files should be indexed?**
   - All yaml files anywhere in the brain? (Broadest — but creates `kluris.yml` risk and picks up unintentional files.)
   - Only yaml files with an explicit frontmatter block / marker? (Narrowest — opt-in, safest.)
   - Only yaml files in specific lobes based on lobe-level config? (Middle ground — but no config mechanism exists today.)

3. **Node type: Option X (`type: 'yaml'`) or Option Y (`type: 'neuron'` + `file_type: 'yaml'`)?**
   - X touches ~18 JS filter sites in mri.py.
   - Y is a zero-filter-change addition.
   - The user's aesthetic preference matters here — does "yaml neuron" feel like "a different thing" or "just another neuron"?

4. **MRI color: fixed yaml color or lobe-tinted with a yaml-specific marker?**
   - Fixed `#f8c76d` (single identifiable color, loses lobe grouping).
   - Lobe color with different saturation (preserves grouping, too subtle).
   - Lobe color with bright yellow outline (best visual, more code).

5. **`check_frontmatter` contract: should yaml neurons be required to have `parent`, `created`, `updated`?**
   - Yes — strict validation, consistent with markdown.
   - No — yaml neurons are opt-in lightweight (filesystem position implies parent, git implies created/updated).
   - Mixed — require a smaller contract (e.g., only `updated:`), infer the rest.

6. **Writing yaml frontmatter: PyYAML round-trip (loses comments) or hash-block string manipulation (preserves file exactly)?**
   - PyYAML is simple but re-serializes the entire file on every write, destroying author comments.
   - Hash-block manipulation treats the frontmatter as a text block, leaves the yaml body byte-for-byte identical.
   - ruamel.yaml preserves comments but is a new dependency (slower, larger).

7. **`kluris neuron` command for yaml: should it support yaml neuron creation directly?**
   - `kluris neuron openapi.yml --lobe projects/payments-api` — does this generate a template?
   - Or is yaml neuron creation always done by the agent's `Write` tool, with `kluris dream` just indexing it after the fact?
   - If templates: are there `NEURON_TEMPLATES` for yaml (like `openapi`, `jsonschema`, `k8s-manifest`)?

8. **Deprecation flow: should yaml neurons support `status: deprecated`, `replaced_by: ./v2.yml`?**
   - Yes — consistent with markdown.
   - No — yaml neurons are typically versioned by filename (`v1.yml`, `v2.yml`).

9. **Skipping `kluris.yml`: single shared `SKIP_FILES_ALL` refactor, or surgical additions to each of the 5 existing SKIP_FILES sets?**
   - Refactor is cleaner but expands scope.
   - Surgical additions are safer for a TDD-driven spec but perpetuate the inconsistency.

10. **Search snippet strategy for yaml neurons: markdown-style prose extraction, or yaml-line-aware (show the yaml key/value path that matches)?**
    - Reusing the markdown snippet logic (first 200 chars around match) is simple but produces ugly snippets like `"info:\n  title: Payments API\n  description:..."`.
    - Yaml-line-aware shows `info.title: Payments API` — more readable but requires PyYAML to parse the whole file first to build the key path.

---

## 8. Recommended Approach (synthesis)

Pulling the above together into the minimum viable coherent design:

### Design choices

1. **Frontmatter format: Option A (hash-style `#---` block)** with an OPT-IN invariant. A yaml file without a `#---` block is NOT a neuron. This eliminates the `kluris.yml` risk structurally (kluris.yml has no `#---` block, so even without SKIP_FILES updates it would be excluded) and makes the feature explicit rather than magical.

2. **Node typing: Option Y (`type: 'neuron'` + `file_type: 'yaml'`)** — minimizes JS touch points, preserves filter idioms.

3. **Color: single fixed `#f8c76d` amber** — small diff, clear visual distinction.

4. **Writing strategy: hash-block string manipulation, not PyYAML round-trip.** Treat the `#---` block as text to prepend/update, leave the yaml body untouched byte-for-byte. No comment loss.

5. **Single shared `SKIP_FILES_ALL` constant** in `kluris.core.linker`, imported by maps.py, mri.py, and cli.py. Include `kluris.yml` explicitly. This is a refactor, but it's bounded (5 edit sites → 1 edit site + 4 imports) and closes a long-standing consistency smell.

6. **`check_frontmatter` contract for yaml: require `parent`, `created`, `updated` if the `#---` block is present.** If the file has no block, it's not a neuron at all (opt-out) and is invisible to all scanners. This halves the validation surface and aligns with the "opt-in" principle.

7. **Search snippet: reuse markdown snippet logic** — simple, consistent, adequate. Leave yaml-line-aware snippets as a future enhancement.

8. **No new `kluris neuron` templates for yaml** — yaml neuron creation stays with the agent's `Write` tool, kluris just indexes. This keeps the CLI surface small.

9. **Agent SKILL.md: add a section explicitly describing yaml neuron authoring** — including the `#---` block template the agent must emit when writing openapi.yml (or similar).

### Scope cut

Features to explicitly defer to a later spec:
- JSON schema / k8s manifest neurons — same mechanism but deferred.
- Yaml-line-aware search snippets.
- Yaml neuron templates in `NEURON_TEMPLATES`.
- `kluris dream` validating yaml syntactic correctness (e.g., "this openapi file doesn't match the 3.1 schema") — too ambitious.
- MRI modal view of yaml content with syntax highlighting.

### Implementation order (for the TDD cycle)

1. **Tests first:** the yaml-neurons test file with failing tests for `read_frontmatter` on hash-style yaml.
2. **Core frontmatter:** extend `kluris.core.frontmatter.read_frontmatter` to detect file extension and dispatch to hash-style parser for `.yml`/`.yaml`.
3. **Core frontmatter writer:** extend `write_frontmatter` and `update_frontmatter` to use hash-style output for yaml files.
4. **Centralize skip files:** create `SKIP_FILES_ALL` in linker.py, import in maps.py + mri.py + cli.py. Add `kluris.yml`.
5. **Linker:** update `_all_md_files` → `_all_neuron_files` (rename) and `_neuron_files` to return `.md` + `.yml` + `.yaml`.
6. **Maps:** update `_get_neurons` to include yaml. Verify `generate_map_md` output.
7. **Search:** no change needed — inherits via `_neuron_files`.
8. **Wake-up collectors:** update rglob pattern and skip logic. Add `file_type` to `recent[]`.
9. **Dream (`_sync_brain_state`):** update rglob pattern. Verify date updates work for yaml.
10. **MRI `build_graph`:** dispatch file_type, add yaml nodes. Update JS `colorForNode` and modal regex.
11. **`kluris neuron` command:** allow `.yml`/`.yaml` filenames, generate a stub with hash-style frontmatter.
12. **SKILL.md updates:** add yaml-awareness section. Update `test_agents.py`.
13. **Regression tests:** verify every walker excludes `kluris.yml`.

---

## Research Completeness Checklist

- [x] Project manifest (`pyproject.toml`) and lock file checked (no lock file — Python uses env-installed packages; `pyproject.toml` is authoritative)
- [x] Directory structure mapped via `src/kluris/` listing
- [x] 15+ relevant files read in detail (`frontmatter.py`, `maps.py`, `linker.py`, `search.py`, `mri.py` (key sections), `cli.py` (key sections), `agents.py`, `brain.py`, `config.py`, `test_mri.py`, `test_linker.py`, `test_maps.py`, `test_search.py`, `test_wake_up.py`, `test_dream.py`, `conftest.py`, `pyproject.toml`)
- [x] Dependency chain traced for `_neuron_files` → search, status, validators; `_all_md_files` → linker walkers; `build_graph` → mri.py HTML
- [x] Test coverage assessed (290 tests, 28 files, 90%+ coverage)
- [x] 4+ web searches conducted (OpenAPI 3.1 extensions, YAML comment-block frontmatter, Obsidian non-md backlinks, python-frontmatter customization)
- [x] Library docs checked (`python-frontmatter` default_handlers.py via GitHub, OpenAPI 3.1 spec overview)
- [x] Library comparison done for frontmatter options (4 alternatives in section 4.1)
- [x] Security implications considered (yaml.safe_load, path traversal, kluris.yml pickup)
- [x] Risk assessment completed (section 6)
- [x] Test framework and runner identified (pytest + CliRunner + tmp_path)
- [x] Test directory structure mapped (`tests/` flat, 28 files)
- [x] 5+ existing test files read (test_mri.py, test_linker.py, test_maps.py, test_search.py, test_wake_up.py, test_dream.py, test_agents.py, conftest.py)
- [x] Mocking patterns analyzed (monkeypatch for env, CliRunner for commands, real git subprocess, tmp_path for disk)
- [x] Docker/testcontainers availability checked (neither used — pure pytest)
- [x] Coverage tooling assessed (pytest-cov 5.0+, no gate in CI that I found)
- [x] Mutation testing tools checked (not configured)
- [x] CI test gates reviewed (PR-only, cross-platform matrix, no coverage threshold)
- [x] Stack-specific testing best practices researched (kluris uses real I/O via tmp_path per the "no live disk" constraint — tests must follow the same pattern)

---

### Files examined (absolute paths)

Source files:
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/pyproject.toml`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/AGENTS.md`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/CLAUDE.md`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/cli.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/frontmatter.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/maps.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/linker.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/search.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/mri.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/brain.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/src/kluris/core/agents.py`

Test files:
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/conftest.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_mri.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_linker.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_maps.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_search.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_wake_up.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_dream.py`
- `/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/tests/test_mri_cmd.py`

### External sources

- [OpenAPI Extensions — Swagger Docs](https://swagger.io/docs/specification/v3_0/openapi-extensions/)
- [OpenAPI Specification v3.1.0](https://spec.openapis.org/oas/v3.1.0.html)
- [frontmatter-format (jlevy) — YAML metadata convention for any file](https://github.com/jlevy/frontmatter-format)
- [python-frontmatter — default_handlers.py source](https://github.com/eyeseast/python-frontmatter/blob/main/frontmatter/default_handlers.py)
- [python-frontmatter customization docs](https://python-frontmatter.readthedocs.io/en/latest/handlers.html)
- [Obsidian forum — Showing backlinks to non-markdown files](https://forum.obsidian.md/t/showing-backlinks-to-non-markdown-files/18776)
- [Obsidian forum — Is it possible to register file links from a non-Markdown file?](https://forum.obsidian.md/t/is-it-possible-to-register-file-links-from-a-non-markdown-file/78883)
- [YAML Frontmatter Explained — Nat Thompson](https://natthompson.com/blog/yaml-frontmatter-explained)