"""Kluris CLI — Click entry point."""

from __future__ import annotations

import json as json_lib
import subprocess
import sys
from pathlib import Path

import click
from rich.console import Console
from rich.table import Table
from rich.tree import Tree

from kluris.core.brain import (
    BRAIN_TYPES,
    generate_neuron_content,
    infer_brain_type,
    lookup_template,
    scaffold_brain,
    validate_brain_name,
)
from kluris.core.config import (
    BrainEntry,
    read_brain_config,
    read_global_config,
    register_brain,
    unregister_brain,
    write_global_config,
)
from kluris.core.git import (
    git_add,
    git_clone,
    git_commit,
    git_init,
    is_git_repo,
    git_log,
    git_push,
    git_status,
)
from kluris.core.linker import (
    check_frontmatter,
    detect_deprecation_issues,
    detect_orphans,
    fix_bidirectional_synapses,
    fix_missing_frontmatter,
    validate_bidirectional,
    validate_synapses,
)
from kluris.core.maps import generate_brain_md, generate_index_md, generate_map_md
from kluris.core.mri import generate_mri_html
from kluris.core.frontmatter import read_frontmatter, update_frontmatter
from kluris.core.agents import AGENT_REGISTRY, COMMANDS, OLD_COMMAND_DIRS, render_commands, install_workflow

console = Console()


def _read_brain_identity(brain_path: Path, fallback_name: str) -> tuple[str, str]:
    """Infer the canonical brain name and description from brain.md."""
    brain_md = brain_path / "brain.md"
    default_description = f"{fallback_name} knowledge base"
    if not brain_md.exists():
        return fallback_name, default_description

    try:
        _, content = read_frontmatter(brain_md)
    except Exception:
        try:
            content = brain_md.read_text(encoding="utf-8")
        except OSError:
            return fallback_name, default_description

    title = ""
    description = ""
    title_seen = False
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("# ") and not title:
            title = line[2:].strip()
            title_seen = True
            continue
        if title_seen and not line.startswith("## "):
            description = line
            break

    name = title if validate_brain_name(title) else fallback_name
    return name, description or default_description


def _brain_directories(brain_path: Path) -> list[Path]:
    """Return all brain directories, deepest first so children get map.md before parents."""
    directories = [
        path for path in brain_path.rglob("*")
        if path.is_dir() and ".git" not in path.parts
    ]
    return sorted(
        directories,
        key=lambda path: (-len(path.relative_to(brain_path).parts), str(path)),
    )


def _run_dream_on_brain(brain_path: Path) -> None:
    """Regenerate maps, brain.md, and index.md for a single brain."""
    try:
        brain_config = read_brain_config(brain_path)
        directories = _brain_directories(brain_path)
        # Pass 1: create all map.md (deepest first so parents see children)
        for lobe in directories:
            generate_map_md(brain_path, lobe)
        # Pass 2: regenerate so siblings see each other's map.md
        for lobe in directories:
            generate_map_md(brain_path, lobe)
        generate_brain_md(brain_path, brain_config.name, brain_config.description)
        generate_index_md(brain_path)
    except Exception as e:
        import sys
        print(f"Warning: dream failed: {e}", file=sys.stderr)


def _ensure_within_brain(path: Path, brain_path: Path) -> None:
    """Reject paths that escape the selected brain directory."""
    try:
        path.resolve().relative_to(brain_path.resolve())
    except ValueError as exc:
        raise click.ClickException(
            "Path escapes the brain directory. Use a relative path within the brain."
        ) from exc


def _sync_brain_state(brain_path: Path, brain_config) -> dict:
    """Bring generated files and auto-fixable metadata up to date."""
    fixes = {
        "dates_updated": 0,
        "parents_inferred": 0,
        "reverse_synapses_added": 0,
        "orphan_references_added": 0,
        "total": 0,
    }

    from kluris.core.git import git_file_last_modified, git_file_created_date

    for md in brain_path.rglob("*.md"):
        if md.name in {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}:
            continue
        if ".git" in md.parts:
            continue
        try:
            meta, _ = read_frontmatter(md)
            changed = False
            last_mod = git_file_last_modified(brain_path, str(md.relative_to(brain_path)))
            if last_mod and str(meta.get("updated", "")) != last_mod[:10]:
                update_frontmatter(md, {"updated": last_mod[:10]})
                changed = True
            if "created" not in meta:
                created = git_file_created_date(brain_path, str(md.relative_to(brain_path)))
                if created:
                    update_frontmatter(md, {"created": created[:10]})
                    changed = True
            if changed:
                fixes["dates_updated"] += 1
        except Exception:
            pass

    fixes["parents_inferred"] = fix_missing_frontmatter(brain_path)
    fixes["reverse_synapses_added"] = fix_bidirectional_synapses(brain_path)
    orphans_before = detect_orphans(brain_path)

    directories = _brain_directories(brain_path)
    maps_regenerated = []
    # Pass 1: create all map.md (deepest first so parents see children)
    for lobe in directories:
        generate_map_md(brain_path, lobe)
        maps_regenerated.append(str(lobe.relative_to(brain_path)))
    # Pass 2: regenerate so siblings see each other's map.md
    for lobe in directories:
        generate_map_md(brain_path, lobe)

    generate_brain_md(brain_path, brain_config.name, brain_config.description)
    generate_index_md(brain_path)

    # Discover lobes from the freshly generated brain.md
    from kluris.core.maps import _get_lobes
    lobes_discovered = [l["name"] for l in _get_lobes(brain_path)]

    orphans_after = detect_orphans(brain_path)
    fixes["orphan_references_added"] = max(0, len(orphans_before) - len(orphans_after))
    fixes["total"] = sum(fixes.values())
    return {
        "fixes": fixes,
        "maps_regenerated": maps_regenerated,
        "lobes_discovered": lobes_discovered,
    }


