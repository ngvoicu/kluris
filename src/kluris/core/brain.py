"""Brain scaffolding, type defaults, and validation."""

from __future__ import annotations

import re
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
    "team": {
        "structure": {
            "architecture": "System design, technical patterns",
            "decisions": "ADRs and key decisions across all domains",
            "product": "PRDs, roadmap, features, user research",
            "standards": "Coding standards, naming conventions, review checklists",
            "services": "Per-service sub-folders with setup, APIs, data models",
            "infrastructure": "Hosting, CI/CD, Docker, networking, deployment",
            "cortex": "Runbooks, playbooks, dev workflows, onboarding, migration plans",
            "wisdom": "Domain knowledge, learnings, dated notes",
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
    """Check if a brain name is valid (lowercase alphanumeric + hyphens)."""
    if not name:
        return False
    if ".." in name or name.startswith("."):
        return False
    return bool(BRAIN_NAME_PATTERN.match(name))


def get_type_defaults(brain_type: str) -> dict:
    """Return the default structure and templates for a brain type."""
    return BRAIN_TYPES.get(brain_type, BRAIN_TYPES["blank"])


def scaffold_brain(
    brain_path: Path,
    name: str,
    description: str,
    brain_type: str,
    custom_config: dict | None = None,
) -> None:
    """Create a brain directory with all scaffolded files."""
    brain_path.mkdir(parents=True, exist_ok=True)

    defaults = get_type_defaults(brain_type)
    structure = (custom_config or {}).get("structure", defaults["structure"])
    neuron_templates = defaults.get("neuron_templates", {})

    # Create lobe directories with empty map.md
    for lobe_name, lobe_desc in structure.items():
        lobe_dir = brain_path / lobe_name
        lobe_dir.mkdir(exist_ok=True)
        map_content = (
            f"---\nauto_generated: true\nparent: ../brain.md\n"
            f"updated: 2026-04-01\n---\n# {lobe_name.replace('-', ' ').title()}\n\n"
            f"{lobe_desc}\n"
        )
        (lobe_dir / "map.md").write_text(map_content, encoding="utf-8")

    # Write kluris.yml — local config only (gitignored)
    config = BrainConfig(
        name=name,
        description=description,
        type=brain_type,
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
        f"---\nauto_generated: true\nupdated: 2026-04-01\n---\n"
        f"# {name}\n\n{description}\n\n## Lobes\n\n{lobe_links}\n\n"
        f"## Neuron Index\n\n0 neurons.\n\n"
        f"| Neuron | Lobe | Tags | Updated |\n"
        f"|--------|------|------|---------|\n\n"
        f"## Reference\n\n"
        f"- [glossary.md](./glossary.md) — Domain-specific terms\n"
    )
    (brain_path / "brain.md").write_text(brain_md, encoding="utf-8")

    # Write glossary.md
    glossary_md = (
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n"
        "# Glossary\n\nProject-specific terms, acronyms, and conventions.\n\n"
        "| Term | Meaning |\n|------|---------||\n"
    )
    (brain_path / "glossary.md").write_text(glossary_md, encoding="utf-8")

    # Write README.md
    tree_lines = [f"├── {lobe}/" for lobe in structure]
    tree_output = "\n".join(tree_lines) if tree_lines else "(empty)"
    readme_md = _generate_readme(name, description, tree_output)
    (brain_path / "README.md").write_text(readme_md, encoding="utf-8")

    # Write .gitignore
    (brain_path / ".gitignore").write_text(GITIGNORE_CONTENT, encoding="utf-8")


def _generate_readme(name: str, description: str, tree_output: str) -> str:
    """Generate the brain README content."""
    return f"""\
# {name}

> {description}

## What is this?

This is a **Kluris brain** — a git-backed, structured knowledge base designed
to be read and written by both humans and AI coding agents.

## Quick start

### For a new team member

```bash
pipx install kluris
kluris clone <this-repo-url> ~/{name}
```

### Start populating the brain

Run `/kluris.learn` in any project. This is the fastest way to build team
knowledge.

### Using with AI agents

| Command | What it does |
|---------|-------------|
| `/kluris <anything>` | **Main command.** Natural language. |
| `/kluris.think <task>` | Load brain knowledge, then work as team expert. |
| `/kluris.remember [topic]` | Extract knowledge from session or topic. |
| `/kluris.learn [focus]` | Deep-scan current project, populate brain. |
| `/kluris.recall <topic>` | Search the brain. |
| `/kluris.neuron <topic>` | Create a new knowledge file. |
| `/kluris.lobe <name>` | Create a new knowledge region. |
| `/kluris.push [msg]` | Commit and push. **Only command that writes to git.** |
| `/kluris.dream [focus]` | Brain maintenance: validate, repair, strengthen. |

### Using the CLI

```bash
kluris status          # Brain tree and recent changes
kluris recall <query>  # Search across neurons
kluris dream           # Regenerate maps, validate links
kluris push            # Commit and push
kluris doctor          # Check prerequisites
kluris help            # All commands
```

## Brain structure

```
{tree_output}
```

## Neuron templates

Use templates for consistent structure. Run `kluris templates` to see what's available.

```bash
kluris neuron use-raw-sql.md --lobe decisions --template decision
```

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
  - ../../standards/naming.md
tags: [auth, keycloak]
created: 2026-03-15
updated: 2026-03-31
---
# Title

Content here.
```

## Rules

1. **Don't edit `map.md` or `brain.md`** — auto-generated by `kluris dream`
2. **Do edit `glossary.md`** — add domain terms and acronyms
3. **Always include frontmatter** — parent, related, tags, created, updated
4. **Use standard markdown links** — `[label](./relative/path.md)`
5. **Focus on decisions and rationale** — "we chose X because Y"
6. **Bidirectional synapses** — if A links to B, add the reverse link in B
7. **Run `/kluris.push` to save** — agents never auto-commit
8. **Run `kluris dream` after structural changes**
"""
