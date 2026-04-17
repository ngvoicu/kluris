"""Brain scaffolding, type defaults, and validation."""

from __future__ import annotations

import re
from datetime import date
from pathlib import Path

import yaml

from kluris.core.config import BrainConfig, GitConfig, AgentsConfig, NeuronTemplate


# --- Brain type defaults ---

# Neuron templates -- available to ALL brains regardless of type
NEURON_TEMPLATES: dict[str, dict] = {
    "decision": {
        "description": "Architecture or product decision record",
        "sections": ["Context", "Decision", "Rationale",
                     "Alternatives considered", "Consequences"],
    },
    "incident": {
        "description": "Incident or outage postmortem",
        "sections": ["Summary", "Timeline", "Root cause",
                     "Impact", "Resolution", "Lessons learned"],
    },
    "runbook": {
        "description": "Operational procedure",
        "sections": ["Purpose", "Prerequisites", "Steps",
                     "Rollback", "Contacts"],
    },
}

# Brain types -- only used for initial scaffolding
BRAIN_TYPES: dict[str, dict] = {
    "product-group": {
        "structure": {
            "projects": "Per-project sub-folders with APIs, data models, setup, conventions",
            "infrastructure": "Hosting, CI/CD, Docker, deployment, environments, env vars",
            "knowledge": "Decisions, learnings, troubleshooting tips, domain expertise",
        },
    },
    "personal": {
        "structure": {
            "projects": "Sub-folder per project: branches, status, TODOs",
            "tasks": "Current priorities, blockers, in-progress work",
            "notes": "Daily notes, ideas, learnings",
        },
    },
    "product": {
        "structure": {
            "prd": "Requirements, user stories, acceptance criteria",
            "features": "Sub-folder per feature: specs, status, feedback",
            "ux": "User research, personas, journey maps, wireframes",
            "analytics": "Metrics, KPIs, experiment results",
            "competitors": "Competitive analysis, market positioning",
            "decisions": "Product decisions and rationale",
        },
    },
    "research": {
        "structure": {
            "literature": "Papers, articles, summaries, key findings",
            "experiments": "Hypotheses, methodology, results",
            "findings": "Synthesized insights, conclusions",
            "datasets": "Data sources, schemas, access notes",
            "tools": "Research tools, scripts, environments",
            "questions": "Open questions, hypotheses to test",
        },
    },
    "blank": {
        "structure": {},
    },
}

BRAIN_NAME_PATTERN = re.compile(r"^[a-z][a-z0-9-]*$")
BRAIN_NAME_RESERVED = frozenset({"all"})
BRAIN_NAME_MAX_LENGTH = 48

GITIGNORE_CONTENT = """\
# Secrets
.env
*.key
*.pem
*.p12
*.pfx
credentials.*
secrets.*

# Local config (not shared — each user has their own agent/git settings)
kluris.yml

# Generated
brain-mri.html

# OS
.DS_Store
Thumbs.db
"""


def _today() -> str:
    return date.today().isoformat()


def lookup_template(name: str, templates: dict) -> dict | None:
    """Look up a neuron template by name. Returns None if not found."""
    return templates.get(name)


def generate_neuron_content(
    title: str,
    parent_map: str,
    template_name: str | None = None,
    sections: list[str] | None = None,
) -> str:
    """Generate neuron markdown content with frontmatter."""
    from datetime import date
    today = date.today().isoformat()

    frontmatter_lines = [
        "---",
        f"parent: {parent_map}",
    ]
    if template_name:
        frontmatter_lines.append(f"template: {template_name}")
    frontmatter_lines.extend([
        "related: []",
        "tags: []",
        f"created: {today}",
        f"updated: {today}",
        "---",
    ])

    body_lines = [f"# {title}", ""]
    if sections:
        for section in sections:
            body_lines.extend([f"## {section}", "", ""])

    return "\n".join(frontmatter_lines) + "\n" + "\n".join(body_lines)


def validate_brain_name(name: str) -> bool:
    """Check if a brain name is valid.

    Rules: non-empty; not a reserved word (e.g. ``all`` collides with
    ``--brain all``); at most BRAIN_NAME_MAX_LENGTH characters; lowercase
    alphanumeric plus hyphens; first char is a letter; no path traversal.
    """
    if not name:
        return False
    if name in BRAIN_NAME_RESERVED:
        return False
    if len(name) > BRAIN_NAME_MAX_LENGTH:
        return False
    if ".." in name or name.startswith("."):
        return False
    return bool(BRAIN_NAME_PATTERN.match(name))