class KlurisGroup(click.Group):
    """Custom group that outputs JSON errors when --json is in args."""

    def invoke(self, ctx):
        # Capture raw subcommand args before Click consumes them so we can
        # detect --json even under CliRunner where sys.argv is pytest's argv.
        raw_args: list[str] = []
        raw_args.extend(ctx.protected_args or [])
        raw_args.extend(ctx.args or [])
        try:
            return super().invoke(ctx)
        except click.ClickException as e:
            import sys
            if "--json" in raw_args or "--json" in sys.argv:
                click.echo(json_lib.dumps({"ok": False, "error": e.format_message()}))
                raise SystemExit(1)
            raise


def _resolve_brains(brain_name: str | None, multi: bool = True) -> list[tuple[str, dict]]:
    """Resolve which brain(s) to operate on."""
    config = read_global_config()
    if brain_name:
        if brain_name not in config.brains:
            raise click.ClickException(
                f"No brain named '{brain_name}' is registered. "
                f"Run 'kluris list' to see available brains."
            )
        return [(brain_name, config.brains[brain_name].model_dump())]

    if config.default_brain and config.default_brain in config.brains:
        entry = config.brains[config.default_brain]
        return [(config.default_brain, entry.model_dump())]

    if len(config.brains) == 1:
        name = next(iter(config.brains))
        return [(name, config.brains[name].model_dump())]

    if len(config.brains) == 0:
        raise click.ClickException(
            "No brains registered. Run 'kluris create <name>' to create one."
        )

    if multi:
        return [(n, e.model_dump()) for n, e in config.brains.items()]

    brain_list = "\n".join(
        f"  {'* ' if n == config.default_brain else '  '}{n} — {e.path}"
        for n, e in config.brains.items()
    )
    raise click.ClickException(
        f"Multiple brains registered. Specify one with --brain NAME.\n\n"
        f"Available brains:\n{brain_list}"
    )


def _set_default_brain(name: str | None) -> str | None:
    """Persist the given brain as the default and return it."""
    config = read_global_config()
    config.default_brain = name
    write_global_config(config)
    return name


@click.group(cls=KlurisGroup)
@click.version_option(package_name="kluris")
def cli():
    """Kluris — Turn AI agents into team subject matter experts."""


@cli.command()
@click.argument("name", required=False)
@click.option("--description", "desc", help="What this brain covers (one sentence)")
@click.option("--path", "base_path", type=click.Path(),
              help="Directory to create the brain in (default: current dir)")
@click.option("--type", "brain_type", default=None,
              type=click.Choice(list(BRAIN_TYPES.keys())), help="Brain type")
