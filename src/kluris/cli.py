"""Kluris CLI — Click entry point."""

from __future__ import annotations

import json as json_lib
import subprocess
from pathlib import Path

import click
from rich.console import Console
from rich.table import Table
from rich.tree import Tree

from kluris.core.brain import (
    BRAIN_TYPES,
    generate_neuron_content,
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
    git_log,
    git_push,
    git_status,
)
from kluris.core.linker import (
    check_frontmatter,
    detect_orphans,
    validate_bidirectional,
    validate_synapses,
)
from kluris.core.maps import generate_brain_md, generate_index_md, generate_map_md
from kluris.core.mri import generate_mri_html
from kluris.core.frontmatter import read_frontmatter, update_frontmatter
from kluris.core.agents import AGENT_REGISTRY, render_commands

console = Console()


def _run_dream_on_brain(brain_path: Path) -> None:
    """Regenerate maps, brain.md, and index.md for a single brain."""
    try:
        brain_config = read_brain_config(brain_path)
        lobes = [d for d in brain_path.iterdir() if d.is_dir() and d.name != ".git"]
        for lobe in lobes:
            generate_map_md(brain_path, lobe)
        generate_brain_md(brain_path, brain_config.name, brain_config.description)
        generate_index_md(brain_path)
    except Exception:
        pass  # Dream is best-effort when triggered by neuron/lobe


class KlurisGroup(click.Group):
    """Custom group that outputs JSON errors when --json is in args."""

    def invoke(self, ctx):
        try:
            return super().invoke(ctx)
        except click.ClickException as e:
            import sys
            if "--json" in sys.argv:
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
            "No brains registered. Run 'kluris create <path>' to create one."
        )

    if multi:
        return [(n, e.model_dump()) for n, e in config.brains.items()]

    brain_list = "\n".join(
        f"  {'* ' if n == config.default_brain else '  '}{n} ({e.type}) — {e.path}"
        for n, e in config.brains.items()
    )
    raise click.ClickException(
        f"Multiple brains registered. Specify one with --brain NAME.\n\n"
        f"Available brains:\n{brain_list}"
    )


@click.group(cls=KlurisGroup)
@click.version_option(package_name="kluris")
def cli():
    """Kluris — Git-backed AI brain manager."""


@cli.command()
@click.argument("path", type=click.Path())
@click.option("--type", "brain_type", default="team",
              type=click.Choice(list(BRAIN_TYPES.keys())), help="Brain type")
@click.option("--from-config", "from_config", type=click.Path(exists=True),
              help="Custom YAML config file for structure")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def create(path: str, brain_type: str, from_config: str | None, as_json: bool):
    """Create a new brain."""
    brain_path = Path(path).resolve()
    name = brain_path.name

    if not validate_brain_name(name):
        raise click.ClickException(
            f"Brain name '{name}' is invalid. "
            "Use lowercase letters, numbers, and hyphens only."
        )

    if (brain_path / "kluris.yml").exists():
        raise click.ClickException(
            f"{brain_path} already contains a kluris.yml. Use a different path."
        )

    custom_config = None
    if from_config:
        import yaml
        custom_config = yaml.safe_load(Path(from_config).read_text(encoding="utf-8"))

    scaffold_brain(brain_path, name, f"{name} knowledge base", brain_type, custom_config)

    git_init(brain_path)
    git_add(brain_path)
    git_commit(brain_path, f"brain: initialize {name}")

    config = read_global_config()
    entry = BrainEntry(path=str(brain_path), description=f"{name} knowledge base", type=brain_type)
    register_brain(name, entry)

    if config.default_brain is None or len(config.brains) == 0:
        config = read_global_config()
        config.default_brain = name
        write_global_config(config)

    # Install slash commands
    _do_install()

    defaults = BRAIN_TYPES.get(brain_type, {})
    lobe_count = len(defaults.get("structure", {}))

    if as_json:
        click.echo(json_lib.dumps({
            "ok": True, "name": name, "path": str(brain_path),
            "type": brain_type, "lobes": lobe_count, "default_brain": name,
        }))
    else:
        console.print(f"Brain created: [bold]{name}[/bold] ({brain_type})")
        console.print(f"  Path: {brain_path}")
        console.print(f"  Lobes: {lobe_count}")
        console.print()
        console.print("[bold green]Run /kluris.learn in any project to start populating your brain.[/bold green]")


