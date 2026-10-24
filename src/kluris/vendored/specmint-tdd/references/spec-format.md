# SPEC.md Format Reference (TDD)

This is the complete format specification for TDD-first spec documents. Use
this as the template when creating new specs. Every spec follows a strict
test-before-implementation structure: write tests first, then implement to
satisfy them.

## Full Template

```markdown
---
id: <spec-id>
title: <Human Readable Title>
status: active | paused | completed | archived
created: <YYYY-MM-DD>
updated: <YYYY-MM-DD>
priority: high | medium | low
tags: [<tag1>, <tag2>]
---

# <Title>

## Overview

<2-4 sentences describing what this spec accomplishes and why. Include
enough context that someone resuming cold can understand the goal without
reading the full conversation history.>

## Acceptance Criteria

- [ ] <When X happens, the system does Y>
- [ ] <The Z component returns W with status code N>
- [ ] <Users can perform action A and see result B>
- [ ] <All auth paths have corresponding red-green-refactor cycles in TDD Log>
- [ ] [NEEDS CLARIFICATION] <Ambiguous criterion that needs discussion>

Testable conditions that define when the spec is "done". Each criterion
is a checkbox — checked off during implementation as it becomes satisfied.
Be specific and verifiable: "returns 401 with error body" not "handles
errors correctly". At spec completion, all criteria must be checked.
Skip for trivial bug fixes, but features always need them — they prevent
scope creep and make verification unambiguous.

## Architecture

<Include at least one diagram showing the system design. Use ASCII art for
simple flows and Mermaid for complex relationships.>

### System Diagram

```
Client --> API Gateway --> Auth Middleware --> Route Handler --> Database
                                                 |
                                            Cache Layer
```

<Or use Mermaid for more complex diagrams:>

```mermaid
graph TD
    A[Client] --> B[API Gateway]
    B --> C{Auth Middleware}
    C -->|Authenticated| D[Route Handler]
    C -->|Unauthorized| E[401 Response]
    D --> F[(Database)]
    D --> G[Cache]
