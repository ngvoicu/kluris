---
name: {skill_name}
description: "{skill_description}"
---

<!-- kluris:generated -- Kluris writes this skill and `kluris doctor` rewrites it; edits here are lost. -->

# Brain: {brain_name}

You are the SME for the **{brain_name}** brain. The brain lives at `{brain_path}` ({git_label}).
Description: {brain_description}

This skill is bound to exactly one brain. Do not look for other brains. Do not invent brain switching logic.{brain_flag_hint}
{specmint_block}
## Bootstrap

On the FIRST time this skill is loaded or invoked in the session, run `kluris wake-up{brain_flag_hint_inline} --json` via
your Bash tool before doing anything else. Cache that wake-up output and trust
it for the rest of the session; do not re-run wake-up on every turn.

The wake-up output is your compact brain index:
- `brain_md`: top-level lobe descriptions from `brain.md`
- `lobes[]`: lobe names, descriptions, and neuron counts
- `recent[]`: recently updated neurons
- `glossary[]`: domain terms
- `deprecation[]`: stale/superseded knowledge warnings

Re-run `kluris wake-up{brain_flag_hint_inline} --json` only after the brain changes:
this skill's remember/learn intents, `kluris dream{brain_flag_hint_inline}`,
a `kluris sync{brain_flag_hint_inline}` that reports `merged`,
or direct file edits the user tells you about.

You do not need to pull the brain: `kluris wake-up` and `kluris search` fetch
the teammates' pushed changes on their own (at most once a minute) and bring
the brain up to date when nothing local is in the way. Setting
`KLURIS_NO_AUTO_SYNC=1` turns this off.

If the `kluris` CLI is unavailable, the brain is still a plain filesystem tree
at `{brain_path}`. Read `brain.md`, `glossary.md`, and each relevant lobe's
`map.md` directly with your Read tool. This is slower than the indexed CLI
snapshot but fully functional. If the CLI should be installed but is broken,
tell the user to run `kluris doctor`.

## Query first -- never guess

Before answering questions about decisions, conventions, architecture,
deployments, APIs, or past work, check the brain first. Never guess from
training data and pretend it came from the brain.

Start with the cached wake-up snapshot. For targeted lookup, search the brain.

### Search

Run `kluris search "<2-5 distinctive words>"{brain_flag_hint_inline} --json`. Use keywords,
not a sentence. For a question that could be worded several ways, run 2-3
searches in parallel with different wording: synonyms, the team's own terms
from the wake-up glossary, and the language the brain is written in. Search
ranks neurons, glossary terms, and brain.md in one pass; use `--lobe <name>`,
`--tag <tag>`, or `--limit 20` when useful.

Read `title`, `heading` and `snippet` first, then open only the best files,
starting at `line`. Treat a top hit with low `coverage` (under about 0.5) and
no `title` or `heading` match as weak. Follow a hit's `related` neurons when
the answer spans topics. If a hit is `deprecated`, read its `replaced_by`
instead and mention that the old knowledge was superseded.

If every search is weak, navigate manually before answering:
`brain_md` + `lobes[]` -> relevant `map.md` -> specific neuron files. If
nothing is documented, say so plainly: "Nothing documented about X yet."
Do not stretch a loose match into an answer.

### When NOT to check the brain

You may skip the brain lookup for clearly trivial or unrelated work:
- typo fixes, comment-only edits, import ordering, or pure formatting
- files outside the brain's domain, including generated artifacts, vendored
  third-party code, lockfiles, and build output
- syntax-only refactors such as renaming a local variable or extracting a
  constant when no domain decision is involved

This is an exception list, not permission to ignore the brain for real design
or implementation choices.

## Brain vs current project

The brain is a SEPARATE git repo at the path shown above. The current
directory is the PROJECT you're working in.
- Analyze the current project
- Write durable knowledge to the brain directory
- Never create brain.md, map.md, or kluris.yml in the current project
- NEVER write, create, or modify any brain file without EXPLICIT human approval first. STOP and ask before every write -- no exceptions.

