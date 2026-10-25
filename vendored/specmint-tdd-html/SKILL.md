---
name: specmint-tdd-html
description: >
  TDD-first spec management for AI coding workflows. Use this skill when the
  user explicitly mentions specs, forging, or structured planning: says "forge",
  "forge a spec", "write a spec for X", "create a spec", "plan X as a spec",
  "resume", "what was I working on", "spec list/status/pause/switch/activate",
  "implement the spec", "implement phase N", "implement all phases",
  "red green refactor", "run the tests", "generate openapi". Also trigger
  when a `.specs/` directory exists at session start. Do NOT trigger on
  general feature requests, coding tasks, or questions that don't mention
  specs or forging.
license: MIT
metadata:
  author: Gabriel Voicu
  version: "3.0.1"
---

# Spec Mint TDD HTML

Turn ephemeral plans into structured, persistent specs built through deep
research and iterative interviews — with strict test-driven development at
every step. Every task starts with a failing test, production code exists
only to make tests pass, and refactoring happens under green. Tests use
testcontainers for real services, mock only at boundaries, and make no
external network calls. Specs render as professional HTML documents
(Mermaid diagrams, syntax-highlighted code diffs, optional wireframe or
hi-fi UI mockups, RGR swimlane TDD log, derived progress + current TDD
phase scorecard) and live in `.specs/` at the project root. They work
with any AI coding tool that can read and edit HTML files.

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
   is authoritative for identity. `data-status` attributes on tasks /
   phases / acceptance criteria are authoritative for lifecycle. The
   `data-tdd-phase` attribute on an IMPL task carries RGR state. The
   registry is a denormalized index for quick lookup.
4. **Active-spec rule**: Target exactly one active spec at a time.
5. **Parser policy**: Use best-effort parsing with clear warnings and repair
   guidance instead of hard failure on malformed rows.
6. **TDD invariant**: No production code without a failing test. The implement
   workflow enforces red-green-refactor at every task. Tests are executed via
   the actual test runner, not assumed to pass. This is sacred — see "Tests
   Are Sacred" and "Blocking Rule" in the implement section.
7. **Progress tracking is sacred**: After completing any task, immediately
   update `SPEC.html` (swap `data-status`, update phase pill if transitioning,
   append TDD log entry on cycle close) AND `registry.md` (progress count,
   date). Run the validate recipe (see `references/validate.md`) to confirm
   the file still parses. Re-read both files to verify the edits landed.
   Never move to the next task without updating both files. Never end a
   session with the registry out of sync with the derived progress in
   `SPEC.html`. This is non-negotiable.

## Coexistence with markdown specs

This skill only manages `.html` specs. If `.specs/<id>/SPEC.md` files exist
(from a markdown-flavored Spec Mint variant), they are not visible to or
operated on by this skill. No auto-conversion, no edits. **The user should
use the markdown-flavored variant for those specs.**

## Session Start

If active-spec context was injected by host tooling, use it directly
instead of reading files. Otherwise, fall back to reading files manually:

1. Read `.specs/registry.md` to check for a spec with `active` status
2. If one exists, briefly mention it:
   "You have an active spec: **User Auth System** (5/12 tasks, Phase 2).
   Say 'resume' to pick up where you left off."
3. Don't force it — the user might want to do something else first

## Deterministic Edge Cases (Best-Effort)

| Situation | Required behavior |
|-----------|-------------------|
| `.specs/registry.md` missing | If `.specs/` exists, report "No registry yet" and offer to initialize it. If `.specs/` is missing, report "No specs yet" and continue normally. |
| `.specs/assets/` missing or stale when a SPEC.html is being written | Refresh it — copy `spec-styles.css` and `spec-runtime.js` from the skill's bundled `assets/` directory into `.specs/assets/`, **overwriting any existing files**. The runtime ships rendering fixes (Mermaid, diagram fullscreen modal, code highlighting, RGR-phase derivation) and must stay in sync with the skill version. |
| Malformed registry row | Skip malformed row, emit warning with row text, continue parsing remaining rows. |
| Multiple `active` rows | Warn user. Pick the row with the newest `Updated` date (or first active row if dates are unavailable) for this run. On next write, normalize to a single active spec. |
| Registry row exists but `.specs/<id>/SPEC.html` missing | Warn and continue. Keep row visible in list/status with `(SPEC.html missing)`. |
| Registry and SPEC conflict | Trust `SPEC.html`, then repair registry values on next write. |
| No active spec | List available specs and ask which to activate or resume. |

## Working on a Spec

### Resuming

When the user says "resume", "what was I working on", or similar:

