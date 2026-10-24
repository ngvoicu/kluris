# Command Contracts

This file defines functional contracts for the `SKILL.md` skill behavior. Use
it as a review checklist before releases.

## Global Contracts

1. `<script id="spec-meta">` JSON in `SPEC.html` is authoritative for identity;
   `data-status` attributes are authoritative for task/phase/AC lifecycle;
   `data-tdd-phase` on IMPL tasks is authoritative for RGR state; registry is
   denormalized.
2. Exactly one active spec should exist after any write operation.
3. All write workflows update `SPEC.html` first (including a successful
   `validate.md` run), then recompute/update registry.
4. Forge workflow never writes application code.
5. Phase status uses `data-status="pending"` | `"in-progress"` | `"completed"`
   | `"blocked"`. Task status uses the same values; AC status uses only
   `pending` / `completed`. Task `data-tdd-phase` (on `task--impl`) uses
   `red` | `green` | `refactor`.
6. **Progress tracking is sacred.** After every task completion: edit
   `SPEC.html` (`data-status` swap, `data-tdd-phase` update if on an IMPL
   task transitioning RGR state, phase transition if applicable, append a
   TDD Log `<article class="tdd-cycle">` when an IMPL task closes its
   cycle, updated date in JSON metadata and visible header dl), run
   `references/validate.md`, update registry (progress, date), then re-read
   both files to verify consistency. Never skip this.
7. **Acceptance criteria are required for feature specs.** Forge must include
   an Acceptance Criteria region with `<li class="ac-item" data-status="pending">`
   testable conditions plus one criterion of the form "Every behavior above
   has a corresponding red-green-refactor cycle recorded in the TDD Log."
   Implement must update `data-status="completed"` on criteria as they are
   satisfied and verify all are met before marking a spec complete.
8. **No mid-task scratchpad state.** HTML specs checkpoint at task /
   RGR-cycle boundaries only — there is no Resume Context section. Pause/
   resume always lands on a clean boundary. The TDD Log carries the durable
   record of what happened in each cycle.

## Command Contracts

### Forge

1. Resolve `<spec-id>` before research output paths are referenced.
2. Collision-check existing spec IDs before creating new files (check
   `.specs/<id>/SPEC.html` and registry).
3. Forge must not run in plan mode; if plan mode is active, require exit
   before continuing (Claude Code only — other tools proceed normally).
4. Refresh `.specs/assets/` on every forge: copy `spec-styles.css` and
   `spec-runtime.js` from the skill's `assets/`, **overwriting any
   existing files**. The runtime is skill-managed; overwrite-on-forge
   ensures existing projects pick up rendering fixes.
5. Create `.specs/<spec-id>/` directory before spawning the research
   subagent or writing any research output.
6. Output scope is `.specs/` artifacts only (`research-*.md`,
   `interview-*.md`, `SPEC.html`, `registry.md` updates, assets on first run).
7. After approval, handoff to the implement workflow instead of
   implementing inside forge.
8. Interview must ask about acceptance criteria ("What does 'done' look like?").
9. **If UI work is in scope, interview must ask about mockup fidelity**
   (`wireframe` / `hi-fi` / `none`) and store the answer in the
   `mockup-fidelity` field of the spec-meta JSON.
10. SPEC.html must be derived from `references/html-template.html` and
    include all canonical regions: `meta`, `toc`, `header`, `overview`,
    `acceptance`, `architecture`, `testing`, `libraries`, `phases`, `code`
    (may be empty), `mockups` (omitted entirely if fidelity is `none`),
    `decisions`, `tdd-log`, `deviations`.
11. Forge must run `references/validate.md` before presenting the spec.

### Implement

1. Supports scope parsing: current flow, phase-specific, all phases, task code.
2. For each completed task: swap `data-status="pending"` →
   `data-status="completed"` in `SPEC.html`; update `data-tdd-phase` if
   on an IMPL task transitioning RGR state; run `references/validate.md`;
   append a TDD Log `<article class="tdd-cycle">` when an IMPL task closes
   its cycle; update registry progress/date. Re-read both files to verify.
3. At phase completion: update phase `data-status` + pill class, promote
   next phase to `in-progress`, review and update satisfied acceptance
   criteria.
4. At spec completion: verify all acceptance criteria have
   `data-status="completed"` before marking complete.
5. Blocked handling:
   - Set blocked tasks to `data-status="blocked"`.
   - Set phase `data-status="blocked"` only when the whole phase is blocked.
   - Record blocker context in the Decision Log or Deviations.

### Resume

1. If no active spec exists, list specs and request target.
2. Parse progress from `SPEC.html` `data-status` counts.
3. Identify current phase (first phase with `data-status="in-progress"`)
   and current task (first task with `data-status="pending"` in that phase).
