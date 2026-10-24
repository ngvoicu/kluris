---
name: specmint-tdd
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

# Spec Mint TDD

Turn ephemeral plans into structured, persistent specs built through deep
research and iterative interviews — with strict test-driven development at
every step. Every task starts with a failing test, production code exists
only to make tests pass, and refactoring happens under green. Tests use
testcontainers for real services, mock only at boundaries, and make no
external network calls. Specs have phases, tasks, acceptance criteria, a
registry, resume context, a decision log, a TDD log, and a deviations
log. They live in `.specs/` at the project root and work with any AI
coding tool that can read markdown.

Whether `.specs/` is committed is repository policy. Respect `.gitignore`
and the user's preference for tracked vs local-only spec state.

## Critical Invariants

1. **Single-file policy**: Keep this workflow in one `SKILL.md` file.
2. **Canonical paths**:
   - Registry: `.specs/registry.md`
   - Per-spec files: `.specs/<id>/SPEC.md`, `.specs/<id>/research-*.md`,
     `.specs/<id>/interview-*.md`
3. **Authority rule**: `SPEC.md` frontmatter is authoritative. Registry is a
   denormalized index for quick lookup.
4. **Active-spec rule**: Target exactly one active spec at a time.
5. **Parser policy**: Use best-effort parsing with clear warnings and repair
   guidance instead of hard failure on malformed rows.
6. **TDD invariant**: No production code without a failing test. The implement
   workflow enforces red-green-refactor at every task. Tests are executed via
   the actual test runner, not assumed to pass. This is sacred — see "Tests
   Are Sacred" and "Blocking Rule" in the implement section.
7. **Progress tracking is sacred**: After completing any task, immediately
   update SPEC.md (checkbox, `← current` marker, phase marker, TDD log)
   AND registry.md (progress count, date). Then re-read both files to
   verify the edits landed correctly. Never move to the next task without
   updating both files. Never end a session with the registry out of sync
   with SPEC.md. This is non-negotiable — if you do nothing else, do this.

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
| Malformed registry row | Skip malformed row, emit warning with row text, continue parsing remaining rows. |
| Multiple `active` rows | Warn user. Pick the row with the newest `Updated` date (or first active row if dates are unavailable) for this run. On next write, normalize to a single active spec. |
| Registry row exists but `.specs/<id>/SPEC.md` missing | Warn and continue. Keep row visible in list/status with `(SPEC.md missing)`. |
| Registry and SPEC conflict | Trust `SPEC.md`, then repair registry values on next write. |
| No active spec | List available specs and ask which to activate or resume. |

## Working on a Spec

### Resuming

When the user says "resume", "what was I working on", or similar:

1. Read `.specs/registry.md` — find the spec with `active` status. If none, list specs and ask which to resume
2. Load `.specs/<id>/SPEC.md`
3. Parse progress:
   - Count completed `[x]` vs total tasks per phase
   - Find current phase (first `[in-progress]` phase)
   - Find current task (`← current` marker, or first unchecked in current phase)
4. Read the **Resume Context** section
5. Present a compact summary:

   ```
   Resuming: User Auth System
   Progress: 5/12 tasks (Phase 2: OAuth Integration)
   Current: [TEST-AUTH-07] Write OAuth callback tests
   TDD Phase: RED (next: write test, run, confirm fail)
   Last Cycle: [IMPL-AUTH-06] GREEN — 4/4 pass
   Context: Finished Google OAuth (TEST-05 red, IMPL-06 green+refactored).
   Starting GitHub OAuth callback tests next.
   Next file: tests/auth/test_oauth_github.py
   ```

6. Begin working on the current task — don't wait for permission

### Implementing a Spec (TDD)

When the user says "implement the spec", "implement phase N", "implement all
phases", or similar:

#### Scope Detection

Parse the user's request to determine scope:
- **"implement the spec"** or **"implement"** — Start from the current task
  (the `← current` marker) and work forward
- **"implement phase N"** or **"implement phase <name>"** — Implement all
  tasks in that specific phase
- **"implement all phases"** or **"implement everything"** — Implement all
  remaining unchecked tasks across all phases, in order

#### TDD Implementation Flow

1. Read `.specs/registry.md` to find the active spec
2. Load `.specs/<id>/SPEC.md` and parse phases/tasks
3. Identify the target tasks based on scope
4. For each task in order, determine if it is a TEST or IMPL task by its
   task code prefix (`TEST-` vs `IMPL-`)