1. Read `.specs/registry.md` — find the spec with `active` status. If none, list specs and ask which to resume
2. Load `.specs/<id>/SPEC.html`
3. Parse progress from `data-status` attributes:
   - Count `<li class="task">` per `data-status` value, per phase
   - Find current phase (first `<details class="phase">` with `data-status="in-progress"`)
   - Find current task (first `<li class="task" data-status="pending">` in that phase)
   - Derive TDD phase: if the current task is `task--test` → RED; if `task--impl` → GREEN. Or read `data-tdd-phase` if the current task is in-progress.
4. Present a compact summary:

   ```
   Resuming: User Auth System
   Progress: 5/12 tasks (Phase 2: OAuth Integration)
   Current: [TEST-AUTH-07] Write OAuth callback tests
   TDD Phase: RED (next: write test, run, confirm fail)
   ```

5. Begin working on the current task — don't wait for permission

There is no separate Resume Context section in HTML specs. The first pending
task in the active phase is implicitly current. Look at the most recent TDD
Log entry (the last `<article class="tdd-cycle">` in `region:tdd-log`) for
context on what just completed.

### Implementing a Spec (TDD)

When the user says "implement the spec", "implement phase N", "implement all
phases", or similar:

#### Scope Detection

Parse the user's request to determine scope:
- **"implement the spec"** or **"implement"** — Start from the first task
  with `data-status="pending"` in the in-progress phase and work forward
- **"implement phase N"** or **"implement phase <name>"** — Implement all
  pending tasks in that specific phase
- **"implement all phases"** or **"implement everything"** — Implement all
  remaining pending tasks across all phases, in order

#### TDD Implementation Flow

1. Read `.specs/registry.md` to find the active spec
2. Load `.specs/<id>/SPEC.html` and parse phases/tasks
3. Identify the target tasks based on scope
4. For each task in order, determine if it is a TEST or IMPL task by its
   task code prefix (`TEST-` vs `IMPL-`)
5. Tasks within a phase alternate: TEST then IMPL, TEST then IMPL. Each
   TEST-IMPL pair is one red-green-refactor cycle.

**For each TEST-IMPL pair (one red-green-refactor cycle):**

**RED — TEST task:**
1. Write the test file at the path specified in the task, with the assertions
   and test descriptions listed
2. **GATE: RUN the tests via Bash.** Do not proceed without running them.
3. **GATE: Confirm tests FAIL.** If all tests fail — good, this is RED.
4. If tests pass unexpectedly: **STOP.** Report the anomaly to the user.
   Do not proceed. Either the feature already exists or the tests are wrong.
5. Mark the TEST task completed: swap `data-status="pending"` →
   `data-status="completed"` on the `<li class="task task--test">`.
6. Stash the failing output for the TDD Log entry that closes this cycle.

**GREEN — IMPL task:**
1. Read the test file from the preceding TEST task — understand exactly what
   the tests expect
2. Update the IMPL task: `data-status="in-progress"` and add
   `data-tdd-phase="green"`.
3. Write the minimum production code to make the tests pass. Only what the
   tests require. Nothing more.
4. **GATE: RUN the tests via Bash.** Do not proceed without running them.
5. **GATE: Confirm tests PASS.** If any test fails, fix the production code
   and run again. Repeat until green. Do not modify tests to make them pass
   (see Tests Are Sacred below).
6. Stash the passing output for the TDD Log entry.

**REFACTOR — still on the IMPL task:**
1. Update `data-tdd-phase="refactor"` on the IMPL task.
2. Clean up the production code — remove duplication, improve naming, extract
   helpers.
3. **GATE: RUN tests again via Bash.** Confirm they are still green.
4. If refactoring broke tests: undo the refactoring, try a different approach.
5. Update the IMPL task: `data-status="completed"`. Leave
   `data-tdd-phase="refactor"` as the historical marker of the last phase.
6. Append a `<article class="tdd-cycle">` to `region:tdd-log` capturing the
   stashed Red output, the stashed Green output, and the Refactor notes
   (or "None — implementation is already minimal."). See
   `references/edit-recipes.md` for the exact HTML.
7. Run the validate recipe (`references/validate.md`).
8. Update progress and date in `.specs/registry.md`.

**Then move to the next TEST-IMPL pair and repeat.**

#### Self-Check Before Every Task

Before starting any task, pause and verify:

- [ ] Am I about to write production code? → Is there a failing test for it?
- [ ] Am I about to skip running tests? → Run them. Always.
- [ ] Am I about to modify a test to make it pass? → Stop. Fix the code instead.
- [ ] Did I log the last task's output in the TDD Log? → Do it now.
- [ ] Did I swap `data-status` on the completed task and run the validator? → Do it now.

If any answer is wrong, correct it before proceeding.

#### Violation Examples

These are common TDD violations. If you recognize yourself doing any of
these, **STOP immediately** and correct course:

| Violation | What it looks like | Correct action |
|-----------|-------------------|---------------|
| Writing code before test | "I'll implement the handler first, then test it" | Write the test first. Watch it fail. Then implement. |
| Skipping test execution | "The tests would pass since I wrote the correct code" | Run the tests via Bash. Read the actual output. |
| Modifying tests to pass | "I'll adjust the assertion to match what the code returns" | The test is the spec. Fix the production code to match. |
| Batching tests | "I'll write all 3 tests, then implement all 3" | Write one test. Implement. Write the next test. Implement. |
| Skipping refactor | "The code is fine, moving to the next test" | Review the code. Decide consciously. Log "none" if no changes. |
| Forgetting TDD Log | "I'll update the log later" | Update it now, after each task. It's the audit trail. |

#### Tests Are Sacred

Tests define expected behavior. They are the specification in code form.
During the GREEN phase, when tests fail:

- **Fix the production code**, not the tests
- The only time you modify a test is when it has an actual bug (wrong import,
  syntax error, broken test setup) — not when the assertion doesn't match
  what your code returns
- If a test expects behavior X and your code does behavior Y, your code is
  wrong — make it do X
- If you believe the test expectation is genuinely incorrect (e.g., the spec
  changed, the user gave new requirements), **STOP and ask the user** before
  modifying the test. Do not silently change test expectations.

#### Blocking Rule

Each IMPL task is preceded by its TEST task. You cannot start an IMPL task
until its TEST task is completed and the tests are confirmed failing. This
is enforced per-task. If about to write implementation code before its test
exists, STOP and write the test first. Non-negotiable.

#### Test Execution Rule

Run the actual test command (`pytest`, `vitest`, `cargo test`, `go test`,
`dotnet test`, etc.) via Bash at every RED, GREEN, and REFACTOR transition.
That is 3 runs minimum per TEST-IMPL cycle. Claims like "tests would pass"
or "tests should fail" are never acceptable — run them and report the
actual output. If the test runner is not available, report the blocker
immediately.

#### Phase Transitions

Phases group by feature, not by test vs implementation. Each phase contains
interleaved TEST-IMPL pairs. When all tasks in a phase are done, swap the
phase's `data-status="in-progress"` → `data-status="completed"` (and update
the pill class), then promote the next phase's `data-status` from `pending`
to `in-progress` (with matching pill update).

#### Update Transaction (sacred — never skip steps)

Progress tracking is sacred (see Critical Invariant #7). Stale tracking
is the single most common failure mode — it makes resume unreliable and
the registry useless.

1. Edit `SPEC.html`: swap `data-status` on the completed task; update
   `data-tdd-phase` if transitioning RGR state on an IMPL task; if all
   tasks in the phase are now completed, transition the phase pill +
   data-status and promote the next phase; on cycle close, append the
   `<article class="tdd-cycle">` entry to `region:tdd-log`; update the
   `"updated"` date in JSON metadata and the visible header dl.
2. **Run the validate recipe** from `references/validate.md`. If it doesn't
   print `OK`, fix the broken JSON or sentinel before continuing.
3. Recompute progress directly from `SPEC.html` `data-status` counts.
4. Edit the matching registry row (status, progress, updated date).
5. **Verify**: Re-read both `SPEC.html` and `registry.md` to confirm the
   edits are correct. If the registry progress doesn't match the SPEC.html
   completed-task count, fix it now.
6. If registry update fails, keep `SPEC.html` as source of truth and emit a
   warning with exact repair action for `.specs/registry.md`.

See `references/edit-recipes.md` for the exact before/after HTML for every
common operation (mark task completed, append TDD log entry, transition
phase, set `data-tdd-phase`, etc.).

**If you notice you forgot to update after a previous task, stop what
you're doing and update now before continuing.**

Also:
- If a task is more complex than expected, split it into subtasks
- Log non-obvious technical decisions to the Decision Log
- If implementation diverges from the spec (errors found, better approach
  discovered, assumptions proved wrong), log it in the **Deviations** section
- **Phase review**: When all tasks in a phase are done, review before moving
  on — re-read the phase's tasks and acceptance criteria, verify each task's
  implementation matches what was specified, check that the TDD Log has
  entries for every TEST-IMPL pair, and check for missing edge cases or spec
  drift. Fix issues before marking the phase complete. Log findings in the
  Decision Log.

#### Verification Gate

Before reporting any phase or spec as complete, provide evidence:
- Run the relevant test suite via the project's test runner
- Show the actual command and output — not a summary, not "tests pass"
- If tests fail, fix the issues before claiming completion
- Never use language like "should pass", "probably works", or "seems correct"

Evidence first, then assertions. The TDD log captures per-task evidence;
this gate ensures phase and spec completion also have fresh verification.

#### Completion

When all in-scope tasks are done:

- **All tasks in the spec complete:**
  - **Run the full test suite** and show the output (verification gate)
  - Verify all Acceptance Criteria have `data-status="completed"`. If any
    remain pending, report which ones and ask the user before marking the
    spec complete.
  - Set every phase `data-status="completed"` (with matching pill update)
  - Set spec status to `completed` in `<script id="spec-meta">` JSON and registry
  - Update the `updated` date
  - Run the validate recipe
  - Present a summary of what was implemented, including TDD Log highlights
  - Suggest next spec to activate if any are paused

- **Only a phase completed (not all tasks):**
  - **Run tests** for the phase's scope and show the output (verification gate)
  - Report the phase completion and remaining work
  - Set the next phase to `data-status="in-progress"` if applicable
  - State whether the next pending task is TEST or IMPL

### Pausing

When the user says "pause", switches specs, or a session is ending:

0. If there is no active spec, report that there is nothing to pause and stop.
1. **Pause at a clean RGR boundary.** HTML specs do not carry mid-task or
   mid-cycle scratchpad state. If you're mid-cycle (TEST written, GREEN not
   yet passing), either finish the cycle or split the IMPL task into
   subtasks first. The natural pause point is between completed TEST-IMPL
   pairs.