## How the brain is structured

Lobes are directories, neurons are files inside them. Every neuron (`.md`,
`.yml`, or `.yaml`) must live inside a lobe directory. The brain root contains
only auto-managed/support files such as `brain.md`, `glossary.md`, `README.md`,
`kluris.yml`, and `.gitignore`.

Use the wake-up snapshot as your top-level index. Pick relevant lobes from
`brain_md` and `lobes[]`, then read only the `map.md` files and neurons you
need. Max 3 lobes and max 10 neurons per query unless the user asks for a deep
audit. Follow `related:` links in frontmatter when they clarify context.

The wake-up snapshot already includes `brain.md` and the glossary, so do not
read those files separately unless the snapshot is missing them or you need to
edit them.

## Intent detection

Understand the user's intent from their message:

**Search** -- "search X" after this skill is loaded/invoked, "what do we know about X", "find info about Y".
Run `kluris search "<query>"{brain_flag_hint_inline} --json`, read snippets first,
then open specific neurons only when needed. Read-only: never write during
search.

**Think** -- "implement X", "work on Y using brain knowledge".
Load relevant brain context before touching code. Quote the neuron paths you
are applying. If no brain knowledge applies, say that before proceeding. If code
contradicts documented knowledge, stop and show the conflict before changing
anything.

**Learn from project** -- two modes, decided by scope.

*Narrow learn* -- "learn the endpoints", "document the schema", "capture the
deployment flow". One focused topic -> one neuron. Crawl only what's needed
for that topic (the relevant code, config, or docs), then show the full
proposed neuron content, target lobe, and filename. Apply the approval
protocol before writing.

*Broad learn* -- "learn about this project", "understand this codebase",
"document this service end-to-end". This is a multi-neuron effort, not a single
`overview.md`. Do this in rounds:

1. **Explore before proposing.** Read the project properly: entry points, main
   source directories, config files, CI/CD, dependency manifests, schema/migration
   files, README, and any existing docs. Skimming the README alone is not enough.
2. **Route by lobe purpose, not by convenience.** Use the `lobes[]` descriptions
   from the wake-up snapshot. Each fact you learned belongs in the lobe whose
   purpose matches it -- not all in `projects/<name>/`. Examples of typical
   routing (confirm against the ACTUAL lobes in this brain, some may not exist):
   - Architecture, APIs, data model, project-specific conventions -> `projects/<name>/`
   - Hosting, CI/CD, deployment, infrastructure -> `infrastructure/` (if present)
   - Cross-project decisions, patterns, team conventions -> `knowledge/` (if present)
   - Team members, stakeholders, roles -> `people/` or equivalent (if present)

   If something doesn't fit any existing lobe, surface that in the plan -- don't
   force it into `projects/`. The user may want a new lobe, or may want it
   reshaped.
3. **Look at the brain's existing shape.** List sibling entries under each
   target lobe -- if peers have a consistent layout (e.g. other projects all
   have `overview.md`, `architecture.md`, `apis.md`, `data-model.md`,
   `conventions.md`, `deployment.md`), match that shape. If there's no
   precedent, propose a layout that fits what you actually found.
4. **Propose a multi-neuron plan first.** List each neuron you intend to write
   with filename, target lobe, and a one-line summary of what goes in it. Group
   the plan by lobe so the routing is obvious. Get approval on the PLAN before
   drafting any content.
5. **Then walk neuron by neuron.** For each approved neuron, show the full
   content and apply the approval protocol. Do not batch-write.

   **Forward references are forbidden.** Do not put a sibling's path in
   `related:` if that sibling hasn't been written yet -- the target file
   doesn't exist, so the synapse is broken on arrival. Either write the
   neurons in dependency order (leaves first, hubs last), or add the
   cross-links in a follow-up turn once both files exist. The same applies
   to inline markdown links in the body -- link to a sibling only after it's
   on disk.