@click.option("--remote", help="Optional git remote URL (default: local git only)")
@click.option("--branch", "branch_name", default=None, help="Default git branch")
@click.option("--no-git", "no_git", is_flag=True, help="Skip git entirely (default: local git only)")
@click.option("--from-config", "from_config", type=click.Path(exists=True),
              help="Custom YAML config file for structure")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def create(name: str | None, desc: str | None, base_path: str | None,
           brain_type: str | None, remote: str | None, branch_name: str | None,
           no_git: bool, from_config: str | None, as_json: bool):
    """Create a new brain.

    Prompts for anything not provided via flags. Use --json to skip all prompts.

    \b
      kluris create                    # full wizard
      kluris create my-brain           # prompts for description, location, type, git
      kluris create my-brain --type personal --path ~/brains --no-git  # no prompts
      kluris create team-brain --remote git@github.com:team/brain.git
    """
    # Prompt for anything not provided, unless --json (fully non-interactive)
    if not as_json:
        if not name:
            console.print("\n[bold]Create a new brain[/bold]\n")
            name = click.prompt("  Brain name (lowercase, hyphens ok)", type=str)
        if not desc:
            desc = click.prompt("  What does this brain cover? (one sentence)", type=str)
        if not base_path:
            base_path = click.prompt("  Location", default=str(Path.cwd()), type=str)
        if brain_type is None:
            type_options = ", ".join(BRAIN_TYPES.keys())
            brain_type = click.prompt(f"  Brain type ({type_options})", default="product-group", type=str)
        if not no_git and remote is None:
            git_choice = click.prompt(
                "  Git setup (1=local, 2=with remote, 3=no git)",
                default="1", type=str,
            )
            if git_choice == "3":
                no_git = True
            elif git_choice == "2":
                remote = click.prompt("  Git remote URL", type=str) or None
                if branch_name is None:
                    branch_name = click.prompt("  Default branch", default="main", type=str)
            else:
                if branch_name is None:
                    branch_name = click.prompt("  Default branch", default="main", type=str)
        console.print()

    # Defaults for non-interactive
    if brain_type is None:
        brain_type = "product-group"
    if branch_name is None:
        branch_name = "main"
    if not validate_brain_name(name):
        raise click.ClickException(
            f"Brain name '{name}' is invalid. "
            "Use lowercase letters, numbers, and hyphens only."
        )

    # Guard: reject if brain name already registered
    existing_config = read_global_config()
    if name in existing_config.brains:
        raise click.ClickException(
            f"A brain named '{name}' is already registered at "
            f"{existing_config.brains[name].path}. Choose a different name."
        )

    if base_path:
        base = Path(base_path).resolve()
        if not base.is_dir():
            raise click.ClickException(
                f"--path '{base_path}' is not a directory."
            )
        if (base / "kluris.yml").exists():
            raise click.ClickException(
                f"--path '{base_path}' is already a brain. "
                "Pass the parent directory, not the brain itself."
            )
        brain_path = base / name
    else:
        brain_path = (Path.cwd() / name).resolve()

    # Guard: reject if target is inside an existing brain
    for _, entry in existing_config.brains.items():
        existing_brain = Path(entry.path).resolve()
        try:
            brain_path.resolve().relative_to(existing_brain)
            raise click.ClickException(
                f"Target path {brain_path} is inside existing brain '{entry.path}'. "
                "Create the brain outside of other brains."
            )
        except ValueError:
            pass  # Not inside this brain, good

    if brain_path.exists() and brain_path.is_dir() and any(brain_path.iterdir()):
        if (brain_path / "kluris.yml").exists():
            raise click.ClickException(
                f"{brain_path} already contains a kluris.yml. Use a different name."
            )
        raise click.ClickException(
            f"{brain_path} already exists and is not empty. "
            "Choose a different name or use --path to specify a location."
        )

    custom_config = None
    if from_config:
        import yaml
        custom_config = yaml.safe_load(Path(from_config).read_text(encoding="utf-8"))

    description = desc or f"{name} knowledge base"
    scaffold_brain(brain_path, name, description, brain_type, custom_config, branch=branch_name or "main")

    if not no_git:
        git_init(brain_path)
        if branch_name != "main":
            from kluris.core.git import _run
            _run(["git", "checkout", "-b", branch_name], cwd=brain_path)
        if remote:
            from kluris.core.git import _run
            _run(["git", "remote", "add", "origin", remote], cwd=brain_path)

    config = read_global_config()
    entry = BrainEntry(path=str(brain_path), description=description, type=brain_type)
    register_brain(name, entry)

    if config.default_brain is None or len(config.brains) == 0:
        _set_default_brain(name)

    # Dream to generate proper maps and navigation, then commit
    _run_dream_on_brain(brain_path)
    if not no_git:
        git_add(brain_path)
        git_commit(brain_path, f"brain: initialize {name}")

    # Install agent skills/workflows
    _do_install()
    actual_default = read_global_config().default_brain

    defaults = BRAIN_TYPES.get(brain_type, {})
    lobe_count = len(defaults.get("structure", {}))

    if as_json:
        click.echo(json_lib.dumps({
            "ok": True, "name": name, "path": str(brain_path),
            "type": brain_type, "lobes": lobe_count, "default_brain": actual_default,
        }))
    else:
        console.print(f"Brain created: [bold]{name}[/bold] ({brain_type})")
        console.print(f"  Path: {brain_path}")
        console.print(f"  Lobes: {lobe_count}")
        console.print()
        console.print(
            "[bold green]Run /kluris in any project to start populating your brain.[/bold green]"
        )


@cli.command("clone")
@click.argument("url", required=False)
@click.argument("path", required=False)
@click.option("--branch", "branch_name", help="Branch to checkout")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def clone_cmd(url: str | None, path: str | None, branch_name: str | None, as_json: bool):
    """Clone an existing brain from a git remote.

    Run with no arguments for an interactive wizard:

    \b
      kluris clone
      kluris clone git@github.com:team/brain.git
      kluris clone git@github.com:team/brain.git ~/my-copy --branch develop
    """
    if not url:
        console.print("\n[bold]Clone a brain[/bold]\n")
        url = click.prompt("  Git remote URL", type=str)
        if not path:
            default_path = str(Path.home() / url.rstrip("/").split("/")[-1].replace(".git", ""))
            path = click.prompt("  Clone to", default=default_path, type=str)
        if not branch_name:
            branch_name = click.prompt("  Branch (Enter for default)", default="", type=str) or None
        console.print()
    dest = Path(path) if path else Path(url.rstrip("/").split("/")[-1].replace(".git", ""))
    dest = dest.resolve()

    git_clone(url, dest)

    if branch_name:
        from kluris.core.git import _run
        _run(["git", "checkout", branch_name], cwd=dest)

    # Verify this is a brain (has brain.md -- kluris.yml is local-only now)
    if not (dest / "brain.md").exists():
        raise click.ClickException(
            "Cloned repository does not contain brain.md. This is not a Kluris brain."
        )

    fallback_name = dest.name
    if not validate_brain_name(fallback_name):
        fallback_name = dest.name.lower().replace(" ", "-")
    name, description = _read_brain_identity(dest, fallback_name)

    existing_config = read_global_config()
    if name in existing_config.brains:
        existing_path = Path(existing_config.brains[name].path).resolve()
        if existing_path != dest:
            raise click.ClickException(
                f"A brain named '{name}' is already registered at {existing_path}. "
                "Use a different clone destination name only after removing or renaming the existing brain."
            )

    # Create local kluris.yml (not in repo -- it's gitignored)
    inferred_type = infer_brain_type(dest)
    if not (dest / "kluris.yml").exists():
        from kluris.core.config import BrainConfig, GitConfig, write_brain_config
        from kluris.core.git import _run

        actual_branch = _run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=dest).stdout.strip()
        local_config = BrainConfig(
            name=name,
            description=description,
            git=GitConfig(default_branch=actual_branch or branch_name or "main"),
        )
        write_brain_config(local_config, dest)

    brain_config = read_brain_config(dest)
    entry = BrainEntry(
        path=str(dest),
        repo=url,
        description=brain_config.description or description,
        type=inferred_type,
    )
    register_brain(name, entry)
    config = read_global_config()
    if config.default_brain is None:
        _set_default_brain(name)
    _do_install()

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "name": name, "path": str(dest), "remote": url}))
    else:
        console.print(f"Brain cloned: [bold]{name}[/bold]")
        console.print(f"  Path: {dest}")