def get_type_defaults(brain_type: str) -> dict:
    """Return the default structure and templates for a brain type."""
    return BRAIN_TYPES.get(brain_type, BRAIN_TYPES["blank"])


def infer_brain_type(brain_path: Path) -> str:
    """Best-effort brain type inference from top-level lobe directories."""
    lobe_names = sorted(
        item.name
        for item in brain_path.iterdir()
        if item.is_dir() and item.name != ".git"
    )
    if not lobe_names:
        return "blank"

    for brain_type, defaults in BRAIN_TYPES.items():
        if sorted(defaults["structure"].keys()) == lobe_names:
            return brain_type

    return "product-group"


def scaffold_brain(
    brain_path: Path,
    name: str,
    description: str,
    brain_type: str,
    custom_config: dict | None = None,
    branch: str = "main",
) -> None:
    """Create a brain directory with all scaffolded files."""
    brain_path.mkdir(parents=True, exist_ok=True)

    defaults = get_type_defaults(brain_type)
    structure = (custom_config or {}).get("structure", defaults["structure"])
    today = _today()

    # Create lobe directories with empty map.md
    for lobe_name, lobe_desc in structure.items():
        lobe_dir = brain_path / lobe_name
        lobe_dir.mkdir(exist_ok=True)
        map_content = (
            f"---\nauto_generated: true\nparent: ../brain.md\ndescription: {lobe_desc}\n"
            f"updated: {today}\n---\n# {lobe_name.replace('-', ' ').title()}\n\n"
            f"{lobe_desc}\n"
        )
        (lobe_dir / "map.md").write_text(map_content, encoding="utf-8")

    # Write kluris.yml — local config only (gitignored)
    config = BrainConfig(
        name=name,
        description=description,
        git=GitConfig(),
        agents=AgentsConfig(),
    )
    config_data = config.model_dump(exclude_none=True)
    (brain_path / "kluris.yml").write_text(
        yaml.dump(config_data, default_flow_style=False, sort_keys=False), encoding="utf-8"
    )

    # Write brain.md
    lobe_links = "\n".join(
        f"- [{lobe}/](./{lobe}/map.md) — {desc}"
        for lobe, desc in structure.items()
    )
    brain_md = (
        f"---\nauto_generated: true\nupdated: {today}\n---\n"
        f"# {name}\n\n{description}\n\n## Lobes\n\n{lobe_links}\n\n"
        f"## Reference\n\n"
        f"- [glossary.md](./glossary.md) — Domain-specific terms\n"
    )
    (brain_path / "brain.md").write_text(brain_md, encoding="utf-8")

    # Write glossary.md
    glossary_md = (
        f"---\nauto_generated: false\nupdated: {today}\n---\n"
        "# Glossary\n\nProject-specific terms, acronyms, and conventions.\n\n"
        "| Term | Meaning |\n|------|---------|\n"
    )
    (brain_path / "glossary.md").write_text(glossary_md, encoding="utf-8")

    # Write README.md
    tree_lines = [f"├── {lobe}/" for lobe in structure]
    tree_output = "\n".join(tree_lines) if tree_lines else "(empty)"
    readme_md = _generate_readme(name, description, tree_output, brain_type, structure)
    (brain_path / "README.md").write_text(readme_md, encoding="utf-8")

    # Write .gitignore
    (brain_path / ".gitignore").write_text(GITIGNORE_CONTENT, encoding="utf-8")