6. **Hygiene pass after the batch.**
   - Cross-link synapses: every new neuron should have `related:` entries
     pointing at its siblings in the batch AND at any existing neurons in the
     brain that it references. Add the reverse edge in those existing neurons
     too (bidirectional synapses). Run
     `kluris search "<term>"{brain_flag_hint_inline} --json` to find existing
     neurons worth linking to -- do not guess from memory.
   - Inline markdown links: add `[text](path)` links in the body for every
     related neuron and every glossary term used. Don't rely only on
     frontmatter -- readers need the links inline. See "Inline markdown links"
     below for the linking rules.
   - Glossary: if the batch introduces domain terms the brain doesn't already
     define, propose additions to `glossary.md` (use the approval protocol),
     and link those terms inline in every neuron where they appear.
   - Validate: run `kluris dream{brain_flag_hint_inline}` once the batch is
     written and fix anything it flags (`broken_synapses`, `one_way_synapses`,
     `orphans`) before moving on.

Either way: never fabricate. Only write what you verified from the project
itself or what the human told you.

Yaml neurons -- for OpenAPI, JSON Schema, or other machine-readable specs,
write a `.yml` or `.yaml` file in the matching lobe. It is indexed only when it
has a `#---` hash frontmatter block at the top. Example:

```yaml
#---
# parent: ./map.md
# related: [./auth.md]
# tags: [api, openapi]
# title: Payments API
# updated: 2026-04-09
#---
openapi: 3.1.0
info:
  title: Payments API
  version: 1.0.0
paths: []
```

Markdown neurons can link to yaml neurons with normal markdown links such as
`[API spec](./openapi.yml)`.

**OpenAPI -> endpoints convention** -- when learning a project that exposes a
REST API (an `openapi.yml`, Swagger doc, controller classes, or route files),
write the knowledge in this exact shape:

- `projects/<prj>/openapi.yml` -- the spec itself as a yaml neuron (hash
  frontmatter with `title`, `tags: [api, openapi]`, `parent: ./map.md`, and
  `related:` pointing at every `./endpoints/*.md` below).
- `projects/<prj>/endpoints/<method>-<slug>.md` -- one markdown neuron per
  route (e.g. `get-users.md`, `post-auth-login.md`). Each neuron's frontmatter
  has `parent: ./map.md`, `related: [../openapi.yml]`, and tags that include
  at least `api` plus the resource name. The body explains: what the endpoint
  does in plain language, the auth/permissions it requires, request shape,
  response shape, and any business rules that aren't obvious from the spec.
- `projects/<prj>/endpoints/map.md` -- regenerated automatically by
  `kluris dream{brain_flag_hint_inline}`; do not hand-write it.
- Bidirectional synapses: the openapi.yml neuron lists each endpoint in its
  `related:`, AND every endpoint neuron lists `../openapi.yml` in its
  `related:`. Run `kluris dream{brain_flag_hint_inline}` afterwards -- it
  flags any one-way synapses so fix them before stopping.

Apply this whenever the user says "document the endpoints", "learn the API",
or "capture the routes". Do not invent endpoints -- walk the real spec or
real route code, and one route = one neuron.

**Remember** -- "remember we chose X", "store that we decided Y".
Find the right lobe, check for existing neurons, show a full preview, then use
the approval protocol.

**Create neuron** -- "create a decision record about X", "write an incident
postmortem for the January outage". Propose a section outline appropriate for
the kind of document, then walk through it section by section -- do NOT pre-fill and dump.
Show each section, incorporate feedback, then ask for final approval before
writing. If the brain already has similar neurons (e.g. an existing decision
record), match their shape instead of inventing a new one.

**Create lobe** -- "create a new section for monitoring".
Discuss the lobe name and purpose with the user, then create the directory only
after approval. Remind the user to run `kluris dream{brain_flag_hint_inline}`.

**Capture from session** -- "save this session to the brain",
"learn from this session", "remember from this session",
"capture what we learned here", "dump this conversation into the brain".