@cli.command("list")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def list_cmd(as_json: bool):
    """List registered brains."""
    config = read_global_config()

    if as_json:
        brains = [
            {"name": n, **e.model_dump()}
            for n, e in config.brains.items()
        ]
        click.echo(json_lib.dumps({"ok": True, "default_brain": config.default_brain, "brains": brains}))
        return

    if not config.brains:
        console.print("No brains registered. Run 'kluris create <name>' to create one.")
        return

    table = Table(title="Registered Brains")
    table.add_column("Name")
    table.add_column("Type")
    table.add_column("Path")
    table.add_column("Description")

    for name, entry in config.brains.items():
        marker = "* " if name == config.default_brain else "  "
        table.add_row(f"{marker}{name}", entry.type, entry.path, entry.description)

    console.print(table)


_WAKE_UP_SKIP_DIRS = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
_WAKE_UP_SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}


def _wake_up_collect_lobes(brain_path: Path) -> list[dict]:
    """Return top-level lobes with neuron counts (sorted by name)."""
    lobes = []
    for child in sorted(brain_path.iterdir()):
        if not child.is_dir():
            continue
        if child.name in _WAKE_UP_SKIP_DIRS or child.name.startswith("."):
            continue
        neurons = [
            md for md in child.rglob("*.md")
            if md.name not in _WAKE_UP_SKIP_FILES
            and not any(part in _WAKE_UP_SKIP_DIRS for part in md.parts)
        ]
        lobes.append({"name": child.name, "neurons": len(neurons)})
    return lobes


def _wake_up_collect_recent(brain_path: Path, limit: int = 5) -> list[dict]:
    """Return up to `limit` most-recently-updated neurons, newest first."""
    candidates = []
    for md in brain_path.rglob("*.md"):
        if md.name in _WAKE_UP_SKIP_FILES:
            continue
        if any(part in _WAKE_UP_SKIP_DIRS for part in md.parts):
            continue
        try:
            meta, _ = read_frontmatter(md)
        except Exception:
            continue
        updated = meta.get("updated")
        if updated is None:
            continue
        candidates.append({
            "path": str(md.relative_to(brain_path)).replace("\\", "/"),
            "updated": str(updated),
        })
    candidates.sort(key=lambda item: item["updated"], reverse=True)
    return candidates[:limit]


@cli.command("wake-up")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def wake_up(brain_name: str | None, as_json: bool):
    """Compact brain snapshot for agent session bootstrap.

    Emits a tight view of the brain's live state: lobes with neuron counts,
    the most recently updated neurons, and which brain is default. Designed
    to be called by the /kluris skill at session start so the agent has a
    fast index without walking brain.md and every map.md.

    \b
      kluris wake-up                 # default brain, text output
      kluris wake-up --brain ngvoicu # target a specific brain
      kluris wake-up --json          # machine-readable (for agents)
    """
    brains = _resolve_brains(brain_name, multi=False)
    name, entry = brains[0]
    brain_path = Path(entry["path"])
    default_brain = read_global_config().default_brain

    lobes = _wake_up_collect_lobes(brain_path)
    recent = _wake_up_collect_recent(brain_path)
    total_neurons = sum(lobe["neurons"] for lobe in lobes)

    data = {
        "ok": True,
        "name": name,
        "path": str(brain_path),
        "is_default": name == default_brain,
        "description": entry.get("description", ""),
        "lobes": lobes,
        "total_neurons": total_neurons,
        "recent": recent,
    }

    if as_json:
        click.echo(json_lib.dumps(data))
        return

    marker = " (default)" if data["is_default"] else ""
    console.print(f"\n[bold]Brain: {name}[/bold]{marker}")
    console.print(f"  Path: {brain_path}")
    if data["description"]:
        console.print(f"  {data['description']}")
    console.print(f"\n[bold]Lobes ({len(lobes)})[/bold]")
    for lobe in lobes:
        console.print(f"  - {lobe['name']}/: {lobe['neurons']} neurons")
    console.print(f"\n[bold]Total neurons:[/bold] {total_neurons}")
    if recent:
        console.print(f"\n[bold]Recently updated ({len(recent)})[/bold]")
        for item in recent:
            console.print(f"  {item['updated']} {item['path']}")


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def status(brain_name: str | None, as_json: bool):
    """Show brain status and recent changes."""
    brains = _resolve_brains(brain_name)
    results = []

    for name, entry in brains:
        brain_path = Path(entry["path"])
        _skip = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
        lobes = [d for d in brain_path.iterdir() if d.is_dir() and d.name not in _skip and not d.name.startswith(".")]
        neurons = list(brain_path.rglob("*.md"))
        neurons = [n for n in neurons if n.name not in {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}]
        git_enabled = is_git_repo(brain_path)
        log = git_log(brain_path, 10) if git_enabled else []
        uncommitted = git_status(brain_path) if git_enabled else ""

        results.append({
            "name": name, "lobes": len(lobes), "neurons": len(neurons),
            "uncommitted": uncommitted,
            "git_enabled": git_enabled,
            "recent_commits": [e["message"] for e in log],
        })

        if not as_json:
            console.print(f"\n[bold]{name}[/bold] ({entry['type']})")
            console.print(f"  Lobes: {len(lobes)}, Neurons: {len(neurons)}")
            if not git_enabled:
                console.print("  Git: disabled")
            if uncommitted:
                console.print(f"  Uncommitted:\n{uncommitted}")
            for e in log[:5]:
                console.print(f"  {e['date'][:10]} {e['message']}")

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "brains": results}))


