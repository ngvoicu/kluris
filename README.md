# Kluris

> Create and manage git-backed AI brains for multi-project, multi-agent teams.

*When your best engineer sleeps, Kluris doesn't. When they leave, Kluris stays.*

## What is Kluris?

Kluris is a CLI tool that creates **brains** -- standalone git repos of
structured markdown that AI coding agents read, search, and update through
globally installed slash commands.

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

## Quick start

```bash
pipx install kluris
kluris doctor        # Check prerequisites
kluris create        # Interactive wizard
```

Or skip the wizard:

```bash
kluris create my-brain --type team
kluris create my-brain --type personal --path ~/brains
kluris create my-brain --remote git@github.com:team/brain.git
```

Then open any project and run `/kluris.learn` -- the AI agent will analyze
your codebase and populate the brain with architecture, conventions, APIs,
and decisions.

### Example workflow

```bash
# 1. Create a brain (wizard or one-liner)
kluris create

# 2. In your backend project, run the slash command:
#    /kluris.learn focus on architecture and API design

# 3. In your frontend project:
#    /kluris.learn focus on components and state management

# 4. Now any agent in any project can use the brain:
#    /kluris.think implement the new auth flow
#    (agent loads architecture decisions, API contracts, conventions)

# 5. After a session with useful decisions:
#    /kluris.remember

# 6. Validate and push
kluris dream         # Regenerate maps, validate links
kluris push          # Commit and push to git

# 7. Visualize the brain
kluris mri           # Generate brain-mri.html
```

## What a brain looks like

```
acme-brain/
├── kluris.yml              # Local config (gitignored -- your agents, branch)
├── brain.md                # Root index + neuron table (auto-generated)
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

| Type | Lobes | Use case |
|------|-------|----------|
| `team` (default) | architecture, decisions, product, standards, services, infrastructure, cortex, wisdom | Shared team knowledge |
| `personal` | projects, tasks, notes | Individual developer |
| `product` | prd, features, ux, analytics, competitors, decisions | Product management |
| `research` | literature, experiments, findings, datasets, tools, questions | Research projects |
| `blank` | (empty) | Build from scratch |

## How it works

1. `kluris create` scaffolds a brain (interactive wizard or flags)
2. `kluris install` generates slash commands for 8 AI agents
3. Agents use `/kluris.learn` to scan projects and populate the brain
4. Team members use `/kluris.think <task>` to load brain context before working
5. `kluris dream` validates links, regenerates maps and neuron index
6. `kluris mri` generates an interactive HTML visualization

## Slash commands (used inside AI agents)

| Command | What it does |
|---------|-------------|
| `/kluris <anything>` | **Main command.** Read, write, or search the brain. Natural language. |
| `/kluris.think <task>` | Load brain knowledge, then work on the task as the team's expert. |
| `/kluris.recall <topic>` | Search the brain and summarize what it knows (read-only). |
| `/kluris.learn [focus]` | Deep-scan the current project and populate the brain. |
| `/kluris.remember [topic]` | Extract knowledge from the current session into the brain. |
| `/kluris.neuron <topic>` | Create a new knowledge file (supports `--template`). |
| `/kluris.lobe <name>` | Create a new knowledge region (folder). |
| `/kluris.push [msg]` | Commit and push brain changes to git. |
| `/kluris.dream [focus]` | AI-powered brain analysis. Run `kluris dream` CLI for mechanical fixes. |

**think vs recall:** `/kluris.think` reads the brain then works on your task
as an expert. `/kluris.recall` just searches and reports what the brain knows
-- it doesn't do any work.

## CLI commands

| Command | What it does |
|---------|-------------|
| `kluris create` | Create a new brain (interactive wizard or `kluris create <name> --type team`) |
| `kluris clone` | Clone a brain from git (interactive or `kluris clone <url> --branch develop`) |
| `kluris list` | List all registered brains |
| `kluris status` | Show brain tree, recent changes, neuron counts |
| `kluris recall <query>` | Search brain and show results |
| `kluris neuron <file>` | Create a neuron (`--lobe`, `--template decision`) |
| `kluris lobe <name>` | Create a lobe (`--parent` for nesting) |
| `kluris dream` | Regenerate maps and neuron index, validate links |
| `kluris push` | Commit and push brain changes to git |
| `kluris mri` | Generate interactive HTML brain visualization |
| `kluris templates` | List available neuron templates |
| `kluris install` | Install slash commands for all AI agents |
| `kluris remove <name>` | Unregister a brain (keeps files) |
| `kluris doctor` | Check prerequisites (git, Python, config dir) |
| `kluris help` | Show all commands |

All commands support `--json` for machine-readable output.

## Neuron templates

Available in every brain. Use `kluris templates` to see them.

```bash
kluris neuron auth-migration.md --lobe decisions --template decision
```

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
  auto_push: true
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
| **Dream** | Brain maintenance -- regenerate, validate, repair |

## Supported agents

Claude Code, Cursor, Windsurf, GitHub Copilot, Codex, Gemini CLI, Kilo Code, Junie

## License

MIT
