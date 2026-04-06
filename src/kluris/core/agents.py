"""Agent registry and skill rendering (Agent Skills standard)."""

from __future__ import annotations

from pathlib import Path

# All agents now use SKILL.md in their skills directory
AGENT_REGISTRY: dict[str, dict] = {
    "claude": {"dir": ".claude", "subdir": "skills"},
    "cursor": {"dir": ".cursor", "subdir": "skills"},
    "windsurf": {"dir": ".codeium/windsurf", "subdir": "skills",
                  "also_workflow": ".codeium/windsurf/global_workflows"},
    "copilot": {"dir": ".copilot", "subdir": "skills"},
    "codex": {"dir": ".codex", "subdir": "skills"},
    "gemini": {"dir": ".gemini", "subdir": "skills"},
    "kilocode": {"dir": ".kilo", "subdir": "skills"},
    "junie": {"dir": ".junie", "subdir": "skills"},
}

# Old command directories to clean up during install
OLD_COMMAND_DIRS: dict[str, list[str]] = {
    "claude": [".claude/commands"],
    "cursor": [".cursor/commands"],
    "windsurf": [".codeium/windsurf/global_workflows", ".windsurf/workflows"],
    "copilot": [".copilot/agents"],
    "codex": [".agents/skills"],
    "gemini": [".gemini/commands"],
    "kilocode": [".config/kilo/commands", ".kilocode/commands"],
    "junie": [".junie/commands"],
}

SKILL_DESCRIPTION = """\
You are the team's subject matter expert. Your knowledge comes from a \
a brain -- a git-backed repo of architecture, decisions, conventions, \
and learnings curated by humans. Use this skill whenever the user mentions: \
brain, team knowledge, "what do we know about", decisions, documentation, \
API endpoints, conventions, deployment, "remember this", "store this", \
or wants to search, learn, document, or work with shared project knowledge. \
Also trigger when the user asks how things work, why decisions were made, \
or needs context from other projects. The brain paths are baked in below \
-- do not search for config files."""

SKILL_BODY = """\
{brain_info}

## You are the team's subject matter expert

The brain is a SEPARATE git repo at the path shown above. The current
directory is the PROJECT you're working in.
- ANALYZE the current project
- WRITE to the brain directory
- Never create brain.md, map.md, or kluris.yml in the current project

## How the brain is structured

Brains have different structures depending on their type. Do NOT assume
which lobes exist -- always read `brain.md` first to discover them.

The brain can be large. NEVER read it all at once. Navigate through indexes:
- `brain.md` is the root -- lists all lobes with one-line descriptions
- Each lobe has a `map.md` -- lists its neurons and sub-lobes
- Sub-lobes have their own `map.md`, and so on
- `glossary.md` defines domain-specific terms

Navigate top-down: brain.md → pick relevant lobes → read their map.md →
drill into the neurons you actually need. Max 3 lobes, max 10 neurons per query.
Follow `related:` links in neuron frontmatter to find connected knowledge.

## Intent detection

Understand the user's intent from their message:

**Search** -- "`/kluris search X`", "what do we know about X", "find info about Y"
Follow the reading protocol: start at brain.md, pick relevant lobes from their
descriptions, read their map.md, then drill into specific neurons. Summarize
findings. Read-only -- never write during a search.
If no relevant neurons are found after checking brain.md and lobe maps, say so
explicitly: "Nothing documented about X yet." Never fabricate or assume brain
content that doesn't exist.

**Think** -- "implement X", "work on Y using brain knowledge"
Before touching code, follow the reading protocol to load relevant context.
Quote the specific neuron paths you're applying (e.g. "Based on
knowledge/use-raw-sql.md..."). If no brain knowledge is relevant to the task,
say "No brain knowledge applies here" before proceeding.
If code contradicts a documented decision, STOP and show the conflict: what the
code does vs what the neuron says. Ask the human how to proceed -- update the
code, update the neuron, or proceed anyway with a note.

**Learn from project** -- "learn the endpoints", "document the schema"
Analyze the CURRENT PROJECT, write to the BRAIN.
Never overwrite existing neurons.
If user asks for OpenAPI: generate `openapi.yml` (OpenAPI 3.1), not markdown.

The brain is sacred. Writing to it is a collaborative process between you
and the human. You are partners building shared knowledge together.

Step 1 -- Discover (silent). Analyze the project, build an internal list of topics.
Step 2 -- Read brain.md to understand which lobes exist and what each is for.
Step 3 -- Summary. "I found N topics. Based on this brain's lobes, here's where
I'd put them. Let's walk through one at a time."
Step 4 -- Wizard (one topic at a time, this is the core loop):
  a. Show the FULL content you intend to write -- not a summary, the actual
     neuron with all sections, frontmatter, and links. The human must see
     exactly what will be written to judge correctness.
  b. State the target lobe and neuron name -- pick based on lobe descriptions.
  c. If you think part of this topic also belongs in another lobe or neuron,
     suggest it: "This also touches [other lobe] -- want a separate neuron
     there with a link?" Let the human decide.
  d. Ask: "Is this correct? Want to change anything?"
  e. The human may approve, edit, add context, or skip
  f. Incorporate feedback, show the updated version if changed
  g. Write ONLY after explicit approval
  h. Move to the next topic
Step 5 -- Recap. What was written, what was skipped. Remind: `kluris dream`
to regenerate maps. If the brain has git (shown above), also `kluris push`.

Lobe routing -- when learning a single project, default to putting everything
under that project's folder (e.g. projects/<name>/). Only route to other lobes
when something is genuinely cross-cutting -- shared infrastructure used by
multiple projects, or a decision that affects ALL projects, not just this one.
If unsure, ask: "This could go in [other lobe] since it's cross-cutting,
or stay in projects/<name>/ since it's specific to this project. Which do
you prefer?"
Read existing neurons in target lobes first -- update or extend, don't create duplicates.
Domain terms and acronyms discovered → include as a wizard step: show the
proposed glossary additions, ask for approval before appending to `glossary.md`.
Glossary format -- one term per line: `**Term** -- Definition in one sentence.`
Keep definitions under 20 words.

**Remember** -- "remember we chose X", "store that we decided Y"
Write a specific piece of knowledge to the brain.
Find the right lobe, check for existing neurons.
Show a preview of what you'd write. Ask: "Is this correct? Want to change anything?"
Write only after approval.

**Create neuron** -- "create a decision record about X"
Templates: decision (Context, Decision, Rationale, Alternatives, Consequences),
incident (Summary, Timeline, Root cause, Impact, Resolution, Lessons learned),
runbook (Purpose, Prerequisites, Steps, Rollback, Contacts).
Show the populated template before writing. Walk through sections one at a time
for complex templates -- don't dump a full decision record without review.

**Create lobe** -- "create a new section for monitoring"
Create directory in brain. Remind user to run `kluris dream`.

## Writing rules

- Frontmatter on every neuron: parent, related, tags, created, updated
- Bidirectional synapses: if A links to B, add reverse link in B
- Focus on decisions and rationale, not just descriptions
- Do NOT edit map.md or brain.md -- auto-generated by `kluris dream`
- After writing, remind user to run `kluris dream` (and `kluris push` if brain has git)

Frontmatter example:
```yaml
parent: projects/btb-core
related:
  - infrastructure/docker-builds.md
  - knowledge/use-raw-sql.md
tags: [api, auth, jwt]
created: 2026-04-06
updated: 2026-04-06
```

## CLI commands (for mechanical operations)

These are terminal commands, not skill actions:
- `kluris dream` -- regenerate maps, auto-fix safe issues, validate remaining links
- `kluris push` -- commit and push brain changes to git
- `kluris mri` -- run preflight fixes, then generate interactive visualization
- `kluris templates` -- list neuron templates
"""


