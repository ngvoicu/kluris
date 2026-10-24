# SPEC.html Format Reference

The complete format specification for HTML spec documents. Use this as the canonical reference when creating or editing specs. The empty template is in `html-template.html`; surgical edit operations are in `edit-recipes.md`.

## File layout

Every project that uses this skill has a `.specs/` directory at the project root:

```
.specs/
├── assets/
│   ├── spec-styles.css        # Refreshed on every forge
│   └── spec-runtime.js        # Refreshed on every forge
├── registry.md                # Markdown table — denormalized spec index
└── <spec-id>/
    ├── SPEC.html              # The spec
    ├── research-01.md         # Research notes (stay markdown)
    ├── interview-01.md        # Interview notes (stay markdown)
    └── artifacts/             # Optional: AI-tool scratch (test logs,
                               #   attempt dumps). Never authoritative.
```

The `.specs/assets/` directory is shared by every spec. AI never hand-edits these — they are the design system, copied from the skill's `assets/` on every forge so existing projects pick up runtime fixes. Each `SPEC.html` references them via relative paths: `../assets/spec-styles.css` and `../assets/spec-runtime.js`.

The per-spec `artifacts/` subdirectory is **optional**: only create it when the AI tool needs to persist scratch files (e.g., test-run logs the tool can't carry across turns in memory). Files inside `artifacts/` are never read back as authoritative — the spec's Decision Log, research-/interview notes, and (for TDD) the TDD Log section inside `SPEC.html` are the durable record. Don't write scratch files anywhere else under `.specs/<id>/`.

`registry.md` stays as a markdown table — the HTML rendering provides no benefit at the index level, and a markdown table parses easily.

## Source-of-truth split

To avoid drift, every piece of state lives in exactly one place:

| Concern | Lives in | Format |
|---------|----------|--------|
| Identity (id, title, status, dates, priority, tags, mockup-fidelity) | `<script type="application/json" id="spec-meta">` in `<head>` | JSON, single line, canonical key order (logical, not alphabetical) |
| Task lifecycle | `data-status` on `<li class="task">` | `pending` \| `completed` \| `blocked` |
| Phase lifecycle | `data-status` on `<details class="phase">` | `pending` \| `in-progress` \| `completed` \| `blocked` |
| Acceptance lifecycle | `data-status` on `<li class="ac-item">` | `pending` \| `completed` |
| Progress counts ("3/12", "33%", "Phase 2 of 4") | Never stored | Derived at render time by `spec-runtime.js` |

**Consequence:** updating a task is one edit — change `data-status="pending"` → `data-status="completed"`. Every progress display on the page recomputes automatically (the runtime's `deriveProgress()` function listens for `data-status` mutations).

## Metadata JSON

The `<script type="application/json" id="spec-meta">` block in `<head>` is the canonical identity blob. Fields:

| Field | Required | Type | Description |
|-------|----------|------|-------------|
| `id` | Yes | string | URL-safe slug, lowercase-hyphenated |
| `title` | Yes | string | Human-readable title |
| `status` | Yes | string | `active` \| `paused` \| `completed` \| `archived` |
| `created` | Yes | string | ISO date `YYYY-MM-DD` |
| `updated` | Yes | string | ISO date `YYYY-MM-DD` |
| `priority` | No | string | `high` \| `medium` \| `low` (default `medium`) |
| `tags` | No | array | Lowercase strings |
| `mockup-fidelity` | No | string | `wireframe` \| `hi-fi` \| `none` — selected during forge when UI is in scope |

Single-line JSON keeps git diffs minimal. **Canonical key order** (always use this order — do not re-sort alphabetically): `id`, `title`, `status`, `created`, `updated`, `priority`, `tags`, `mockup-fidelity`. Logical grouping, not alphabetical.

## Region sentinels

Every top-level section is wrapped in matching comments:

```html
<!-- region:NAME -->
<section ...>...</section>
<!-- endregion:NAME -->
```

The 12 canonical region names: `meta`, `toc`, `header`, `overview`, `acceptance`, `architecture`, `libraries`, `phases`, `code`, `mockups` (optional), `decisions`, `deviations`. TDD specs add `testing` (after architecture) and `tdd-log` (before deviations).

Sentinels are non-negotiable anchors:
- AI tools use them to locate a section for editing without scanning the whole file
- The validate recipe checks every opener has a matching closer
- Adding a new region requires both opener AND closer

## Section structure

### 1. Header (`region:header`)

`<header class="spec-header">` containing: title, ID, status chips, meta dl (Created / Updated / Tags / Mockup fidelity), and a scorecard with four cells (Tasks / Phases / Acceptance / Blockers). The scorecard values are `<span data-progress-target="...">` — derived at render time.

### 2. Overview (`region:overview`)

Single `<section class="section">` with a `<p>` (2-4 sentences). What is being built and why.

### 3. Acceptance Criteria (`region:acceptance`)

`<ul class="ac-list">` with `<li class="ac-item" id="ac-N" data-ac="N" data-status="...">` rows. Each row has a `.ac-check` span and a `.ac-text` span. To flag unresolved questions, prepend the text with `<span class="ac-flag">Needs clarification</span>`.

Specific, testable conditions. Check them off during implementation by changing `data-status` to `completed`.

### 4. Architecture (`region:architecture`)

One `<section class="section">` containing one or more `<figure class="diagram">` blocks. Each diagram is a `<pre class="mermaid">` with a `.diagram__label` caption. Supported Mermaid types: `flowchart`, `sequenceDiagram`, `erDiagram`, `stateDiagram-v2`, `journey`, `timeline`, `gantt`, `block-beta`, `architecture-beta`, `c4Context`. Mermaid is the default for every diagram type — no ASCII required.

### 5. Library Choices (`region:libraries`)

`<table class="table">` with columns: Need / Library / Version / Alternatives / Rationale. Library names in `<code>` tags.

### 6. Phases & Tasks (`region:phases`)

A `<div class="phases">` containing one `<details class="phase" open data-phase="N" data-status="...">` per phase. Each phase has:
- A `<summary class="phase__header">` with the phase index, title, status pill, and progress span
- A `<div class="phase__body">` with an optional `<span class="progress progress--strip" data-progress-bar="phase-N">` and a `<ul class="task-list">`

**Tasks** are `<li class="task" id="task-CODE" data-task="CODE" data-status="...">`. They contain:
- `<span class="task__check">` — visual checkbox, styled from `data-status`
- `<span class="task__code">` — the task code (e.g., `INV-01`)
- `<span class="task__text">` — the description
- Optional `<span class="task__tags">` with `<span class="task__tag">` chips

**Task codes** use `<PREFIX>-<NN>`:
- Prefix: 2-4 letter uppercase abbreviation of the spec ID (`user-auth-system` → `AUTH`)
- Number: two-digit, auto-increments across ALL phases starting at 01 (not per-phase)

**All phases are `open` by default.** Users can collapse individual phases. There is no "current task" marker — the first task with `data-status="pending"` is implicitly current.

### 7. Code Previews (`region:code`)

`<figure class="code-diff" data-file="..." data-language="...">` blocks. Each has:
- `<figcaption>` with filename and diff stats
- `<pre class="language-diff-LANG diff-highlight">` containing the unified diff

PrismJS `diff-highlight` plugin provides simultaneous red/green line backgrounds AND syntax highlighting for the underlying language.

**Coverage rule:** Code Previews are **expected on every feature spec**, not optional. Include one canonical figure per category — new interface/contract, new data model or schema, non-trivial business logic, before→after of significant refactors, the shape of each new test pattern. Skip boilerplate, repeated patterns, and codegen output. Typical sizing: 2-4 previews for a small spec, 5-10 for medium, 10-20 for large (API + DB + UI). See SKILL.md for the full selection rules.

**Unified vs split:** Unified is the default (one `<pre>`). For changes >30 lines or spanning multiple files / non-contiguous hunks, add `data-view="split"` to the `<figure>` to render side-by-side.

### 8. UI Mockups (`region:mockups`)

`<figure class="mockup mockup--wireframe">` or `<figure class="mockup mockup--hifi">`. Each has:
- Optional `<figcaption>` describing the screen + state
- Optional `<div class="mockup__chrome">` for browser-frame look (3 dots + URL pill)
- `<div class="mockup__body">` containing the mockup itself

**Wireframe** mockups use `.wf-*` primitives from `spec-styles.css`. Composed patterns in `wireframe-library.md`.

**Hi-fi** mockups use `.ui-*` components from `spec-styles.css`. Composed patterns in `mockup-library.md`. Both are bespoke component classes — no CDN dependency.

**No ASCII art inside `<figure class="mockup">`.** This is a hard rule. No boxes drawn with `+`, `|`, `-`; no pipe-delimited tables; no monospace pseudo-diagrams. If you need a table use `.wf-table` (`style="--cols: N;"`); if cards use `.wf-card`; if a layout isn't covered by the library, compose new structure from `.wf-*` / `.ui-*` primitives. The runtime validator flags ASCII-art `<pre>` blocks inside mockup figures.

**Annotations** (`<div class="wf-annotation" data-points-to="target-id">`) render as labeled callouts with SVG arrows pointing at the referenced element. The arrows are drawn by `spec-runtime.js` after layout; redrawn on resize.

Omit this section entirely when `mockup-fidelity: none`.

### 9. Decision Log (`region:decisions`)

`<table class="log-table">` with columns: Date / Decision / Rationale. Date cells use `<td class="log-table__date">` for monospace styling.

### 10. Deviations (`region:deviations`)

`<table class="log-table">` with columns: Task / Spec said / Actually did / Why. Empty at forge time; filled during implementation when behavior diverges from the spec.

## Edit conventions (AI authoring rules)

1. **Region sentinels** are stable anchors — never delete one without deleting its pair.
2. **Stable IDs** on every phase, task, and AC item (`id="task-INV-03"`, `id="phase-2"`, `id="ac-5"`). IDs are derived from the task code / phase number / AC number.
3. **One attribute per line** on state-bearing elements when the line would otherwise be long:
   ```html
   <li class="task"
       id="task-INV-05"
       data-task="INV-05"
       data-status="pending">
   ```
4. **One element per line** for list rows — tasks, AC items, log table rows, decision rows. Insertions touch one line.
5. **JSON in `<script id="spec-meta">`** stays single-line with canonical key order (logical, not alphabetical). Small edits don't reflow large blocks.
6. **`<details>` for collapsibles** — no JS needed; the `open` attribute controls default state.
7. **Progress strings are NEVER hand-edited.** `<span data-progress-target="...">` holds whatever placeholder text you want; the runtime overwrites it.

## Validation gate

Run the recipe in `validate.md` after every edit. SKILL.md's Update Transaction makes this non-optional — never declare a task complete without it.

## Pause/resume limitation

HTML specs checkpoint state at **task boundaries only**. Mid-task partial state ("I refactored handleSubmit, success path works, debugging 4xx") is not captured. Pause cleanly between tasks. If you need to pause mid-task, finish or split the task first.

## TDD additions

TDD-only sections are documented in the `specmint-tdd-html` skill's `references/spec-format.md`. The additions are:
- Testing Architecture region (after architecture)
- TDD Log region (before deviations) — swimlane rendering
- Task tags include `[TEST-XX-NN]` / `[IMPL-XX-NN]` with `→ satisfies` references
- Phase structure: feature-named phases with alternating TEST-IMPL pairs

This file documents the core variant. Both variants share the same HTML structure for every common section.