2. Make sure every completed task has `data-status="completed"` and every
   `data-tdd-phase` on a completed IMPL task reflects the last phase reached.
3. Append any pending TDD Log entries to `region:tdd-log` for cycles that
   were closed during this session.
4. Add any session decisions to the Decision Log.
5. Change spec status from `active` to `paused`:
   - In `<script id="spec-meta">` JSON: `"status":"paused"`
   - Visible header pill: `pill--in-progress`/`Active` → `pill--pending`/`Paused`
6. Update the `"updated"` date (JSON + visible dl).
7. Mirror status + date in `.specs/registry.md`.
8. Run the validate recipe (`references/validate.md`).
9. Confirm to the user that progress was saved.

**Known limitation: HTML specs checkpoint state at task / cycle boundaries
only.** The TDD Log entries are the durable record of what happened — fill
them in completely (Red output, Green output, Refactor notes) as cycles
close, not retroactively. The Decision Log can hold brief notes but is
designed for durable technical decisions, not running scratchpad.

### Switching Between Specs

1. Validate the target spec ID first. If missing, list available specs.
2. Confirm `.specs/<target-id>/SPEC.html` exists. If not, stop with an error.
3. If target is already active, report and stop.
4. Pause the current active spec if one exists (full pause workflow).
5. Set target status to `active` in JSON metadata and in `.specs/registry.md`.
6. Resume the target spec (full resume workflow).

## Spec Format (HTML — TDD)

The detailed format reference lives in `references/spec-format.md`. The
canonical empty template is `references/html-template.html`. Edit recipes
for every common operation (including TDD-specific ones like setting
`data-tdd-phase`, appending TDD log entries, adding TEST-IMPL pairs) are
in `references/edit-recipes.md`. Wireframe and hi-fi mockup patterns are
in `references/wireframe-library.md` / `references/mockup-library.md`. The
post-edit validator is in `references/validate.md`. Language-agnostic
testing reference is in `references/testing-knowledge.md`.

**Key facts about the format:**

- Each `SPEC.html` references `../assets/spec-styles.css` and
  `../assets/spec-runtime.js`. Those two files are shared by every spec
  in the project and are refreshed from the skill's bundled `assets/` on every
  forge (overwrite, not skip-if-present) so existing projects pick up
  runtime fixes.
- Identity (`id`, `title`, `status`, `created`, `updated`, `priority`,
  `tags`, `mockup-fidelity`) lives in `<script type="application/json"
  id="spec-meta">` in `<head>`, single-line JSON, canonical key order
  (`id`, `title`, `status`, `created`, `updated`, `priority`, `tags`,
  `mockup-fidelity` — logical, not alphabetical).
- Lifecycle state lives in `data-status` on tasks / phases / AC items.
  Values: `pending`, `in-progress`, `completed`, `blocked`. AC uses only
  `pending` / `completed`.
- **TDD RGR state lives in `data-tdd-phase` on `<li class="task task--impl">`.**
  Values: `red` | `green` | `refactor`. Set when starting / advancing the
  cycle; left in place after the task completes as a record of the last
  phase reached.
- Progress strings, RGR cycle counts, and the "Current TDD phase" badge
  are **never authored** — they are computed at page load by
  `spec-runtime.js`. The runtime derives the current TDD phase from the
  in-progress IMPL task's `data-tdd-phase`, or (if no task is in-progress)
  from the type of the first pending task (TEST → RED, IMPL → GREEN).
- Every top-level section is wrapped in `<!-- region:NAME -->` /
  `<!-- endregion:NAME -->` sentinels. TDD specs have 14 canonical regions:
  meta, toc, header, overview, acceptance, architecture, **testing**,
  libraries, phases, code, mockups (optional), decisions, **tdd-log**,
  deviations.
