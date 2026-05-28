---
name: specmint-core-html
description: >
  Persistent spec management for AI coding workflows, producing canonical
  SPEC.html files. Use this companion when the user explicitly mentions specs,
  forging, structured planning, resume/status/pause/switch, implementing a
  spec, or generating OpenAPI docs. Do NOT trigger on ordinary coding tasks
  that do not ask for spec management.
---

# Spec Mint Core HTML — Kluris Companion

Turn ephemeral plans into structured, persistent specs built through deep
research and iterative interviews. This HTML variant uses `.specs/<id>/SPEC.html`
as the canonical spec document, not a markdown spec plus a separate view. The
HTML file carries metadata, lifecycle state, diagrams, code previews, mockups,
decision logs, and deviations in one human-openable artifact.

When this file is referenced from a Kluris-generated brain skill, read and
follow it directly from `~/.kluris/companions/specmint-core-html/SKILL.md`.
Do not ask the user to install Specmint separately. Kluris vendors companions
as single `SKILL.md` files, so every rule needed for companion mode is below.

## Critical Invariants

1. **Canonical files**
   - Registry: `.specs/registry.md` (markdown index)
   - Per-spec document: `.specs/<id>/SPEC.html`
   - Research notes: `.specs/<id>/research-*.md`
   - Interview notes: `.specs/<id>/interview-*.md`
   - Scratch, if needed: `.specs/<id>/artifacts/`
2. **Authority rule**: the JSON inside `<script type="application/json" id="spec-meta">`
   is authoritative for identity and spec status. `data-status` attributes on
   phases, tasks, and acceptance criteria are authoritative for progress.
3. **Active-spec rule**: target exactly one active spec at a time.
4. **HTML is the source of truth**: do not create or update a parallel markdown
   spec for an HTML companion spec.
5. **Progress tracking is sacred**: after completing any task, immediately
   update `SPEC.html` (`data-status`, phase transitions, updated date) and
   `.specs/registry.md` (progress/date). Re-read both files before moving on.
6. **Single-file companion mode**: use inline HTML/CSS/JS patterns in the
   `SPEC.html` you write. Do not rely on unavailable sibling template or asset
   files from a separate plugin install.

## Registry Format

Create `.specs/registry.md` if missing:

```markdown
# Spec Registry

| ID | Title | Status | Priority | Progress | Updated |
|----|-------|--------|----------|----------|---------|
| user-auth-system | User Auth System | active | high | 0/12 | 2026-05-23 |
```

The registry is a denormalized index only. If it conflicts with `SPEC.html`,
trust `SPEC.html` and repair the registry on the next write.

## Session Start

If `.specs/registry.md` exists, check for an `active` row. If one exists,
briefly mention it and invite `resume`; do not force spec work if the user asks
for something unrelated.

## Resuming

When the user says `resume`, `status`, or asks what was in progress:

1. Read `.specs/registry.md` and find the active spec.
2. Load `.specs/<id>/SPEC.html`.
3. Parse task progress from `<li class="task" data-status="...">`.
4. Current phase is the first `<details class="phase" data-status="in-progress">`.
5. Current task is the first pending task in that phase.
6. Present:

```text
Resuming: <Title> (<id>)
Progress: <done>/<total> tasks
Phase: <phase title>
Current: <task text>
```

Begin work on the current task unless the user asked only for a status report.

## Forging an HTML Spec

The forge workflow produces `.specs/` files only. It does not implement
application code.

1. Generate a lowercase hyphenated spec ID.
2. Collision-check `.specs/<id>/SPEC.html` and the registry.
3. Create `.specs/<id>/` and `.specs/registry.md` if needed.
4. Research deeply:
   - map the relevant codebase paths and dependency flow;
   - inspect tests and existing conventions;
   - search current docs/best practices when external APIs or libraries matter;
   - identify risks, edge cases, and open questions.
5. Save research to `.specs/<id>/research-01.md`.
6. Interview the user with 3-6 targeted questions that research could not
   answer. Save answers to `.specs/<id>/interview-01.md`.
7. Repeat research/interview if important choices remain unclear.
8. Write `.specs/<id>/SPEC.html` from the self-contained HTML format below.
9. Update the registry row to `active` and `0/N` progress.
10. Validate the HTML/JSON structure, present the spec, and wait for explicit
    approval before implementation.

## SPEC.html Format

