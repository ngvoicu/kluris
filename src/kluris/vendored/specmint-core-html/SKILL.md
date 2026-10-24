---
name: specmint-core-html
description: >
  Persistent spec management for AI coding workflows, producing rich HTML
  SPEC.html files. Use this skill when the user explicitly mentions specs,
  forging, or structured planning: says "forge", "forge a spec", "write a
  spec for X", "create a spec", "plan X as a spec", "resume", "what was I
  working on", "spec list/status/pause/switch/activate", "implement the
  spec", "implement phase N", "implement all phases", "generate openapi",
  or exits plan mode (offer to save as a spec). Also trigger when a `.specs/`
  directory exists at session start. Do NOT trigger on general feature
  requests, coding tasks, or questions that don't mention specs or forging.
license: MIT
metadata:
  author: Gabriel Voicu
  version: "3.0.0"
---

# Spec Mint Core HTML

Turn ephemeral plans into structured, persistent specs built through deep
research and iterative interviews. Specs render as professional HTML
documents — Mermaid diagrams, syntax-highlighted code diffs, UI mockups
(wireframe and hi-fi), status pills, derived progress dashboards — and
live in `.specs/` at the project root. They work with any AI coding tool
that can read and edit HTML files.

Whether `.specs/` is committed is repository policy. Respect `.gitignore`
and the user's preference for tracked vs local-only spec state.

## Critical Invariants

1. **Single-file policy**: Keep this workflow in one `SKILL.md` file.
2. **Canonical paths**:
   - Registry: `.specs/registry.md` (markdown, denormalized index)
   - Shared assets: `.specs/assets/spec-styles.css` + `.specs/assets/spec-runtime.js`
   - Per-spec files: `.specs/<id>/SPEC.html`, `.specs/<id>/research-*.md`,
     `.specs/<id>/interview-*.md`
   - Optional scratch: `.specs/<id>/artifacts/` — any AI-tool transient
     files (test-run logs, attempt dumps, debug traces). Never authoritative.
     Don't write scratch files anywhere else under `.specs/<id>/`.
3. **Authority rule**: The `<script id="spec-meta">` JSON inside `SPEC.html`
   is authoritative for identity. The `data-status` attributes on tasks /
   phases / acceptance criteria are authoritative for lifecycle. The
   registry is a denormalized index for quick lookup.
4. **Active-spec rule**: Target exactly one active spec at a time.
5. **Parser policy**: Use best-effort parsing with clear warnings and repair
   guidance instead of hard failure on malformed rows.
6. **Progress tracking is sacred**: After completing any task, immediately
   update `SPEC.html` (`data-status` swap and any phase transition) AND
   `registry.md` (progress count, date). Run the validate recipe (see
   `references/validate.md`) to confirm the file still parses. Re-read both
   files to verify the edits landed. Never move to the next task without
   updating both files. Never end a session with the registry out of sync
   with the derived progress in `SPEC.html`. This is non-negotiable.

## Session Start

If active-spec context was injected by host tooling, use it directly
instead of reading files. Otherwise, fall back to reading files manually:

1. Read `.specs/registry.md` to check for a spec with `active` status
2. If one exists, briefly mention it:
   "You have an active spec: **User Auth System** (5/12 tasks, Phase 2).
   Say 'resume' to pick up where you left off."
3. Don't force it — the user might want to do something else first

## Coexistence with markdown specs

This skill only manages `.html` specs. If `.specs/<id>/SPEC.md` files exist
(from a markdown-flavored Spec Mint variant), they are not visible to or
operated on by this skill. No auto-conversion, no edits. The user should
use the markdown-flavored variant for those specs.

## Deterministic Edge Cases (Best-Effort)

| Situation | Required behavior |
|-----------|-------------------|
| `.specs/registry.md` missing | If `.specs/` exists, report "No registry yet" and offer to initialize it. If `.specs/` is missing, report "No specs yet" and continue normally. |
| `.specs/assets/` missing or stale when a SPEC.html is being written | Refresh it — copy `spec-styles.css` and `spec-runtime.js` from the skill's bundled `assets/` directory into `.specs/assets/`, **overwriting any existing files**. The runtime ships rendering fixes (Mermaid, diagram fullscreen modal, code highlighting) and must stay in sync with the skill version. |
| Malformed registry row | Skip malformed row, emit warning with row text, continue parsing remaining rows. |
| Multiple `active` rows | Warn user. Pick the row with the newest `Updated` date (or first active row if dates are unavailable) for this run. On next write, normalize to a single active spec. |
| Registry row exists but `.specs/<id>/SPEC.html` missing | Warn and continue. Keep row visible in list/status with `(SPEC.html missing)`. |
| Registry and SPEC conflict | Trust `SPEC.html`, then repair registry values on next write. |
| Validate recipe fails after an edit | Stop. Fix the broken JSON or sentinel pair before continuing. |
| No active spec | List available specs and ask which to activate or resume. |

## Working on a Spec