def _generate_readme(name: str, description: str, tree_output: str,
                     brain_type: str, structure: dict) -> str:
    """Generate the brain README content with type-specific lobe explanations."""

    # Build lobe table
    if structure:
        lobe_rows = "\n".join(
            f"| `{lobe}/` | {desc} |"
            for lobe, desc in structure.items()
        )
        lobe_section = (
            f"## Lobes\n\n"
            f"| Lobe | What goes in it |\n"
            f"|------|----------------|\n"
            f"{lobe_rows}\n"
        )
    else:
        lobe_section = "## Lobes\n\n(empty -- add lobes with `kluris lobe <name>`)\n"

    return f"""\
# {name}

> {description}

## What is this?

This is a **Kluris brain** -- shared team knowledge that turns AI agents into
subject matter experts. Curated by humans, read by every agent on the team.

> 🎥 **New to kluris?** The [10-slide guided tour at kluris.io](https://kluris.io/presentation.html)
> walks through install, first brain, agent workflows, multi-brain, git
> collaboration, and the MRI visualization — end to end.

## Quick start

### For a new team member

```bash
pipx install kluris
kluris clone <this-repo-url>
```

### Start building your team knowledge

One command does everything: `/kluris-{name} <natural language>`.

**Important:** `/kluris-{name}` is a slash command for coding agents -- use it
inside Claude Code, Codex, Cursor, Windsurf, Cline, Devin, Gemini CLI, or any
AI coding tool that supports skills. It is NOT a terminal command. For terminal
operations, use the `kluris` CLI directly (see [CLI commands](#cli-commands)
below).

{lobe_section}

## Brain structure

```
{tree_output}
```

## How to use /kluris-{name}

`/kluris-{name}` is a slash command for AI coding agents (Claude Code, Codex,
Cursor, Windsurf, Cline, Devin, Gemini CLI, and others). Type it inside your
coding agent -- not in a regular terminal. The agent detects your intent and
acts accordingly. Search and guided documentation happen through the slash
command; the `kluris` CLI is for mechanical operations like `dream`, `push`,
and `status` that you run in a terminal.

If `{name}` is the only kluris brain registered on your machine, kluris also
exposes a bare `/kluris` slash command as an alias -- both forms produce the
same skill body and you can use either. With multiple brains registered, each
brain installs as `/kluris-<name>` so you can address them unambiguously.

### Bootstrap (automatic)

On the first `/kluris-{name}` call of each session, the agent runs
`kluris wake-up --brain {name} --json` via its shell and caches a compact
snapshot of the brain: lobes with neuron counts, the 5 most recently updated
neurons, total neuron count. You never run it manually. The agent refreshes
the snapshot after mutating commands (`/kluris-{name} remember`,
`/kluris-{name} learn`, `kluris neuron --brain {name}`,
`kluris lobe --brain {name}`, `kluris dream --brain {name}`,
`kluris push --brain {name}`).

If you want to peek at what the agent sees, run
`kluris wake-up --brain {name}` yourself.

### Search -- ask the SME

```
/kluris-{name} search authentication
/kluris-{name} search Docker setup
/kluris-{name} what do we know about authentication?
/kluris-{name} how does the Docker setup work?
/kluris-{name} what conventions do we follow for API naming?
/kluris-{name} find everything related to Keycloak
```

Under the hood the agent calls `kluris search "<query>" --brain {name} --json`
which ranks matches across neurons, the glossary, and brain.md in one pass
and returns the top results as JSON. You can run the same command yourself
from a terminal:

```bash
kluris search "authentication" --brain {name}                        # pretty table
kluris search "oauth" --brain {name} --lobe knowledge --json         # JSON, scoped to knowledge/
kluris search "SIT" --brain {name} --tag infra --limit 5             # filter by tag
```

Read-only. Results with `deprecated: true` point at superseded neurons
and the agent will prefer their `replaced_by` target.

### Think -- work on a task using brain knowledge

```
/kluris-{name} add a new API endpoint for user preferences
/kluris-{name} fix the auth token refresh -- use brain knowledge
/kluris-{name} refactor the data layer following our conventions
/kluris-{name} implement the notification system
```

The agent reads the brain first (project docs, infrastructure, knowledge),
then works on the task. If the task contradicts a documented decision,
it flags the conflict before proceeding.

### Learn -- collaboratively document a project into the brain

```
/kluris-{name} learn the API endpoints from this project
/kluris-{name} learn the database schema
/kluris-{name} learn about the Docker and deployment setup
/kluris-{name} learn everything about this service
```

This is a collaborative wizard, not a dump. The agent analyzes the project,
then walks through findings one at a time:
1. Shows a small preview of what it intends to write
2. Suggests the target lobe and neuron name
3. Asks: "Is this correct? Want to change anything?"
4. You approve, edit, add context the code doesn't show, or skip
5. Writes only after your explicit approval

The agent routes findings to the correct lobes and suggests cross-lobe
links when a topic spans multiple areas.

### Remember -- store a specific decision or piece of knowledge

```
/kluris-{name} remember we chose raw SQL over JPA for performance
/kluris-{name} remember the frontend health check is at /api/health
/kluris-{name} remember we use Cloudflare Tunnel with zero public ports
/kluris-{name} store that all timestamps must be TIMESTAMPTZ
```

The agent finds the right lobe, shows a preview, and asks for confirmation
before writing.

### Create -- make a new neuron from a template

```
/kluris-{name} create a decision record about migrating to Keycloak
/kluris-{name} create an incident report for the January outage
/kluris-{name} create a runbook for deploying to production
/kluris-{name} create a new lobe for monitoring
```

For structured templates (decision, incident, runbook), the agent walks
through sections one at a time so you can review each part.

## CLI commands

```bash
kluris search "<query>" --brain {name}  # Ranked search across neurons + glossary + brain.md
kluris status --brain {name}             # Brain tree, recent changes, neuron counts
kluris wake-up --brain {name}            # Compact snapshot for agent bootstrap (--json for machines)
kluris templates                          # List available neuron templates
kluris dream --brain {name}               # Regenerate maps, auto-fix safe issues, validate remaining links
kluris branch --brain {name}              # Show, switch, or create branches
kluris push --brain {name}                # Commit and push to the current branch
kluris pull --brain {name}                # Pull remote changes for the current branch
kluris mri --brain {name}                 # Generate visualization (prints link to open in browser)
kluris help                               # All commands
```

If `{name}` is your only registered brain, you can drop `--brain {name}` from
every CLI invocation -- kluris auto-resolves the single brain.

## Neuron templates

| Template | Sections |
|----------|----------|
| `decision` | Context, Decision, Rationale, Alternatives considered, Consequences |
| `incident` | Summary, Timeline, Root cause, Impact, Resolution, Lessons learned |
| `runbook` | Purpose, Prerequisites, Steps, Rollback, Contacts |

## File format

Every neuron uses YAML frontmatter + markdown:

```yaml
---
parent: ../map.md
related:
  - ../../infrastructure/environments.md
tags: [auth, keycloak]
created: 2026-03-15
updated: 2026-03-31
---
# Title

Content here.
```

### Deprecating a decision

When a decision is superseded, mark the old neuron instead of deleting it --
history matters. Add these optional frontmatter fields:

```yaml
---
parent: ../map.md
status: deprecated
deprecated_at: 2026-04-01
replaced_by: ./use-clerk.md
tags: [auth]
created: 2025-11-12
updated: 2026-04-01
---
```

`kluris dream` reports four kinds of deprecation warnings (non-blocking):

- `active_links_to_deprecated` -- an active neuron's `related:` points at a
  deprecated neuron; update the link to the replacement.
- `deprecated_without_replacement` -- a deprecated neuron has no
  `replaced_by`; add one so readers have a migration path.
- `replaced_by_missing` -- `replaced_by` points at a file that does not
  exist.
- `replaced_by_not_active` -- `replaced_by` points at something that is not
  an active neuron (another deprecated neuron -- a dead chain -- or a
  non-neuron file like `map.md`).

Dream still exits clean on deprecation warnings alone. `kluris wake-up`
shows a `deprecation_count` summary; `kluris dream --json` returns the full
`deprecation[]` list with each issue's `kind`, `file`, and optional `target`.

## Local config (kluris.yml)

Each brain has a `kluris.yml` that is **gitignored** -- your local config,
not shared. Each team member can have different settings.

```yaml
name: my-brain
description: my-brain knowledge base
git:
  commit_prefix: "brain:"
```

## Rules

1. **Don't edit `map.md` or `brain.md`** -- auto-generated by `kluris dream`
2. **Do edit `glossary.md`** -- add domain terms and acronyms
3. **Always include frontmatter** -- parent, related, tags, created, updated
4. **Use standard markdown links** -- `[label](./relative/path.md)`
5. **Focus on decisions and rationale** -- "we chose X because Y"
6. **Bidirectional synapses** -- if A links to B, add the reverse link in B
7. **Run `kluris dream` after adding neurons** -- keeps maps and brain.md fresh
8. **Run `kluris push` to save** -- commits and pushes to git (if brain uses git)
"""