4. Derive TDD phase from `data-tdd-phase` (if any in-progress task has it)
   or from the current task type (TEST → RED, IMPL → GREEN).
5. Read the most recent `<article class="tdd-cycle">` in `region:tdd-log`
   for last-cycle context.
6. Present a compact summary. No separate Resume Context section to read.

### Pause

1. If no active spec exists, report no-op and stop.
2. Finalize state at a clean task / RGR-cycle boundary — every completed
   task has the right `data-status`, every completed IMPL task has a TDD
   Log entry.
3. Append any pending TDD Log entries for cycles closed during the session.
4. Add session decisions to the Decision Log.
5. Set status `paused` (JSON + visible pill) and sync registry.
6. Run `references/validate.md`.

### Switch

1. Validate target ID and target `SPEC.html` existence before pausing
   current spec.
2. If target already active, report and stop.
3. Pause current (if any), activate target, resume target, sync registry.

### List

1. Handle missing registry gracefully.
2. Group by status in order: active, paused, completed, archived.
3. If `SPEC.html` missing for a row, keep row visible and flag it.
4. Compute task counts by reading each `SPEC.html` and counting
   `<li class="task">` elements by `data-status`.

### Status

1. Show detailed phase/task breakdown for active spec.
2. Surface current TDD phase, RGR cycles done/total, last TDD Log entry.
3. If no active spec, guide to activate one.

### OpenAPI

1. Generate/update `.openapi/openapi.yaml` and `.openapi/endpoints/*.md`.
2. Preserve manual additions when updating existing files.
3. Report endpoint/schema/security counts and manual-review candidates.
4. OpenAPI output stays as YAML + markdown — not HTML.

## Universal Skill Contract

1. `SKILL.md` must include cross-tool behavior for all declared triggers.
2. If `generate openapi` is listed as a trigger, OpenAPI workflow behavior must
   be defined in `SKILL.md`.
3. Reference docs can specialize behavior but cannot violate critical
   invariants from `SKILL.md`.
4. `SKILL.md` must be self-contained for standalone users (`npx skills add`).
   Any content essential for spec creation must be inlined (e.g., the SPEC.html
   template skeleton). References to `references/*.md` are plain pointers.
5. The research subagent (references/researcher.md) is spawned via the Task
   tool as described in SKILL.md, with a graceful inline fallback for tools
   that don't support subagents.

## TDD Contracts

These contracts enforce test-driven development discipline across all commands
and the universal skill. They are **non-negotiable** — violations break the
TDD guarantee.

### Global TDD Contracts

1. **No production code without a failing test.** Every IMPL task must have a
   corresponding TEST task that was written and executed first. If no failing
   test exists for the code being written, stop and write the test.

2. **Tests MUST be executed.** Writing a test file is not enough. The test
   runner must be invoked and the output (pass/fail counts, failure messages)
   must be recorded in the TDD Log.

3. **TDD Log updated after every task.** After completing any TEST or IMPL
   task, add a row to the TDD Log with the red/green/refactor outputs.

4. **Tasks alternate TEST-IMPL within each phase.** Phases group by feature.
   Within each phase, tasks follow the pattern: `[TEST-XX-01]`, `[IMPL-XX-02]`,
   `[TEST-XX-03]`, `[IMPL-XX-04]`. Each TEST-IMPL pair is one red-green-refactor
   cycle. No separate TEST and IMPL phases.

5. **Continuous task numbering with TEST-/IMPL- prefixes.** Task codes
   increment across all phases: `[TEST-XX-01]`, `[IMPL-XX-02]`,
   `[TEST-XX-03]`, `[IMPL-XX-04]`, etc. No resets per phase.

6. **IMPL tasks immediately follow their TEST tasks.** Every IMPL task line
   includes `-> satisfies [TEST-XX-NN]` linking to the test it makes pass.
   The IMPL task is always the next task after its TEST task.

7. **Refactor only when green.** Refactoring (renaming, extracting, restructuring)
   happens only after all tests pass. Refactoring must not change test outcomes.

8. **Test isolation.** Each test is independent. No test relies on another test's
   side effects. Tests can run in any order and produce the same results.

8b. **Tests are sacred.** Tests define expected behavior. During the GREEN phase,
    if tests fail, fix the production code — never modify test assertions to match
    what the code returns. The only reason to touch a test is an actual bug (wrong
    import, syntax error, broken fixture). If a test expectation seems wrong, STOP
    and ask the user before changing it.

### Forge TDD Contracts

9. **Testing Architecture section required.** Every spec produced by forge MUST
   include the Testing Architecture section (framework, tools, isolation
   strategy, coverage targets, test commands, anti-patterns). Specs without
   this section are incomplete.