@cli.command("clone")
@click.argument("url")
@click.argument("path", required=False)
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def clone_cmd(url: str, path: str | None, as_json: bool):
    """Clone an existing brain from a git remote."""
    dest = Path(path) if path else Path(url.rstrip("/").split("/")[-1].replace(".git", ""))
    dest = dest.resolve()

    git_clone(url, dest)

    if not (dest / "kluris.yml").exists():
        raise click.ClickException(
            "Cloned repository does not contain kluris.yml. This is not a Kluris brain."
        )

    brain_config = read_brain_config(dest)
    name = brain_config.name

    if not validate_brain_name(name):
        raise click.ClickException(
            f"Brain name '{name}' is invalid. "
            "Use lowercase letters, numbers, and hyphens only."
        )

    entry = BrainEntry(path=str(dest), repo=url, description=brain_config.description, type=brain_config.type)
    register_brain(name, entry)
    _do_install()

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "name": name, "path": str(dest), "type": brain_config.type, "remote": url}))
    else:
        console.print(f"Brain cloned: [bold]{name}[/bold]")
        console.print(f"  Path: {dest}")


@cli.command("list")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def list_cmd(as_json: bool):
    """List all registered brains."""
    config = read_global_config()

    if as_json:
        brains = [
            {"name": n, **e.model_dump()}
            for n, e in config.brains.items()
        ]
        click.echo(json_lib.dumps({"ok": True, "default_brain": config.default_brain, "brains": brains}))
        return

    if not config.brains:
        console.print("No brains registered. Run 'kluris create <path>' to create one.")
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


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def status(brain_name: str | None, as_json: bool):
    """Show brain status and recent changes."""
    brains = _resolve_brains(brain_name)
    results = []

    for name, entry in brains:
        brain_path = Path(entry["path"])
        lobes = [d for d in brain_path.iterdir() if d.is_dir() and d.name != ".git"]
        neurons = list(brain_path.rglob("*.md"))
        neurons = [n for n in neurons if n.name not in {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}]
        log = git_log(brain_path, 10)
        uncommitted = git_status(brain_path)

        results.append({
            "name": name, "lobes": len(lobes), "neurons": len(neurons),
            "uncommitted": uncommitted,
            "recent_commits": [e["message"] for e in log],
        })

        if not as_json:
            console.print(f"\n[bold]{name}[/bold] ({entry['type']})")
            console.print(f"  Lobes: {len(lobes)}, Neurons: {len(neurons)}")
            if uncommitted:
                console.print(f"  Uncommitted:\n{uncommitted}")
            for e in log[:5]:
                console.print(f"  {e['date'][:10]} {e['message']}")

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "brains": results}))


@cli.command()
@click.argument("query")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def recall(query: str, brain_name: str | None, as_json: bool):
    """Search brain content."""
    brains = _resolve_brains(brain_name)
    all_results = []

    for name, entry in brains:
        brain_path = Path(entry["path"])
        query_lower = query.lower()
        for md_file in brain_path.rglob("*.md"):
            if ".git" in md_file.parts:
                continue
            try:
                lines = md_file.read_text(encoding="utf-8").splitlines()
                for i, line in enumerate(lines, 1):
                    if query_lower in line.lower():
                        rel = str(md_file.relative_to(brain_path))
                        all_results.append({
                            "brain": name, "file": rel,
                            "line": str(i), "text": line.strip(),
                        })
            except (OSError, UnicodeDecodeError):
                continue

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "query": query, "results": all_results}))
    else:
        if not all_results:
            console.print(f"No results for '{query}'")
        for r in all_results:
            console.print(f"  {r['brain']}/{r['file']}:{r['line']} {r['text']}")