Use this structure. Keep region comments; they make future edits reliable.

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Spec: User Auth System</title>
  <script type="application/json" id="spec-meta">{"id":"user-auth-system","title":"User Auth System","status":"active","created":"2026-05-23","updated":"2026-05-23","priority":"high","tags":["auth"],"mockup-fidelity":"hi-fi"}</script>
  <style>
    body{font-family:Inter,system-ui,sans-serif;margin:0;background:#0f172a;color:#e5e7eb;line-height:1.55}
    main{max-width:1100px;margin:0 auto;padding:32px}.card{background:#111827;border:1px solid #334155;border-radius:16px;padding:20px;margin:16px 0}
    .pill{display:inline-block;border-radius:999px;padding:2px 10px;background:#334155}.pill--completed{background:#065f46}.pill--in-progress{background:#1d4ed8}.pill--pending{background:#475569}.pill--blocked{background:#7f1d1d}
    .task,.ac-item{margin:10px 0;padding:10px;border-left:4px solid #475569;background:#0b1220}.task[data-status="completed"],.ac-item[data-status="completed"]{border-color:#10b981}.task[data-status="in-progress"],.phase[data-status="in-progress"]{border-color:#60a5fa}.task[data-status="blocked"],.phase[data-status="blocked"]{border-color:#f87171}
    pre{overflow:auto;background:#020617;border-radius:12px;padding:16px}.mockup{background:#f8fafc;color:#0f172a;border-radius:16px;padding:20px}.table{width:100%;border-collapse:collapse}.table td,.table th{border:1px solid #334155;padding:8px;text-align:left}
  </style>
</head>
<body>
<main>
  <header class="card">
    <h1>User Auth System</h1>
    <p><span class="pill pill--in-progress">Active</span> <span class="pill">high</span></p>
    <dl><dt>Created</dt><dd>2026-05-23</dd><dt>Updated</dt><dd>2026-05-23</dd></dl>
  </header>

  <!-- region:OVERVIEW -->
  <section class="card"><h2>Overview</h2><p>What is being built and why.</p></section>
  <!-- endregion:OVERVIEW -->

  <!-- region:ACCEPTANCE -->
  <section class="card"><h2>Acceptance Criteria</h2><ul>
    <li class="ac-item" data-status="pending">User can sign in with GitHub.</li>
  </ul></section>
  <!-- endregion:ACCEPTANCE -->

  <!-- region:ARCHITECTURE -->
  <section class="card"><h2>Architecture</h2><pre class="mermaid">flowchart LR
    A["User"] --> B["Frontend"]
    B --> C["Backend API"]
  </pre></section>
  <!-- endregion:ARCHITECTURE -->

  <!-- region:LIBRARIES -->
  <section class="card"><h2>Library Choices</h2><table class="table"><thead><tr><th>Need</th><th>Choice</th><th>Why</th></tr></thead><tbody><tr><td>Auth</td><td>Existing stack</td><td>Matches project conventions.</td></tr></tbody></table></section>
  <!-- endregion:LIBRARIES -->

  <!-- region:PHASES -->
  <section class="card"><h2>Phases & Tasks</h2>
    <details class="phase" open data-status="in-progress"><summary><strong>Phase 1: Foundations</strong> <span class="pill pill--in-progress">In progress</span></summary><ul class="task-list">
      <li class="task" data-status="pending"><span class="task-code">UAS-01</span> Add auth configuration in <code>src/auth/config.ts</code>.</li>
    </ul></details>
  </section>
  <!-- endregion:PHASES -->

  <!-- region:CODE_PREVIEWS -->
  <section class="card"><h2>Code Previews</h2><figure class="code-diff"><figcaption>Public contract</figcaption><pre><code>+ export async function signInWithGithub(code: string): Promise&lt;UserSession&gt;</code></pre></figure></section>
  <!-- endregion:CODE_PREVIEWS -->

  <!-- region:MOCKUPS -->
  <section class="card"><h2>UI Mockups</h2><figure class="mockup"><h3>Sign in</h3><button>Continue with GitHub</button></figure></section>
  <!-- endregion:MOCKUPS -->

  <!-- region:DECISIONS -->
  <section class="card"><h2>Decision Log</h2><table class="table"><thead><tr><th>Date</th><th>Decision</th><th>Rationale</th></tr></thead><tbody></tbody></table></section>
  <!-- endregion:DECISIONS -->

  <!-- region:DEVIATIONS -->
  <section class="card"><h2>Deviations</h2><table class="table"><thead><tr><th>Task</th><th>Spec Said</th><th>Actually Did</th><th>Why</th></tr></thead><tbody></tbody></table></section>
  <!-- endregion:DEVIATIONS -->
</main>
</body>
</html>
```

### Authoring Rules

- Metadata JSON must be valid and single-source-of-truth for `id`, `title`,
  `status`, `created`, `updated`, `priority`, `tags`, and `mockup-fidelity`.
- Valid lifecycle values: `pending`, `in-progress`, `completed`, `blocked`.
- Task codes use a 2-4 uppercase prefix plus incrementing number, e.g. `AUTH-01`.
- Every non-trivial spec needs at least one Mermaid diagram and at least one
  code preview showing a meaningful contract, schema, API, or test pattern.
- Quote Mermaid labels whenever they contain punctuation, slashes, parentheses,
  commas, colons, arrows, or spaces beyond simple words.
- UI work should include a real-looking mockup unless the user says mockups are
  unnecessary.
- Never leave `TBD`, `TODO`, `placeholder`, `figure out`, or unresolved choices
  in the final spec. Ask the user instead.

## Implementing a Spec

When the user asks to implement:

1. Read the active registry row and load `SPEC.html`.
2. Select scope: current task, a named phase, or all remaining phases.
3. For each task in order:
   - implement the code;
   - run the relevant tests or checks;
   - update the task `data-status` to `completed`;
   - if all tasks in a phase are complete, mark the phase completed and promote
     the next phase to `in-progress`;
   - update the `updated` date in `spec-meta` and the visible header;
   - recompute progress directly from task `data-status` counts;
   - update `.specs/registry.md`;
   - re-read both files to verify progress is in sync.
4. Log durable decisions in the Decision Log.
5. Log implementation drift in Deviations.
6. Before claiming phase/spec completion, show the actual command output from
   the relevant test suite.

## Pausing and Completion

Pause by setting spec status to `paused` in `spec-meta`, changing the visible
status pill, updating the registry, and recording any durable decision/context.

Complete only when all tasks and acceptance criteria are complete, all phases
are complete, the full relevant test suite has been run, and the registry row
matches the derived task progress from `SPEC.html`.