- Task codes use `<TYPE>-<PREFIX>-<NN>`: `TYPE` is `TEST` or `IMPL`,
  `PREFIX` is a 2-4 letter uppercase abbreviation of the spec ID; numbers
  auto-increment continuously across all phases starting at 01.
- **TEST-IMPL alternation within every phase.** Phases are named by feature
  ("Auth Foundation"), never by test-vs-impl ("Tests for Auth" / "Implement
  Auth"). A phase with only TEST tasks or only IMPL tasks is wrong.
- All phases render `open` by default. No "current task" marker — the
  first pending task in the active phase is implicitly current.
- TDD Log entries are `<article class="tdd-cycle">` blocks appended to
  `region:tdd-log` as cycles close. They render as a 3-lane swimlane
  (RED / GREEN / REFACTOR) with monospace test output inside each lane.

## Forging Specs

When asked to forge, plan, spec out, or "write a spec for X", follow the
full forge workflow: setup, research deeply, interview the user, iterate
until clear, then write the spec.

**Plan mode:** If your tool is in a read-only plan mode, do not run forge in
that mode. Ask the user to leave it (in Claude Code: Shift+Tab) and start the
forge workflow again.

**The forge workflow never produces application code.** Its outputs are only
`.specs/` files: research notes, interview notes, and the SPEC.html.

### Step 1: Setup

1. Generate a spec ID from the title (lowercase, hyphenated):
   `"User Auth System"` -> `user-auth-system`
2. **Collision check**: If `.specs/<id>/SPEC.html` already exists or the ID
   appears in `.specs/registry.md`, warn the user and ask:
   - **Resume** the existing spec
   - **Rename** the new spec (suggest `<id>-v2` or ask for a new title)
   - **Archive** the old spec and create a new one in its place
   Do not proceed until the user chooses.
3. Initialize directories: `mkdir -p .specs/<id> .specs/assets`
4. **Refresh shared assets.** Copy `spec-styles.css` and `spec-runtime.js`
   from the skill's bundled `assets/` directory into `.specs/assets/`,
   **overwriting any existing files**. The runtime is skill-managed and
   never hand-edited — it ships rendering fixes (Mermaid initialization,
   click-to-fullscreen diagram modal with wheel-zoom + drag-pan, PrismJS
   code highlighting, RGR-phase derivation) that must stay in sync with
   the skill version. Under Kluris that directory is
   `~/.kluris/companions/specmint-tdd-html/assets/`.
5. If `.specs/registry.md` doesn't exist, initialize it with the header row.

### Step 2: Deep Research

Research is the foundation of a good spec. Be exhaustive — use every
available resource so the spec won't need revision mid-build.

Research runs on two parallel tracks:

#### Track A: Spawn a Research Subagent

**If the tool supports subagents:** Spawn a research subagent with your
tool's subagent/task feature (e.g. Claude Code's Agent tool), using the brief
in `references/researcher.md`, for exhaustive parallel research. Provide: the
user's request, spec ID, output path `.specs/<id>/research-01.md`, and any
Context7 findings from Track B. The research subagent maps the project
architecture, reads 15-30 files, runs 3+ web searches, compares library
candidates, assesses risks, and analyzes the full test infrastructure.

**If subagents are not available:** Perform the research inline yourself,
using `references/researcher.md` as your checklist — scan the project
structure, read relevant files (15-30 for non-trivial features), search the
web for best practices and library comparisons, and analyze the existing test
infrastructure (frameworks, runners, mocking patterns, testcontainers,
coverage tools). Save findings to `.specs/<id>/research-01.md`.

#### Track B: Context7 & Cross-Skill Research (in parallel)

While the research subagent runs (or between inline research steps):

- **Context7**: If available, pull up-to-date documentation for 2-5 key
  libraries. Check API changes, deprecated features, and recommended patterns.
- **Cross-skill loading**: Load relevant skills when available:
  - **frontend-design**: For UI-heavy specs
  - **datasmith-pg**: For database specs
  - **webapp-testing**: For testing strategy
  - **vercel-react-best-practices**: For Next.js/React
- **UI research** (if applicable): Screenshots, component hierarchy,
  modern UI patterns, accessibility requirements

#### Merging Research

Combine all findings. The research should cover: architecture, relevant
code, tech stack, library comparisons, internet research, Context7 docs,
UI research (if applicable), risk assessment, test infrastructure analysis,
and open questions.

### Step 3: Interview Round 1

Present research findings and ask targeted questions:

1. **Summarize findings** (2-3 paragraphs)
2. **State assumptions** — "Based on the codebase, I'm assuming X. Correct?"
3. **Ask 3-6 targeted questions** that research couldn't answer:
   - Architecture decisions ("New module or extend existing one?")
   - Scope boundaries ("Should this handle X edge case?")
   - Technical choices ("Stick with Library A or try Library B?")
   - User-facing behavior ("What should happen when X fails?")
   - **Testing preferences** ("The project uses pytest with testcontainers —
     should we follow that pattern or is there a reason to change?")
   - **Isolation strategy** ("Mock the payment gateway at the HTTP boundary,
     or use a testcontainer with a sandbox endpoint?")
   - **Coverage targets** ("Any minimum coverage requirement?")
   - **Acceptance criteria** ("What does 'done' look like? Any specific
     conditions that must be true when this is complete?")
   - **Mockup fidelity (only if UI work is in scope)** — "Mockups in this
     spec should render as `wireframe` (clean grayscale boxes, structural,
     no design commitment), `hi-fi` (real-looking polished components), or
     `none` (prose + diagrams are enough)?" Record the answer for the
     `mockup-fidelity` field in the spec-meta JSON.
4. **Propose a rough approach** and ask for reactions

**STOP after presenting questions.** Wait for the user to answer. Do not
answer your own questions, assume answers, or continue until the user
responds. Save to `.specs/<id>/interview-01.md`.

### Step 4: Deeper Research + Interview Loop

Based on answers, do another round of research — explore chosen paths,
check feasibility, find issues. Save to `.specs/<id>/research-02.md`.
Then present findings and ask about trade-offs, edge cases, implementation
sequence, and scope refinement.

**Repeat until:** no ambiguous tasks remain, the user is satisfied, and
every task can be described concretely. Two rounds is typical.

### Step 5: Write the Spec

Synthesize all research and interviews into a `SPEC.html`. **Start from the
canonical template** at `references/html-template.html` — copy it to
`.specs/<id>/SPEC.html` and fill in every placeholder. Use
`references/edit-recipes.md` for the exact HTML structure of each common
element (TEST-IMPL pair, TDD log entry, diagrams, code-diff figures,
mockups). Reference `references/testing-knowledge.md` for testing framework
and tooling decisions.

The spec must include:

- **`<script id="spec-meta">` JSON** (canonical key order: `id`, `title`,
  `status`, `created`, `updated`, `priority`, `tags`, `mockup-fidelity`)
- **Spec header card** with TDD chip (`<span class="pill pill--info pill--no-dot">TDD</span>`),
  status pill, priority chip, dates, tags, scorecard (Tasks / RGR cycles /
  Acceptance / Current TDD phase)
- **Overview** (2-4 sentences)
- **Acceptance Criteria** with one criterion of the form "Every behavior
  above has a corresponding red-green-refactor cycle recorded in the TDD Log"
- **Architecture diagrams** — Mermaid (`flowchart`, `sequenceDiagram`,
  `erDiagram`, `stateDiagram-v2`, `timeline`, `journey`, `gantt`,
  `block-beta`, `architecture-beta`, `c4Context`, `treemap`). Loaded
  on-demand from CDN by `spec-runtime.js` when a `<pre class="mermaid">`
  block exists.

  **Mermaid authoring rules — read every time, do not skim.** Every
  parse error this skill has ever produced has the same root cause:
  the author judged a label or alias "didn't need quoting" and was wrong.
  The rules below remove the judgment call.

  1. **No HTML entities inside `<pre class="mermaid">`.** Write
     `A --> B`, not `A --&gt; B`. Mermaid parses pre-content as plain
     text; entity strings are read literally and break parsing.

  2. **ALWAYS quote participant aliases** in `sequenceDiagram` — every
     single one, no exceptions:
     ```
     participant U as "User"
     participant API as "Backend API (port 8080)"
     participant DB as "PostgreSQL"
     ```
     ❌ `participant API as Backend API (port 8080)` — unquoted parens
     ✅ `participant API as "Backend API (port 8080)"`

  3. **ALWAYS quote message labels** in `sequenceDiagram` — every arrow:
     ```
     U->>API: "POST /endpoint (body)"
     API->>DB: "SELECT col1, col2, col3"
     DB-->>API: "SQL stream (application/sql;charset=UTF-8)"
     ```
     The lexer treats `:` `(` `)` `,` `;` as syntax tokens in unquoted
     form. Wrap every label.

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
     Use `_` where you'd reach for `.` or `-` in an ID.

  6. **One statement per line.** Terminate every arrow with a target.

  7. **Self-check loop after every Mermaid block.** Before moving on,
     re-read every `participant` line and every arrow line. If any
     contains `(`, `)`, `,`, `:`, `;`, or `/` outside a quoted span,
     wrap it now.

  8. **Validator-clean is required.** After writing, open the rendered
     `SPEC.html`, run `specmintValidate()` in the console, and fix
     every `[mermaid]` error. Source for each failing block is on
     `data-mermaid-source`. A spec with any `figure.diagram--error`
     is not done.

  **Canonical sequenceDiagram template** (copy, then customize — every
  alias and every label is quoted; do not deviate):
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

  **Canonical flowchart template:**
  ```
  flowchart LR
      A["Start"] --> B{"Has token?"}
      B -- yes --> C["Call /api (with token)"]
      B -- no  --> D["Redirect /login"]
      C --> E["200 OK"]
  ```
- **Testing Architecture** (mandatory) — five sub-tables: Test Framework &
  Tools, Isolation Strategy, Coverage Targets, Test Commands, plus an
  Anti-Patterns list
- **Library Choices** — comparison table with rationale
- **Feature phases** with interleaved TEST-IMPL task pairs wrapped in
  `<li class="task-pair">`. Each pair = one red-green-refactor cycle. Phases
  are named by feature ("Auth Foundation"), **never** by test-vs-impl
  ("Tests for Auth" / "Implement Auth"). No `(TEST)` or `(IMPL)` suffixes
  on phase names. Every phase contains alternating TEST-IMPL tasks.
- **Tasks** with `[TEST-PREFIX-NN]` and `[IMPL-PREFIX-NN]` codes; IMPL tasks
  include `→ satisfies [TEST-XX-NN]` reference
- **Code Previews** — `<figure class="code-diff">` blocks with PrismJS
  `language-diff-LANG diff-highlight` showing the meaningful code
  deltas. **Expected on every feature spec.** Skip only when the spec
  genuinely produces no code (pure research / docs).

  **Include one canonical figure per category** (not every instance):
  - The signature/contract of each new public interface, exported
    function, class, or API endpoint
  - The shape of each new data model, schema, or migration
  - Non-trivial business logic (algorithm, validation, transformation)
    where the body itself matters
  - The "before → after" of each refactor or significant signature
    change that captures a design decision
  - One canonical test per new test pattern — the shape, not every
    test body (the full body lives in the TEST task itself during
    implementation, not in the forged spec)

  **Skip:** boilerplate (imports, scaffolding, route registration
  already implied by phases); repeated identical patterns (show one,
  note the rest follow); codegen output / formatted JSON / build
  artifacts; the full test bodies that will be written during
  RED phases.

  **Sizing.** Small spec (1-2 phases, one module): 2-4 previews.
  Medium (3-5 phases): 4-8. Large (6+ phases across API + DB + UI):
  8-15. (TDD specs run slightly leaner than core because the TEST-IMPL
  task list and Testing Architecture section carry some of the
  "what changes" weight.) If a spec produces hundreds of changes but
  only has 1-2 previews, you missed the point — surface the most
  important deltas.

  Unified diff by default; `data-view="split"` for changes >30 lines,
  multi-hunk, or where the before/after comparison itself is the point.
- **UI Mockups** (omit entirely if `mockup-fidelity: none`) — hi-fi
  `<figure class="mockup mockup--hifi">` blocks by default; reserve
  `mockup--wireframe` for the rare case where the spec is intentionally
  structural-only with no real labels yet.

  **Default to hi-fi.** Almost every spec has concrete copy. The moment
  you have real text to show (button label, column header, status,
  placeholder), use the `.ui-*` components in
  `references/mockup-library.md` — they render as real-looking UI.

  **The wireframe library (`.wf-*`) is for empty skeletons only.**
  `.wf-heading`, `.wf-text`, `.wf-pill`, `.wf-input` are skeleton
  bars; they render correctly only when the tag is empty. If you have
  real content, switch that block to `.ui-*`. Mixing real text inside
  `.wf-text` / `.wf-heading` is the #1 cause of "ugly grey blob behind
  text" bug reports.

  Read `references/mockup-library.md` (`.ui-*` patterns: Login,
  Dashboard, Data table, Modal, Toast, Validation form, Wizard,
  Alert+tabs, Settings, Card grid) before authoring.
  `references/wireframe-library.md` is the fallback for
  structural-only sketches.

  **Never use ASCII art inside `<figure class="mockup">`** — no boxes
  drawn with `+`, `|`, `-`; no pipe-delimited tables; no monospace
  pseudo-diagrams. For grids use `.ui-table` (real `<table>` markup).
  For cards use `.ui-card`. Compose new structure from primitives if
  needed — do **not** fall back to ASCII. The runtime validator flags
  ASCII inside mockup figures.
- **Decision Log** — initially populated with key decisions from interviews
- **TDD Log** (empty at forge time — filled during implementation as cycles close)
- **Deviations** (empty at forge time)

**Coherence review (mandatory before presenting):**
1. Entire spec tells a coherent story
2. Phases are in logical dependency order
3. Every task is concrete and actionable (file paths, function names)
4. Architecture diagram matches task descriptions
5. Testing strategy covers all feature tasks
6. Library choices are consistent throughout
7. Overview accurately summarizes what phases deliver
8. No gaps — everything implementation needs is covered by a task
9. Verify acceptance criteria are specific, testable, and cover the key
   behaviors the user expects
10. **CRITICAL: Phases group by feature, not by test-vs-impl.** A phase
    with only TEST tasks or only IMPL tasks is WRONG. A phase named
    "Tests for X" or "Implement X" or with `(TEST)`/`(IMPL)` suffix is
    WRONG. Restructure: merge test-only and impl-only phases into a single
    feature phase with alternating TEST-IMPL pairs inside.
11. Every `[IMPL-XX-NN]` task immediately follows its `[TEST-XX-NN]` task
12. Every `[TEST-XX-NN]` task is followed by an `[IMPL-XX-NN]` that satisfies it
13. **Placeholder check**: Search the spec for "TBD", "TODO", "placeholder",
    "TBC", "to be determined", "will be decided", "figure out" — replace
    every instance with a concrete decision or remove the section
14. **Internal consistency**: Verify task count in overview matches actual
    tasks, all task code references are valid, `-> satisfies` references
    point to existing TEST tasks, library versions don't conflict
15. **Scope check**: Compare the spec against the interview answers — does
    it deliver what was discussed? Nothing more, nothing less?
16. **Ambiguity check**: For each task, ask "could an implementer complete
    this without asking me a question?" If no, add detail until yes.
17. **Run the validate recipe** (`references/validate.md`) — confirm the
    file parses cleanly.

Save to `.specs/<id>/SPEC.html`. Update `.specs/registry.md` — set status
to `active`. Mark first phase `data-status="in-progress"` (with matching
pill). Run the validate recipe one final time.

**Present the spec and wait for approval.** Do not begin implementing until
the user explicitly approves.

## Generating OpenAPI Docs

When the user says "generate openapi", "update api docs", or similar:
scan the codebase for API routes, schemas, and security config. Write
`.openapi/openapi.yaml` (OpenAPI 3.1.1) with `operationId` per operation,
reusable `$ref` schemas, and accurate parameters/responses/security. Write
per-endpoint docs under `.openapi/endpoints/{method}-{path-slug}.md`.
Preserve manual additions when updating existing files. Report totals.

## Before Session Ends

If the session is ending:

1. Pause the active spec (run full pause workflow)
2. Make sure every completed task has the right `data-status` and any
   completed RGR cycles have a TDD Log entry
3. Confirm to the user that progress was saved

## Directory Layout

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

**SPEC.html `<script id="spec-meta">` JSON is authoritative.** The registry is a denormalized
index for quick lookups. Always update both together. If they conflict,
SPEC.html wins.

## Canonical Output Templates

Use these concise formats consistently:

**Resume**
```
Resuming: <Title> (<id>)
Progress: <done>/<total> tasks
RGR cycles: <done>/<total>
Phase: <phase name>
Current: [<TEST|IMPL>-<PREFIX>-<NN>] <task text>
TDD Phase: RED | GREEN | REFACTOR
Last cycle: [<TASK-CODE>] <state> — <test result>
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

1. Set status to `archived` in `<script id="spec-meta">` JSON and registry
2. Research files can optionally be deleted (SPEC.html has all decisions)

Specs can be archived from `completed` or `paused` status.

## Deleting a Spec

1. Delete `.specs/<id>/` directory
2. Remove the row from `.specs/registry.md`

This is irreversible — consider archiving instead.

## Kluris Companion Compatibility

Kluris embeds this playbook as the `specmint-tdd-html` companion. Read and
follow it directly from `~/.kluris/companions/specmint-tdd-html/SKILL.md`.
Paths in this file that start with `references/` or `assets/` resolve
against `~/.kluris/companions/specmint-tdd-html/`. `.specs/...` paths, and
the relative `../assets/` links inside a `SPEC.html`, are project paths. Do
not ask the user to install specmint separately. `SPEC.html` stays plain
HTML with a JSON metadata blob: open it in any browser, edit it in any text
editor, and keep it diff-friendly with the conventions in
`references/spec-format.md`.

## Behavioral Notes

**Be proactive about spec management.** If you notice the user has made
progress, update the spec without being asked. If a session is ending,
offer to pause and save context.

**Specs should evolve.** It's fine to add tasks, reorder phases, or split
a phase as understanding deepens.

**The Decision Log matters.** Log non-obvious technical choices with
rationale. Future-you resuming this spec will thank present-you.

**The TDD Log matters.** It is the audit trail proving red-green-refactor
discipline. Fill it in as you go — not retroactively.

**Don't over-structure.** A spec with 3 phases and 15 tasks is useful. A
spec with 12 phases and 80 tasks is a project plan, not a coding spec.

**Respect the user's flow.** Don't interrupt deep coding work to update
the spec. Batch updates for natural pauses.