@cli.command()
@click.argument("file_path")
@click.option("--lobe", help="Target lobe folder")
@click.option("--template", "template_name", help="Neuron template name")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def neuron(file_path: str, lobe: str | None, template_name: str | None,
           brain_name: str | None, as_json: bool):
    """Create a new neuron."""
    brains = _resolve_brains(brain_name, multi=False)
    name, entry = brains[0]
    brain_path = Path(entry["path"])

    if lobe:
        target_dir = brain_path / lobe
    else:
        target_dir = brain_path / Path(file_path).parent if "/" in file_path else brain_path

    target_dir.mkdir(parents=True, exist_ok=True)
    target_file = target_dir / Path(file_path).name

    parent_map = "./map.md"
    sections = None
    if template_name:
        brain_config = read_brain_config(brain_path)
        templates = brain_config.neuron_templates
        tmpl = lookup_template(template_name, {k: v.model_dump() for k, v in templates.items()})
        if tmpl is None:
            available = ", ".join(templates.keys())
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
        lobe_path = brain_path / parent_dir / name
    else:
        lobe_path = brain_path / name

    lobe_path.mkdir(parents=True, exist_ok=True)

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
    """Brain maintenance — regenerate maps, validate links, update dates."""
    brains = _resolve_brains(brain_name)
    all_issues = {"broken_synapses": 0, "one_way_synapses": 0, "orphans": 0,
                  "frontmatter_issues": 0, "dates_updated": 0}
    healthy = True

    for name, entry in brains:
        brain_path = Path(entry["path"])
        brain_config = read_brain_config(brain_path)

        # Auto-update dates from git
        from kluris.core.git import git_file_last_modified, git_file_created_date
        for md in brain_path.rglob("*.md"):
            if md.name in {"map.md", "brain.md", "index.md", "glossary.md", "README.md"}:
                continue
            if ".git" in md.parts:
                continue
            try:
                meta, _ = read_frontmatter(md)
                updated = False
                last_mod = git_file_last_modified(brain_path, str(md.relative_to(brain_path)))
                if last_mod:
                    update_frontmatter(md, {"updated": last_mod[:10]})
                    updated = True
                if "created" not in meta:
                    created = git_file_created_date(brain_path, str(md.relative_to(brain_path)))
                    if created:
                        update_frontmatter(md, {"created": created[:10]})
                        updated = True
                if updated:
                    all_issues["dates_updated"] += 1
            except Exception:
                pass

        # Regenerate maps
        lobes = [d for d in brain_path.iterdir() if d.is_dir() and d.name != ".git"]
        for lobe in lobes:
            generate_map_md(brain_path, lobe)
            for sub in lobe.rglob("*"):
                if sub.is_dir() and (sub / "map.md").exists():
                    generate_map_md(brain_path, sub)

        generate_brain_md(brain_path, brain_config.name, brain_config.description)
        generate_index_md(brain_path)

        # Validate
        broken = validate_synapses(brain_path)
        one_way = validate_bidirectional(brain_path)
        orphans = detect_orphans(brain_path)
        fm_issues = check_frontmatter(brain_path)

        all_issues["broken_synapses"] += len(broken)
        all_issues["one_way_synapses"] += len(one_way)
        all_issues["orphans"] += len(orphans)
        all_issues["frontmatter_issues"] += len(fm_issues)

        if broken or one_way or orphans or fm_issues:
            healthy = False

        if not as_json:
            console.print(f"\n[bold]{name}[/bold] health report:")
            console.print(f"  {'[green]OK[/green]' if not broken else f'[red]{len(broken)} broken[/red]'} synapses")
            console.print(f"  {'[green]OK[/green]' if not one_way else f'[yellow]{len(one_way)} one-way[/yellow]'} bidirectional")
            console.print(f"  {'[green]OK[/green]' if not orphans else f'[yellow]{len(orphans)} orphans[/yellow]'}")
            console.print(f"  {'[green]OK[/green]' if not fm_issues else f'[yellow]{len(fm_issues)} issues[/yellow]'} frontmatter")
            console.print(f"  {all_issues['dates_updated']} dates updated")

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "healthy": healthy, **all_issues}))

    if not as_json:
        if healthy:
            console.print("\n[bold green]Brain is healthy.[/bold green]")
        else:
            console.print("\n[bold yellow]Issues found. Run dream again after fixing.[/bold yellow]")

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
        status_out = git_status(brain_path)
        if not status_out:
            if not as_json:
                console.print(f"{name}: nothing to push")
            results.append({"name": name, "files_committed": 0, "branch": "main", "pushed": False})
            continue

        git_add(brain_path)
        brain_config = read_brain_config(brain_path)
        message = msg or f"{brain_config.git.commit_prefix} update"
        git_commit(brain_path, message)

        pushed = False
        try:
            git_push(brain_path, "origin", brain_config.git.default_branch)
            pushed = True
        except Exception:
            if not as_json:
                console.print(f"  [yellow]Warning: no remote configured. Committed locally.[/yellow]")

        files = len(status_out.strip().splitlines())
        results.append({"name": name, "files_committed": files, "branch": brain_config.git.default_branch, "pushed": pushed})

        if not as_json:
            console.print(f"{name}: {files} files committed" + (" and pushed" if pushed else ""))

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "brains": results}))


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--output", "output_path", help="Output HTML file path")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def mri(brain_name: str | None, output_path: str | None, as_json: bool):
    """Generate interactive brain visualization."""
    brains = _resolve_brains(brain_name)

    for name, entry in brains:
        brain_path = Path(entry["path"])
        out = Path(output_path) if output_path else brain_path / "brain-mri.html"
        stats = generate_mri_html(brain_path, out)

        if as_json:
            click.echo(json_lib.dumps({"ok": True, "output_path": str(out), **stats}))
        else:
            console.print(f"MRI complete — {out}")
            console.print(f"  {stats['nodes']} nodes, {stats['edges']} edges")