### Resuming

When the user says "resume", "what was I working on", or similar:

1. Read `.specs/registry.md` — find the spec with `active` status. If none, list specs and ask which to resume
2. Load `.specs/<id>/SPEC.html`
3. Parse progress:
   - Count `<li class="task">` elements grouped by `data-status` per phase
   - Find current phase (first `<details class="phase">` with `data-status="in-progress"`)
   - Find current task (first task in current phase with `data-status="pending"`)
4. Present a compact summary:

   ```
   Resuming: User Auth System
   Progress: 5/12 tasks (Phase 2: OAuth Integration)
   Current: Implement Google OAuth callback handler
   ```

5. Begin working on the current task — don't wait for permission

There is no separate "Resume Context" section in HTML specs. The first
pending task in the current phase is implicitly the next task. Decision
Log and Deviations carry the context history.

### Implementing

**After completing each task, immediately edit `SPEC.html`** to record
progress. Do not wait until the end of a session or until asked — update
the spec as you go. This is sacred (see Critical Invariant #6).

1. Change the task's `data-status="pending"` → `data-status="completed"`
2. When all tasks in a phase are done:
   - Phase `data-status="in-progress"` → `data-status="completed"`
   - Update the phase pill (`pill--in-progress` → `pill--completed`)
   - Next phase `data-status="pending"` → `data-status="in-progress"`
   - Update its pill correspondingly
   - Review Acceptance Criteria — update `data-status` on any that are now satisfied
3. Update the `"updated"` field in the `<script id="spec-meta">` JSON
4. Update the visible `<dd>` for "Updated" in the spec header dl
5. Update progress (`X/Y`) and `updated` date in `.specs/registry.md`

See `references/edit-recipes.md` for the exact before/after snippets for
every operation.

**Update transaction (required order — never skip steps):**
1. Edit `SPEC.html` (task `data-status`, phase transitions if applicable,
   updated date in JSON and visible dl).
2. **Run the validate recipe** from `references/validate.md`. If it doesn't
   print `OK`, fix the broken JSON or sentinel before continuing.
3. Recompute progress directly from `SPEC.html` `data-status` counts.
4. Edit the matching registry row (status, progress, updated date).
5. **Verify**: Re-read both `SPEC.html` and `registry.md` to confirm the
   edits are correct. If the registry progress doesn't match the SPEC.html
   completed-task count, fix it now.
6. If registry update fails, keep `SPEC.html` as source of truth and emit a
   warning with exact repair action for `.specs/registry.md`.

**If you notice you forgot to update after a previous task, stop what
you're doing and update now before continuing.** Stale tracking is the
single most common failure mode — it makes resume unreliable and the
registry useless.

Also:
- If a task is more complex than expected, split it into subtasks
- Log non-obvious technical decisions to the Decision Log
- If implementation diverges from the spec (errors found, better approach
  discovered, assumptions proved wrong), log it in the **Deviations** section

### Pausing

When the user says "pause", switches specs, or a session is ending:

0. If there is no active spec, report that there is nothing to pause and stop.
1. Make sure all completed work is reflected in the spec — every task you
   actually finished has `data-status="completed"`, every phase transition
   is applied.
2. Add any session decisions to the **Decision Log**
3. Change the spec status from `active` to `paused`:
   - In the `<script id="spec-meta">` JSON: `"status":"paused"`
   - In the visible status pill: `pill--in-progress`/`Active` → `pill--pending`/`Paused`
4. Update the `"updated"` date (in JSON + visible dl)
5. Mirror status + date to `.specs/registry.md`
6. Run the validate recipe

**Known limitation: HTML specs checkpoint state at task boundaries only.**
Mid-task partial state (e.g., "I refactored handleSubmit, success path
works, debugging 4xx") is not captured by the spec format. If you need to
pause mid-task, finish the task or split it into subtasks first. The
Decision Log can carry brief in-flight notes but is not designed as a
running scratchpad — keep it to durable decisions.

### Switching Between Specs

1. Validate the target spec ID first. If missing, list available specs.
2. Confirm `.specs/<target-id>/SPEC.html` exists. If not, stop with an error.
3. If target is already active, report and stop.
4. Pause the current active spec if one exists (full pause workflow).
5. Set target status to `active` in JSON metadata and in `.specs/registry.md`.
6. Resume the target spec (full resume workflow).

## Spec Format (HTML)

The detailed format reference lives in `references/spec-format.md`. The
canonical empty template is `references/html-template.html`. Edit recipes
for every common operation are in `references/edit-recipes.md`. Wireframe
and hi-fi mockup component patterns are in `references/wireframe-library.md`
and `references/mockup-library.md`. The post-edit validator is in
`references/validate.md`.

**Key facts about the format:**

- Each `SPEC.html` references `../assets/spec-styles.css` and
  `../assets/spec-runtime.js`. Those two files are shared by every spec
  in the project and are refreshed from the skill's `assets/` on every
  forge (overwrite, not skip-if-present) so existing projects pick up
  runtime fixes.
- Identity (`id`, `title`, `status`, `created`, `updated`, `priority`,
  `tags`, `mockup-fidelity`) lives in a `<script type="application/json"
  id="spec-meta">` blob in `<head>`, single-line JSON, canonical key order
  (`id`, `title`, `status`, `created`, `updated`, `priority`, `tags`,
  `mockup-fidelity` — logical, not alphabetical).
- Lifecycle state (task status, phase status, AC status) lives in
  `data-status` attributes. Values: `pending`, `in-progress`, `completed`,
  `blocked`. AC items use only `pending` / `completed`.
- Progress strings ("3/12 tasks", "Phase 2 of 4", "33%") are **never
  authored** — they are computed at page load by `spec-runtime.js` from
  `data-status` counts. Any `<span data-progress-target="...">` will be
  overwritten on render.
- Every top-level section is wrapped in `<!-- region:NAME -->` /
  `<!-- endregion:NAME -->` sentinels. Use them as anchors when editing.
- Task codes: `<PREFIX>-<NN>` where prefix is a 2-4 letter uppercase
  abbreviation of the spec ID; numbers auto-increment across all phases
  starting at 01.
- All phases render `open` by default (`<details class="phase" open ...>`).
  No "current task" marker — the first pending task in the active phase
  is implicitly current.

## Forging Specs

When asked to forge, plan, spec out, or "write a spec for X", follow the
full forge workflow: setup, research deeply, interview the user, iterate
until clear, then write the spec.

If the environment is in read-only plan mode, do not run forge in that mode.
Ask the user to exit plan mode (Shift+Tab) and start the forge workflow again.

**The forge workflow never produces application code.** Its outputs are only
`.specs/` files: research notes, interview notes, refreshed shared assets,
and the SPEC.html. If the user says "write a spec", that means
write a SPEC.html — not implement the feature. Implementation happens
separately, after the user reviews and approves the spec.

### Step 1: Setup

1. Generate a spec ID from the title (lowercase, hyphenated):
   `"User Auth System"` -> `user-auth-system`
2. **Collision check**: If `.specs/<id>/SPEC.html` already exists or the ID
   appears in `.specs/registry.md`, warn the user and ask:
   - **Resume** the existing spec
   - **Rename** the new spec (suggest `<id>-v2` or ask for a new title)
   - **Archive** the old spec and create a new one in its place
   Do not proceed until the user chooses.
3. Initialize directories:
   ```bash
   mkdir -p .specs/<id> .specs/assets
   ```
4. **Refresh the shared assets.** Copy `spec-styles.css` and
   `spec-runtime.js` from the skill's bundled `assets/` directory into
   `.specs/assets/`, **overwriting any existing files**. The runtime is
   skill-managed and never hand-edited — it ships rendering fixes
   (Mermaid initialization, click-to-fullscreen diagram modal with
   wheel-zoom + drag-pan, PrismJS code highlighting, SVG annotation
   arrows) that must stay in sync with the skill version. Under Kluris that directory is
   `~/.kluris/companions/specmint-core-html/assets/`. These
   files are shared by every spec in the project.
5. If `.specs/registry.md` doesn't exist, initialize it:
   ```markdown
   # Spec Registry

   | ID | Title | Status | Priority | Progress | Updated |
   |----|-------|--------|----------|----------|---------|
   ```

### Step 2: Deep Research

Research is the foundation of a good spec. Be exhaustive — use every available
resource. The goal is to gather enough context that the spec won't need revision
mid-build.

Research runs on two parallel tracks to maximize thoroughness and speed:

#### Track A: Spawn a Research Subagent

**Always spawn a research subagent** for codebase + internet
research, using the brief in `references/researcher.md`. Don't skip this —
the research subagent is purpose-built for exhaustive multi-source analysis
and runs in parallel so it doesn't slow down the workflow.

Spawn it with the Task tool, using the brief in `references/researcher.md`
and providing:
- The user's request (what they want to build/change)
- The spec ID and output path: `.specs/<id>/research-01.md`
- Any Context7 findings you've already gathered (Track B)
- Specific areas to focus on, if known

The research subagent will:
- Map the full project architecture (read manifests, lock files, directory tree)
- Read 15-30 relevant code files and trace dependency chains
- Run 3+ web searches for best practices and current patterns
- Compare 2-4 library candidates for every choice point
- Assess security risks and performance implications
- Produce a structured research document with a completeness checklist

#### Track B: Context7 & Cross-Skill Research (in parallel)

While the research subagent runs, do these yourself — they use MCP tools that
the research subagent doesn't have access to:

- **Context7**: If available (resolve-library-id / query-docs tools), pull
  up-to-date documentation for every key library involved. Check API changes,
  deprecated features, and recommended patterns for the specific versions in
  use. Do this for 2-5 key libraries — the ones central to the feature being
  built.
- **Cross-skill loading**: Load other available skills when relevant:
  - **frontend-design**: For UI-heavy specs — creative, professional design
  - **datasmith-pg**: For database specs — schema design, migrations, indexing
  - **webapp-testing**: For testing strategy — Playwright patterns
  - **vercel-react-best-practices**: For Next.js/React performance
  - Any other relevant skill that's available
- **UI research** (if applicable): Take screenshots, map component hierarchy,
  research modern UI patterns, note accessibility requirements

#### Merging Research

When the research subagent completes, read its output at
`.specs/<id>/research-01.md`. Merge your Context7 and cross-skill findings
into the research notes — either append to the file or keep them in mind
for the interview. The combined research should cover:
architecture, relevant code, tech stack, library comparisons, internet
research, Context7 docs, UI research (if applicable), risk assessment,
and open questions.

### Step 3: Interview Round 1

Present your research findings and ask targeted questions. Your research
should inform specific questions, not generic ones.

1. **Summarize findings** (2-3 paragraphs — not a wall of text)
2. **State assumptions** — "Based on the codebase, I'm assuming we'll use X
   pattern because that's what similar features use. Correct?"
3. **Ask 3-6 targeted questions** that research couldn't answer:
   - Architecture decisions ("New module or extend existing one?")
   - Scope boundaries ("Should this handle X edge case?")
   - Technical choices ("Stick with Library A or try Library B?")
   - User-facing behavior ("What should happen when X fails?")
   - Acceptance criteria ("What does 'done' look like? Any specific
     conditions that must be true when this is complete?")
   - **Mockup fidelity (only if UI work is in scope)** — "Mockups in this
     spec should render as `wireframe` (clean grayscale boxes, structural,
     no design commitment), `hi-fi` (real-looking polished components),
     or `none` (prose + diagrams are enough)?" Record the answer for use
     in Step 5.
4. **Propose a rough approach** and ask for reactions

**STOP after presenting questions.** Wait for the user to answer before
proceeding. Do not answer your own questions, do not assume answers, and do
not continue to Step 4 or Step 5 until the user has responded. The interview
is a conversation — the user's answers shape the spec. If you skip this, the
spec will be based on guesses instead of decisions.

Save to `.specs/<id>/interview-01.md` with: questions asked, user answers,
key decisions, and any new research needed.

### Step 4: Deeper Research + Interview Loop

Based on the user's answers, do another round of research — explore the
specific paths they chose, check feasibility, find potential issues. Save
to `.specs/<id>/research-02.md`.

Then present deeper findings and ask about trade-offs, edge cases,
implementation sequence, and scope refinement. Save each interview round
to `interview-02.md`, `interview-03.md`, etc.

**Repeat research-then-interview until:**
- You have enough clarity to write a spec with no ambiguous tasks
- The user is satisfied with the direction
- Every task can be described concretely (not "figure out X")

Two rounds is typical. Don't rush it — but don't drag it out either.

### Step 5: Write the Spec

Synthesize all research notes, interview answers, and decisions into a
comprehensive `SPEC.html`. **Start from `references/html-template.html`** —
copy it to `.specs/<id>/SPEC.html` and fill in every placeholder. Use
`references/edit-recipes.md` for the exact HTML structure of each common
element (tasks, AC items, diagrams, code-diff figures, mockups, log
rows).

The spec should be thorough and detailed — someone reading it should be
able to implement the feature without guessing. Include:

- **`<script id="spec-meta">` JSON** (id, title, status, created, updated,
  priority, tags, mockup-fidelity)
- **Spec header** — title, status pill, priority chip, dates, tags, scorecard
- **Overview** (2-4 sentences — what's being built and why)
- **Acceptance Criteria** — Testable conditions defining "done", each in a
  `<li class="ac-item" data-status="pending">`. Specific and verifiable,
  not vague. Use `<span class="ac-flag">Needs clarification</span>` for
  unresolved questions.
- **Architecture Diagram(s)** — Mermaid `flowchart`, `sequenceDiagram`,
  `erDiagram`, `stateDiagram-v2`, etc. Every non-trivial spec should have
  at least one diagram. Mermaid covers every diagram type we need.
- **Library Choices** — `<table class="table">` comparing evaluated
  libraries with the selected pick and rationale. Include version numbers.
- **Phases & Tasks** — 3-6 phases is typical. Each phase is a
  `<details class="phase" open>` with status pill, progress strip, and a
  `<ul class="task-list">`. Tasks are concrete and actionable (file paths,
  function names, expected behavior).
- **Code Previews** — `<figure class="code-diff">` blocks showing the
  meaningful code deltas the spec will produce. **Expected on every
  feature spec.** Skip only when the spec genuinely produces no code
  (pure research / docs).

  **Include one canonical figure per category** (not every instance):
  - The signature/contract of each new public interface, exported
    function, class, or API endpoint
  - The shape of each new data model, schema, or migration
  - Non-trivial business logic — algorithm, validation, transformation
    — where the body itself matters
  - The "before → after" of each refactor or significant signature
    change that captures a design decision
  - One canonical test per new test pattern (not every test body)

  **Skip:** boilerplate (imports, scaffolding, route registration
  already implied by phases); repeated identical patterns (show one,
  note the rest follow); codegen output / formatted JSON / build
  artifacts.

  **Sizing.** Small spec (1-2 phases, one module): 2-4 previews. Medium
  (3-5 phases): 5-10. Large (6+ phases across API + DB + UI): 10-20.
  If a spec produces hundreds of changes but only has 1-2 previews, you
  missed the point — surface the most important deltas.

  Unified diff by default; `data-view="split"` for changes >30 lines,
  multi-hunk, or where the before/after comparison itself is the point.
- **UI Mockups** — One or more `<figure class="mockup">` blocks, using
  hi-fi (`mockup--hifi`) by default. Reserve wireframe
  (`mockup--wireframe`) for the rare case where the spec is
  intentionally structural-only with no real labels yet. Omit
  entirely if `mockup-fidelity: none`.

  **Default to hi-fi** — almost every spec has concrete copy (button
  text, column headers, row data, status badges, placeholder hints).
  The moment you have real text to show, use the `.ui-*` components
  in `references/mockup-library.md`. They render as real-looking UI
  (real buttons, real inputs, real tables, real tags), so users can
  evaluate the design.

  **The wireframe library (`.wf-*`) is for empty skeletons only.**
  `.wf-heading`, `.wf-text`, `.wf-pill`, `.wf-input` are skeleton
  bars; they render correctly only when the tag is empty. If you
  have real content (a heading like "Sites", a label like "AERE1",
  a status like "active"), you are authoring hi-fi — switch to
  `.ui-*` for that block. Mixing real text inside `.wf-text` /
  `.wf-heading` is the #1 cause of "ugly grey blob behind text"
  bug reports.

  Read `references/mockup-library.md` (`.ui-*` patterns: Login form,
  Dashboard, Data table, Empty state, Modal dialog, Toast, Form with
  validation, Multi-step wizard, Alert+tabs, Settings panel, Card
  grid) before authoring. `references/wireframe-library.md` is the
  fallback for structural-only sketches.

  **Never use ASCII art inside `<figure class="mockup">`** — no boxes
  drawn with `+`, `|`, `-`; no pipe-delimited tables; no monospace
  pseudo-diagrams. If you need a grid, use `.ui-table` (real
  `<table><thead><tbody>` markup); if you need cards, use `.ui-card`.
  If a layout you need isn't in the library, compose new structure
  from the primitives — do **not** fall back to ASCII. ASCII inside
  mockups is treated as a render bug by the validator and surfaces
  as a warning.
- **Decision Log** — Empty initially; populated as work progresses.
- **Deviations** — Empty initially; populated during implementation when
  behavior diverges from the spec.

**Diagram guidelines:**
- Mermaid is the recommended path for every diagram type. The skill loads
  Mermaid v11 from a CDN automatically when a `<pre class="mermaid">`
  block exists on the page.
- Use the right diagram type: `flowchart` for system flows, `sequenceDiagram`
  for request lifecycles, `erDiagram` for data models, `stateDiagram-v2`
  for state machines, `timeline`, `journey`, `gantt`, `block-beta`,
  `architecture-beta`, `c4Context`, `treemap`.
- Include at least one diagram per spec (architecture, data flow, or state).

**Mermaid authoring rules — read every time, do not skim.**

Every parse error this skill has ever produced has the same root cause:
the author judged a label or alias "didn't need quoting" and was wrong.
The rules below are written to remove that judgment call entirely.

1. **No HTML entities inside `<pre class="mermaid">`.** Write `A --> B`,
   `A & B`, `"foo"` — not `A --&gt; B`, `A &amp; B`, `&quot;foo&quot;`.
   Mermaid parses pre-content as plain text; entity strings are read
   literally and break parsing.

2. **ALWAYS quote participant aliases** in `sequenceDiagram` — every
   single one, even if it looks "safe". The cost of an extra `"…"` is
   zero; the cost of a parse error is a broken render. Templates:
   ```
   participant U as "User"
   participant API as "Backend API (port 8080)"
   participant DB as "PostgreSQL"
   ```
   ❌ BAD — `participant API as Backend API (port 8080)` (unquoted parens)
   ✅ GOOD — `participant API as "Backend API (port 8080)"`

3. **ALWAYS quote message labels** in `sequenceDiagram` — every single
   arrow's label. The lexer treats `:` `(` `)` `,` `;` as syntax tokens
   in the unquoted form. Templates:
   ```
   U->>API: "POST /endpoint (body)"
   API->>DB: "SELECT message, site_macro, email_template"
   DB-->>API: "SQL stream (application/sql;charset=UTF-8)"
   ```
   ❌ BAD — `U->>API: POST /endpoint (body)` (unquoted parens)
   ❌ BAD — `API->>DB: SELECT col1, col2, col3` (unquoted commas)
   ✅ GOOD — both wrapped in double quotes

4. **ALWAYS quote `flowchart` node labels that aren't pure identifiers.**
   The square / curly brackets define the node shape; the **content**
   between them must be quoted whenever it contains anything outside
   `[A-Za-z0-9 _.-]`. Yes, that includes `/`, `(`, `)`, `,`, `;`, `:`,
   `+`, `=`, `<`, `>`, `[`, `]`, `—` (em dash), and `<br/>`.

   The CLI validate gate (`references/validate.md`) **rejects** any
   `<pre class="mermaid">` block with an unquoted label that uses
   forbidden chars. It runs after every edit; you cannot ship past it.

   Common offenders the validator has caught:
   - `API[/api/foo/]` — slashes (and the trapezoid `[/…/]` shape eats
     the inner slashes). Fix: `API["/api/foo"]`.
   - `PAGE[app/(app)/page.tsx]` — slash + parens. Fix:
     `PAGE["app/(app)/page.tsx"]`.
   - `DTO[Foo[]<br/>bar]` — `[]` + `<br/>`. Fix: `DTO["Foo[]<br/>bar"]`.
   - `CAPSULE[group/capsule]` — slash. Fix: `CAPSULE["group/capsule"]`.
   - `subgraph INS[components/insight/heatmap/]` — slashes inside a
     subgraph title. Fix: `subgraph INS["components/insight/heatmap/"]`.
   - `BEFORE[Before — selector machinery]` — em dash. Fix:
     `BEFORE["Before — selector machinery"]`.

   The cylinder shape `NAME[(...)]` is **not** an exception: the inner
   text still needs quotes — `NAME[("text with /")]` not
   `NAME[(text with /)]`. Same applies to `{(text)}`, `>(text)<`, etc.

   ```
   A["My Node (with parens)"]
   B["Process: do thing"]
   C{"Decision: any X?"}
   D[("Database — primary store")]
   ```

5. **Keep IDs identifier-safe** — letters, digits, underscores only.
   `participant API as "..."` is fine; `participant API.v2 as "..."`
   is not. Use `_` where you'd otherwise reach for `.` or `-`.

6. **One statement per line.** No `;` chaining edges. Terminate every
   arrow with a target — bare `A -->` at end of line is a syntax error.

7. **Self-check loop after every Mermaid block.** Before moving on,
   re-read every `participant` line and every arrow line. If any of
   them contains `(`, `)`, `,`, `:`, `;`, or `/` outside a quoted span,
   wrap it now — do not wait for the validator to catch it.

8. **Validator-clean is required before the spec is "done".** After
   writing the spec, open the rendered `SPEC.html` in a browser, run
   `specmintValidate()` in the page console, and fix every error
   reported under area `mermaid`. The original source is preserved on
   each failing `<pre class="mermaid">` via `data-mermaid-source` —
   read it, fix it, reload, re-run. A spec with any
   `figure.diagram--error` is not done.

### Canonical sequenceDiagram template (copy this, then customize)

```
sequenceDiagram
    participant U as "User"
    participant FE as "Frontend (React, /sites + /setup)"
    participant API as "Backend API"
    participant DB as "PostgreSQL"

    U->>FE: "click submit"
    FE->>API: "POST /api/sites (body)"
    API->>DB: "INSERT INTO sites"
    DB-->>API: "201 Created"
    API-->>FE: "200 OK"
    FE-->>U: "show success toast"
```

Every alias and every label is in double quotes. Do not deviate.

### Canonical flowchart template

```
flowchart LR
    A["Start"] --> B{"Has token?"}
    B -- yes --> C["Call /api (with token)"]
    B -- no  --> D["Redirect /login"]
    C --> E["200 OK"]
```

**Solution quality standards:**
- Proposed solutions should be simple, maintainable, and professional
- Prefer clean, modern patterns over clever hacks
- Choose the best available libraries — compare options, pick the most mature
  and well-maintained
- UI designs should be creative, sleek, and professional — not generic
- Code architecture should be innovative where appropriate but always clean

**Coherence and logic review (mandatory before presenting):**
1. Read through the entire spec as a whole — does it tell a coherent story?
2. Check that phases are in logical dependency order — no phase requires
   work from a later phase
3. Verify every task is concrete and actionable (file paths, function names)
4. Confirm the architecture diagram matches the task descriptions
5. Verify library choices are consistent throughout (no conflicting picks)
6. Ensure the overview accurately summarizes what the phases will deliver
7. Look for gaps — is there anything the implementation would need that
   isn't covered by a task?
8. Verify acceptance criteria are specific, testable, and cover the key
   behaviors the user expects
9. **Placeholder check**: Search the spec for "TBD", "TODO", "placeholder",
   "TBC", "to be determined", "will be decided", "figure out" — replace
   every instance with a concrete decision or remove the section
10. **Internal consistency**: Verify task count in scorecard matches actual
    tasks, all task code references are valid, library versions in different
    sections don't conflict
11. **Scope check**: Compare the spec against the interview answers — does
    it deliver what was discussed? Nothing more, nothing less?
12. **Ambiguity check**: For each task, ask "could an implementer complete
    this without asking me a question?" If no, add detail until yes.
13. **Run the validate recipe** — confirm the file parses cleanly.

Save to `.specs/<id>/SPEC.html`. Update `.specs/registry.md` — set
status to `active`. Run the validate recipe one more time before
presenting.

**Present the spec and wait for approval.** Show the user the complete
spec (or open it in a browser if applicable) and ask: "Does this look
right? Want to adjust anything before we start?" Do not begin implementing
until the user explicitly approves. The forge workflow produces only spec
files (SPEC.html, research-*.md, interview-*.md) — never application code.
Implementation starts only after the user approves the spec and says to
proceed.

**Phase/task guidelines:**
- Mark Phase 1 with `data-status="in-progress"` and the matching pill, the
  rest with `data-status="pending"`
- Every phase renders `open` — `<details class="phase" open ...>`

## Implementing a Spec

When the user says "implement the spec", "implement phase N", "implement all
phases", or similar:

### Scope Detection

Parse the user's request to determine scope:
- **"implement the spec"** or **"implement"** → Start from the first task
  with `data-status="pending"` in the in-progress phase and work forward
- **"implement phase N"** or **"implement phase <name>"** → Implement all
  tasks in that specific phase
- **"implement all phases"** or **"implement everything"** → Implement all
  remaining pending tasks across all phases, in order

### Implementation Flow

1. Read `.specs/registry.md` to find the active spec
2. Load `.specs/<id>/SPEC.html` and parse phases/tasks (look at `data-status`
   on each `<li class="task">`)
3. Identify the target tasks based on scope
4. For each task, in order:
   a. Implement it — write the actual code
   b. Edit `SPEC.html`: change the task's `data-status="pending"` →
      `data-status="completed"` (see `references/edit-recipes.md` for the
      exact swap)
   c. When all tasks in a phase complete:
      - Update phase `data-status` and pill class
      - Promote next phase to `in-progress`
   d. Update `"updated"` date in JSON + visible dl
   e. Update progress and date in `.specs/registry.md`
   f. **Run the validate recipe** from `references/validate.md`
5. Log any new decisions to the Decision Log
6. If implementation diverges from the spec, log it in the Deviations section
7. **Phase review**: When all tasks in a phase are done, review before
   moving on — re-read the phase's tasks and acceptance criteria, verify
   each task's implementation matches what was specified, and check for
   missing edge cases or spec drift. Fix issues before marking the phase
   complete. Log findings in the Decision Log.
8. If blocked on a task:
   - Set `data-status="blocked"` on the task
   - Record blocker details in the Decision Log (or add a Deviations row)
   - Set the phase to `blocked` only when the whole phase is blocked
   - Continue with another unblocked task only if sequencing allows it

### Testing During Implementation

When implementing, follow the testing strategy from the spec:
- Write tests as specified in the testing tasks
- Run tests after each task to verify correctness
- If a test task exists for the feature task you just completed, implement
  the test task immediately after

### Verification Gate

Before reporting any phase or spec as complete, provide evidence:
- Run the relevant test suite via the project's test runner
- Show the actual command and output — not a summary, not "tests pass"
- If tests fail, fix the issues before claiming completion
- Never use language like "should pass", "probably works", or "seems correct"

Evidence first, then assertions. This applies at task, phase, and spec
completion boundaries.

### Completion

When all tasks are done:
- **Run the full test suite** and show the output (verification gate)
- Verify all Acceptance Criteria have `data-status="completed"`. If any
  remain pending, report which ones and ask the user before marking the
  spec complete.
- Set all phase `data-status="completed"`
- Set spec status to `completed` in JSON metadata and registry
- Update the `updated` date
- Run the validate recipe
- Present a summary of what was implemented
- Suggest next spec to activate if any are paused

## Generating OpenAPI Docs

When the user says "generate openapi", "update api docs", or similar:

1. Scan the codebase for API routes/handlers/controllers and request/response
   schemas.
2. Infer auth/security schemes and endpoint grouping (tags).
3. Write `.openapi/openapi.yaml` (OpenAPI 3.1.1) with:
   - `operationId` for every operation
   - Reusable `components/schemas` and `$ref` usage
   - Accurate parameters, request bodies, responses, and security
4. Write one endpoint doc per route under `.openapi/endpoints/` using
   `{method}-{path-slug}.md` names (e.g., `get-api-users-id.md`).
5. Preserve manual additions in existing `.openapi/` files when updating.
6. Report totals: endpoints, schemas, security schemes, and manual-review
   candidates.

(OpenAPI output stays as YAML + markdown — only the spec files are HTML.)

## Before Session Ends

If the session is ending:

1. Pause the active spec (run full pause workflow)
2. Make sure every completed task has the right `data-status`
3. Confirm to the user that progress was saved

## Directory Layout

All state lives in `.specs/` at the project root:

```
.specs/
├── assets/
│   ├── spec-styles.css       # Shared design system (refreshed every forge)
│   └── spec-runtime.js       # Shared runtime (refreshed every forge)
├── registry.md               # Markdown table — denormalized index
└── <spec-id>/
    ├── SPEC.html             # The spec document
    ├── research-01.md        # Deep research findings (markdown)
    ├── interview-01.md       # Interview notes (markdown)
    ├── artifacts/            # Optional: AI-tool scratch (test logs,
    │                         #   attempt dumps). Never authoritative.
    └── ...
```

`.specs/<spec-id>/artifacts/` is optional: only create it when the AI tool
needs to persist scratch (e.g., test-run logs it can't keep in conversation
memory). Most runs have no artifacts. The directory is never read back as
authoritative state — the spec's TDD Log section, Decision Log, and
research-/interview notes are the durable record.

## Registry Format

`.specs/registry.md` is a simple markdown table:

```markdown
# Spec Registry

| ID | Title | Status | Priority | Progress | Updated |
|----|-------|--------|----------|----------|---------|
| user-auth-system | User Auth System | active | high | 5/12 | 2026-02-10 |
| api-refactor | API Refactoring | paused | medium | 2/8 | 2026-02-09 |
```

**SPEC.html `<script id="spec-meta">` is authoritative.** The registry is
a denormalized index for quick lookups. Always update both together —
when you change status or progress in the SPEC.html, immediately mirror
those changes in the registry. If they ever conflict, SPEC.html wins.

## Listing Specs

Read `.specs/registry.md` and present specs grouped by status:

```
Active:
  -> user-auth-system: User Auth System (5/12 tasks, Phase 2)

Paused:
  || api-refactor: API Refactoring (2/8 tasks, Phase 1)

Completed:
  + ci-pipeline: CI Pipeline Setup (8/8 tasks)
```

## Canonical Output Templates

Use these concise formats consistently:

**Resume**
```
Resuming: <Title> (<id>)
Progress: <done>/<total> tasks
Phase: <phase name>
Current: <task text>
```

**List**
```
Active:
  -> <id>: <Title> (<done>/<total>, <phase>) [<priority>]
Paused:
  || <id>: <Title> (<done>/<total>, <phase>) [<priority>]
Completed:
  + <id>: <Title> (<done>/<total>) [<priority>]
```

**Status**
```
<Title> [<status>, <priority>]
Created: <date> | Updated: <date>
Phase <n>: <name> [<marker>]
Progress: <done>/<total> (<pct>%)
Current: <task text or none>
```

## Archiving a Spec

Archive completed specs to keep the registry clean:

1. Set status to `archived` in JSON metadata and registry
2. Research files (research-*.md, interview-*.md) in `.specs/<id>/` can
   optionally be deleted (the SPEC.html has all the decisions and context)

Specs can be archived from `completed` or `paused` status. To reactivate
an archived spec, set its status back to `active`.

## Deleting a Spec

To remove a spec entirely:

1. Delete `.specs/<id>/` (contains SPEC.html, research notes, interviews)
2. Remove the row from `.specs/registry.md`

This is irreversible — consider archiving instead if you might need it later.

## Kluris Companion Compatibility

Kluris embeds this playbook as the `specmint-core-html` companion. Read and
follow it directly from `~/.kluris/companions/specmint-core-html/SKILL.md`.
Paths in this file that start with `references/` or `assets/` resolve
against `~/.kluris/companions/specmint-core-html/`. `.specs/...` paths, and
the relative `../assets/` links inside a `SPEC.html`, are project paths. Do
not ask the user to install specmint separately. `SPEC.html` stays plain
HTML with a JSON metadata blob: open it in any browser, edit it in any text
editor, and keep it diff-friendly with the conventions in
`references/spec-format.md`.

## Behavioral Notes

**Be proactive about spec management.** If you notice the user has been
working for a while and made progress, update the spec without being asked.
If a session is ending, offer to pause and save progress.

**Specs should evolve.** It's fine to add tasks, reorder phases, or split a
phase into two as understanding deepens. Specs aren't contracts — they're
living documents that adapt as you learn more about the problem.

**The Decision Log matters.** When the user makes a non-obvious technical
choice (library selection, architecture pattern, API design), log it with
the rationale. Future-you resuming this spec will thank present-you.

**Don't over-structure.** A spec with 3 phases and 15 tasks is useful. A
spec with 12 phases and 80 tasks is a project plan, not a coding spec.
Keep it lean enough to parse and act on in one read.

**Respect the user's flow.** Don't interrupt deep coding work to update
the spec. Batch updates for natural pauses — task completion, phase
transitions, or session boundaries.

**Never hand-author derived progress strings.** Any `<span data-progress-target="...">`
content gets overwritten by `spec-runtime.js`. Leave a placeholder like
"0/0" and let the runtime fill it in.