10. **Testing interview questions.** During the interview phase, forge MUST ask
    at least 2 questions about testing preferences:
    - Preferred test framework and runner (or detect from project)
    - Integration test strategy (Testcontainers, test DB, mocks)
    - Coverage expectations
    - Any existing testing patterns to follow

11. **Interleaved task structure (most commonly violated contract).** Forge
    MUST produce phases with alternating TEST-IMPL task pairs. A phase
    containing only TEST tasks or only IMPL tasks is INVALID. A phase named
    "Tests for X" or "Implement X" or with a `(TEST)`/`(IMPL)` suffix is
    INVALID. The correct structure is feature-named phases with interleaved
    tasks: `[TEST-XX-01] Write test`, then `[IMPL-XX-02] Implement ->
    satisfies [TEST-XX-01]`, then `[TEST-XX-03] Write test`, then
    `[IMPL-XX-04] Implement -> satisfies [TEST-XX-03]`. The coherence
    review MUST verify this before presenting the spec. If any phase fails
    this check, restructure it before presenting to the user.

12. **Research includes test infrastructure.** The research subagent MUST
    analyze the project's existing test infrastructure (framework, patterns,
    coverage tools, anti-patterns) as part of the research phase.

### Implement TDD Contracts

13. **Red-green-refactor enforcement per task pair.** During implementation,
    each TEST-IMPL pair follows:
    - TEST task: write test, run, confirm fail (RED)
    - IMPL task: write minimum code, run, confirm pass (GREEN)
    - REFACTOR: clean up, run, confirm still green
    Then move to the next TEST-IMPL pair. This is true TDD — one cycle at a
    time, not batched.

14. **Blocking rule: per-task, not per-phase.** Each IMPL task MUST NOT start
    until its corresponding TEST task is completed and tests are confirmed
    failing. This is enforced at the task level. An IMPL task cannot run
    before its TEST task, regardless of phase boundaries.

15. **Test execution mandate — 3 runs per cycle.** The implement command MUST
    run tests via Bash at every RED, GREEN, and REFACTOR transition. That is 3
    runs minimum per TEST-IMPL cycle. Claims like "tests would pass" are never
    acceptable. The exact command from the spec's Testing Architecture is used.

16. **Failed tests block progress — fix code, not tests.** If tests fail after
    an IMPL task, fix the production code and run again. Never modify test
    assertions to make them pass. Tests define expected behavior. The only
    reason to touch a test is an actual bug (wrong import, syntax error,
    broken setup). If a test expectation seems genuinely wrong, STOP and ask
    the user.

17. **TDD Log is mandatory.** The implement command MUST update the TDD Log
    table after each task. Missing TDD Log entries indicate the TDD process
    was not followed.

### Resume TDD Contracts

18. **TDD phase indicator.** Resume output MUST include the current TDD phase
    (RED, GREEN, or REFACTOR), derived from the in-progress IMPL task's
    `data-tdd-phase` or (if no task is in-progress) from the first pending
    task type. The last `<article class="tdd-cycle">` in `region:tdd-log`
    carries the previous cycle's context.

### Pause TDD Contracts

19. **TDD state capture.** When pausing, the pause command MUST:
    - Ensure every completed task has the right `data-status`
    - Ensure every completed IMPL task has its `data-tdd-phase` reflecting
      the last phase reached
    - Append any pending `<article class="tdd-cycle">` entries to
      `region:tdd-log` for cycles closed during the session
    Pause always happens at a clean task / cycle boundary — there is no
    Resume Context section to write to.

### Status TDD Contracts

20. **TDD indicators in status output.** The status command MUST show:
    - Current TDD phase (RED/GREEN/REFACTOR)
    - TEST vs IMPL task counts and completion per phase
    - Number of TDD Log entries vs expected entries
    - Last test run results summary

## Icon Standards

All commands and SKILL.md must use these standardized icons:

**Registry-level** (active/paused/completed specs):
- `->` active
- `||` paused
- `+` completed

**Task-level** (done/in-progress/pending):
- `+` done
- `->` in-progress / current
- `o` pending

## Release Checklist

1. Paths referenced in docs and templates exist (excluding placeholder paths).
2. Command contracts in this file still match `SKILL.md`.
3. TDD contracts are enforced by the universal skill.
4. `SKILL.md` is self-contained for standalone use (template references and
   research subagent fallback documented).
5. Status icons are consistent across all files (see Icon Standards above).
6. `assets/spec-styles.css` and `assets/spec-runtime.js` exist and are
   distributed to consumer projects' `.specs/assets/` on first forge.
7. `references/html-template.html` validates with `references/validate.md`.
8. `references/html-template.html` uses canonical JSON key order in
   `<script id="spec-meta">`.
