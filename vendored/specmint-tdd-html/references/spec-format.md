# SPEC.html Format Reference (TDD)

The complete format specification for TDD HTML spec documents. Every TDD spec follows a strict test-before-implementation structure: write tests first, then implement to satisfy them. Tasks within a phase alternate TEST → IMPL, each pair forming one red-green-refactor cycle.

The empty template is in `html-template.html`; surgical edit operations are in `edit-recipes.md`. Wireframe and hi-fi mockup patterns are in `wireframe-library.md` / `mockup-library.md`. The post-edit validator is in `validate.md`.

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
    ├── research-01.md         # Research notes (markdown)
    ├── interview-01.md        # Interview notes (markdown)
    └── artifacts/             # Optional: AI-tool scratch (test logs,
                               #   attempt dumps). Never authoritative.
```

The `.specs/assets/` directory is shared by every spec. AI never hand-edits these — they are the design system, copied from the skill's `assets/` on every forge so existing projects pick up runtime fixes.

The per-spec `artifacts/` subdirectory is **optional**: only create it when the AI tool needs to persist scratch files (e.g., test-run logs the tool can't carry across turns in memory). Files inside `artifacts/` are never read back as authoritative — the spec's TDD Log section, Decision Log, and research-/interview notes are the durable record. Don't write scratch files anywhere else under `.specs/<id>/`.

## Source-of-truth split

| Concern | Lives in | Format |
|---------|----------|--------|
| Identity (id, title, status, dates, priority, tags, mockup-fidelity) | `<script type="application/json" id="spec-meta">` in `<head>` | JSON, single line, **canonical key order** (id, title, status, created, updated, priority, tags, mockup-fidelity) |
| Task lifecycle | `data-status` on `<li class="task">` | `pending` \| `completed` \| `blocked` |
| Phase lifecycle | `data-status` on `<details class="phase">` | `pending` \| `in-progress` \| `completed` \| `blocked` |
| Acceptance lifecycle | `data-status` on `<li class="ac-item">` | `pending` \| `completed` |
| **TDD phase (RGR state inside an IMPL task)** | `data-tdd-phase` on `<li class="task task--impl">` | `red` \| `green` \| `refactor` |
| Progress counts, "Current TDD phase", RGR cycle counts | Never stored | Derived at page load by `spec-runtime.js` |

## Region sentinels

14 canonical region names: `meta`, `toc`, `header`, `overview`, `acceptance`, `architecture`, `testing`, `libraries`, `phases`, `code`, `mockups` (optional), `decisions`, `tdd-log`, `deviations`. (The TDD variant adds `testing` and `tdd-log` to the 12 core regions; `mockups` is omitted when `mockup-fidelity` is `none`.)

## TDD-specific section structure

### Testing Architecture (`region:testing`) — TDD-only, mandatory

Placed after Architecture and before Library Choices. Five sub-tables documenting the testing stack:

1. **Test Framework & Tools** — runner, mocking, HTTP testing, coverage, mutation testing, E2E
2. **Isolation Strategy** — domain (no mocks; pure), service (mock ports), repository (testcontainers or in-memory), HTTP clients (MSW/WireMock), end-to-end (full stack)
3. **Coverage Targets** — line, branch, mutation
4. **Test Commands** — exact CLI commands
5. **Anti-Patterns to Avoid** — project-specific list (e.g., no SQLite stand-ins for Postgres)

### Phases & Tasks (`region:phases`) — TDD structure

**Phases group by feature area, not by test vs implementation.** Each phase contains interleaved TEST-IMPL task pairs. Each pair is one red-green-refactor cycle.

**ANTI-PATTERN — never use:**
- A phase with only TEST tasks (`Phase 1: Tests for X`)
- A phase with only IMPL tasks (`Phase 2: Implement X`)
- Phase names with `(TEST)` or `(IMPL)` suffixes

These violate TDD because they batch test-writing before implementation. Correct: a single feature phase with TEST-01, IMPL-02, TEST-03, IMPL-04 alternating inside.

**Task pair structure:**

```html
<li class="task-pair">
  <ol class="task-list" style="margin:0;">
    <li class="task task--test"
        id="task-TEST-RL-01"
        data-task="TEST-RL-01"
        data-status="pending">
      <span class="task__check"></span>
      <span class="task__code">TEST-RL-01</span>
      <span class="task__text">Write tests for <code>refill()</code>: ...</span>
      <span class="task__rgr task__rgr--red">RED</span>
    </li>
    <li class="task task--impl"
        id="task-IMPL-RL-02"
        data-task="IMPL-RL-02"
        data-status="pending">
      <span class="task__check"></span>
      <span class="task__code">IMPL-RL-02</span>
      <span class="task__text">Implement <code>refill()</code> in <code>src/...</code> → satisfies <code>[TEST-RL-01]</code>.</span>
    </li>
  </ol>
