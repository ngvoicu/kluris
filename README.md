# Kluris

> Create and manage git-backed AI brains for multi-project, multi-agent teams.

*When your best engineer sleeps, Kluris doesn't. When they leave, Kluris stays.*

## What is Kluris?

Kluris is a CLI tool that creates **brains** -- standalone git repos of
structured markdown that AI coding agents read, search, and update through
globally installed agent skills and workflows.

**Kluris = the tool. A brain = the git repo it creates.**

### Why not a wiki, Notion, or CLAUDE.md?

- **Wikis and Notion** are for humans. Agents can't natively read them, search
  across them, or write back. Kluris brains are markdown in git -- AI-native.
- **CLAUDE.md** is per-project and per-tool. A brain sits above all your
  projects and works with 8 different AI agents simultaneously.
- **Agent memory** is selective and agent-controlled -- the agent decides what
  to keep. A brain is a collaboration: the agent reads your projects and
  proposes what to document, but the human reviews every piece, decides what
  to store, and corrects or enriches the content before it's written.

One brain serves all your projects. Every AI agent on the team reads the same
knowledge. Version-controlled, human-curated, shared across the entire team.

## Install

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

```bash
kluris doctor        # check prerequisites
kluris create        # interactive wizard -- name, type, location, git
```

Then open any project directory and use `/kluris`:

```
/kluris learn everything about this service
```

The agent analyzes your code and walks you through each finding one at a
time. You see a small preview before anything is written, and you approve, edit,
or skip every piece.

### Joining an existing brain

```bash
kluris clone git@github.com:team/brain.git    # clone and register
```

### Learning a project

Open any project directory and use `/kluris` -- the agent analyzes your code
and walks you through each finding. You review, edit, and approve before
anything is written.

```
/kluris learn the API endpoints and data model
/kluris learn the Docker and deployment setup
/kluris learn everything about this service
```

The agent starts with a preview before writing. You can change the
target lobe, edit the content, add context the code doesn't show, or skip.

### Storing decisions and knowledge

```
/kluris remember we chose raw SQL over JPA for query complexity
/kluris remember all timestamps must be TIMESTAMPTZ
/kluris create a decision record about the auth architecture
/kluris create an incident report for the January outage
```

### Using brain knowledge while coding

```
/kluris search auth flow
/kluris search Docker setup
/kluris what do we know about the auth flow?
/kluris implement the new endpoint following our conventions
/kluris fix the token refresh -- use brain knowledge
```

The agent reads the brain first, then works on the task. If your code
contradicts a documented decision, it flags the conflict.

### Maintaining the brain

```bash
kluris dream         # regenerate maps, fix links, validate structure
kluris push          # commit and push to git (if brain uses git)
kluris status        # brain tree, neuron counts, recent changes
kluris mri           # interactive visualization
```

## What a brain looks like

```
acme-brain/
├── kluris.yml              # Local config (gitignored -- your agents, branch)
├── brain.md                # Root lobes directory (auto-generated)
├── glossary.md             # Domain terms (hand-edited)
├── README.md               # Usage guide
├── projects/
│   ├── map.md              # Lobe index (auto-generated)
│   └── btb-backend/
│       ├── map.md
│       ├── data-model.md   # <- neuron
│       └── auth-flow.md    # <- neuron
├── infrastructure/
│   ├── map.md
│   ├── docker-builds.md    # <- neuron
│   └── environments.md     # <- neuron
└── knowledge/
    ├── map.md
    └── use-raw-sql.md      # <- neuron (decision template)
```

Folders are **lobes** (knowledge regions). Files are **neurons** (knowledge
units). Links between neurons are **synapses**. Auto-generated `map.md` files
keep everything navigable.

## Brain types (scaffolding only)

Types determine the initial folder structure. After creation, every brain
works the same -- all templates and commands are available regardless of type.
You can add or remove lobes freely after creation.

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
├── btb-backend/
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

1. `kluris create` scaffolds a brain with an interactive wizard
2. `kluris install-skills` installs the `/kluris` skill for your AI agents
3. Open any project and use `/kluris search`, `/kluris learn`, `/kluris remember`, or `/kluris create`
4. The agent reads your code, proposes what to document, you review and approve
5. `kluris dream` regenerates maps, fixes links, validates structure
6. `kluris push` commits and pushes to git

## CLI commands

| Command | What it does |
|---------|-------------|
| `kluris create` | Create a new brain (interactive wizard) |
| `kluris clone <url>` | Clone a brain from git |
| `kluris list` | List all registered brains |
| `kluris use <name>` | Switch the default brain |
| `kluris status` | Brain tree, neuron counts, recent changes |
| `kluris neuron <name>` | Create a neuron (optionally with `--lobe` and `--template`) |
| `kluris lobe <name>` | Create a new lobe (optionally with `--parent` for nesting) |
| `kluris dream` | Regenerate maps, fix links, validate structure |
| `kluris push` | Commit and push brain changes to git |
| `kluris mri` | Generate interactive HTML brain visualization |
| `kluris templates` | List available neuron templates |
| `kluris install-skills` | Install the `/kluris` skill for your AI agents |
| `kluris uninstall-skills` | Remove the `/kluris` skill from AI agent directories |
| `kluris remove <name>` | Unregister a brain (keeps files on disk) |
| `kluris doctor` | Check prerequisites (git, Python, config) |
| `kluris help` | Show command help |

All commands support `--json` for machine-readable output.
The CLI is for mechanical operations. Search and guided documentation happen through `/kluris` inside your agent.

## Neuron templates

Available in every brain. Use `kluris templates` to see them.

| Template | Sections |
|----------|----------|
| `decision` | Context, Decision, Rationale, Alternatives considered, Consequences |
| `incident` | Summary, Timeline, Root cause, Impact, Resolution, Lessons learned |
| `runbook` | Purpose, Prerequisites, Steps, Rollback, Contacts |

## Local config (kluris.yml)

Each brain has a `kluris.yml` that is **gitignored** -- it's your local config,
not shared. Each team member can have different settings.

```yaml
name: my-brain
description: my-brain knowledge base
git:
  default_branch: main
  commit_prefix: "brain:"
```

## Brain vocabulary

| Term | Meaning |
|------|---------|
| **Brain** | Git repo of structured markdown |
| **Lobe** | Folder / knowledge region |
| **Neuron** | Single knowledge file |
| **Synapse** | Link between neurons (bidirectional) |
| **Map** | `map.md` -- auto-generated lobe index |
| **MRI** | Interactive brain visualization |
| **Dream** | Brain maintenance -- regenerate maps, update dates, auto-fix safe issues, validate remaining links |

## Supported agents

Claude Code, Cursor, Windsurf, GitHub Copilot, Codex, Gemini CLI, Kilo Code, Junie

## License

MIT