5. Tasks within a phase alternate: TEST then IMPL, TEST then IMPL. Each
   TEST-IMPL pair is one red-green-refactor cycle.

**For each TEST-IMPL pair (one red-green-refactor cycle):**

**RED — TEST task:**
1. Mark the TEST task with `← current`
2. Write the test file at the path specified in the task, with the assertions
   and test descriptions listed
3. **GATE: RUN the tests via Bash.** Do not proceed without running them.
4. **GATE: Confirm tests FAIL.** If all tests fail — good, this is RED.
5. If tests pass unexpectedly: **STOP.** Report the anomaly to the user.
   Do not proceed. Either the feature already exists or the tests are wrong.
6. Log the red output in the TDD Log using this exact format:
   `| [TEST-XX-NN] | <command>: N tests, N failed — <key failure message> | — | — |`
7. Check off the task: `- [ ]` -> `- [x]`, remove `← current`

**GREEN — IMPL task:**
1. Mark the IMPL task with `← current`
2. Read the test file from the preceding TEST task — understand exactly what
   the tests expect
3. Write the minimum production code to make the tests pass. Only what the
   tests require. Nothing more.
4. **GATE: RUN the tests via Bash.** Do not proceed without running them.
5. **GATE: Confirm tests PASS.** If any test fails, fix the production code
   and run again. Repeat until green. Do not modify tests to make them pass
   (see Tests Are Sacred below).
6. Log the green output in the TDD Log:
   `| [IMPL-XX-NN] | — | <command>: N passed, 0 failed | — |`

**REFACTOR — still on the IMPL task:**
1. Clean up the production code — remove duplication, improve naming, extract
   helpers
2. **GATE: RUN tests again via Bash.** Confirm they are still green.
3. If refactoring broke tests: undo the refactoring, try a different approach.
4. Log refactoring in the TDD Log Refactor column (or "none")
5. Check off the IMPL task: `- [ ]` -> `- [x]`, remove `← current`
6. Update progress and date in `.specs/registry.md`

**Then move to the next TEST-IMPL pair and repeat.**

#### Self-Check Before Every Task

Before starting any task, pause and verify:

- [ ] Am I about to write production code? → Is there a failing test for it?
- [ ] Am I about to skip running tests? → Run them. Always.
- [ ] Am I about to modify a test to make it pass? → Stop. Fix the code instead.
- [ ] Did I log the last task's output in the TDD Log? → Do it now.
- [ ] Did I update the checkbox and `← current` marker? → Do it now.

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
interleaved TEST-IMPL pairs. When all tasks in a phase are done, the phase
is `[completed]` and the next phase becomes `[in-progress]`.

#### Update Transaction (sacred — never skip steps)

