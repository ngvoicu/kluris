# Command Contracts

This file defines functional contracts for the `SKILL.md` skill. Use it as a
review checklist before releases.

## Global Contracts

1. `<script id="spec-meta">` JSON in `SPEC.html` is authoritative for identity;
   `data-status` attributes are authoritative for lifecycle; registry is
   denormalized.
2. Exactly one active spec should exist after any write operation.
3. All write workflows update `SPEC.html` first (including a successful
   `validate.md` run), then recompute/update registry.
4. Forge workflow never writes application code.
5. Phase status uses `data-status="pending"` | `"in-progress"` | `"completed"` |
   `"blocked"`. Task status uses the same values; AC status uses only
   `pending` / `completed`.
6. **Progress tracking is sacred.** After every task completion: edit
   `SPEC.html` (`data-status` swap, phase transition if applicable, updated
   date), run `references/validate.md`, update registry (progress, date),
   then re-read both files to verify consistency. Never skip this.
7. **Acceptance criteria are required for feature specs.** Forge must include
   an Acceptance Criteria region with `<li class="ac-item" data-status="pending">`
   testable conditions. Implement must update `data-status="completed"` on
   criteria as they are satisfied and verify all are met before marking a
   spec complete.
8. **No mid-task state.** HTML specs checkpoint at task boundaries only —
   there is no Resume Context section. Pause/resume always lands on a
   clean task boundary.

## Command Contracts

### Forge

1. Resolve `<spec-id>` before research output paths are referenced.
2. Collision-check existing spec IDs before creating new files (check
   `.specs/<id>/SPEC.html` and registry).
3. Forge must not run in plan mode; if plan mode is active, require exit
   before continuing.
4. Refresh `.specs/assets/` on every forge: copy `spec-styles.css` and
   `spec-runtime.js` from the skill's `assets/` directory, **overwriting
   any existing files**. The runtime is skill-managed; overwrite-on-forge
   ensures existing projects pick up rendering fixes.
5. Output scope is `.specs/` artifacts only (`research-*.md`,
   `interview-*.md`, `SPEC.html`, `registry.md` updates, assets on first run).
6. After approval, hand off to the implement workflow instead of
   implementing inside forge.
7. Interview must ask about acceptance criteria ("What does 'done' look like?").
8. **If UI work is in scope, interview must ask about mockup fidelity**
   (`wireframe` / `hi-fi` / `none`) and store the answer in the
   `mockup-fidelity` field of the spec-meta JSON.
9. SPEC.html must include all canonical regions: `meta`, `toc`, `header`,
   `overview`, `acceptance`, `architecture`, `libraries`, `phases`, `code`
   (may be empty), `mockups` (omitted entirely if fidelity is `none`),
   `decisions`, `deviations`.
10. Forge must run `references/validate.md` before presenting the spec.
11. SPEC.html must be derived from `references/html-template.html`.

### Implement

1. Supports scope parsing: current flow, phase-specific, all phases, task code.
2. For each completed task: swap `data-status="pending"` →
   `data-status="completed"` in `SPEC.html`, run `references/validate.md`,
   update registry progress/date. Re-read both files to verify consistency.
3. At phase completion: update phase `data-status` + pill class, promote
   next phase to `in-progress`, review and update satisfied acceptance
   criteria.
4. At spec completion: verify all acceptance criteria have
   `data-status="completed"` before marking complete.
5. Blocked handling:
   - Set blocked tasks to `data-status="blocked"`.
   - Mark phase `data-status="blocked"` only when the whole phase is blocked.
   - Record blocker context in the Decision Log or Deviations.

### Resume

1. If no active spec exists, list specs and request target.
2. Parse progress from `SPEC.html` `data-status` counts.
3. Identify current phase (first phase with `data-status="in-progress"`)
   and current task (first task with `data-status="pending"` in that phase).
4. Present a compact summary. No separate Resume Context section to read.

### Pause

1. If no active spec exists, report no-op and stop.
2. Finalize state at the current task boundary — every completed task has
   the right `data-status`.
3. Add session decisions to the Decision Log.
4. Set status `paused` (JSON + visible pill) and sync registry.
5. Run `references/validate.md`.

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
2. If no active spec, guide to activate one.

### OpenAPI

1. Generate/update `.openapi/openapi.yaml` and `.openapi/endpoints/*.md`.
2. Preserve manual additions when updating existing files.
3. Report endpoint/schema/security counts and manual-review candidates.
4. OpenAPI output stays as YAML + markdown — not HTML.

## Universal Skill Contract

1. `SKILL.md` must include cross-tool behavior for all declared triggers.
2. If `generate openapi` is listed as a trigger, OpenAPI workflow behavior
   must be defined in `SKILL.md`.

## Release Checklist

1. Paths referenced in docs and templates exist (excluding placeholder paths).
2. Command contracts in this file still match `SKILL.md`.
3. `assets/spec-styles.css` and `assets/spec-runtime.js` exist and are
   distributed to consumer projects' `.specs/assets/` on first forge.
4. `references/html-template.html` validates with `references/validate.md`.