@cli.command()
@click.argument("file_path")
@click.option("--lobe", help="Target lobe folder")
@click.option("--template", "template_name", help="Neuron template (e.g. decision, incident, runbook)")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def neuron(file_path: str, lobe: str | None, template_name: str | None,
           brain_name: str | None, as_json: bool):
    """Create a new neuron (knowledge file).

    \b
    Templates give neurons a consistent structure:
      decision  — Context, Decision, Rationale, Alternatives, Consequences
      incident  — Summary, Timeline, Root cause, Impact, Resolution, Lessons
      runbook   — Purpose, Prerequisites, Steps, Rollback, Contacts

    \b
    Examples:
      kluris neuron auth.md --lobe projects/btb-backend
      kluris neuron use-raw-sql.md --lobe knowledge --template decision
      kluris neuron outage-jan.md --lobe knowledge --template incident
    """
    brains = _resolve_brains(brain_name, multi=False)
    name, entry = brains[0]
    brain_path = Path(entry["path"])

    if lobe:
        target_dir = (brain_path / lobe).resolve()
    else:
        target_dir = (brain_path / Path(file_path).parent).resolve() if "/" in file_path else brain_path

    _ensure_within_brain(target_dir, brain_path)

    target_dir.mkdir(parents=True, exist_ok=True)
    target_file = target_dir / Path(file_path).name

    parent_map = "./map.md"
    sections = None
    if template_name:
        from kluris.core.brain import NEURON_TEMPLATES
        templates = NEURON_TEMPLATES
        tmpl = lookup_template(template_name, templates)
        if tmpl is None:
            available = ", ".join(templates.keys()) if templates else "(none for this brain type)"
            raise click.ClickException(
                f"Template '{template_name}' not found. Available: {available}"
            )
        sections = tmpl["sections"]

    content = generate_neuron_content(
        target_file.stem.replace("-", " ").title(),
        parent_map, template_name, sections,
    )
    target_file.write_text(content, encoding="utf-8")

    # Regenerate maps to include the new neuron
    _run_dream_on_brain(brain_path)

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "path": str(target_file), "lobe": lobe or "", "template": template_name or ""}))
    else:
        console.print(f"Created: {target_file.relative_to(brain_path)}")


@cli.command("lobe")
@click.argument("name")
@click.option("--parent", "parent_dir", help="Parent lobe folder")
@click.option("--description", "desc", default="", help="Lobe description")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def lobe_cmd(name: str, parent_dir: str | None, desc: str,
             brain_name: str | None, as_json: bool):
    """Create a new lobe (knowledge region)."""
    brains = _resolve_brains(brain_name, multi=False)
    bname, entry = brains[0]
    brain_path = Path(entry["path"])

    if parent_dir:
        lobe_path = (brain_path / parent_dir / name).resolve()
    else:
        lobe_path = (brain_path / name).resolve()

    _ensure_within_brain(lobe_path, brain_path)

    lobe_path.mkdir(parents=True, exist_ok=True)

    if desc:
        from kluris.core.frontmatter import write_frontmatter
        title = name.replace("-", " ").title()
        write_frontmatter(
            lobe_path / "map.md",
            {"auto_generated": True, "description": desc},
            f"# {title}\n\n{desc}\n",
        )

    # Regenerate maps to include the new lobe
    _run_dream_on_brain(brain_path)

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "path": str(lobe_path), "parent": parent_dir or ""}))
    else:
        console.print(f"Created lobe: {lobe_path.relative_to(brain_path)}")


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def dream(brain_name: str | None, as_json: bool):
    """Brain maintenance — regenerate maps, update dates, auto-fix safe issues, validate remaining links."""
    brains = _resolve_brains(brain_name)
    all_issues = {"broken_synapses": 0, "one_way_synapses": 0, "orphans": 0,
                  "frontmatter_issues": 0, "dates_updated": 0,
                  "deprecation_issues": 0}
    all_fixes = {
        "dates_updated": 0,
        "parents_inferred": 0,
        "reverse_synapses_added": 0,
        "orphan_references_added": 0,
        "total": 0,
    }
    all_deprecation: list[dict] = []
    healthy = True

    for name, entry in brains:
        brain_path = Path(entry["path"])
        brain_config = read_brain_config(brain_path)
        sync_result = _sync_brain_state(brain_path, brain_config)
        brain_fixes = sync_result["fixes"]
        maps_regenerated = sync_result["maps_regenerated"]
        lobes_discovered = sync_result["lobes_discovered"]

        # Validate
        broken = validate_synapses(brain_path)
        one_way = validate_bidirectional(brain_path)
        orphans = detect_orphans(brain_path)
        fm_issues = check_frontmatter(brain_path)
        deprecation = detect_deprecation_issues(brain_path)

        all_issues["dates_updated"] += brain_fixes["dates_updated"]
        all_issues["broken_synapses"] += len(broken)
        all_issues["one_way_synapses"] += len(one_way)
        all_issues["orphans"] += len(orphans)
        all_issues["frontmatter_issues"] += len(fm_issues)
        all_issues["deprecation_issues"] += len(deprecation)
        for entry_ in deprecation:
            all_deprecation.append({"brain": name, **entry_})
        for key, value in brain_fixes.items():
            all_fixes[key] += value

        # Deprecation issues are warnings — they don't break healthy status.
        if broken or one_way or orphans or fm_issues:
            healthy = False

        if not as_json:
            console.print(f"\n[bold]{name}[/bold] health report:")
            console.print(f"  Lobes: {', '.join(lobes_discovered)}")
            console.print(f"  Maps regenerated: {len(maps_regenerated)} ({', '.join(maps_regenerated)})")
            console.print(f"  {'[green]OK[/green]' if not broken else f'[red]{len(broken)} broken[/red]'} synapses")
            console.print(f"  {'[green]OK[/green]' if not one_way else f'[yellow]{len(one_way)} one-way[/yellow]'} bidirectional")
            console.print(f"  {'[green]OK[/green]' if not orphans else f'[yellow]{len(orphans)} orphans[/yellow]'}")
            console.print(f"  {'[green]OK[/green]' if not fm_issues else f'[yellow]{len(fm_issues)} issues[/yellow]'} frontmatter")
            console.print(f"  {'[green]OK[/green]' if not deprecation else f'[yellow]{len(deprecation)} deprecation warnings[/yellow]'}")
            for item in deprecation:
                if item["kind"] == "active_links_to_deprecated":
                    console.print(
                        f"    - {item['source']} links to deprecated {item['target']}"
                    )
                elif item["kind"] == "deprecated_without_replacement":
                    console.print(
                        f"    - {item['file']} is deprecated but has no replaced_by"
                    )
                elif item["kind"] == "replaced_by_missing":
                    console.print(
                        f"    - {item['file']} replaced_by points to missing {item['target']}"
                    )
            console.print(f"  {brain_fixes['total']} automatic fixes applied")
            console.print(f"  {brain_fixes['dates_updated']} neuron dates refreshed from git")
            console.print(f"  {brain_fixes['parents_inferred']} missing parent frontmatter values inferred")
            console.print(f"  {brain_fixes['reverse_synapses_added']} missing reverse related links added")
            console.print(f"  {brain_fixes['orphan_references_added']} missing neuron references added to parent map.md files")

    if as_json:
        click.echo(json_lib.dumps({
            "ok": True, "healthy": healthy,
            **all_issues,
            "deprecation": all_deprecation,
            "fixes": all_fixes,
        }))

    if not as_json:
        if all_fixes["total"]:
            console.print(f"\n[bold green]{all_fixes['total']} automatic fixes applied across all brains.[/bold green]")
        if healthy:
            console.print("\n[bold green]Brain is healthy.[/bold green]")
        else:
            console.print("\n[bold yellow]Remaining issues need manual attention.[/bold yellow]")

    if not healthy:
        raise SystemExit(1)