Progress tracking is sacred (see Critical Invariant #7). Stale tracking
is the single most common failure mode — it makes resume unreliable and
the registry useless.

1. Edit `SPEC.md` (checkbox, current marker, phase marker, resume context,
   TDD log entry).
2. Recompute progress directly from `SPEC.md` checkboxes.
3. Edit the matching registry row (status, progress, updated date).
4. **Verify**: Re-read both `SPEC.md` and `registry.md` to confirm the
   edits are correct. If the registry progress doesn't match the SPEC.md
   checkbox count, fix it now.
5. If registry update fails, keep `SPEC.md` as source of truth and emit a
   warning with exact repair action for `.specs/registry.md`.

**If you notice you forgot to update after a previous task, stop what
you're doing and update now before continuing.**

Also:
- If a task is more complex than expected, split it into subtasks
- Update resume context at natural pauses
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
  - Verify all Acceptance Criteria are checked off. If any remain unchecked,
    report which ones and ask the user before marking the spec complete.
  - Set all phases to `[completed]`
  - Set spec status to `completed` in frontmatter and registry
  - Update the `updated` date
  - Present a summary of what was implemented, including TDD Log highlights
  - Suggest next spec to activate if any are paused

- **Only a phase completed (not all tasks):**
  - **Run tests** for the phase's scope and show the output (verification gate)
  - Report the phase completion and remaining work
  - Set the next phase to `[in-progress]` if applicable
  - State whether the next phase is TEST or IMPL

### Pausing

When the user says "pause", switches specs, or a session is ending:

0. If there is no active spec, report that there is nothing to pause and stop.
1. Capture what was happening:
   - Which task was in progress
   - What files were being modified (paths, function names)
   - Key decisions made this session
   - Any blockers or open questions
   - Current TDD phase (RED, GREEN, or REFACTOR)
   - Tests written but not yet satisfied by production code
   - Last test run output (command and result summary)
2. Write this to the **Resume Context** section in SPEC.md
3. Update checkboxes to reflect actual progress
4. Move `← current` marker to the right task
5. Add any session decisions to the **Decision Log**
6. Update `status: paused` in frontmatter
7. Update the `updated` date

**Resume Context is the most important part of pausing.** Write it as if
briefing a colleague who will pick up tomorrow. Include specific file paths,
function names, the exact next step, and the TDD state. Vague context like
"was working on auth" is useless — write "implementing `verifyRefreshToken()`
in `src/auth/tokens.ts` to satisfy `TEST-AUTH-03`. Tests in
`tests/auth/test_tokens.py` are RED — 2 of 3 assertions failing on refresh
rotation. Next step: hook up rotation logic to the `/auth/refresh` endpoint."

### Switching Between Specs

1. Validate the target spec ID first. If missing, list available specs.
2. Confirm `.specs/<target-id>/SPEC.md` exists. If not, stop with an error.
3. If target is already active, report and stop.
4. Pause the current active spec if one exists (full pause workflow).
5. Set target status to `active` in frontmatter and in `.specs/registry.md`.
6. Resume the target spec (full resume workflow).

## Spec Format

### Frontmatter

YAML frontmatter with: `id`, `title`, `status`, `created`, `updated`,
optional `priority` and `tags`.

Status values: `active`, `paused`, `completed`, `archived`

### Phase Markers

`[pending]`, `[in-progress]`, `[completed]`, `[blocked]`

### Task Markers

- `- [ ] [TEST-CODE-01]` unchecked test task, `- [x] [TEST-CODE-01]` done
- `- [ ] [IMPL-CODE-02]` unchecked impl task, `- [x] [IMPL-CODE-02]` done
- Test task codes: `[TEST-PREFIX-NN]` — for tasks that write failing tests
- Impl task codes: `[IMPL-PREFIX-NN]` — for tasks that write production code
  to satisfy tests. Each impl task includes `-> satisfies [TEST-XX-NN]`
  referencing the test task it makes pass.
- Tasks alternate within each phase: TEST, IMPL, TEST, IMPL. Each TEST-IMPL
  pair is one red-green-refactor cycle.
- Prefix is a short (2-4 letter) uppercase abbreviation of the spec
  (e.g., `user-auth-system` -> `AUTH`). Numbers auto-increment continuously
  across all phases starting at `01`.
- `<- current` after the task text marks the active task
- `[NEEDS CLARIFICATION]` after the task code on unclear tasks
- Each test task specifies: file path, test descriptions, and isolation
  strategy (testcontainers, mocks at boundaries, in-memory where appropriate)
- Each impl task specifies: file path, function/class names, and which test
  task it satisfies

### Acceptance Criteria

Testable conditions that define when the spec is "done". Written during
forge, verified after each phase completes. Format: checkboxes with
specific, verifiable statements — not vague goals.

```markdown
## Acceptance Criteria

- [ ] Users can sign in with Google OAuth and receive a JWT
- [ ] Expired tokens return 401 with `{"error": "token_expired"}`
- [ ] Refresh tokens rotate on each use (old token is invalidated)
- [ ] All auth paths have corresponding red-green-refactor cycles in TDD Log
```

Check off criteria as they are satisfied during implementation. At phase
completion, review which acceptance criteria are now met. At spec
completion, all criteria must be checked — if any remain unchecked, the
spec is not done.

### Testing Architecture

Specs include a **Testing Architecture** section specifying:
- Test framework and runner (e.g., pytest, vitest, JUnit 5, cargo test)
- Test command (the exact command to run tests)
- Isolation strategy: testcontainers for real databases/services, mocks only
  at system boundaries (HTTP clients, third-party APIs), no external network
  calls in tests
- E2E testing approach:
  - **Backend e2e**: full API request lifecycle (HTTP in → middleware → handler
    → service → DB → response) using testcontainers for real databases and
    mocked external APIs (MSW, WireMock). No browser needed.
  - **Frontend e2e**: browser-based user flows (Playwright, Cypress) when
    applicable
  - All e2e tests run in isolation — no external network calls
- Coverage targets (line and branch minimums)
- Test directory structure and naming conventions
- Special setup requirements (fixtures, factories, seed data)

### TDD Log

A markdown table tracking the red-green-refactor cycle for every task:

```markdown
## TDD Log

| Task | Red | Green | Refactor |
|------|-----|-------|----------|
| [TEST-AUTH-01] | 3 tests fail: `Cannot find module './jwt'` | — | — |
| [IMPL-AUTH-02] | — | 3/3 pass after implementing JwtService | Extracted token config to constants |
| [TEST-AUTH-03] | 2 tests fail: `Expected rotated token` | — | — |
| [IMPL-AUTH-04] | — | 5/5 pass | Renamed `createToken` -> `issueAccessToken` |
```

The TDD Log is an audit trail proving discipline was followed. It is filled
in during implementation, not during forging.

### Resume Context

Blockquote section with specific file paths, function names, exact next step,
and TDD state (current phase, failing tests, last run output). This is what
makes cross-session continuity work.

### Decision Log

Markdown table with date, decision, and rationale columns. Log non-obvious
technical choices (library selection, architecture pattern, API design).

### Deviations

Markdown table tracking where implementation diverged from the spec:
task, what the spec said, what was actually done, and why. Only log
changes that would surprise someone comparing the spec to the code.

### SPEC.md Template

Use this skeleton when creating new specs.
`references/spec-format.md` has the full template with examples.

```markdown
---
id: <spec-id>
title: <Human Readable Title>
status: active
created: <YYYY-MM-DD>
updated: <YYYY-MM-DD>
priority: high | medium | low
tags: [<tag1>, <tag2>]
---

# <Title>

## Overview
<2-4 sentences: what and why>

## Acceptance Criteria
- [ ] <Testable condition 1>
- [ ] <Testable condition 2>
- [ ] <Testable condition 3>

## Architecture
<ASCII art or Mermaid diagram>

## Testing Architecture

### Test Framework & Tools
| Tool | Choice | Version | Purpose |
|------|--------|---------|---------|
| Test framework | <name> | <ver> | Unit and integration tests |
| Mocking library | <name> | <ver> | Dependency isolation |
| DB testing | <name> | <ver> | Real database tests |

### Isolation Strategy
| Layer | Approach | Services |
|-------|----------|----------|
| Domain logic | No mocks; pure functions | None |
| Service layer | Mock ports/interfaces | <deps> |
| Data access | Testcontainers | <DB/cache> |
| HTTP clients | MSW / WireMock | <external APIs> |

### Coverage Targets
| Metric | Target |
|--------|--------|
| Line coverage | <XX%> |
| Branch coverage | <XX%> |

### Test Commands
| Command | Purpose |
|---------|---------|
| `<test command>` | Run all tests |

## Library Choices
| Need | Library | Version | Alternatives | Rationale |
|------|---------|---------|-------------|-----------|

## Phase 1: <Feature A> [in-progress]
- [ ] [TEST-XX-01] <test task with file path, assertions, isolation> <- current
- [ ] [IMPL-XX-02] <impl task> -> satisfies [TEST-XX-01]
- [ ] [TEST-XX-03] <test task>
- [ ] [IMPL-XX-04] <impl task> -> satisfies [TEST-XX-03]

## Phase 2: <Feature B> [pending]
- [ ] [TEST-XX-05] <test task>
- [ ] [IMPL-XX-06] <impl task> -> satisfies [TEST-XX-05]

CRITICAL: Phases group by FEATURE, not by test-vs-impl. Every phase
has interleaved TEST-IMPL pairs. A phase containing only TEST tasks
or only IMPL tasks is WRONG. A phase named "Tests for X (TEST)" or
"Implement X (IMPL)" is WRONG. Correct: "Phase 1: Auth Foundation"
with TEST-01, IMPL-02, TEST-03, IMPL-04 alternating inside it.

---

## Resume Context
> <TDD Phase, failing tests, last run, next step, file paths>

## Decision Log
| Date | Decision | Rationale |
|------|----------|-----------|

## TDD Log
| Task | Red | Green | Refactor |
|------|-----|-------|----------|

## Deviations
| Task | Spec Said | Actually Did | Why |
|------|-----------|-------------|-----|
```

## Forging Specs

When asked to forge, plan, spec out, or "write a spec for X", follow the
full forge workflow: setup, research deeply, interview the user, iterate
until clear, then write the spec.

**Plan mode:** If your tool is in a read-only plan mode, do not run forge in
that mode. Ask the user to leave it (in Claude Code: Shift+Tab) and start the
forge workflow again.

**The forge workflow never produces application code.** Its outputs are only
`.specs/` files: research notes, interview notes, and the SPEC.md.

### Step 1: Setup

1. Generate a spec ID from the title (lowercase, hyphenated):
   `"User Auth System"` -> `user-auth-system`
2. **Collision check**: If `.specs/<id>/SPEC.md` already exists or the ID
   appears in `.specs/registry.md`, warn the user and ask:
   - **Resume** the existing spec
   - **Rename** the new spec (suggest `<id>-v2` or ask for a new title)
   - **Archive** the old spec and create a new one in its place
   Do not proceed until the user chooses.
3. Initialize directories: `mkdir -p .specs/<id>`
4. If `.specs/registry.md` doesn't exist, initialize it with the header row.

### Step 2: Deep Research

Research is the foundation of a good spec. Be exhaustive — use every
available resource so the spec won't need revision mid-build.

Research runs on two parallel tracks:

#### Track A: Spawn a Research Subagent

**If your tool supports subagents:** Spawn a research subagent with your
tool's subagent/task feature (e.g. Claude Code's Agent tool), using the brief
in `references/researcher.md`. Provide: the user's request, spec ID, output
path `.specs/<id>/research-01.md`, and any Context7 findings from Track B.
The research subagent maps the project architecture, reads 15-30 files, runs
3+ web searches, compares library candidates, assesses risks, and analyzes the
full test infrastructure.

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