def _do_install(as_json: bool = False):
    """Install slash commands for all agents across all brains."""
    config = read_global_config()
    all_agents: set[str] = set()

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

    for agent_name in sorted(all_agents):
        if agent_name not in AGENT_REGISTRY:
            continue
        reg = AGENT_REGISTRY[agent_name]
        base = home / reg["dir"] / reg["subdir"]

        # Clean slate: remove existing kluris.* files
        if base.exists():
            for old_file in base.glob("kluris*"):
                try:
                    if old_file.is_file():
                        old_file.unlink()
                    elif old_file.is_dir():
                        import shutil
                        shutil.rmtree(old_file)
                except OSError:
                    pass

        try:
            files = render_commands(agent_name, base)
        except OSError:
            continue
        total_files += len(files)
        agent_count += 1

    return {"agents": agent_count, "commands_per_agent": 9, "total_files": total_files}


@cli.command()
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def install(as_json: bool):
    """Install slash commands for AI agents."""
    result = _do_install(as_json)

    if as_json:
        click.echo(json_lib.dumps({"ok": True, **result}))
    else:
        console.print(f"Installed {result['total_files']} commands for {result['agents']} agents")


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


@cli.command("help")
@click.argument("command", required=False)
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def help_cmd(command: str | None, as_json: bool):
    """Show help for kluris commands."""
    commands_info = [
        ("create", "Create a new brain"),
        ("clone", "Clone a brain from a git remote"),
        ("list", "List registered brains"),
        ("status", "Show brain status"),
        ("recall", "Search brain content"),
        ("neuron", "Create a new neuron"),
        ("lobe", "Create a new lobe"),
        ("dream", "Brain maintenance — validate and regenerate"),
        ("push", "Commit and push brain changes"),
        ("mri", "Generate interactive brain visualization"),
        ("install", "Install slash commands for AI agents"),
        ("remove", "Unregister a brain"),
        ("doctor", "Check prerequisites and environment"),
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

    console.print("kluris — Git-backed AI brain manager\n")
    console.print("Commands:")
    for name, desc in commands_info:
        console.print(f"  {name:<10} {desc}")
    console.print(f"\nConfig: {config_path}")
    console.print(f"Brains: {len(config.brains)} registered")