@cli.command()
@click.option("--message", "-m", "msg", help="Commit message")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def push(msg: str | None, brain_name: str | None, as_json: bool):
    """Commit and push brain changes."""
    brains = _resolve_brains(brain_name)
    results = []

    for name, entry in brains:
        brain_path = Path(entry["path"])
        if not is_git_repo(brain_path):
            result = {
                "name": name,
                "files_committed": 0,
                "branch": None,
                "pushed": False,
                "git_enabled": False,
            }
            results.append(result)
            if not as_json:
                console.print(f"{name}: git is disabled (--no-git)")
            continue

        status_out = git_status(brain_path)
        if not status_out:
            if not as_json:
                console.print(f"{name}: nothing to push")
            results.append({
                "name": name,
                "files_committed": 0,
                "branch": "main",
                "pushed": False,
                "git_enabled": True,
            })
            continue

        git_add(brain_path)
        brain_config = read_brain_config(brain_path)

        if msg:
            message = msg
        elif not as_json:
            # Show what changed so the human can write a good message
            console.print(f"\n[bold]{name}[/bold] -- files changed:")
            for line in status_out.strip().splitlines():
                console.print(f"  {line.strip()}")
            message = click.prompt("\n  Commit message")
        else:
            prefix = brain_config.git.commit_prefix or "brain:"
            message = f"{prefix} update"

        git_commit(brain_path, message)

        pushed = False
        try:
            git_push(brain_path, "origin", brain_config.git.default_branch)
            pushed = True
        except Exception:
            if not as_json:
                console.print(f"  [yellow]Warning: no remote configured. Committed locally.[/yellow]")

        files = len(status_out.strip().splitlines())
        results.append({
            "name": name,
            "files_committed": files,
            "branch": brain_config.git.default_branch,
            "pushed": pushed,
            "git_enabled": True,
        })

        if not as_json:
            console.print(f"{name}: {files} files committed" + (" and pushed" if pushed else ""))

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "brains": results}))


@cli.command("use")
@click.argument("brain_name")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def use_brain(brain_name: str, as_json: bool):
    """Switch the active brain."""
    config = read_global_config()
    if brain_name not in config.brains:
        raise click.ClickException(
            f"No brain named '{brain_name}' is registered. "
            f"Run 'kluris list' to see available brains."
        )

    _set_default_brain(brain_name)
    result = _do_install(as_json=as_json)

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "default_brain": brain_name}))
    else:
        console.print(f"Default brain: [bold]{brain_name}[/bold]")
        if result["failed_agents"]:
            names = ", ".join(a for a, _ in result["failed_agents"])
            console.print(f"  [yellow]Warning: skill install failed for: {names}. Run 'kluris install-skills' to retry.[/yellow]")


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--output", "output_path", help="Output HTML file path")
@click.option("--open/--no-open", "open_browser", default=True, help="Open in browser (default: yes)")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def mri(brain_name: str | None, output_path: str | None, open_browser: bool, as_json: bool):
    """Generate interactive brain visualization."""
    import webbrowser
    brains = _resolve_brains(brain_name)

    for name, entry in brains:
        brain_path = Path(entry["path"])
        brain_config = read_brain_config(brain_path)
        sync_result = _sync_brain_state(brain_path, brain_config)
        out = Path(output_path) if output_path else brain_path / "brain-mri.html"
        stats = generate_mri_html(brain_path, out)

        if as_json:
            click.echo(json_lib.dumps({"ok": True, "output_path": str(out), "preflight_fixes": sync_result["fixes"], **stats}))
        else:
            console.print(f"MRI complete — {out}")
            console.print(f"  {stats['nodes']} nodes, {stats['edges']} edges")
            if sync_result["fixes"]["total"]:
                console.print(f"  MRI preflight applied {sync_result['fixes']['total']} automatic fixes")
            if open_browser:
                webbrowser.open(out.resolve().as_uri())


