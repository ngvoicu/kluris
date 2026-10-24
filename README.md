# Kluris

> Kluris turns your AI agents into a subject matter expert that never sleeps and never quits.

*When your best engineer sleeps, Kluris doesn't. When they leave, Kluris stays.*

> 🎥 **New here?** Take the [guided tour at kluris.ngvoicu.dev](https://kluris.ngvoicu.dev/presentation.html) — install, first brain, agent workflows, multi-brain, git collaboration, and the MRI visualization, end to end.

## What is Kluris?

Kluris gives every AI agent on your team shared knowledge -- architecture,
decisions, conventions, learnings -- so they work like an SME who knows your
entire codebase, not a generic assistant starting from scratch every time.

Knowledge is stored in a **brain**: a git-backed repo of structured markdown
that agents read, search, and apply automatically. The human and agent curate
the brain together -- the agent proposes what to document, the human reviews
and approves every piece.

## Why it saves tokens

A brain is a pre-digested summary your agent reads instead of the raw files.
Without it, the agent crawls the whole repo (and every sibling repo it needs
context from) on every new chat. With a brain, it loads one compact snapshot
and jumps straight to the neuron it needs.

**Without kluris** — illustrative cold-start on a medium repo:

```text
tree + README + CLAUDE.md         ~3,000 tokens
grep for related symbols          ~2,000 tokens
read 4-8 relevant files           ~15,000 tokens
read 2-3 sibling-project files    ~8,000 tokens
                                  ─────────────
                                  ~28,000 tokens just to orient
```

**With kluris** — same task, brain-backed:

```text
kluris wake-up --json snapshot    ~1,200 tokens (brain.md + lobes + recent + glossary)
kluris search "<query>" --json    ~400 tokens   (ranked hits with snippets)
read 1-2 matching neurons         ~1,500 tokens
                                  ─────────────
                                  ~3,100 tokens — ~9x less context burned on orientation
```

Rough estimates, not a benchmark — actual numbers depend on repo size, agent,
and prompt style. The shape holds: the brain replaces a wide crawl with a
targeted lookup. Savings compound the more projects you touch. Agents that
would have re-read five repos' worth of source per morning read one brain
snapshot instead -- same answers, a fraction of the context window, which
means more room for the actual work.

### Why not a wiki, Notion, or CLAUDE.md?

- **Wikis and Notion** are for humans. Agents can't read them, search across
  them, or write back. A brain is markdown in git -- AI-native.
- **CLAUDE.md** is per-project and per-tool. A brain sits above all your
  projects and works with every AI agent on the team.
- **Agent memory** is agent-controlled -- the agent decides what to keep.
  A brain is human-curated -- you decide what goes in, review every entry,
  and correct anything that's wrong.

One brain serves all your projects. Every agent reads the same knowledge.
Version-controlled, human-curated, shared across the entire team.

## Two surfaces: terminal vs. AI agent

Kluris lives in two places. Knowing which is which makes everything else click.

| Surface | Prompt | Where you type it | What it is | Examples |
|---------|:------:|-------------------|-----------|----------|
| **Terminal** | `$ kluris …` | Your shell — bash, zsh, fish, PowerShell | The `kluris` Python CLI | `kluris create`, `kluris dream`, `kluris status`, `kluris mri`, `kluris doctor` |
| **AI agent** | `> /kluris-<name> …` | Inside your coding agent — Claude Code, Cursor, Windsurf, Codex, Copilot, Gemini CLI, Hermes Agent, Kilo, Junie | The per-brain skill Kluris keeps refreshed automatically | `/kluris-acme learn …`, `/skill kluris-acme`, `hermes -s kluris-acme` |

Throughout this README:

- Code blocks labelled ```` ```bash ```` or tagged **"In your terminal"** are meant for the shell. They start with `$` (or `>` on Windows PowerShell).
- Code blocks tagged **"Inside your AI coding agent"** are meant for Claude Code / Cursor / Windsurf / Codex / Copilot / Gemini / Hermes / Kilo / Junie. Most start with `/kluris-<name>`; Hermes uses `/skill kluris-<name>` or `hermes -s kluris-<name>`.

Every command you'll see below belongs to exactly one of these two surfaces.

## Install

**In your terminal** — install the `kluris` CLI with pipx. Pick your OS:



**macOS:**
```bash
brew install pipx && pipx ensurepath
```

**Linux:**
```bash
python3 -m pip install --user pipx && pipx ensurepath
```

**Windows:**
```bash
pip install pipx && pipx ensurepath
```

Then restart your terminal and:

```bash
pipx install kluris
```

## Quick start

**In your terminal** — create your first brain:

```bash
kluris doctor        # check prerequisites
kluris create        # interactive wizard -- name, description, location, type, git, companions
```

Then, **inside your AI coding agent** (Claude Code, Cursor, Windsurf, …) — open any project directory and use your brain's named skill:

```text
> /kluris-acme learn everything about this service
```

The agent analyzes your code and walks you through each finding one at a
time. You see a small preview before anything is written, and you approve, edit,
or skip every piece.

### How the agent bootstraps (automatic)

On the first `/kluris-<name>` call of a session, the agent runs
`kluris wake-up --brain <name> --json`
through its shell to load a compact snapshot of the brain: the `brain.md` body,
lobes with neuron counts, the 5 most recently updated neurons, the full glossary,
and any deprecation warnings. That's enough context for the agent to decode
jargon and avoid citing superseded neurons without touching the filesystem
again for the rest of the session. You never call it manually. The agent
refreshes the snapshot after mutating commands (`/kluris-<name> remember`,
`/kluris-<name> learn`, `kluris dream --brain <name>`) or direct brain-file edits.

If you want to see what the agent sees, run it yourself **in your terminal**:

```bash
kluris wake-up            # pretty text
kluris wake-up --json     # machine-readable
kluris wake-up --brain X  # target a specific brain when more than one is registered
```

### Working with multiple brains

Each registered brain installs as its own skill from the beginning:
`kluris-<name>` (usually invoked as `/kluris-<name>`; Hermes uses `/skill kluris-<name>`).
Every per-brain skill is bound to exactly one brain, so the agent never has to
guess which one you mean and project pointers stay stable as more brains are added.

CLI commands prompt interactively when 2+ brains are registered:

- Fan-out commands (`dream`, `status`, `mri`, `companion` commands) show
  `[1] acme [2] personal [3] all`. Pick a single brain or apply to every brain.
- Single-brain commands (`wake-up`, `search`, `pack`) show
  `[1] acme [2] personal` (no `all` option).

Pass `--brain NAME` to skip the picker, or `--brain all` on fan-out commands
to act on every brain at once. Scripts and CI must always pass `--brain`
because non-TTY contexts disable the picker — set `KLURIS_NO_PROMPT=1` to
force non-interactive mode even from a TTY (useful for wrappers like Claude
Code that inherit a terminal but cannot block on prompts).

### Joining an existing brain

A Kluris brain is a plain git repository. To adopt one, clone it with `git`
and register the local directory with kluris:

**In your terminal:**

```bash
git clone git@github.com:team/brain.git ~/brains/acme   # plain git
kluris register ~/brains/acme                            # adopt it
```

Registration is in-place -- Kluris does not copy or move the source. If a
teammate handed you a zip, unzip it first (`unzip brain.zip -d ~/brains/acme`)
and then run `kluris register ~/brains/acme`.

### Onboarding a project

When you start work in a new project, two things make every future
session smoother. Both happen **inside your AI coding agent** (Claude
Code, Cursor, Windsurf, …) with the project open.

**1. Wire the brain into the project.** Drop a small pointer into
the project's `CLAUDE.md` and `AGENTS.md` so every coding agent that
lands in the repo sees the brain. The skill (and only the skill)
reads and writes the brain — never edit brain files by hand.

```text
> /kluris-acme setup this project
```

The agent surveys the project, checks whether `CLAUDE.md` and
`AGENTS.md` exist, and either creates them, appends the pointer, or
reformulates an old pointer it finds. Each file change goes through
the standard approval protocol.

The same `## Knowledge base` section goes in **both** files — each
must stand on its own because not every coding agent reads
`CLAUDE.md`:

```markdown
## Knowledge base

Read and write to the **<brain>** brain through kluris (never edit
brain files by hand). Use the `/kluris-<brain>` skill — search,
learn, remember, create.
```

That's it. The skill teaches the agent the rest.

**2. Have the agent learn the project.** The agent analyzes your
code and walks you through each finding. You review, edit, and
approve before anything is written.

```text
> /kluris-acme learn the API endpoints and data model
> /kluris-acme learn the Docker and deployment setup
> /kluris-acme learn everything about this service
```

The agent starts with a preview before writing. You can change the
target lobe, edit the content, add context the code doesn't show, or
skip.

### Storing decisions and knowledge

**Inside your AI coding agent:**

```text
> /kluris-acme remember we chose raw SQL over JPA for query complexity
> /kluris-acme remember all timestamps must be TIMESTAMPTZ
> /kluris-acme remember from this session
> /kluris-acme create a decision record about the auth architecture
> /kluris-acme create an incident report for the January outage
```

`/kluris-<name> remember from this session` is the sweep variant — the agent
replays the current chat, pulls out everything that belongs in the brain,
and walks you through each proposed neuron one at a time (approve, edit,
or skip). Use it at the end of a pairing session instead of re-typing every
decision as its own `remember`.

### Using brain knowledge while coding

**Inside your AI coding agent:**

```text
> /kluris-acme search auth flow
> /kluris-acme search Docker setup
> /kluris-acme what do we know about the auth flow?
> /kluris-acme implement the new endpoint following our conventions
> /kluris-acme fix the token refresh -- use brain knowledge
> /kluris-acme use brain knowledge and codebase and let's write a spec for OAuth sign-in
> /kluris-acme use brain knowledge and codebase and let's implement auth
```

The agent reads the brain first, then works on the task. If your code
contradicts a documented decision, it flags the conflict. Prefixing with
*"use brain knowledge and codebase"* tells the agent to ground the work
in **both** surfaces — the decisions and conventions in the brain **and**
the actual current shape of the code — before writing a spec or changing
a line. It's the right posture for anything non-trivial.

### Reviewing the brain itself

**Inside your AI coding agent:**

```text
> /kluris-acme review this brain and create more synapses
> /kluris-acme review this brain for gaps and stale neurons
```

The agent reads across every neuron, spots connections you never wrote
down, and proposes new synapses — one by one, with approve-all or
one-by-one flow. Handy after a `learn` binge or a big merge.

### Maintaining the brain

**In your terminal:**

```bash
kluris dream         # regenerate maps, fix links, validate structure
kluris status        # brain tree, neuron counts, recent changes
kluris mri           # generate visualization, prints the link to open in your browser
```

Brains are git repos. Use `git push` / `git pull` / `git checkout` from the
brain directory like any other repo. Run `kluris dream` first if you've made
structural changes so the auto-generated maps land in the same commit.

### Deprecating a decision

When a decision is superseded, mark the old neuron instead of deleting it --
the history is valuable. Add these optional frontmatter fields to the old
neuron:

```yaml
---
status: deprecated
deprecated_at: 2026-04-01
replaced_by: ./use-clerk.md
---
```

`kluris dream` reports four kinds of deprecation warnings (non-blocking):

- `active_links_to_deprecated`: an active neuron's `related:` points at a
  deprecated one -- update the link to point at the replacement.
- `deprecated_without_replacement`: a deprecated neuron has no `replaced_by`
  -- add one so readers have a migration path.
- `replaced_by_missing`: `replaced_by` points at a file that doesn't exist.
- `replaced_by_not_active`: `replaced_by` points at something that isn't an
  active neuron (another deprecated neuron, or a non-neuron file like
  `map.md`).

Agents see a `deprecation_count` summary via `kluris wake-up` and the full
list via `kluris dream --json`. They flag affected topics when asked.

## What a brain looks like

```
acme-brain/
├── kluris.yml              # Local config (gitignored -- agents, companions)
├── brain.md                # Root lobes directory (auto-generated)
├── glossary.md             # Domain terms (hand-edited)
├── README.md               # Usage guide
├── .gitignore              # Secrets, kluris.yml, brain-mri.html
├── projects/
│   ├── map.md              # Lobe index (auto-generated)
│   └── btb-core/
│       ├── map.md
│       ├── data-model.md   # <- neuron
│       └── auth-flow.md    # <- neuron
├── infrastructure/
│   ├── map.md
│   ├── docker-builds.md    # <- neuron
│   └── environments.md     # <- neuron
└── knowledge/
    ├── map.md
    └── use-raw-sql.md      # <- neuron (a decision record)
```

Folders are **lobes** (knowledge regions). Files are **neurons** (knowledge
units). Links between neurons are **synapses**. Auto-generated `map.md` files
keep everything navigable.

## Visualize with `kluris mri`

Run `kluris mri` to generate a self-contained HTML file that renders the brain
as an interactive, C4-style architecture map. Kluris prints a `file://` link to
the file; it does not open a browser for you. The file is written to
`brain-mri.html` in the brain directory (gitignored by default) — pass
`--output <file>` to put it somewhere else.

The map shows one level at a time, as flat cards on a canvas:

- It opens on the brain's top-level lobes (plus `glossary.md`). Click a folder
  to drill into it — lobe, then sublobe, then neurons, as deep as your folders
  go. The breadcrumb and the back / forward buttons walk you back up.
- Every card shows its kind (`LOBE`, `SUBLOBE`, `NEURON`), a title and a stats
  line. Synapses between sibling cards are drawn as edges with a count.
- Click a neuron to read it in a detail modal. Drag to pan, scroll to zoom.
- The left panel is the file tree. The right panel searches neurons, paths,
  lobes, tags and YAML (press `/` to focus it) and lists recently updated
  neurons.

![kluris mri — the root view of the ngvoicu-sme brain: its lobes and glossary as C4-style boxes joined by synapse counts, with the file tree on the left and Find and Recent on the right](https://kluris.ngvoicu.dev/mri-overview.png)

The HTML file is yours — no server, no account, no external calls. Email it,
drop it in Slack, or open it straight from disk.

## Pack a brain as a Docker chat server

Not everyone on your team has a coding agent. `kluris pack` turns a brain into
a self-contained **Docker chat server**: a web chat that answers questions by
searching and reading the brain, next to a read-only explorer of its files. The
brain is baked into the image read-only — the chat cannot change it.

**In your terminal** — you need Docker with Compose v2:

```bash
kluris pack --brain acme       # writes ./acme-pack/ (--output <dir> to choose)
cd acme-pack
cp .env.example .env           # then fill in your LLM settings
docker compose up --build
# open http://localhost:8765
```

- **LLM settings** go in `.env`: the endpoint, key and model of an Anthropic or
  OpenAI-compatible provider, or OAuth client credentials for an internal
  gateway. `.env.example` documents every setting.
- **Where the pack lands.** By default `./<brain-name>-pack`, and never inside a
  brain. `--exclude '<glob>'` (repeatable) keeps files out of the bundled brain.
  After you edit the brain, `kluris pack --brain acme --force` (same output
  directory) rebuilds the pack and keeps your `.env`; then run
  `docker compose up --build` again.
- **There is no authentication, by design.** Anyone who can reach port `8765`
  can chat, read everything in the bundled brain and read the saved
  conversations, and compose publishes the port on all interfaces. Run it on a
  trusted host or network, and put your own access control (reverse proxy, VPN,
  private network) in front if it is reachable more widely.

The generated pack carries its own `README.md` that covers configuration: LLM
settings and tuning, TLS for private gateways, the system prompt, pushing the
image to a registry, and re-packing.

## Brain types (scaffolding only)

Types determine the initial folder structure. After creation, every brain
works the same -- all commands are available regardless of type. You can add
or remove lobes freely after creation.

### product-group (default)

For a group of projects/services that share knowledge. Example: a platform
with 3 backends, a frontend, and shared infrastructure.

| Lobe | What goes in it |
|------|----------------|
| `projects/` | Per-project sub-folders -- APIs, data models, setup, conventions |
| `infrastructure/` | Hosting, CI/CD, Docker, deployment, environments, env vars |
| `knowledge/` | Decisions, learnings, troubleshooting tips, domain expertise |

The `projects/` lobe nests deeper -- one sub-folder per project:

```
projects/
├── map.md
├── btb-core/
│   ├── map.md
│   ├── auth-flow.md
│   └── endpoints/
│       ├── map.md
│       └── post-auth-login.md
├── btb-frontend/
│   ├── map.md
│   └── state-management.md
└── btb-summon/
    └── map.md
```

Project neurons link to infrastructure neurons for deployment details
and environments -- never duplicate infra content across lobes.

### personal

For an individual developer's knowledge -- projects, tasks, and notes.

| Lobe | What goes in it |
|------|----------------|
| `projects/` | Sub-folder per project: branches, status, TODOs |
| `tasks/` | Current priorities, blockers, in-progress work |
| `notes/` | Daily notes, ideas, learnings |

### product

For product management -- requirements, features, and user research.

| Lobe | What goes in it |
|------|----------------|
| `prd/` | Requirements, user stories, acceptance criteria |
| `features/` | Sub-folder per feature: specs, status, feedback |
| `ux/` | User research, personas, journey maps, wireframes |
| `analytics/` | Metrics, KPIs, experiment results |
| `competitors/` | Competitive analysis, market positioning |
| `decisions/` | Product decisions and rationale |

### research

For research projects -- literature, experiments, and findings.

| Lobe | What goes in it |
|------|----------------|
| `literature/` | Papers, articles, summaries, key findings |
| `experiments/` | Hypotheses, methodology, results |
| `findings/` | Synthesized insights, conclusions |
| `datasets/` | Data sources, schemas, access notes |
| `tools/` | Research tools, scripts, environments |
| `questions/` | Open questions, hypotheses to test |

### blank

Empty -- build your own structure from scratch.

## How it works

1. **Terminal** — `kluris create` creates a brain (interactive wizard)
2. **Terminal** — Kluris refreshes the `kluris-<name>` skill automatically
3. **Terminal** — optional companions add embedded specmint workflows per brain
4. **Inside your AI coding agent** — open any project and load/invoke `kluris-<name>`; the agent becomes an SME
5. Agent and human curate the brain together — you review and approve every entry
6. **Terminal** — `kluris dream` maintains brain structure
7. **Terminal** — `kluris mri` visualizes the brain

## Commands reference

Kluris has two surfaces — the terminal CLI and the per-brain skills you load inside your AI coding agent. Here they are side by side.

### In your terminal — `$ kluris *`

Run these in bash, zsh, fish, or PowerShell. They handle setup, maintenance, and anything the agent calls internally.

| Command | What it does |
|---------|-------------|
| `kluris create` | Create a new brain (interactive wizard) |
| `kluris register <path>` | Register an existing brain directory on disk |
| `kluris list` | List registered brains |
| `kluris status` | Brain tree, neuron counts, recent changes |
| `kluris search <query>` | Ranked search across neurons, glossary, brain.md (`--lobe`, `--tag`, `--limit`, `--json`) |
| `kluris wake-up` | Compact brain snapshot for agent session bootstrap — includes `brain_md`, `glossary`, `deprecation` (`--json`) |
| `kluris companion add specmint-core\|specmint-tdd\|specmint-core-html\|specmint-tdd-html` | Opt a brain into an embedded companion playbook |
| `kluris companion list` | List known companions, installed runtime copies, and brain opt-ins |
| `kluris companion remove specmint-core\|specmint-tdd\|specmint-core-html\|specmint-tdd-html` | Remove a companion opt-in from a brain |
| `kluris dream` | Regenerate maps, fix links, validate structure (`--json`, `--broken-only`) |
| `kluris pack` | Pack a brain into a self-contained Docker chat server — see [Pack](#pack-a-brain-as-a-docker-chat-server) (`--output`, `--exclude`, `--force`, `--json`) |
| `kluris mri` | Generate the brain visualization as a standalone HTML file and print its link (`--output`) |
| `kluris remove <name>` | Unregister a brain (keeps files on disk) |
| `kluris doctor` | Check prerequisites, refresh agent skills, and refresh companion playbooks after `pipx upgrade kluris`. Pass `--no-refresh` to skip writes. |
| `kluris help` | Show command help |

All CLI commands support `--json` for machine-readable output.

Sync, commit, and branch operations go through `git` directly. Brains are
plain git repos — use `git -C <brain-path> push / pull / status / checkout`
from the brain directory like any other repo.

### Inside your AI coding agent — `> /kluris-<name> ...`

Type these inside Claude Code, Cursor, Windsurf, GitHub Copilot, Codex, Gemini CLI, Hermes Agent, Kilo, or Junie. Every registered brain installs as `kluris-<name>`; most agents expose it as `/kluris-<name>`, and Hermes loads it with `/skill kluris-<name>` or `hermes -s kluris-<name>`. Examples below use `/kluris-acme` for clarity.

| Pattern | What the agent does |
|---------|---------------------|
| `/kluris-acme learn <topic>` | Analyzes your code and proposes neurons one at a time (you approve each) |
| `/kluris-acme remember <fact>` | Captures a decision as a neuron in the right lobe |
| `/kluris-acme remember from this session` | Sweeps the current chat and proposes neurons for everything worth keeping |
| `/kluris-acme search <term>` | Searches the brain for a topic |
| `/kluris-acme what do we know about ...` | Answers a question grounded in the brain |
| `/kluris-acme review this brain` | Audits the brain — proposes new synapses, flags gaps and stale neurons |
| `/kluris-acme use brain knowledge and codebase and let's ...` | Grounds spec-writing or implementation in **both** the brain and the actual code |
| `/kluris-acme implement <task>` | Implements a task following your brain's conventions |
| `/kluris-acme fix <bug>` | Fixes a bug using brain knowledge, flags any conflicts |
| `/kluris-acme create a decision record about ...` | Walks you through a new neuron section by section, e.g. a decision record or an incident postmortem |
| `/kluris-acme create a new section for monitoring` | Proposes a new lobe and creates it once you approve |
| `/kluris-acme fix the broken synapses` | Repairs the broken links `kluris dream` reports, one proposal at a time |
| `/kluris-acme setup this project` | Adds the `## Knowledge base` pointer to the project's `CLAUDE.md` and `AGENTS.md` |

Agent patterns are free-form — say it naturally. Under the hood the agent calls `kluris search` for lookups and `kluris wake-up` for the session bootstrap, but you never type those yourself when using the generated skill.

## Local config (kluris.yml)

Each brain has a `kluris.yml` that is **gitignored** -- it's your local config,
not shared. Each team member can have different settings.

```yaml
name: my-brain
description: my-brain knowledge base
agents:
  commands_for:       # agents that get a kluris-<name> skill (all nine by default)
  - claude
  - cursor
  - windsurf
  - copilot
  - codex
  - gemini
  - hermes
  - kilocode
  - junie
companions: []        # see "Pair with Specmint"
```

Trim `agents.commands_for` to the agents you use and run `kluris doctor`:
Kluris then stops writing skills for the others (it installs for every agent
listed by any registered brain, and it does not delete skills it already
installed).

## Brain vocabulary

| Term | Meaning |
|------|---------|
| **Brain** | Git repo of shared team knowledge |
| **Lobe** | Folder / knowledge region |
| **Neuron** | Single knowledge file |
| **Synapse** | Link between neurons (bidirectional) |
| **Map** | `map.md` -- auto-generated lobe index |
| **MRI** | Interactive brain visualization |
| **Dream** | Brain maintenance -- regenerate maps, update dates, auto-fix safe issues, validate remaining links |

## Supported agents

Kluris installs a skill into nine agents: Claude Code, Cursor, Windsurf, GitHub Copilot, Codex, Gemini CLI, Hermes Agent, Kilo Code, Junie

Kluris writes one `kluris-<name>/SKILL.md` per brain into each of these agents' skill folders:

| Agent | Skills directory |
|-------|------------------|
| Claude Code | `~/.claude/skills/` |
| Cursor | `~/.cursor/skills/` |
| Windsurf | `~/.codeium/windsurf/skills/` (plus a `kluris-<name>.md` workflow in `~/.codeium/windsurf/global_workflows/`) |
| GitHub Copilot | `~/.copilot/skills/` |
| Codex | `~/.codex/skills/` |
| Gemini CLI | `~/.gemini/skills/` |
| Hermes Agent | `~/.hermes/skills/` and every `~/.hermes/profiles/*/skills/` |
| Kilo Code | `~/.kilo/skills/` |
| Junie | `~/.junie/skills/` |

It also writes the universal `~/.agents/skills/` folder. The skill is plain instructions plus shell calls to the `kluris` CLI, so any other agent that loads `SKILL.md` skills from one of those folders (or is pointed at the file) and can run a shell can use it too. [OpenCode](https://opencode.ai/docs/skills/), for one, loads skills from `~/.claude/skills/` and `~/.agents/skills/`.

Most agents load the generated skill as `/kluris-<name>`. Hermes users can load it with `/skill kluris-<name>` inside a session or start Hermes with `hermes -s kluris-<name>`. If you create a new Hermes profile after creating/registering a brain, run `kluris doctor` to refresh that profile's skills.

## Pair with Specmint

Kluris is the brain. [Specmint](https://specmint.ngvoicu.dev) turns a feature request
into a persistent spec built from deep research and iterative interviews.
Pair them and the research phase starts half-done — grounded in your code
*and* the knowledge your team already agreed to.

**Inside your AI coding agent:**

```text
> /kluris-acme let's spec out adding OAuth sign-in with GitHub
```

When a brain has a companion enabled, its `kluris-<name>` skill tells the agent
to pause before multi-step work (a new feature, a refactor across files, a
migration) and follow the embedded Specmint playbook: set up, research,
interview, then write the spec. Small single-file edits skip it. The skill's
brain-first rules ("query first -- never guess") stay in force alongside it.

Four flavors are installable per brain as Kluris companions:

| Companion | What it is |
|-----------|-----------|
| [`specmint-core`](https://github.com/ngvoicu/specmint-core) | Spec-first workflow — Research · Interview · Spec · Implement |
| [`specmint-tdd`](https://github.com/ngvoicu/specmint-tdd) | Same forge flow with strict TDD — a failing test before any implementation |
| `specmint-core-html` | Spec-first workflow where `.specs/<id>/SPEC.html` is the canonical spec document |
| `specmint-tdd-html` | TDD-focused workflow where `.specs/<id>/SPEC.html` carries tasks, RGR state, logs, and progress |

Use the HTML variants when you want the spec itself to be a visual, shareable, presentation-ready HTML document. Use the non-HTML variants when you prefer plain markdown specs as the canonical source of truth.

Companions ship inside the `kluris` Python package, re-vendored from Specmint
v3. Each one is a folder: a `SKILL.md` plus the `references/` it points at and,
for the two HTML flavors, `assets/` (the spec's runtime script and stylesheet).
Enabling one copies that folder into `~/.kluris/companions/<name>/` and adds a
short reference snippet to that brain's generated Kluris skill; the companions
are read from there, never installed as skills of their own. `kluris doctor`
refreshes the installed copies, so `pipx upgrade kluris && kluris doctor` also
brings the playbooks up to date. `kluris companion remove` drops a brain's
opt-in and keeps the copy under `~/.kluris/companions/`.

**Enable for one brain:**

```bash
kluris companion add specmint-core --brain my-brain
kluris companion add specmint-tdd --brain my-brain
kluris companion add specmint-core-html --brain my-brain
kluris companion add specmint-tdd-html --brain my-brain
```

**Enable for every registered brain:**

```bash
kluris companion add specmint-core --brain all
kluris companion add specmint-tdd --brain all
kluris companion add specmint-core-html --brain all
kluris companion add specmint-tdd-html --brain all
```

More at [specmint.ngvoicu.dev](https://specmint.ngvoicu.dev).

## License

MIT

---

<!-- ngvoicu author section — identical across all ngvoicu repos, keep in sync -->
## AI-native toolkit

This project is part of a larger AI-native toolkit — and of a way of working your whole team can adopt: talks (["Becoming an AI Native Company"](https://ngvoicu.dev/becoming-an-ai-native-company/)), hands-on team training that teaches employees to use AI, and [AI adoption consulting for engineering teams](https://ngvoicu.dev/#consulting).

- Site: [ngvoicu.dev](https://ngvoicu.dev)
- Contact: [office@ngvoicu.dev](mailto:office@ngvoicu.dev) · +40 734 704 910

Toolkit: [Specmint](https://specmint.ngvoicu.dev) (durable AI coding specs) · [Kluris](https://kluris.ngvoicu.dev) (team knowledge brains) · [ConsensFlow](https://consensflow.ngvoicu.dev) (cross-agent second opinions)