</li>
```

Notes:
- `.task--test` renders with a red left border; `.task--impl` with green
- The `.task__rgr` badge is optional but recommended — it shows RED/GREEN/REFACTOR state for the task
- IMPL tasks include `→ satisfies [TEST-XX-NN]` referencing the test they make pass
- Task codes use `<TYPE>-<PREFIX>-<NN>`: `TYPE` is `TEST` or `IMPL`, `PREFIX` is a 2-4 letter spec abbreviation, `NN` is continuous numbering across all phases starting at 01

**`data-tdd-phase` on an IMPL task** (optional but recommended):
- `red` — test exists, implementation about to start (rare — usually transitions through quickly)
- `green` — implementation makes tests pass
- `refactor` — cleaning up code under green

When the runtime sees a `data-status="in-progress"` task with `data-tdd-phase`, it surfaces the phase in the scorecard. Otherwise it derives RED/GREEN from the next pending task type.

### TDD Log (`region:tdd-log`) — TDD-only, mandatory

Replaces the markdown 4-column table with a vertical timeline of swimlane cards. One `<article class="tdd-cycle">` per TEST-IMPL pair:

```html
<article class="tdd-cycle">
  <header class="tdd-cycle__header">
    <span><code>[TEST-RL-01]</code> → <code>[IMPL-RL-02]</code> &middot; <strong>refill()</strong></span>
    <span class="muted">2026-05-10</span>
  </header>
  <div class="tdd-lanes">
    <div class="lane lane--red">
      <div class="lane__title">Red</div>
      <pre class="lane__output">vitest run tests/bucket.test.ts
3 tests fail: Cannot find module './bucket'</pre>
    </div>
    <div class="lane lane--green">
      <div class="lane__title">Green</div>
      <pre class="lane__output">vitest run tests/bucket.test.ts
3 passed, 0 failed</pre>
    </div>
    <div class="lane lane--refactor">
      <div class="lane__title">Refactor</div>
      <p>Extracted capacity clamp into a named constant.</p>
    </div>
  </div>
</article>
```

Three lanes side-by-side at desktop widths; stacks on narrow screens. Color-coded backgrounds (red/green/blue tints). Test runner output renders inside `<pre class="lane__output">` to preserve monospace alignment.

The TDD Log is the **audit trail** that proves discipline was followed. Fill it in as you go — not retroactively.

### Scorecard additions (TDD)

The header scorecard's four cells differ from the core variant:

1. **Tasks** — `X/Y` total (derived)
2. **RGR cycles** — `X/Y` (derived from completed IMPL task count vs total IMPL task count)
3. **Acceptance** — `X/Y` (derived)
4. **Current TDD phase** — RED / GREEN / REFACTOR (derived)

## Common sections (shared with core variant)

For the common sections — header, overview, acceptance criteria, architecture (Mermaid), library choices, code previews, UI mockups (wireframe/hi-fi), decisions, deviations — see how the core variant handles them. Same HTML, same edit recipes.

## Edit conventions (AI authoring rules)

1. Region sentinels are stable anchors.
2. Stable IDs on every phase, task, and AC item.
3. Continuous task numbering across all phases (`TEST-RL-01`, `IMPL-RL-02`, `TEST-RL-03`, ...).
4. TEST-IMPL alternation within every phase. Never a TEST without its paired IMPL.
5. One attribute per line on state-bearing elements when the line would otherwise be long.
6. Single-line JSON in `<script id="spec-meta">` with canonical key order.
7. `<details>` for collapsibles.
8. Progress strings are NEVER hand-edited.
9. TDD Log entries are appended as `<article class="tdd-cycle">` blocks at the end of `region:tdd-log` after each RGR transition.

## Pause/resume limitation

HTML specs checkpoint state at **task boundaries**. The natural TDD checkpoint is between TEST-IMPL pairs — a clean RGR cycle boundary, not mid-cycle. If you must pause mid-cycle, finish or split the current pair first.

The `data-tdd-phase` attribute on an IMPL task captures *which* RGR phase you're in for the current cycle, but does NOT carry mid-task scratchpad prose. Finish the GREEN before pausing; refactor decisions go in the TDD Log entry's refactor lane when the cycle completes.

## Validation gate

Run the recipe in `validate.md` after every edit. SKILL.md's Update Transaction makes this non-optional.