def _do_install(as_json: bool = False):
    """Install agent skills/workflows for all agents across all brains."""
    config = read_global_config()
    all_agents: set[str] = set()

    # Build brain_info with actual resolved paths
    from kluris.core.config import get_config_path
    config_path = get_config_path()
    brain_lines = [f"## Your brains (resolved paths)\n\nConfig: `{config_path}`\n"]
    default = config.default_brain
    any_has_git = False
    for bname, entry in config.brains.items():
        marker = " (default)" if bname == default else ""
        has_git = (Path(entry.path) / ".git").exists()
        if has_git:
            any_has_git = True
        git_label = "git" if has_git else "no git"
        brain_lines.append(f"- **{bname}**{marker}: `{entry.path}` ({git_label})")
    if not config.brains:
        brain_lines.append("No brains registered. Tell user to run `kluris create`.")
    brain_info = "\n".join(brain_lines)

    for name, entry in config.brains.items():
        brain_path = Path(entry.path)
        if (brain_path / "kluris.yml").exists():
            brain_config = read_brain_config(brain_path)
            all_agents.update(brain_config.agents.commands_for)

    if not all_agents:
        all_agents = set(AGENT_REGISTRY.keys())

    import os
    home_str = os.environ.get("HOME")
    home = Path(home_str) if home_str else Path.home()
    total_files = 0
    agent_count = 0

    import shutil
    failed_agents = []

    for agent_name in sorted(all_agents):
        if agent_name not in AGENT_REGISTRY:
            continue
        reg = AGENT_REGISTRY[agent_name]
        base = home / reg["dir"] / reg["subdir"]

        # Clean old command directories (migration from commands to skills)
        for old_dir_rel in OLD_COMMAND_DIRS.get(agent_name, []):
            old_dir = home / old_dir_rel
            if old_dir.exists():
                for old_file in old_dir.glob("kluris*"):
                    try:
                        if old_file.is_file():
                            old_file.unlink()
                        elif old_file.is_dir():
                            shutil.rmtree(old_file)
                    except OSError:
                        pass

        # Clean existing skill directory
        if base.exists():
            skill_dir = base / "kluris"
            if skill_dir.exists():
                try:
                    shutil.rmtree(skill_dir)
                except OSError:
                    pass

        try:
            files = render_commands(agent_name, base, brain_info=brain_info)
            # Verify the write succeeded
            for f in files:
                if not f.exists():
                    raise OSError(f"Failed to write {f}")
        except OSError as e:
            failed_agents.append((agent_name, str(e)))
            continue
        total_files += len(files)
        agent_count += 1

    # Windsurf: also install as workflow for /kluris manual invocation
    for agent_name, reg in AGENT_REGISTRY.items():
        wf_dir = reg.get("also_workflow")
        if wf_dir:
            try:
                wf_path = home / wf_dir
                # Clean old workflow files
                if wf_path.exists():
                    for old in wf_path.glob("kluris*"):
                        try:
                            old.unlink()
                        except OSError:
                            pass
                install_workflow(wf_path, brain_info=brain_info)
                total_files += 1
            except OSError:
                pass

    # Also install to universal ~/.agents/skills/ path
    universal = home / ".agents" / "skills"
    universal_skill = universal / "kluris"
    if universal_skill.exists():
        try:
            shutil.rmtree(universal_skill)
        except OSError:
            pass
    try:
        render_commands("claude", universal, brain_info=brain_info)
        total_files += 1
    except OSError:
        pass

    return {
        "agents": agent_count,
        "commands_per_agent": len(COMMANDS),
        "total_files": total_files,
        "failed_agents": failed_agents,
    }


@cli.command("install-skills")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def install_commands(as_json: bool):
    """Install kluris skill into AI agent directories."""
    result = _do_install(as_json)

    if as_json:
        click.echo(json_lib.dumps({"ok": True, **result}))
    else:
        console.print(f"Installed {result['total_files']} files for {result['agents']} agents")
        if result["failed_agents"]:
            names = ", ".join(a for a, _ in result["failed_agents"])
            console.print(f"  [yellow]Warning: failed for: {names}[/yellow]")