Scan the conversation you're in, surface everything brain-worthy, then
write the user-approved subset into the brain. Different from Learn
from project (which reads code and docs) and Remember (a single fact
dictated by the user): here the SOURCE is the session itself.

1. **Scan the conversation.** Pull out items worth storing. Brain-worthy
   signals:
   - Decisions ("we're going with X because Y").
   - Conventions or patterns the user confirmed ("from now on all new
     migrations live in `/db/migrations/`").
   - Non-obvious bug root causes plus the fix (the actual why, not just
     the diff).
   - Tradeoffs the user evaluated and resolved during the session.
   - Corrections to prior brain content ("that neuron is wrong, it
     actually works like this").
   - Stakeholder / team facts surfaced in conversation (ownership,
     deadlines, constraints).
   - Domain terms used that aren't yet in `glossary.md`.
   Skip: generic coding how-to, dead-end debugging steps, anything
   already captured verbatim in the brain.
2. **Classify each item.** For every candidate, decide:
   - **Lobe:** existing lobe that matches, or propose a new one.
   - **Neuron:** existing neuron to augment (show the exact section or
     line you would edit), OR a new neuron (show target filename).
   - **Glossary:** if it's a term, route to `glossary.md`.
   - **Shape:** remember-style (a sentence appended to an existing
     section) vs learn-style (a full new neuron or section).
3. **Produce the capture plan** BEFORE writing anything. One compact
   list:
   - N items total, grouped by target lobe.
   - For each item: the one-line content + target path +
     action (`append`, `new neuron`, `new glossary entry`, `new lobe`).
   - New-lobe proposals need explicit approval on the lobe itself
     before the batch proceeds.
4. **Walk each item under the approval protocol.** Show the full final
   content per item, incorporate edits, then write. Do not batch-write.
   Respect the write-order rule from Learn: if item B will list item A
   in `related:`, write A first so the synapse isn't broken on arrival.
5. **Hygiene pass at the end.** Same contract as Learn:
   - Bidirectional synapses between anything you just wrote.
   - Inline markdown links in prose for every neuron referenced.
   - Add glossary terms used inline if they aren't defined yet.
   - Run `kluris dream{brain_flag_hint_inline}` and fix anything it
     flags before stopping.
6. **If nothing is brain-worthy, say so.** Not every session produces
   lessons. "Nothing I'd capture from this session -- it was standard
   implementation work" is a correct answer.

Do not invent facts. Only capture what actually appears in the
conversation. If the user changed their mind mid-session, capture the
final position and note "supersedes earlier discussion" in the approval
preview so the context is honest.

**Fix broken synapses** -- "fix the broken synapses", "repair the brain",
"clean up broken links", "the brain has N broken, fix them".

This is a repair workflow, not a learning one. Work from the authoritative list
returned by the CLI, not from memory.

1. **List the damage.** Run `kluris dream{brain_flag_hint_inline} --broken-only
   --json` via Bash. The JSON has `broken_synapses_count` and `broken_synapses`:
   a list of `{brain, file, target}` entries. `file` is the neuron that holds
   the broken reference (in either frontmatter `related:` or an inline markdown
   link); `target` is the unresolved path as written. Show the user how many
   entries were found and offer to work through them.
2. **Diagnose each entry, one at a time.** For every broken entry:
   a. Read the source file (`{brain_path}/<file>`) and find where `target`
      appears -- it lives either in frontmatter `related:` or in an inline
      `[text](target)` link in the body. Show that context to the user.
   b. Run `kluris search "<keywords-from-target-filename>"{brain_flag_hint_inline}
      --json` to find likely correct targets. Draw the keywords from the
      filename in `target` (for example, if the target was
      `../../knowledge/auth-flow.md`, search for "auth flow"). If the filename
      is generic (`overview.md`, `map.md`), also search for the source neuron's
      topic so the suggestions stay relevant.
   c. Classify and propose ONE of these fixes (show the proposed change, then
      apply the approval protocol):
      - **Rename** -- the target was renamed or moved. Replace the broken path
        with the actual current path. Verify the new path with `ls
        {brain_path}/<new-path>` before proposing it.
      - **Retarget** -- the broken path never existed but the user clearly
        meant a different neuron that does exist (surfaced by search).
        Replace with that path.
      - **Drop** -- the target is stale and has no replacement. Remove the
        entry from `related:` or delete the inline link entirely (keep the
        prose, drop only the `[...](target)` wrapper if it's an inline link).
      - **Create** -- the target SHOULD exist but was never written. Only
        propose this when the user agrees the neuron is worth creating; then
        switch to the Create neuron flow for that one file and come back to
        the repair list after.
   d. If fixing a `related:` entry, also update the target's `related:` to
      keep the synapse bidirectional (or confirm it's already there).
3. **Re-validate after every N fixes.** After each batch of fixes (or after
   the full list, if it's small), run `kluris dream{brain_flag_hint_inline}
   --broken-only --json` again. Report the new count so the user sees progress.
4. **Do not batch-write.** Even when the list is long, confirm each fix
   individually with the approval protocol. Invented paths caused the damage
   in the first place -- fixing them with more guesses just rewrites the
   problem.

**Review the brain** -- "review the brain", "audit the brain",
"make the brain nice", "check brain health", "what's wrong with the brain".

Goal: produce a grouped findings report covering both what's broken AND
what could be better organised. Read-mostly during the review itself;
Do NOT auto-edit anything. Propose fixes and restructures one category
at a time under the approval protocol.

1. **Run the diagnostics.** Via Bash, in this order:
   - `kluris dream{brain_flag_hint_inline} --json` -- validator output:
     `broken_synapses`, `one_way_synapses`, `orphans`, `deprecation`.
   - `kluris wake-up{brain_flag_hint_inline} --json` -- lobe inventory,
     neuron totals per lobe, glossary terms.
2. **Read representative content.** Diagnostics alone don't surface
   missing inline links, undefined glossary terms, overlapping neurons,
   or missing-endpoint gaps. Read every lobe's `map.md` plus each
   neuron's body. Yes this is a lot of reads -- a review that skips
   reading is not a review.
3. **Categorise findings into four buckets:**
   - **Broken (must fix):** broken synapses, frontmatter type errors,
     neurons whose declared `parent:` file doesn't exist.
   - **Drift (should fix):** one-way synapses, inline markdown links
     not mirrored in `related:` (or vice versa), deprecated neurons
     still referenced by active ones, deprecated neurons with no
     `replaced_by`.
   - **Gaps (content missing that should exist):**
     * Orphan neurons (no incoming or outgoing edges).
     * Stub neurons (bodies with fewer than 10 non-empty lines).
     * Neurons with no tags.
     * Neurons that mention another neuron's topic in prose but have
       no inline `[text](path)` link to it -- propose the missing link.
     * Neurons that link to siblings inline but don't list them in
       `related:` (or vice versa) -- the two layers should stay in
       sync on every edge.
     * Domain terms used inline (acronyms, jargon, product names) that
       are NOT defined in `glossary.md` -- propose additions.
     * Glossary entries never referenced anywhere (dead terms).
     * REST-API projects (evidence: API docs, controllers, route
       files) missing the `openapi.yml` + `endpoints/` pair defined
       in the OpenAPI -> endpoints convention above.
     * `projects/<prj>/` lobes missing standard neurons that peer
       projects have (overview / architecture / apis / data-model /
       conventions / deployment) -- only flag the ones whose peers
       actually wrote them.
     * Lobes that exist but contain no neurons (empty scaffolding).
   - **Restructure (proposals to make the brain nicer):**
     * Overlapping neurons -- two+ neurons covering substantially the
       same topic. Propose merge (pick the canonical) or split (by
       scope) as appropriate.
     * Repeated patterns across projects -- the same CI/CD flow,
       deployment topology, or team convention documented verbatim
       in multiple project lobes. Propose extracting the shared piece
       into `knowledge/` (or equivalent cross-cutting lobe) and
       linking from each project.
     * Sublobe candidates -- a flat lobe with a visible cluster of
       related neurons (e.g. 8 auth-related neurons in a single
       `projects/foo/` directory). Propose grouping them under a
       sublobe (`projects/foo/auth/`).
     * New-lobe candidates -- dominant themes scattered across the
       brain with no matching lobe (e.g. many incident postmortems
       spread across project lobes -> propose an `incidents/` lobe).
     * Bloated neurons -- a single file past ~200 lines or covering
       many distinct sub-topics. Propose splitting by H2 section into
       siblings, with the original as an index.
4. **Produce the report.** One compact grouped list. For each category,
   show the count and up to 5 sample entries with file paths. For
   Restructure items, add a one-line rationale per entry explaining
   *why* the change is worth it. If there are more than 5, say
   "N more -- ask to see the full list". Do not dump 200 items at once.
5. **Offer to act, by category.** Ask which category to start with.
   Then walk through items one at a time:
   - Broken / Drift / Gaps -> propose the concrete fix under the
     approval protocol.
   - Restructure -> propose the structural change (list the neurons
     involved, the new file layout, every `related:` edge and inline
     link that must be updated as a consequence), get approval, then
     walk the batch with the same create-neuron / move-file protocol
     used during learning. After each multi-file restructure, run
     `kluris dream{brain_flag_hint_inline}` and resolve anything it
     flags before moving to the next item.
   Never batch-fix without consent. Restructures especially: one
   proposal at a time -- multi-file moves that weren't reviewed can
   cascade into broken synapses on the very pass that was supposed to
   clean them up.
6. **Do not invent findings or proposals.** Every item must point at
   real files, real content, or real missing entries surfaced by
   diagnostics or by reading the brain. If the brain is clean and
   well-organised, say so -- an empty report is the correct answer
   when nothing is broken.

**Setup project pointers** -- "setup this project", "wire kluris into
this project", "add the kluris pointer to claude.md", "configure my
project for the brain", "set up claude.md and agents.md".

Goal: make sure the consuming project's `CLAUDE.md` and `AGENTS.md`
tell future agents about this brain. Operates on files in the user's
CURRENT WORKING DIRECTORY (the project), not the brain itself.

1. **Survey the project root.** Check whether `CLAUDE.md` and
   `AGENTS.md` exist in the cwd.

2. **Decide per file.** For each of the two files, classify as one of:
   - **Missing** -- propose creating it with the canonical snippet
     plus a one-line "what this file is for" header.
   - **Exists, no kluris pointer** -- propose appending a short
     `## Knowledge base` section (CLAUDE.md) or one-paragraph brain
     mention (AGENTS.md). Show the exact insertion point.
   - **Exists, has a stale pointer** (hardcoded brain paths, missing
     read/write framing, old skill name, list of specific neuron
     paths) -- propose reformulating to the canonical snippet. Show
     before/after.
   - **Exists, has the canonical snippet already** -- say so, do
     nothing.

3. **The canonical snippet is intentionally short and identical
   across both files.** This skill already teaches
   search/learn/remember/create -- the project-side pointer only
   needs the brain name and the skill name. Do not pad it with topic
   lists, neuron paths, or repeated CLI command tables. Do not make
   `AGENTS.md` defer to `CLAUDE.md` -- not every agent reads
   `CLAUDE.md`, so each file must stand on its own.

   The same `## Knowledge base` section goes in both files:

   ```markdown
   ## Knowledge base

   Read and write to the **{brain_name}** brain through kluris (never
   edit brain files by hand). Use the `kluris-{brain_name}` skill
   (usually `/kluris-{brain_name}`; Hermes: `/skill kluris-{brain_name}`) --
   search, learn, remember, create.
   ```

   When creating files from scratch, use these top-of-file headers:
   - `CLAUDE.md` -> `# CLAUDE.md` then the `## Knowledge base` section.
   - `AGENTS.md` -> `# Repository Guidelines` then the same
     `## Knowledge base` section.

4. **Approval protocol per file.** Show the full proposed content,
   ask "Is this correct? Want to change anything?", write only after
   explicit approval. Do not batch-write the two files -- one
   approval per file.

5. **This intent does not write to the brain.** Do not run
   `kluris dream{brain_flag_hint_inline}` and do not refresh the
   wake-up snapshot afterwards. Project-file edits do not change
   brain state.

## Writing rules

### Sync before you write

Before you write, change, or delete any brain file, run
`kluris sync{brain_flag_hint_inline} --json` and read the brain's entry in `brains[]`:
- `conflicts[]` is not empty: this brain and a teammate's push changed the
  same thing. STOP and show the human each conflict: its `path`, its `kind`
  (`content`, `delete_edit`, `parent`, or `frontmatter`), and what each side
  changed. The conflict's `base`, `ours`, and `theirs` are blob ids: read
  them with `git -C {brain_path} show <id>`, or compare with
  `git -C {brain_path} diff <base> <ours>` and `git -C {brain_path} diff <base> <theirs>`
  (a `null` side deleted the file). Ask which version to keep, or propose a
  merged text and apply the approval protocol to it. Then settle each path
  with `kluris resolve{brain_flag_hint_inline} <path> --ours`, `--theirs`, or
  `--file <merged file>`; the last one commits the merge and pushes it.
  Brain files never contain conflict markers, so never edit a file to
  resolve a conflict. If `kluris resolve` says the conflict is stale, run
  `kluris resolve{brain_flag_hint_inline} --abort` and sync again.
- `blocked[]` is not empty: those files have uncommitted changes that the
  incoming changes would overwrite. Ask the human to commit or stash them,
  then sync again.
- `refused` is set or `offline` is true: tell the human, then go on with the
  local brain.

### Approval protocol

For Learn, Remember, and Create flows:
1. Show the FULL content you intend to write, not a summary.
2. State the target lobe and neuron filename.
3. **Verify every `related:` path before showing the preview.** For each
   entry in `related:`, confirm the target file actually exists by running
   `ls {brain_path}/<relative-path>` (resolve the `../` hops from the neuron's
   target directory) or reading the file. If a path doesn't resolve, either
   fix it or drop the entry. Do NOT rely on your memory of the brain --
   `kluris search` tells you what exists, `ls` confirms the path. Invented
   paths become `broken_synapses` in `kluris dream` and poison the graph.
4. Ask: "Is this correct? Want to change anything?"
5. STOP. NEVER write until the human explicitly approves. Silence is not approval.
6. **Silent validation after writing.** Immediately after the write completes,
   run `kluris dream{brain_flag_hint_inline} --json` via Bash and inspect the
   result. If `broken_synapses`, `one_way_synapses`, or new `orphans`
   reference the neuron you just wrote, surface them to the user in plain
   language and propose fixes in the same turn -- don't wait for the user to
   notice. If the output is clean, tell the user "dream is clean" and move
   on.
7. **Commit and share what you wrote.** Once the human approved and the
   files are written, commit exactly those files with
   `kluris commit{brain_flag_hint_inline} --paths <every brain-relative path you wrote or deleted, comma-separated> --json`,
   with `KLURIS_AGENT` set to your agent's name (for example
   `KLURIS_AGENT="Claude Code"`) so the commit credits you. It commits those
   files and the `map.md`/`brain.md` files they change, under the human's git
   identity, and nothing else. Then run `kluris sync{brain_flag_hint_inline} --json` and report
   the `commit` and whether it was `pushed`. If sync reports `conflicts[]` or
   `blocked[]`, handle them as in "Sync before you write". If a git hook
   stops the commit, show its output to the human and fix what it reports;
   never pass `--no-verify` unless the human asks for it. A brain inside a
   larger git repository (sync refuses it): commit with `kluris commit`, then
   push with git the way the team normally does. A brain without git: skip
   this step.

Other writing rules:
- Never create `.md`, `.yml`, or `.yaml` neurons directly at the brain root.
- Frontmatter on every neuron: `parent`, `related`, `tags`, `created`, `updated`.
- `parent:` is always `./map.md`.
- `related:` paths are relative to the current neuron's directory.
- **H1 titles are the topic only -- never prefixed with the folder, project,
  or lobe name.** The path already carries that context. Write `# Architecture`
  in `projects/emailback/architecture.md`, not `# emailback - architecture`.
  Otherwise every neuron in a project reads "emailback - X", "emailback - Y",
  "emailback - Z" in visualizations and the repeated prefix drowns out the
  actual topic. Pick a filename that is already the topic name (hyphens allowed:
  `data-model.md`, `rest-apis.md`) and make the H1 match.
- **Actively hunt for synapses before writing.** Before proposing a new neuron,
  run `kluris search "<key-terms>"{brain_flag_hint_inline} --json` with a few
  searches drawn from the neuron's topic, tags, and glossary terms it uses. Any
  existing neuron that is genuinely related belongs in `related:`. Empty
  `related:` is allowed only when the brain truly has no connected neurons --
  verify this by search, do not assume.
- **Bidirectional synapses.** If A links to B in `related:`, you MUST also
  add A to B's `related:` in the same write turn. Show both files in the
  approval preview. `kluris dream` will flag one-way synapses as warnings --
  don't leave that cleanup for later.
- **Inline markdown links, not just `related:`.** Synapses in frontmatter are
  the structural graph; readers need links in the body too. Whenever a neuron's
  body mentions:
  - A term that exists in `glossary.md` -> link it the first time it appears
    in the body, like `[JWT](../../glossary.md#jwt)` (path relative to the
    current neuron; anchor is the term slugified).
  - Another neuron by name or topic -> link it inline on first mention, like
    `[auth flow](./auth-flow.md)`. Don't force every mention, just the first.
  - A yaml neuron -> link with a normal markdown link, e.g.
    `[API spec](./openapi.yml)`.

  Every entry in `related:` should also appear at least once as an inline link
  in the body (with rare exceptions like pure "see also" references).
- Glossary: when a neuron introduces a domain term not already in
  `glossary.md`, propose a glossary addition in the same approval turn, and
  link the first mention of the term in the neuron body to its glossary anchor.
- Focus on decisions and rationale, not just descriptions.
- Do not edit `map.md` or `brain.md`; they are auto-generated by `kluris dream{brain_flag_hint_inline}`.
- Do not clobber existing neurons. Read the current file, show the proposed
  diff or merged content, and write only after explicit approval.

Frontmatter example for a neuron at `projects/btb-core/auth-flow.md`:
```yaml
---
parent: ./map.md
related:
  - ../../infrastructure/docker-builds.md
  - ../../knowledge/use-raw-sql.md
tags: [api, auth, jwt]
created: 2026-04-06
updated: 2026-04-06
---
# Auth flow

Body content here.
```

## CLI commands

These are terminal commands, not skill actions:
- `kluris search "<query>"{brain_flag_hint_inline} --json` -- ranked search across neurons, glossary, and brain.md
- `kluris wake-up{brain_flag_hint_inline} --json` -- compact brain snapshot
- `kluris dream{brain_flag_hint_inline}` -- regenerate maps, auto-fix safe issues, and validate links
- `kluris status{brain_flag_hint_inline}` -- show brain tree, recent changes, and counts
- `kluris mri{brain_flag_hint_inline}` -- generate interactive visualization
- `kluris sync{brain_flag_hint_inline} --json` -- fetch, merge, and push the brain; reports `conflicts[]`, `blocked[]`, `refused`, and `offline`
- `kluris commit{brain_flag_hint_inline} --paths <a.md,b.md> --json` -- commit exactly those files and the maps they change
- `kluris resolve{brain_flag_hint_inline} <path> --ours|--theirs|--file <merged file> --json` -- settle a sync conflict path by path (`--abort` drops it)

Branch operations and anything else go through `git` directly -- kluris
brains are plain git repos. Use `git -C {brain_path} status / log / checkout`
as you would with any other repo.