```

<For data models, use ER diagrams:>

```mermaid
erDiagram
    USER ||--o{ ORDER : places
    ORDER ||--|{ LINE_ITEM : contains
    PRODUCT ||--o{ LINE_ITEM : "ordered in"
```

<For state machines and workflows:>

```mermaid
stateDiagram-v2
    [*] --> Draft
    Draft --> Review
    Review --> Approved
    Review --> Rejected
    Approved --> Published
    Rejected --> Draft
```

## Testing Architecture

### Test Framework & Tools

| Tool | Choice | Version | Purpose |
|------|--------|---------|---------|
| Test framework | <Vitest/Jest/pytest/JUnit 5/etc.> | <ver> | Unit and integration test runner |
| Mocking library | <MSW/vi.mock/pytest-mock/Mockito/etc.> | <ver> | Dependency isolation |
| DB testing | <Testcontainers/test DB/etc.> | <ver> | Real database integration tests |
| HTTP mocking | <MSW/WireMock/VCR/etc.> | <ver> | External API isolation |
| Coverage tool | <v8/istanbul/coverage.py/JaCoCo/etc.> | <ver> | Code coverage reporting |
| Mutation testing | <Stryker/Pitest/mutmut/etc.> | <ver> | Test quality verification |
| E2E (backend) | <Supertest/httpx/MockMvc/httptest/etc.> | <ver> | Full API request lifecycle tests |
| E2E (frontend) | <Playwright/Cypress/etc.> | <ver> | Browser-based user flow tests |

### Isolation Strategy

| Layer | Isolation Approach | Services |
|-------|--------------------|----------|
| Domain / Business logic | No mocks; pure functions | None |
| Service / Use cases | Mock ports (interfaces) | <list injected deps> |
| Repository / Data access | Testcontainers | <PostgreSQL/Redis/etc.> |
| HTTP clients | MSW / WireMock | <list external APIs> |
| API handlers | Supertest / httptest | Full handler stack |
| E2E (backend) | Testcontainers + mocked externals | Full request lifecycle: HTTP → middleware → handler → service → DB → response |
| E2E (frontend) | Playwright / Cypress | Browser-based user flows |

### Coverage Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Line coverage | <XX%> | Measured by <tool> |
| Branch coverage | <XX%> | Measured by <tool> |
| Mutation score | <XX%> | Measured by <tool>; focus on critical paths |

### Test Commands

| Command | Purpose |
|---------|---------|
| `<npm test / pytest / go test>` | Run all unit tests |
| `<npm run test:integration>` | Run integration tests |
| `<npm run test:coverage>` | Generate coverage report |
| `<npm run test:mutation>` | Run mutation testing |
| `<npm run test:e2e>` | Run backend end-to-end tests |
| `<npx playwright test>` | Run browser end-to-end tests (if applicable) |

### Anti-Patterns to Avoid

- <List specific anti-patterns relevant to this project's stack>
- <e.g., No SQLite as PostgreSQL stand-in; use Testcontainers>
- <e.g., No mocking internal modules; mock at port boundaries>
- <e.g., No sleep-based waits; use polling assertions>

## Library Choices

| Need | Library | Version | Alternatives Considered | Rationale |
|------|---------|---------|------------------------|-----------|
| <need> | <chosen> | <ver> | <alt1>, <alt2> | <why this one> |

## Phase 1: <Feature A> [in-progress]

- [x] [TEST-XX-01] <Write tests for component/function Y>
- [x] [IMPL-XX-02] <Implement Y> -> satisfies [TEST-XX-01]
- [ ] [TEST-XX-03] <Write tests for edge case Z> <- current
- [ ] [IMPL-XX-04] <Handle edge case Z> -> satisfies [TEST-XX-03]
- [ ] [TEST-XX-05] <Write integration tests for flow W>
- [ ] [IMPL-XX-06] <Wire up integration flow W> -> satisfies [TEST-XX-05]

## Phase 2: <Feature B> [pending]

- [ ] [TEST-XX-07] <Write tests for ...>
- [ ] [IMPL-XX-08] <Implement ...> -> satisfies [TEST-XX-07]
- [ ] [TEST-XX-09] <Write tests for ...>
- [ ] [IMPL-XX-10] <Implement ...> -> satisfies [TEST-XX-09]

---

## Resume Context

> <Detailed description of where you left off. Include:>
> - What was just completed
> - What's currently in progress
> - TDD Phase: RED | GREEN | REFACTOR
> - Failing Tests: <list of currently failing test names, or "none">
> - Last Test Run: `<exact command>` at <time>, <pass/fail count>
> - Specific file paths and function/component names
> - The exact next step to take
> - Any blockers or open questions
>
> Write this as if briefing a colleague picking up your work tomorrow.

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| <YYYY-MM-DD> | <What was decided> | <Why this choice was made> |

## TDD Log

| Task | Red (Failure Output) | Green (Pass Output) | Refactor (Changes) |
|------|---------------------|--------------------|--------------------|
| [TEST-XX-01] | `Expected: 201, Received: undefined` | — | — |
| [IMPL-XX-02] | — | `3 passed, 0 failed` | Extracted validation to `validateInput()` |
| [TEST-XX-03] | `Expected: 400, Received: undefined` | — | — |
| [IMPL-XX-04] | — | `5 passed, 0 failed` | — |

## Deviations

| Task | Spec Said | Actually Did | Why |
|------|-----------|-------------|-----|
| <Task name> | <What was planned> | <What was implemented> | <Reason for the change> |
```

## Field Definitions

### Frontmatter Fields

| Field | Required | Description |
|-------|----------|-------------|
| `id` | Yes | URL-safe identifier, lowercase hyphenated (e.g., `user-auth-system`) |
| `title` | Yes | Human-readable name |
| `status` | Yes | One of: `active`, `paused`, `completed`, `archived` |
| `created` | Yes | ISO date when spec was created |
| `updated` | Yes | ISO date of last modification |
| `priority` | No | `high`, `medium`, or `low` (default: medium) |
| `tags` | No | YAML array of categorization tags |

### Phase Status Markers

Phase headings include a status marker in square brackets:

| Marker | Meaning |
|--------|---------|
| `[pending]` | Not started yet |
| `[in-progress]` | Currently being worked on |
| `[completed]` | All tasks done |
| `[blocked]` | Waiting on something external |

Only **one phase** should be `[in-progress]` at a time (though this isn't
strictly enforced — sometimes parallel phases make sense).

### Phase Structure (TDD)

Phases group by **feature area**, not by test vs implementation. Each phase
contains interleaved TEST-IMPL task pairs — each pair is one red-green-refactor
cycle:

```markdown
## Phase 1: Auth Foundation [in-progress]
- [x] [TEST-AUTH-01] Write tests for JWT verification
- [x] [IMPL-AUTH-02] Implement JWT verification -> satisfies [TEST-AUTH-01]
- [ ] [TEST-AUTH-03] Write tests for token refresh <- current
- [ ] [IMPL-AUTH-04] Implement token refresh -> satisfies [TEST-AUTH-03]
```

This is true TDD: write one test (RED), make it pass (GREEN), refactor,
then write the next test. Not batched.

**ANTI-PATTERN — separate TEST and IMPL phases:**

The following structure is WRONG and must never be produced:

```markdown
## Phase 1: Tests for Auth (TEST)        <-- WRONG
- [ ] [TEST-AUTH-01] Write tests for JWT
- [ ] [TEST-AUTH-02] Write tests for refresh
## Phase 2: Implement Auth (IMPL)        <-- WRONG
- [ ] [IMPL-AUTH-03] Implement JWT
- [ ] [IMPL-AUTH-04] Implement refresh
```

This violates TDD because all tests are written before any implementation.
If you see this pattern, merge the phases: one "Auth Foundation" phase
with TEST-01, IMPL-02, TEST-03, IMPL-04 alternating inside. Phase names
must be feature names with no `(TEST)` or `(IMPL)` suffixes.

### Task Format (TDD)

Tasks use markdown checkboxes with a typed task code prefix:

```markdown
- [ ] [TEST-XX-01] Write tests for user creation <- current
- [ ] [IMPL-XX-02] Implement user creation -> satisfies [TEST-XX-01]
- [ ] [TEST-XX-03] Write tests for user deletion
- [ ] [IMPL-XX-04] Implement user deletion -> satisfies [TEST-XX-03]
```

**Task codes** use the format `<TYPE>-<PREFIX>-<NN>`:

- **Type**: `TEST` for test-writing tasks, `IMPL` for implementation tasks.
- **Prefix**: A short (2-4 letter) uppercase abbreviation of the spec.
  Pick the most recognizable word or use initials:
  - `user-auth-system` -> `AUTH`
  - `api-refactor` -> `API`
  - `fix-upload-bug` -> `UPL`
- **Number**: Two-digit, auto-incrementing across ALL phases (not per-phase).
  Start at `01`. Tasks alternate TEST, IMPL, TEST, IMPL within each phase.

IMPL tasks include a `-> satisfies [TEST-XX-NN]` reference linking back to
the test task they satisfy. Each IMPL task immediately follows its TEST task.

The `<- current` marker indicates which task the AI should work on next.
Only one task should have this marker at a time.

**Task granularity**: Each task should represent roughly one focused work
session (30 min to 2 hours of work). If a task feels like it would take a
full day, break it into subtasks.

### Uncertainty Markers

Use `[NEEDS CLARIFICATION]` for any requirement or task where ambiguity
remains after interviews:

```markdown
- [ ] [TEST-AUTH-05] [NEEDS CLARIFICATION] Write tests for rate limiting behavior
```

These markers signal that the task needs more discussion before
implementation. Don't start a task with this marker — resolve it first
(run another interview round or ask the user directly).

### Testing Architecture

The Testing Architecture section is placed after Architecture and before
Library Choices. It documents the full testing stack and strategy:

1. **Test Framework & Tools** — Table of every testing tool with version
   and purpose. Covers runners, mocking, DB testing, HTTP mocking, coverage,
   mutation testing, and E2E.
2. **Isolation Strategy** — Table mapping each architectural layer to its
   isolation approach and the services it interacts with.
3. **Coverage Targets** — Line coverage, branch coverage, and mutation
   score targets with measuring tools.
4. **Test Commands** — Exact CLI commands for running each test category.
5. **Anti-Patterns to Avoid** — Project-specific anti-patterns based on
   the stack (reference `testing-knowledge.md` for the full catalog).

### Resume Context (TDD)

The Resume Context section is freeform markdown inside a blockquote. It
should answer these questions:

1. **What just happened?** — What was completed in the last session
2. **What's the current state?** — Which files changed, what works, what doesn't
3. **TDD Phase** — Are you in RED (writing failing tests), GREEN (making
   tests pass), or REFACTOR (cleaning up while tests stay green)?
4. **Failing Tests** — List currently failing test names with brief failure reasons
5. **Last Test Run** — Exact command, timestamp, pass/fail count
6. **What's next?** — The specific next action to take
7. **Where exactly?** — File paths, function names, line ranges
8. **Any gotchas?** — Blockers, open questions, things that didn't work

**Good example:**
```markdown
> Just completed the TEST-AUTH-03 / IMPL-AUTH-04 cycle (token refresh).
> Tests pass, code refactored. Now starting the next cycle.
>
> Last Cycle: [IMPL-AUTH-04] GREEN — 5/5 pass, refactored token naming
> Current: [TEST-AUTH-05] Write OAuth callback tests
> TDD Phase: RED (about to write test, run, confirm fail)
> Last Test Run: `npm test -- --grep auth` at 14:32, 5 passed / 0 failed
>
> Next step: Write OAuth callback test in
> `tests/unit/auth/oauth.test.ts`. Test: valid code exchange, expired
> state param, invalid provider. Mock Google API with MSW.
>
> Key file: `tests/unit/auth/oauth.test.ts` (create — writing test)
> Key file: `src/auth/oauth.ts` (doesn't exist yet — will implement in IMPL-AUTH-06)
```

**Bad example:**
```markdown
> Was working on authentication tests. Made some progress.
```

### Decision Log

Track non-obvious technical decisions so you (or another AI session) can
understand why things are the way they are:

```markdown
| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-02-10 | Vitest over Jest | Faster execution, native ESM, same project uses Vite |
| 2026-02-10 | Testcontainers over test DB | Hermetic tests; no shared state between CI runs |
| 2026-02-11 | MSW over nock | MSW intercepts at network level; works in browser and Node |
```

Good decisions to log:
- Library/framework choices (especially testing tools)
- Architecture patterns
- Testing strategy decisions (isolation boundaries, what to mock)
- API design decisions
- Trade-offs made (speed vs fidelity, coverage vs maintenance)
- Things you tried and rejected (and why)

### TDD Log

Track the red-green-refactor cycle for each task. This creates an audit
trail showing that TDD discipline was followed:

```markdown
| Task | Red (Failure Output) | Green (Pass Output) | Refactor (Changes) |
|------|---------------------|--------------------|--------------------|
| [TEST-AUTH-01] | `Expected: 201, Received: undefined` (3 fail) | — | — |
| [IMPL-AUTH-02] | — | `3 passed, 0 failed` | Extracted shared fixtures to `test/helpers/auth.ts` |
| [TEST-AUTH-03] | `Expected rotated token, got same` (2 fail) | — | — |
| [IMPL-AUTH-04] | — | `5 passed, 0 failed` | Renamed `createToken` -> `issueAccessToken` |
```

Each TEST-IMPL pair forms one red-green-refactor cycle:
- **TEST task**: Record RED output (expected failure). GREEN comes from the next IMPL task.
- **IMPL task**: Record GREEN (tests passing) and REFACTOR changes.
- Keep entries concise — one line per task.

### Deviations

Track cases where implementation diverged from the original spec. This
happens when errors are found, assumptions prove wrong, or a better
approach is discovered during coding:

```markdown
| Task | Spec Said | Actually Did | Why |
|------|-----------|-------------|-----|
| Token refresh | In-memory token store | Redis token store via Testcontainers | Needed TTL support for token expiry |
| Rate limiting | Custom middleware | express-rate-limit + Redis store | Battle-tested; no point reimplementing |
```

Good deviations to log:
- Tasks where the approach changed during implementation
- API signatures that differ from what was planned
- Libraries swapped for alternatives
- Tasks skipped because they turned out unnecessary
- Extra tasks added to handle discovered edge cases
- Test strategy changes (e.g., switched from mocking to integration)

**Don't log minor adjustments** (renamed a variable, reordered parameters).
Only log changes that would surprise someone reading the spec and comparing
it to the code.

## Minimal Spec Example (TDD)

For small tasks, a spec can be much simpler but still follows test-first:

```markdown
---
id: fix-upload-bug
title: Fix File Upload Bug
status: active
created: 2026-02-10
updated: 2026-02-10
priority: high
---

# Fix File Upload Bug

## Overview

Files over 10MB fail silently on upload. The multipart parser truncates
the stream. Need to fix the size limit and add proper error handling.

## Testing Architecture

### Test Framework & Tools

| Tool | Choice | Version | Purpose |
|------|--------|---------|---------|
| Test framework | Vitest | 3.0.4 | Unit and integration test runner |
| HTTP testing | Supertest | 7.1.0 | HTTP assertion library |

### Test Commands

| Command | Purpose |
|---------|---------|
| `npx vitest run` | Run all unit tests |
| `npx vitest run tests/integration` | Run integration tests |

## Phase 1: Upload Fix [in-progress]

- [x] [TEST-UPL-01] Write test reproducing the bug with a 15MB file
- [x] [IMPL-UPL-02] Fix multipart parser size limit in `src/upload/parser.ts` -> satisfies [TEST-UPL-01]
- [ ] [TEST-UPL-03] Write boundary tests (exactly 10MB, 10.1MB, 100MB) <- current
- [ ] [IMPL-UPL-04] Handle boundary sizes in parser -> satisfies [TEST-UPL-03]
- [ ] [TEST-UPL-05] Write test for proper error response on oversized files
- [ ] [IMPL-UPL-06] Add proper 413 error response -> satisfies [TEST-UPL-05]

---

## Resume Context

> Completed first cycle: TEST-UPL-01 (red: timeout on 15MB upload) then
> IMPL-UPL-02 (green: fixed fileSize limit in busboy config). Starting
> boundary condition tests next.
>
> Last Cycle: [IMPL-UPL-02] GREEN — 1/1 pass
> Current: [TEST-UPL-03] Write boundary tests
> TDD Phase: RED (about to write test, run, confirm fail)
> Last Test Run: `npx vitest run tests/integration/upload.test.ts` at 10:25, 1 passed / 0 failed
>
> Next step: Write boundary tests in `tests/integration/upload.test.ts`
> for exactly 10MB, 10.1MB, and 100MB. The handler is in
> `src/upload/parser.ts` using `busboy`. Fixed `limits.fileSize` in IMPL-02.

## TDD Log

| Task | Red (Failure Output) | Green (Pass Output) | Refactor (Changes) |
|------|---------------------|--------------------|--------------------|
| [TEST-UPL-01] | `Timeout: test exceeded 5000ms` | — | — |
| [IMPL-UPL-02] | — | `1 passed, 0 failed` | — |
```

## Complex Spec Example (TDD)

For larger features with multiple phases:

```markdown
---
id: real-time-collab
title: Real-Time Collaboration
status: active
created: 2026-02-01
updated: 2026-02-10
priority: high
tags: [feature, websocket, collaboration]
---

# Real-Time Collaboration

## Overview

Add real-time collaborative editing to the document editor. Multiple
users should see each other's cursors and edits in real time, with
conflict resolution handled via CRDTs.

## Testing Architecture

### Test Framework & Tools

| Tool | Choice | Version | Purpose |
|------|--------|---------|---------|
| Test framework | Vitest | 3.0.4 | Unit and integration tests |
| WebSocket testing | Socket.io-client | 4.8.0 | WS client for integration tests |
| Mocking | MSW | 2.7.0 | HTTP mocking for auth endpoints |
| Coverage | v8 (Vitest) | built-in | Line and branch coverage |
| E2E | Playwright | 1.50.0 | Multi-browser collaboration tests |

### Isolation Strategy

| Layer | Isolation Approach | Services |
|-------|--------------------|----------|
| CRDT logic | No mocks; pure Yjs operations | None |
| WS connection manager | Mock Socket.io server | Socket.io |
| Auth middleware | Mock JWT verification | Auth service |
| E2E | Full stack | WS server, auth, database |

### Coverage Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Line coverage | 85% | Focus on CRDT and WS layers |
| Branch coverage | 75% | Especially error/reconnection paths |
| Mutation score | 70% | Critical paths: merge conflict, auth |

### Test Commands

| Command | Purpose |
|---------|---------|
| `npx vitest run` | Run all unit tests |
| `npx vitest run tests/integration` | Run integration tests |
| `npx vitest --coverage` | Generate coverage report |
| `npx playwright test` | Run E2E collaboration tests |

## Phase 1: WebSocket Infrastructure [completed]

- [x] [TEST-RTC-01] Write tests for WS server connection lifecycle
- [x] [IMPL-RTC-02] Set up WebSocket server with Socket.io -> satisfies [TEST-RTC-01]
- [x] [TEST-RTC-03] Write tests for room-based connections (join, leave, broadcast)
- [x] [IMPL-RTC-04] Implement room-based connections -> satisfies [TEST-RTC-03]
- [x] [TEST-RTC-05] Write tests for WS auth middleware (valid JWT, expired, missing)
- [x] [IMPL-RTC-06] Add authentication middleware for WS -> satisfies [TEST-RTC-05]
- [x] [TEST-RTC-07] Write tests for reconnection and state recovery
- [x] [IMPL-RTC-08] Handle reconnection and connection state -> satisfies [TEST-RTC-07]

## Phase 2: CRDT Integration [in-progress]

- [x] [TEST-RTC-09] Write tests for Yjs document sync between two clients
- [x] [IMPL-RTC-10] Integrate Yjs as CRDT library -> satisfies [TEST-RTC-09]
- [x] [TEST-RTC-11] Write tests for document sync provider initialization
- [x] [IMPL-RTC-12] Create document sync provider -> satisfies [TEST-RTC-11]
- [ ] [TEST-RTC-13] Write tests for awareness protocol (cursors, selections) <- current
- [ ] [IMPL-RTC-14] Implement awareness protocol -> satisfies [TEST-RTC-13]
- [ ] [TEST-RTC-15] Write tests for undo/redo with CRDT state
- [ ] [IMPL-RTC-16] Add undo/redo manager -> satisfies [TEST-RTC-15]

## Phase 3: UI Layer [pending]

- [ ] [TEST-RTC-17] Write tests for remote cursor rendering
- [ ] [IMPL-RTC-18] Render remote cursors with user colors/names -> satisfies [TEST-RTC-17]
- [ ] [TEST-RTC-19] Write tests for selection highlights
- [ ] [IMPL-RTC-20] Show selection highlights -> satisfies [TEST-RTC-19]
- [ ] [TEST-RTC-21] Write tests for presence indicator
- [ ] [IMPL-RTC-22] Add presence indicator -> satisfies [TEST-RTC-21]
- [ ] [TEST-RTC-23] Write tests for offline state UI
- [ ] [IMPL-RTC-24] Handle offline state gracefully in UI -> satisfies [TEST-RTC-23]

## Phase 4: Edge Cases & Performance [pending]

- [ ] [TEST-RTC-25] Write load test for 10+ simultaneous users
- [ ] [IMPL-RTC-26] Optimize for 10+ simultaneous users -> satisfies [TEST-RTC-25]
- [ ] [TEST-RTC-27] Write perf tests for large document sync
- [ ] [IMPL-RTC-28] Handle large document performance -> satisfies [TEST-RTC-27]
- [ ] [TEST-RTC-29] Write tests for rate limiting rapid edits
- [ ] [IMPL-RTC-30] Add rate limiting for rapid edits -> satisfies [TEST-RTC-29]
- [ ] [TEST-RTC-31] Write conflict scenario integration tests
- [ ] [IMPL-RTC-32] Harden conflict resolution -> satisfies [TEST-RTC-31]

---

## Resume Context

> Completed Yjs sync cycle (TEST-09 red, IMPL-10 green) and provider
> init cycle (TEST-11 red, IMPL-12 green, refactored provider config).
> Starting awareness protocol test next.
>
> Last Cycle: [IMPL-RTC-12] GREEN — 6/6 pass, refactored provider config
> Current: [TEST-RTC-13] Write awareness protocol tests
> TDD Phase: RED (about to write test, run, confirm fail)
> Last Test Run: `npx vitest run tests/unit/collab/` at 15:42, 6 passed / 0 failed
>
> Next step: Write awareness protocol tests in
> `tests/unit/collab/awareness.test.ts`. Tests: local cursor broadcast,
> remote cursor receive, cursor removal on disconnect, selection range.
>
> Key file: `tests/unit/collab/awareness.test.ts` (create — writing test)
> Key file: `src/collab/awareness.ts` (doesn't exist — will create in IMPL-RTC-14)

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2026-02-01 | Socket.io over raw WS | Built-in reconnection, rooms, and fallback to polling |
| 2026-02-03 | Yjs over Automerge | Better editor integration (ProseMirror/CodeMirror bindings) |
| 2026-02-05 | Room-per-document | Simpler than multiplexing, isolates failure domains |
| 2026-02-06 | Vitest over Jest for WS tests | Faster; native ESM; same runner as rest of project |
| 2026-02-08 | Awareness as separate protocol | Cursor positions need higher update frequency than doc edits |

## TDD Log

| Task | Red (Failure Output) | Green (Pass Output) | Refactor (Changes) |
|------|---------------------|--------------------|--------------------|
| [TEST-RTC-01] | `ECONNREFUSED 127.0.0.1:3001` (4 fail) | — | — |
| [IMPL-RTC-02] | — | `4 passed` | Extracted server config to `src/config/ws.ts` |
| [TEST-RTC-03] | `timeout waiting for room join event` (3 fail) | — | — |
| [IMPL-RTC-04] | — | `7 passed` | — |
| [TEST-RTC-05] | `Expected 401, got undefined` (5 fail) | — | — |
| [IMPL-RTC-06] | — | `12 passed` | Reused JWT verify from `src/auth/verify.ts` |
| [TEST-RTC-07] | `reconnection callback not called` (2 fail) | — | — |
| [IMPL-RTC-08] | — | `14 passed` | — |
| [TEST-RTC-09] | `connection refused` (2 fail) | — | — |
| [IMPL-RTC-10] | — | `16 passed` | — |
| [TEST-RTC-11] | `module not found` (2 fail) | — | — |
| [IMPL-RTC-12] | — | `18 passed` | Extracted provider config to `src/config/crdt.ts` |

## Deviations

| Task | Spec Said | Actually Did | Why |
|------|-----------|-------------|-----|
| WS authentication | JWT in query params | JWT in first message after connect | Query params are logged by proxies — security risk |
```