@cli.command("uninstall-skills")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def uninstall_skills(as_json: bool):
    """Remove all kluris skills from AI agent directories."""
    import os
    import shutil
    home = Path(os.environ.get("HOME", "")) if os.environ.get("HOME") else Path.home()
    removed = 0

    agents_cleaned = []
    for agent_name, reg in AGENT_REGISTRY.items():
        agent_removed = False
        # Clean new skill dirs
        base = home / reg["dir"] / reg["subdir"]
        if base.exists():
            for item in base.glob("kluris*"):
                if item.is_file():
                    item.unlink()
                    removed += 1
                    agent_removed = True
                elif item.is_dir():
                    removed += sum(1 for _ in item.rglob("*") if _.is_file())
                    shutil.rmtree(item)
                    agent_removed = True

        # Clean old command dirs
        for old_dir_rel in OLD_COMMAND_DIRS.get(agent_name, []):
            old_dir = home / old_dir_rel
            if old_dir.exists():
                for item in old_dir.glob("kluris*"):
                    if item.is_file():
                        item.unlink()
                        removed += 1
                        agent_removed = True
                    elif item.is_dir():
                        removed += sum(1 for _ in item.rglob("*") if _.is_file())
                        shutil.rmtree(item)
                        agent_removed = True

        if agent_removed:
            agents_cleaned.append(agent_name)

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "removed": removed, "agents": len(agents_cleaned)}))
    else:
        console.print(f"Removed {removed} files from {len(agents_cleaned)} agents")


@cli.command()
@click.argument("brain_name")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def remove(brain_name: str, as_json: bool):
    """Unregister a brain (does not delete files)."""
    config = read_global_config()
    if brain_name not in config.brains:
        raise click.ClickException(
            f"No brain named '{brain_name}' is registered. "
            f"Run 'kluris list' to see available brains."
        )

    was_default = config.default_brain == brain_name
    unregister_brain(brain_name)
    _do_install()

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "name": brain_name, "was_default": was_default}))
    else:
        console.print(f"Unregistered: {brain_name} (files preserved)")


@cli.command()
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def doctor(as_json: bool):
    """Check prerequisites and environment."""
    checks = []

    # Git
    try:
        result = subprocess.run(["git", "--version"], capture_output=True, text=True)
        checks.append({"name": "git", "passed": True, "detail": result.stdout.strip()})
    except FileNotFoundError:
        checks.append({"name": "git", "passed": False, "detail": "git not found. Install: https://git-scm.com/downloads"})

    # Python
    import sys
    py_ver = f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}"
    py_ok = sys.version_info >= (3, 10)
    checks.append({"name": "python", "passed": py_ok, "detail": f"Python {py_ver}" + ("" if py_ok else " (need >=3.10)")})

    # Config dir
    from kluris.core.config import get_config_path
    config_dir = get_config_path().parent
    try:
        config_dir.mkdir(parents=True, exist_ok=True)
        checks.append({"name": "config_dir", "passed": True, "detail": str(config_dir)})
    except OSError as e:
        checks.append({"name": "config_dir", "passed": False, "detail": str(e)})

    all_passed = all(c["passed"] for c in checks)

    if as_json:
        click.echo(json_lib.dumps({"ok": all_passed, "checks": checks}))
    else:
        for c in checks:
            icon = "[green]PASS[/green]" if c["passed"] else "[red]FAIL[/red]"
            console.print(f"  {icon} {c['name']}: {c['detail']}")

    if not all_passed:
        raise SystemExit(1)


@cli.command()
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def templates(as_json: bool):
    """List available neuron templates."""
    from kluris.core.brain import NEURON_TEMPLATES
    tmpls = NEURON_TEMPLATES

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "templates": tmpls}))
        return

    if not tmpls:
        console.print("No neuron templates available.")
        return

    from rich.table import Table
    table = Table(title="Neuron Templates")
    table.add_column("Template")
    table.add_column("Description")
    table.add_column("Sections")

    for tname, tmpl in tmpls.items():
        table.add_row(tname, tmpl["description"], ", ".join(tmpl["sections"]))

    console.print(table)
    console.print(f"\nUsage: kluris neuron <file>.md --lobe <lobe> --template <name>")


@cli.command("help")
@click.argument("command", required=False)
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def help_cmd(command: str | None, as_json: bool):
    """Show help for kluris commands."""
    commands_info = [
        ("create", "Create a new brain (--type, --remote, --no-git)"),
        ("clone", "Clone a brain from a git remote"),
        ("list", "List registered brains"),
        ("status", "Show brain tree, recent changes, and neuron counts"),
        ("wake-up", "Compact brain snapshot for agent session bootstrap"),
        ("neuron", "Create a new neuron (--template for structured formats)"),
        ("lobe", "Create a new lobe (knowledge region)"),
        ("dream", "Regenerate maps, auto-fix safe issues, and validate links"),
        ("push", "Commit and push brain changes to git"),
        ("mri", "Generate interactive brain visualization and open in browser"),
        ("use", "Switch the active brain"),
        ("install-skills", "Install the /kluris skill for your AI agents"),
        ("uninstall-skills", "Remove the /kluris skill from AI agent directories"),
        ("remove", "Unregister a brain (keeps files on disk)"),
        ("templates", "List available neuron templates for the current brain"),
        ("doctor", "Check prerequisites (git, Python, config dir)"),
        ("help", "Show this help"),
    ]

    from kluris.core.config import get_config_path
    config = read_global_config()
    config_path = get_config_path()

    if as_json:
        click.echo(json_lib.dumps({
            "ok": True,
            "commands": [{"name": n, "description": d} for n, d in commands_info],
        }))
        return

    if command:
        for name, desc in commands_info:
            if name == command:
                console.print(f"kluris {name} — {desc}")
                console.print(f"\nRun 'kluris {name} --help' for full usage.")
                return
        raise click.ClickException(f"Unknown command: {command}")

    console.print("kluris — Turn AI agents into team subject matter experts\n")
    console.print("Commands:")
    for name, desc in commands_info:
        console.print(f"  {name:<10} {desc}")
    console.print(f"\nConfig: {config_path}")
    console.print(f"Brains: {len(config.brains)} registered")