# Single COMMANDS dict for backward compat with render_commands
COMMANDS = {
    "kluris": {
        "description": SKILL_DESCRIPTION,
        "body": SKILL_BODY,
    },
}


def render_skill(brain_info: str = "") -> str:
    """Render the kluris SKILL.md content."""
    body = SKILL_BODY.replace("{brain_info}", brain_info)
    desc = SKILL_DESCRIPTION.replace('"', '\\"')
    return (
        "---\n"
        f"name: kluris\n"
        f'description: "{desc}"\n'
        "---\n\n"
        f"{body}\n"
    )


def _render_workflow(brain_info: str = "") -> str:
    """Render a Windsurf workflow .md file (for /kluris manual invocation)."""
    body = SKILL_BODY.replace("{brain_info}", brain_info)
    desc = SKILL_DESCRIPTION[:200].replace('"', '\\"')
    return (
        f"---\n"
        f'description: "{desc}"\n'
        f"---\n\n"
        f"{body}\n"
    )


def render_commands(agent_name: str, output_dir: Path, brain_info: str = "") -> list[Path]:
    """Install kluris SKILL.md for an agent."""
    skill_dir = output_dir / "kluris"
    skill_dir.mkdir(parents=True, exist_ok=True)
    content = render_skill(brain_info)
    path = skill_dir / "SKILL.md"
    path.write_text(content, encoding="utf-8")
    return [path]


def install_workflow(workflow_dir: Path, brain_info: str = "") -> Path:
    """Install a Windsurf workflow .md for /kluris manual invocation."""
    workflow_dir.mkdir(parents=True, exist_ok=True)
    wf_file = workflow_dir / "kluris.md"
    wf_file.write_text(_render_workflow(brain_info), encoding="utf-8")
    return wf_file


def install_for_agent(agent_name: str, home: Path | None = None) -> list[Path]:
    """Install kluris skill for a specific agent at the correct home path."""
    reg = AGENT_REGISTRY[agent_name]
    base = (home or Path.home()) / reg["dir"] / reg["subdir"]
    return render_commands(agent_name, base)
