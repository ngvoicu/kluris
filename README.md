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
- **Agent memory** is session-scoped and ephemeral. A brain is persistent,
  version-controlled, and shared across the entire team.

One brain serves all your projects. Every AI agent on the team reads the same
knowledge. When someone leaves, nothing is lost.

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
kluris doctor        # Check prerequisites
kluris create        # Interactive wizard
```

By default, `kluris create <name>` makes a local git repo with no remote.
Add `--remote <url>` to connect one, or `--no-git` to skip git entirely.

Or skip the wizard:

```bash
kluris create my-brain --type product-group
kluris create my-brain --type personal --path ~/brains
kluris create my-brain --remote git@github.com:team/brain.git
```

Then open any project and run `/kluris learn the endpoints` -- the AI agent will
analyze that part of the codebase, show you a plan, and ask before writing
knowledge into the brain.

### Example workflow

```bash
# 1. Create a brain (wizard or one-liner)
kluris create

# 2. In your backend project, run the slash command:
#    /kluris learn about architecture and API design

# 3. In your frontend project:
#    /kluris learn about components and state management

# 4. Now any agent in any project can use the brain:
#    /kluris implement the new auth flow
#    (agent loads architecture decisions, API contracts, conventions)

# 5. After a session with useful decisions:
#    /kluris remember what we learned

# 6. Validate and push
kluris dream         # Regenerate maps, auto-fix safe issues, validate remaining links
kluris push          # Commit and push to git

# 7. Visualize the brain
kluris mri           # Run preflight fixes, then generate brain-mri.html
```

## What a brain looks like

```
acme-brain/
├── kluris.yml              # Local config (gitignored -- your agents, branch)
├── brain.md                # Root lobes directory (auto-generated)
├── glossary.md             # Domain terms (hand-edited)
├── README.md               # Usage guide
├── architecture/
│   ├── map.md              # Lobe index (auto-generated)
│   ├── auth-keycloak.md    # <- neuron
│   └── data-flow.md        # <- neuron
├── decisions/
│   ├── map.md
│   └── use-raw-sql.md      # <- neuron (decision template)
├── services/
│   ├── map.md
│   └── btb-backend/
│       ├── map.md
│       └── data-model.md
└── ...
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
| `architecture/` | System design, technical patterns, data flow, tech stack choices |
| `decisions/` | ADRs and key decisions across all domains (tech, product, business) |
| `product/` | PRDs, roadmap, features, user research |
| `standards/` | Coding standards, naming conventions, review checklists |
| `services/` | Per-service sub-folders -- each service gets its own map.md, APIs, data models |
| `infrastructure/` | Hosting, CI/CD, Docker, networking, deployment, environments |
| `cortex/` | Runbooks, playbooks, dev workflows, onboarding, migration plans |
| `wisdom/` | Domain knowledge, learnings, things figured out along the way |

The `services/` lobe nests deeper -- one sub-folder per service:

```
services/
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

1. `kluris create` scaffolds a brain (interactive wizard or flags)
2. `kluris install-skills` installs the Kluris skill for 8 AI agents
3. Use `/kluris` to search, learn, remember, and work with brain knowledge
4. Agents read the brain and apply team knowledge to tasks
5. `kluris dream` regenerates maps, auto-fixes safe issues, and validates remaining links
6. `kluris mri` runs the same safe preflight fixes as `dream`, then generates an interactive HTML visualization

## Release and publish

The PyPI publish pipeline is triggered by pushing a git tag that matches `v*`.
The workflow lives in [`.github/workflows/publish.yml`](/Users/gabrielvoicu/Projects/ngvoicu/kluris/kluris-cli/.github/workflows/publish.yml#L1) and listens for pushed tags such as `v1.0.8`.

Typical release flow:

```bash
# 1. Bump the package version in pyproject.toml and src/kluris/__init__.py

# 2. Verify the release candidate
pytest tests/ -q

# 3. Commit the release
git add pyproject.toml src/kluris/__init__.py tests/
git commit -m "chore: release v1.0.8"

# 4. Create the publish tag
git tag v1.0.8

# 5. Push the commit and the tag
git push origin main
git push origin v1.0.8
```

Once the `v1.0.8` tag reaches GitHub, the publish pipeline builds the package and publishes that version to PyPI.

## Slash command

One command does everything: `/kluris <natural language>`

```
/kluris what do we know about auth
/kluris learn the API endpoints from this project
/kluris remember what we learned we chose raw SQL over JPA for performance
/kluris implement the new auth flow using brain knowledge
/kluris create a decision record about migrating to Keycloak
/kluris create openapi docs for this service
```

The agent reads your intent and acts accordingly -- search, learn, remember,
think, create neurons, or generate OpenAPI docs. It reads the brain config
to find your brain path, then reads from and writes to that directory.

All writes ask for approval before creating files.

## CLI commands

| Command | What it does |
|---------|-------------|
| `kluris create` | Create a new brain (interactive wizard or `kluris create <name> --type product-group`) |
| `kluris clone` | Clone a brain from git (interactive or `kluris clone <url> --branch develop`) |
| `kluris list` | List all registered brains |
| `kluris status` | Show brain tree, recent changes, neuron counts |
| `kluris recall <query>` | Search brain and show results |
| `kluris dream` | Regenerate maps, auto-fix safe issues, and validate remaining links |
| `kluris push` | Commit and push brain changes to git |
| `kluris mri` | Run preflight fixes, then generate an interactive HTML brain visualization |
| `kluris use <name>` | Set the default brain |
| `kluris templates` | List available neuron templates |
| `kluris install-skills` | Install the Kluris skill into AI agent directories |
| `kluris uninstall-skills` | Remove the Kluris skill from AI agent directories |
| `kluris remove <name>` | Unregister a brain (keeps files) |
| `kluris doctor` | Check prerequisites (git, Python, config dir) |
| `kluris help` | Show all commands |

All commands support `--json` for machine-readable output.

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
agents:
  commands_for: [claude, cursor, windsurf, copilot, codex, kilocode, gemini, junie]
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