Synthesize all research and interviews into a SPEC.md using the template
above. The spec should include:

- YAML frontmatter, Overview
- **Acceptance Criteria** — Testable checkbox conditions defining "done".
  Derived from interview answers. Check them off during implementation.
- Architecture Diagram
- **Testing Architecture** (mandatory) — framework, isolation strategy,
  coverage targets, test commands, anti-patterns
- **Library Choices** — comparison table with rationale
- **Feature phases** with interleaved TEST-IMPL task pairs (red-green-refactor
  per pair, not batched). Phases are named by feature ("Auth Foundation"),
  NEVER by test-vs-impl ("Tests for Auth" / "Implement Auth"). No `(TEST)`
  or `(IMPL)` suffixes on phase names. Every phase contains both TEST and
  IMPL tasks alternating.
- **Tasks** with `[TEST-PREFIX-NN]` and `[IMPL-PREFIX-NN]` codes alternating
  within each phase
- TDD Log (empty), Resume Context, Decision Log, Deviations table

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

Save to `.specs/<id>/SPEC.md`. Update `.specs/registry.md` — set status
to `active`. Mark first phase `[in-progress]`, first task `← current`.

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
2. Write detailed resume context
3. Confirm to the user that context was saved

## Directory Layout

```
.specs/
├── registry.md               # Denormalized index for status/progress lookups
└── <spec-id>/
    ├── SPEC.md               # The spec document
    ├── research-01.md        # Deep research findings
    ├── interview-01.md       # Interview notes
    └── ...
```

## Registry Format

`.specs/registry.md` is a simple markdown table:

```markdown
# Spec Registry

| ID | Title | Status | Priority | Progress | Updated |
|----|-------|--------|----------|----------|---------|
| user-auth-system | User Auth System | active | high | 5/12 | 2026-02-10 |
| api-refactor | API Refactoring | paused | medium | 2/8 | 2026-02-09 |
```

**SPEC.md frontmatter is authoritative.** The registry is a denormalized
index for quick lookups. Always update both together. If they conflict,
SPEC.md wins.

## Canonical Output Templates

Use these concise formats consistently:

**Resume**
```
Resuming: <Title> (<id>)
Progress: <done>/<total> tasks
Phase: <phase name>
Current: <task text>
TDD Phase: RED | GREEN | REFACTOR
Failing Tests: <count and names>
Last Test Run: <result>
Context: <one to three lines from Resume Context>
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

## Completing a Spec

1. Verify all tasks are checked (warn if not, but allow override)
2. Set status to `completed` in frontmatter and registry
3. Update the `updated` date in both
4. Suggest next spec to activate if any are paused

## Archiving a Spec

1. Set status to `archived` in frontmatter and registry
2. Research files can optionally be deleted (SPEC.md has all decisions)

Specs can be archived from `completed` or `paused` status.

## Deleting a Spec

1. Delete `.specs/<id>/` directory
2. Remove the row from `.specs/registry.md`

This is irreversible — consider archiving instead.

## Kluris Companion Compatibility

Kluris embeds this playbook as the `specmint-tdd` companion. Read and
follow it directly from `~/.kluris/companions/specmint-tdd/SKILL.md`. Paths
in this file that start with `references/` resolve against
`~/.kluris/companions/specmint-tdd/`. `.specs/...` paths are project paths.
Do not ask the user to install specmint separately. Specs remain plain
markdown/YAML files that humans can edit and Git can diff.

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
