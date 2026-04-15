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
    BRAIN_NAME_MAX_LENGTH,
    BRAIN_NAME_RESERVED,
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
    git_conflicted_files,
    git_current_branch,
    git_fetch,
    git_has_upstream,
    git_init,
    git_log,
    git_merge,
    git_pull,
    git_push,
    git_status,
    is_git_repo,
)
from kluris.core.linker import (
    _neuron_files,
    check_frontmatter,
    detect_deprecation_issues,
    detect_orphans,
    fix_bidirectional_synapses,
    fix_missing_frontmatter,
    validate_bidirectional,
    validate_synapses,
)
from kluris.core.maps import generate_brain_md, generate_map_md
from kluris.core.mri import generate_mri_html
from kluris.core.frontmatter import read_frontmatter, update_frontmatter
from kluris.core.agents import AGENT_REGISTRY, OLD_COMMAND_DIRS, render_commands, install_workflow

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

    from kluris.core.git import git_log_file_dates, is_git_repo

    # One subprocess call to fetch ALL date info (was N per-file calls).
    # Short-circuit on no-git brains so we don't fork git for nothing.
    if is_git_repo(brain_path):
        latest_by_path, created_by_path = git_log_file_dates(brain_path)
    else:
        latest_by_path, created_by_path = {}, {}

    # Walk markdown + opted-in yaml neurons. Delegates to linker's
    # `_all_neuron_files` which already applies the yaml opt-in gate and
    # excludes `kluris.yml` from the SKIP_FILES set.
    from kluris.core.linker import _all_neuron_files as _dream_neuron_files
    neuron_skip = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", "kluris.yml"}
    for neuron_file in _dream_neuron_files(brain_path):
        if neuron_file.name in neuron_skip:
            continue
        if ".git" in neuron_file.parts:
            continue
        try:
            meta, body = read_frontmatter(neuron_file)
            rel_path = str(neuron_file.relative_to(brain_path)).replace("\\", "/")
            is_yaml = neuron_file.suffix.lower() in {".yml", ".yaml"}
            patch: dict = {}
            last_mod = latest_by_path.get(rel_path)
            if last_mod and str(meta.get("updated", "")) != last_mod[:10]:
                patch["updated"] = last_mod[:10]
            # `created` is only enforced for markdown neurons (the lighter
            # yaml contract skips it — see linker.check_frontmatter).
            if not is_yaml and "created" not in meta:
                created = created_by_path.get(rel_path)
                if created:
                    patch["created"] = created[:10]
            if patch:
                # Single write per neuron, single read (no `update_frontmatter`
                # second-load thanks to `preloaded=(meta, body)`).
                update_frontmatter(neuron_file, patch, preloaded=(meta, body))
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
        # `ctx.args` holds remaining unparsed tokens in Click 8.2+; older
        # Click releases (8.0, 8.1) keep them in `ctx.protected_args`. We
        # consult both so we work across the 8.x → 9.x transition without
        # pinning a specific minor version.
        import warnings
        raw_args: list[str] = list(ctx.args or [])
        with warnings.catch_warnings():
            warnings.filterwarnings(
                "ignore",
                message=".*protected_args.*",
                category=DeprecationWarning,
            )
            legacy = getattr(ctx, "protected_args", None) or []
        raw_args.extend(legacy)
        try:
            return super().invoke(ctx)
        except click.ClickException as e:
            import sys
            if "--json" in raw_args or "--json" in sys.argv:
                click.echo(json_lib.dumps({"ok": False, "error": e.format_message()}))
                raise SystemExit(1)
            raise


def _is_interactive() -> bool:
    """Whether stdin is a TTY. Wrapped in a helper so tests can monkeypatch it.

    CliRunner replaces sys.stdin during invoke, so monkeypatching
    ``sys.stdin.isatty`` directly does not survive the swap. Tests should
    monkeypatch this function instead: ``monkeypatch.setattr(cli_module,
    "_is_interactive", lambda: True)``.
    """
    return sys.stdin.isatty()


def _check_brain_paths(resolved: list[tuple[str, dict]]) -> None:
    """Raise ClickException if any resolved brain has a missing or non-dir path.

    Centralizes the stale-path check that used to live only inside ``wake-up``.
    Now every command that goes through ``_resolve_brains`` gets it for free.
    """
    missing = [
        (n, e["path"]) for n, e in resolved
        if not Path(e["path"]).exists() or not Path(e["path"]).is_dir()
    ]
    if not missing:
        return
    msg = "\n".join(f"  - {n}: {p}" for n, p in missing)
    raise click.ClickException(
        f"The following registered brain(s) have missing or invalid paths:\n{msg}\n"
        f"Run `kluris remove <name>` or fix the path in the global config."
    )


def _pick_brain_interactively(
    brains: dict,
    *,
    allow_all: bool,
) -> list[str]:
    """Prompt the user to pick a brain (or 'all'). Returns a list of brain names.

    The picker uses ``IntRange`` for re-prompts on invalid input. Stale brain
    paths are annotated with ``(missing)`` so the user can spot config bugs.
    """
    names = list(brains.keys())  # preserve dict insertion order
    options = list(names)
    if allow_all:
        options.append("all")

    click.echo("Multiple brains registered. Pick one:")
    for i, name in enumerate(options, start=1):
        if name == "all":
            click.echo(f"  [{i}] all")
            continue
        entry = brains[name]
        missing = "" if Path(entry.path).exists() else "  (missing)"
        click.echo(f"  [{i}] {name}  ({entry.path}){missing}")

    raw = click.prompt(
        "Choice",
        type=click.IntRange(1, len(options)),
        show_choices=False,
    )
    chosen = options[raw - 1]
    if chosen == "all":
        return names
    return [chosen]


def _resolve_brains(
    brain_name: str | None,
    *,
    allow_all: bool = False,
    as_json: bool = False,
) -> list[tuple[str, dict]]:
    """Resolve which brain(s) to operate on.

    Resolution order:

    1. ``brain_name == "all"`` — fan out across every registered brain
       (only valid on commands that pass ``allow_all=True``).
    2. Explicit ``brain_name`` — that brain or a clean error if missing.
    3. Zero brains — clean error pointing the user at ``kluris create``.
    4. One brain — auto-resolve.
    5. Multi-brain + interactive TTY — show the picker.
    6. Multi-brain + non-interactive (``--json``, no TTY, or ``KLURIS_NO_PROMPT=1``)
       — error with the available brains listed.

    Every successful path runs ``_check_brain_paths`` so commands never receive
    a brain whose directory has gone missing.
    """
    import os

    config = read_global_config()
    brains = config.brains

    if brain_name == "all":
        if not allow_all:
            raise click.ClickException(
                "--brain all is only supported on dream, push, status, mri."
            )
        if not brains:
            raise click.ClickException(
                "No brains registered. Run 'kluris create <name>' to create one."
            )
        resolved = [(n, brains[n].model_dump()) for n in brains]
        _check_brain_paths(resolved)
        return resolved

    if brain_name:
        if brain_name not in brains:
            raise click.ClickException(
                f"No brain named '{brain_name}' is registered. "
                f"Run 'kluris list' to see available brains."
            )
        resolved = [(brain_name, brains[brain_name].model_dump())]
        _check_brain_paths(resolved)
        return resolved

    if len(brains) == 0:
        raise click.ClickException(
            "No brains registered. Run 'kluris create <name>' to create one."
        )

    if len(brains) == 1:
        name = next(iter(brains))
        resolved = [(name, brains[name].model_dump())]
        _check_brain_paths(resolved)
        return resolved

    # 2+ brains, no --brain
    no_prompt = os.environ.get("KLURIS_NO_PROMPT") == "1"
    if as_json or no_prompt or not _is_interactive():
        hint = " or 'all'" if allow_all else ""
        raise click.ClickException(
            f"Multiple brains registered. Pass --brain NAME{hint}. "
            f"Available: {', '.join(brains.keys())}"
        )

    picked = _pick_brain_interactively(brains, allow_all=allow_all)
    resolved = [(n, brains[n].model_dump()) for n in picked]
    _check_brain_paths(resolved)
    return resolved


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
    if name in BRAIN_NAME_RESERVED:
        raise click.ClickException(
            f"'{name}' is a reserved brain name (used by --brain {name}). "
            "Pick a different name."
        )
    if len(name) > BRAIN_NAME_MAX_LENGTH:
        raise click.ClickException(
            f"Brain name '{name}' is too long ({len(name)} chars). "
            f"Maximum is {BRAIN_NAME_MAX_LENGTH}."
        )
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

    entry = BrainEntry(path=str(brain_path), description=description, type=brain_type)
    register_brain(name, entry)

    # Dream to generate proper maps and navigation, then commit
    _run_dream_on_brain(brain_path)
    if not no_git:
        git_add(brain_path)
        git_commit(brain_path, f"brain: initialize {name}")

    # Install agent skills/workflows
    _do_install()

    defaults = BRAIN_TYPES.get(brain_type, {})
    lobe_count = len(defaults.get("structure", {}))

    if as_json:
        click.echo(json_lib.dumps({
            "ok": True, "name": name, "path": str(brain_path),
            "type": brain_type, "lobes": lobe_count,
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
            console.print(
                "  [dim]Tip: leave blank to use the remote default branch. "
                "Or type any name -- if it exists on the remote we'll track it, "
                "otherwise we'll create it locally so you can push to it later.[/dim]"
            )
            branch_name = click.prompt("  Branch name", default="", show_default=False, type=str) or None
        console.print()
    dest = Path(path) if path else Path(url.rstrip("/").split("/")[-1].replace(".git", ""))
    dest = dest.resolve()

    try:
        git_clone(url, dest)
    except subprocess.CalledProcessError as exc:
        stderr = (exc.stderr or "").strip()
        detail = stderr or f"git exited with status {exc.returncode}"
        raise click.ClickException(
            f"git clone failed for {url}\n{detail}\n\n"
            "Common causes: missing credentials (set up a PAT, SSH key, or "
            "git-credential-manager), unreachable host, or the repository does not exist."
        ) from exc

    try:
        if branch_name:
            from kluris.core.git import checkout_or_create_branch
            checkout_or_create_branch(dest, branch_name)

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
        _do_install()
    except Exception as exc:
        # Clone succeeded but a later step failed (bad branch, not a brain,
        # duplicate name). Remove the partial clone so the user can retry
        # with a different argument instead of having to rm -rf manually.
        import shutil
        shutil.rmtree(dest, ignore_errors=True)
        if isinstance(exc, click.ClickException):
            raise
        if isinstance(exc, subprocess.CalledProcessError):
            stderr = (exc.stderr or "").strip() or str(exc)
            raise click.ClickException(f"Clone setup failed: {stderr}") from exc
        raise click.ClickException(f"Clone setup failed: {exc}") from exc

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
        click.echo(json_lib.dumps({"ok": True, "brains": brains}))
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
        table.add_row(name, entry.type, entry.path, entry.description)

    console.print(table)


_WAKE_UP_SKIP_DIRS = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
# kluris.yml is the brain's local config; it must NEVER be indexed.
_WAKE_UP_SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", "kluris.yml"}


def _wake_up_is_yaml_neuron(path: Path) -> bool:
    """Return True if a yaml file has the kluris opt-in `#---` block."""
    from kluris.core.linker import _has_yaml_opt_in_block
    return _has_yaml_opt_in_block(path)


def _wake_up_iter_neurons(root: Path):
    """Yield neuron files (markdown + opted-in yaml) under `root`, honoring
    wake-up's skip rules. Raw yaml files without a `#---` block are skipped.
    """
    for suffix in ("*.md", "*.yml", "*.yaml"):
        for item in root.rglob(suffix):
            if item.name in _WAKE_UP_SKIP_FILES:
                continue
            if any(part in _WAKE_UP_SKIP_DIRS for part in item.parts):
                continue
            if item.suffix.lower() in {".yml", ".yaml"}:
                if not _wake_up_is_yaml_neuron(item):
                    continue
            yield item


def _wake_up_collect_lobes(brain_path: Path) -> list[dict]:
    """Return top-level lobes with total neuron count AND a per-lobe
    `yaml_count` that tracks opted-in yaml neurons specifically.
    """
    lobes = []
    for child in sorted(brain_path.iterdir()):
        if not child.is_dir():
            continue
        if child.name in _WAKE_UP_SKIP_DIRS or child.name.startswith("."):
            continue
        total = 0
        yaml_count = 0
        for item in _wake_up_iter_neurons(child):
            total += 1
            if item.suffix.lower() in {".yml", ".yaml"}:
                yaml_count += 1
        lobes.append({"name": child.name, "neurons": total, "yaml_count": yaml_count})
    return lobes


def _wake_up_collect_recent(brain_path: Path, limit: int = 5) -> list[dict]:
    """Return up to `limit` most-recently-updated neurons, newest first.
    Each entry now carries a `file_type` field ('yaml' | 'markdown').
    """
    candidates = []
    for item in _wake_up_iter_neurons(brain_path):
        try:
            meta, _ = read_frontmatter(item)
        except Exception:
            continue
        updated = meta.get("updated")
        if updated is None:
            continue
        file_type = "yaml" if item.suffix.lower() in {".yml", ".yaml"} else "markdown"
        candidates.append({
            "path": str(item.relative_to(brain_path)).replace("\\", "/"),
            "updated": str(updated),
            "file_type": file_type,
        })
    candidates.sort(key=lambda item: item["updated"], reverse=True)
    return candidates[:limit]


# Max size of brain.md content returned in wake-up (keeps the bootstrap
# payload bounded even if someone hand-edits brain.md with a wall of text).
_WAKE_UP_BRAIN_MD_MAX_BYTES = 4000


def _wake_up_collect_brain_md(brain_path: Path) -> str:
    """Return the raw body of brain.md (frontmatter stripped), capped to bound the payload.

    Rationale: brain.md lists every top-level lobe with its one-line description.
    Including it in wake-up means the agent has that entire index in its bootstrap
    context, so it doesn't have to Read the file separately on every session.
    """
    brain_md = brain_path / "brain.md"
    if not brain_md.exists():
        return ""
    try:
        _meta, body = read_frontmatter(brain_md)
    except Exception:
        return ""
    if not isinstance(body, str):
        return ""
    body = body.strip()
    if len(body.encode("utf-8")) > _WAKE_UP_BRAIN_MD_MAX_BYTES:
        # Truncate at a rune boundary so the JSON stays valid UTF-8.
        truncated = body.encode("utf-8")[:_WAKE_UP_BRAIN_MD_MAX_BYTES]
        body = truncated.decode("utf-8", errors="ignore") + "\n\n[... truncated]"
    return body


# Regexes for the two glossary line formats kluris supports:
#   - Markdown table row:     | Term | Definition |
#   - Bold-dash one-liner:    **Term** -- Definition
# Glossary parsing is forgiving: we skip header/separator rows and any line
# that doesn't match either shape. Empty definitions are dropped.
import re as _re

_GLOSSARY_TABLE_ROW = _re.compile(r"^\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*$")
_GLOSSARY_BOLD_DASH = _re.compile(r"^\*\*(.+?)\*\*\s*[-–—]+\s*(.+?)\s*$")
_GLOSSARY_HEADER_TERMS = {"term", "meaning", "definition", ":------", ":---", "---", "------"}


def _wake_up_collect_glossary(brain_path: Path) -> list[dict]:
    """Parse glossary.md and return ``[{term, definition}]`` entries.

    Supports both formats kluris ships and recommends:

    - Markdown table rows (the scaffolded format): ``| Term | Meaning |``
    - Bold-dash one-liners (the SKILL.md-recommended format): ``**Term** -- Definition``

    Header rows, separator rows, and lines that don't match either format are
    silently skipped so the collector is robust against free-form glossary
    additions. Frontmatter is stripped.
    """
    glossary = brain_path / "glossary.md"
    if not glossary.exists():
        return []
    try:
        _meta, body = read_frontmatter(glossary)
    except Exception:
        return []
    if not isinstance(body, str):
        return []

    entries: list[dict] = []
    for raw_line in body.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        # Try bold-dash format first (more specific).
        m = _GLOSSARY_BOLD_DASH.match(line)
        if m:
            term, definition = m.group(1).strip(), m.group(2).strip()
            if term and definition and term.lower() not in _GLOSSARY_HEADER_TERMS:
                entries.append({"term": term, "definition": definition})
            continue

        # Then try markdown table rows.
        m = _GLOSSARY_TABLE_ROW.match(line)
        if m:
            term, definition = m.group(1).strip(), m.group(2).strip()
            term_lower = term.lower()
            # Skip the header row and separator row (| ----- | ----- |)
            if (
                term_lower in _GLOSSARY_HEADER_TERMS
                or definition.lower() in _GLOSSARY_HEADER_TERMS
                or set(term) <= {"-", ":", " "}
            ):
                continue
            if term and definition:
                entries.append({"term": term, "definition": definition})

    return entries


@cli.command("search")
@click.argument("query")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--lobe", help="Filter to neurons under this lobe")
@click.option("--tag", help="Filter to neurons with this frontmatter tag")
@click.option("--limit", type=int, default=10, help="Max number of results (default 10)")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def search(query: str, brain_name: str | None, lobe: str | None,
           tag: str | None, limit: int, as_json: bool):
    """Search a brain's neurons, glossary, and brain.md for a query string.

    \b
    Searches:
      - neuron file paths, titles, and frontmatter tags
      - neuron body text (with a 200-char snippet around the first match)
      - glossary terms and definitions
      - brain.md body content

    \b
    Examples:
      kluris search "oauth"
      kluris search "raw sql" --lobe knowledge --json
      kluris search "auth" --tag oauth --limit 5
    """
    if not query:
        raise click.ClickException("Query cannot be empty.")

    from kluris.core.search import search_brain

    brains = _resolve_brains(brain_name, allow_all=False, as_json=as_json)
    name, entry = brains[0]
    brain_path = Path(entry["path"])

    results = search_brain(
        brain_path,
        query,
        limit=limit,
        lobe_filter=lobe,
        tag_filter=tag,
    )

    if as_json:
        click.echo(json_lib.dumps({
            "ok": True,
            "brain": name,
            "query": query,
            "total": len(results),
            "results": results,
        }))
        return

    if not results:
        console.print(f"[dim]No results for '{query}' in {name}.[/dim]")
        return

    console.print(f"\n[bold]{len(results)} result(s) for '{query}' in {name}[/bold]\n")
    for r in results:
        marker = " [yellow](deprecated)[/yellow]" if r["deprecated"] else ""
        console.print(f"  [bold]{r['score']:>3}[/bold]  {r['title']}{marker}")
        console.print(f"        [dim]{r['file']}[/dim]")
        if r["snippet"]:
            console.print(f"        {r['snippet']}")
        console.print()


@cli.command("wake-up")
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def wake_up(brain_name: str | None, as_json: bool):
    """Compact brain snapshot for agent session bootstrap.

    Emits a tight view of the brain's live state: the brain.md body, lobes
    with neuron counts, the 5 most recently updated neurons, the glossary,
    and any deprecation warnings. Designed to be called by the /<skill>
    slash command at session start so the agent has a fast index without
    walking brain.md, glossary.md, and every map.md.

    \b
      kluris wake-up                 # only-brain, text output
      kluris wake-up --brain foo     # target a specific brain when 2+ are registered
      kluris wake-up --json          # machine-readable (for agents)
    """
    brains = _resolve_brains(brain_name, allow_all=False, as_json=as_json)
    name, entry = brains[0]
    brain_path = Path(entry["path"])
    lobes = _wake_up_collect_lobes(brain_path)
    recent = _wake_up_collect_recent(brain_path)
    total_neurons = sum(lobe["neurons"] for lobe in lobes)
    total_yaml_neurons = sum(lobe.get("yaml_count", 0) for lobe in lobes)
    brain_md_body = _wake_up_collect_brain_md(brain_path)
    glossary_entries = _wake_up_collect_glossary(brain_path)
    try:
        deprecation_issues = detect_deprecation_issues(brain_path)
    except Exception:
        deprecation_issues = []
    deprecation_count = len(deprecation_issues)

    data = {
        "ok": True,
        "name": name,
        "path": str(brain_path),
        "description": entry.get("description", ""),
        "brain_md": brain_md_body,
        "lobes": lobes,
        "total_neurons": total_neurons,
        "total_yaml_neurons": total_yaml_neurons,
        "recent": recent,
        "glossary": glossary_entries,
        "deprecation_count": deprecation_count,
        "deprecation": deprecation_issues,
    }

    if as_json:
        click.echo(json_lib.dumps(data))
        return

    console.print(f"\n[bold]Brain: {name}[/bold]")
    console.print(f"  Path: {brain_path}")
    if data["description"]:
        console.print(f"  {data['description']}")
    console.print(f"\n[bold]Lobes ({len(lobes)})[/bold]")
    for lobe in lobes:
        console.print(f"  - {lobe['name']}/: {lobe['neurons']} neurons")
    console.print(f"\n[bold]Total neurons:[/bold] {total_neurons}")
    if glossary_entries:
        console.print(f"\n[bold]Glossary ({len(glossary_entries)} terms)[/bold]")
    if deprecation_count:
        console.print(
            f"\n[bold yellow]Deprecation warnings:[/bold yellow] {deprecation_count} "
            f"(run `kluris dream` for details)"
        )
    if recent:
        console.print(f"\n[bold]Recently updated ({len(recent)})[/bold]")
        for item in recent:
            console.print(f"  {item['updated']} {item['path']}")


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def status(brain_name: str | None, as_json: bool):
    """Show brain status and recent changes."""
    brains = _resolve_brains(brain_name, allow_all=True, as_json=as_json)
    results = []

    for name, entry in brains:
        brain_path = Path(entry["path"])
        _skip = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
        lobes = [d for d in brain_path.iterdir() if d.is_dir() and d.name not in _skip and not d.name.startswith(".")]
        # Use the shared neuron-discovery helper so `status` agrees with
        # `wake-up`, validators, and MRI on what counts as a neuron. The
        # raw rglob we used before would count markdown under .git/,
        # node_modules/, etc.
        neurons = _neuron_files(brain_path)
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
@click.option("--force", is_flag=True, help="Overwrite the neuron if it already exists")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def neuron(file_path: str, lobe: str | None, template_name: str | None,
           brain_name: str | None, force: bool, as_json: bool):
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

    Fails if the target file already exists. Use --force to overwrite.
    """
    brains = _resolve_brains(brain_name, allow_all=False, as_json=as_json)
    name, entry = brains[0]
    brain_path = Path(entry["path"])

    if lobe:
        target_dir = (brain_path / lobe).resolve()
    else:
        target_dir = (brain_path / Path(file_path).parent).resolve() if "/" in file_path else brain_path

    _ensure_within_brain(target_dir, brain_path)

    target_dir.mkdir(parents=True, exist_ok=True)
    target_file = target_dir / Path(file_path).name

    if target_file.exists() and not force:
        rel = target_file.relative_to(brain_path)
        raise click.ClickException(
            f"Neuron '{rel}' already exists. Edit it directly, pick a "
            f"different filename, or pass --force to overwrite."
        )

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
@click.option("--force", is_flag=True, help="Allow creating into an existing directory")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def lobe_cmd(name: str, parent_dir: str | None, desc: str,
             brain_name: str | None, force: bool, as_json: bool):
    """Create a new lobe (knowledge region).

    Fails if the target directory already exists. Use --force to create into
    an existing directory (useful if you want to add a description to a lobe
    that was created outside kluris).
    """
    brains = _resolve_brains(brain_name, allow_all=False, as_json=as_json)
    bname, entry = brains[0]
    brain_path = Path(entry["path"])

    if parent_dir:
        lobe_path = (brain_path / parent_dir / name).resolve()
    else:
        lobe_path = (brain_path / name).resolve()

    _ensure_within_brain(lobe_path, brain_path)

    if lobe_path.exists() and not force:
        rel = lobe_path.relative_to(brain_path)
        raise click.ClickException(
            f"Lobe '{rel}/' already exists. Pick a different name, or pass "
            f"--force to reuse the directory."
        )

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
    brains = _resolve_brains(brain_name, allow_all=True, as_json=as_json)
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
    brains = _resolve_brains(brain_name, allow_all=True, as_json=as_json)
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
            # Report the configured branch instead of a hardcoded "main"
            # so JSON consumers see the truth.
            try:
                clean_branch = read_brain_config(brain_path).git.default_branch
            except Exception:
                clean_branch = "main"
            if not as_json:
                console.print(f"{name}: nothing to push")
            results.append({
                "name": name,
                "files_committed": 0,
                "branch": clean_branch,
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


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def pull(brain_name: str | None, as_json: bool):
    """Pull remote changes; optionally merge origin/<default> into current branch."""
    import os

    brains = _resolve_brains(brain_name, allow_all=True, as_json=as_json)
    results: list[dict] = []
    any_conflict = False

    no_prompt = os.environ.get("KLURIS_NO_PROMPT") == "1"
    can_prompt = _is_interactive() and not as_json and not no_prompt

    for name, entry in brains:
        brain_path = Path(entry["path"])

        if not is_git_repo(brain_path):
            result = {
                "name": name,
                "git_enabled": False,
                "branch": None,
                "pulled": False,
                "files_changed": 0,
                "merged_default": False,
                "conflicts": [],
            }
            results.append(result)
            if not as_json:
                console.print(f"{name}: git is disabled (--no-git)")
            continue

        if git_status(brain_path):
            raise click.ClickException(
                f"{name}: brain has uncommitted changes. "
                "Commit with 'kluris push' or stash them first."
            )

        brain_config = read_brain_config(brain_path)
        default = brain_config.git.default_branch

        try:
            git_fetch(brain_path)
        except subprocess.CalledProcessError as exc:
            stderr = (exc.stderr or "").strip() or f"git fetch exited {exc.returncode}"
            raise click.ClickException(f"{name}: fetch failed -- {stderr}") from exc

        current = git_current_branch(brain_path)
        head_before = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=brain_path,
            capture_output=True, text=True, check=True,
        ).stdout.strip()

        pulled = False
        merged_default = False
        conflicts: list[str] = []

        if git_has_upstream(brain_path):
            try:
                git_pull(brain_path)
                pulled = True
            except subprocess.CalledProcessError:
                conflicts = git_conflicted_files(brain_path)

        if not conflicts and current != default and can_prompt:
            if click.confirm(
                f"  {name}: also merge origin/{default} into {current}?",
                default=True,
            ):
                try:
                    git_merge(brain_path, f"origin/{default}")
                    merged_default = True
                except subprocess.CalledProcessError:
                    conflicts = git_conflicted_files(brain_path)

        head_after = subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=brain_path,
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        if head_before != head_after and not conflicts:
            diff = subprocess.run(
                ["git", "diff", "--name-only", head_before, head_after],
                cwd=brain_path, capture_output=True, text=True, check=False,
            )
            files_changed = len([l for l in diff.stdout.splitlines() if l.strip()])
        else:
            files_changed = 0

        results.append({
            "name": name,
            "git_enabled": True,
            "branch": current,
            "pulled": pulled,
            "files_changed": files_changed,
            "merged_default": merged_default,
            "conflicts": conflicts,
        })

        if conflicts:
            any_conflict = True
            if not as_json:
                console.print(f"\n[bold red]{name}: merge conflicts in:[/bold red]")
                for f in conflicts:
                    console.print(f"  {f}")
                console.print(
                    "\nResolve the conflicts in your editor, then:\n"
                    f"  git -C {brain_path} mergetool    # use your configured merge tool\n"
                    f"  kluris push                       # commit the merged result and push\n"
                    "  OR\n"
                    f"  git -C {brain_path} merge --abort  # throw away the pull\n"
                )
            continue

        if not as_json:
            parts = []
            if pulled:
                parts.append(f"pulled from origin/{current}")
            if merged_default:
                parts.append(f"merged origin/{default}")
            if not parts:
                parts.append("nothing to pull")
            console.print(f"{name}: {', '.join(parts)} ({files_changed} files changed)")

    if as_json:
        click.echo(json_lib.dumps({"ok": not any_conflict, "brains": results}))

    if any_conflict:
        raise SystemExit(1)


def _is_wsl() -> bool:
    """Return True when running under Windows Subsystem for Linux."""
    import os
    if os.environ.get("WSL_DISTRO_NAME"):
        return True
    try:
        with open("/proc/version", encoding="utf-8") as f:
            return "microsoft" in f.read().lower()
    except OSError:
        return False


def _windows_path_if_wsl(path: Path) -> str | None:
    """Translate a Linux path to a Windows UNC path via `wslpath -w`.

    Returns the translated path when running under WSL and wslpath succeeds,
    or ``None`` otherwise. Lets us print a copy-pasteable Windows path for
    WSL users so they can open the MRI in their host browser.
    """
    if not _is_wsl():
        return None
    try:
        result = subprocess.run(
            ["wslpath", "-w", str(path)],
            capture_output=True, text=True, check=True,
        )
        return result.stdout.strip() or None
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None


@cli.command()
@click.option("--brain", "brain_name", help="Specific brain")
@click.option("--output", "output_path", help="Output HTML file path")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def mri(brain_name: str | None, output_path: str | None, as_json: bool):
    """Generate interactive brain visualization."""
    brains = _resolve_brains(brain_name, allow_all=True, as_json=as_json)

    if output_path and len(brains) > 1:
        raise click.ClickException(
            "--output cannot be combined with multi-brain fan-out (each brain "
            "would overwrite the others' output). Pass --output without "
            "--brain all, or pick brains one at a time."
        )

    results: list[dict] = []
    for name, entry in brains:
        brain_path = Path(entry["path"])
        brain_config = read_brain_config(brain_path)
        sync_result = _sync_brain_state(brain_path, brain_config)
        out = Path(output_path) if output_path else brain_path / "brain-mri.html"
        stats = generate_mri_html(brain_path, out)
        results.append({
            "name": name,
            "output_path": str(out),
            "preflight_fixes": sync_result["fixes"],
            **stats,
        })

        if not as_json:
            console.print(f"MRI complete — {out}")
            # Build the right file:// URL for the platform. In WSL the Linux
            # file:// URI (file:///home/...) isn't openable from the Windows
            # host -- convert the wslpath-translated UNC form into a file://
            # URL (file://wsl.localhost/...) that Windows Terminal can Ctrl-
            # click through. Terminals auto-detect file:// URLs and make them
            # click-openable; a bare path would be ignored.
            win_path = _windows_path_if_wsl(out)
            if win_path:
                uri = "file://" + win_path.lstrip("\\").replace("\\", "/")
            else:
                uri = out.resolve().as_uri()
            console.print(f"  Open: [link={uri}]{uri}[/link]")
            console.print(f"  {stats['nodes']} nodes, {stats['edges']} edges")
            if sync_result["fixes"]["total"]:
                console.print(f"  MRI preflight applied {sync_result['fixes']['total']} automatic fixes")

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "brains": results}))


def _sweep_kluris(base: Path, old_dirs_rel: list[str], home: Path) -> None:
    """Delete every ``kluris*`` artifact from a destination.

    Used by :func:`_do_install` before writing new skills. Handles both
    forward (1 brain → N) and backward (N brains → 1) transitions because
    the glob catches both ``kluris/`` and ``kluris-*/`` in one pass.
    """
    import shutil

    # Clean legacy command directories (migration from commands to skills)
    for rel in old_dirs_rel:
        old_dir = home / rel
        if not old_dir.exists():
            continue
        for old_file in old_dir.glob("kluris*"):
            try:
                if old_file.is_file():
                    old_file.unlink()
                elif old_file.is_dir():
                    shutil.rmtree(old_file)
            except OSError:
                pass

    # Clean any existing kluris* skill artifacts in the install destination
    if not base.exists():
        return
    for item in base.glob("kluris*"):
        try:
            if item.is_file():
                item.unlink()
            elif item.is_dir():
                shutil.rmtree(item)
        except OSError:
            pass


def _compute_skills_to_render(brains: dict) -> list[tuple[str, str, object]]:
    """Decide which skills to render based on brain count.

    Returns a list of ``(skill_name, brain_name, brain_entry)`` triples.
    With 1 brain registered the skill is named ``kluris`` (with the brain
    path baked in). With 2+ brains each gets its own ``kluris-<name>``
    skill so the agent can address them unambiguously.
    """
    if len(brains) == 0:
        return []
    if len(brains) == 1:
        only_name, only_entry = next(iter(brains.items()))
        return [("kluris", only_name, only_entry)]
    return [(f"kluris-{n}", n, e) for n, e in brains.items()]


def _do_install(as_json: bool = False):
    """Install agent skills/workflows for all agents across all brains."""
    import os
    import shutil

    config = read_global_config()
    skills_to_render = _compute_skills_to_render(config.brains)

    # Discover which agents the brains opt into via local kluris.yml.
    all_agents: set[str] = set()
    for _name, entry in config.brains.items():
        brain_path = Path(entry.path)
        if (brain_path / "kluris.yml").exists():
            brain_config = read_brain_config(brain_path)
            all_agents.update(brain_config.agents.commands_for)
    if not all_agents:
        all_agents = set(AGENT_REGISTRY.keys())

    home_str = os.environ.get("HOME")
    home = Path(home_str) if home_str else Path.home()
    total_files = 0
    agent_count = 0
    failed_agents: list[tuple[str, str]] = []

    def _render_kwargs(brain_name: str, entry) -> dict:
        return {
            "skill_name": "",  # filled in per skill
            "brain_name": brain_name,
            "brain_path": entry.path,
            "has_git": (Path(entry.path) / ".git").exists(),
            "brain_description": entry.description,
        }

    for agent_name in sorted(all_agents):
        if agent_name not in AGENT_REGISTRY:
            continue
        reg = AGENT_REGISTRY[agent_name]
        base = home / reg["dir"] / reg["subdir"]

        # Stage all new skills to sibling temp directories so an in-flight
        # failure cannot leave the user with no skill at all.
        staged: list[tuple[str, Path]] = []
        try:
            base.mkdir(parents=True, exist_ok=True)
            for skill_name, brain_name, entry in skills_to_render:
                staging = base / f".{skill_name}.tmp"
                if staging.exists():
                    shutil.rmtree(staging)
                kwargs = _render_kwargs(brain_name, entry)
                kwargs["skill_name"] = skill_name
                files = render_commands(
                    agent_name, base, target_dir=staging, **kwargs
                )
                for f in files:
                    if not f.exists():
                        raise OSError(f"Failed to stage {f}")
                staged.append((skill_name, staging))
        except OSError as e:
            for _, s in staged:
                try:
                    shutil.rmtree(s, ignore_errors=True)
                except OSError:
                    pass
            failed_agents.append((agent_name, str(e)))
            continue

        # Past this point we sweep the old artifacts and rename the staged
        # skills into place. Sweep BEFORE rename so any leftover ``kluris*``
        # dirs (legacy or per-brain from a prior install) are removed.
        _sweep_kluris(base, OLD_COMMAND_DIRS.get(agent_name, []), home)

        try:
            for skill_name, staging in staged:
                target = base / skill_name
                if target.exists():
                    shutil.rmtree(target)
                staging.replace(target)
                total_files += 1
            agent_count += 1
        except OSError as e:
            failed_agents.append((agent_name, str(e)))
            continue

    # Windsurf workflow files (one per skill, not per agent loop because
    # only Windsurf opts in via ``also_workflow``). Staged writes + rename
    # so a partial failure leaves the old workflow files in place.
    for agent_name, reg in AGENT_REGISTRY.items():
        wf_dir_rel = reg.get("also_workflow")
        if not wf_dir_rel:
            continue
        wf_dir = home / wf_dir_rel

        # Stage all new workflow files to temp names.
        wf_staged: list[tuple[str, Path]] = []
        try:
            wf_dir.mkdir(parents=True, exist_ok=True)
            for skill_name, brain_name, entry in skills_to_render:
                staging = wf_dir / f".{skill_name}.tmp.md"
                if staging.exists():
                    staging.unlink()
                kwargs = _render_kwargs(brain_name, entry)
                kwargs["skill_name"] = skill_name
                # install_workflow writes to `<wf_dir>/<skill_name>.md`, but
                # we want the staged file instead. Render manually:
                from kluris.core.agents import _render_workflow as _rw
                staging.write_text(_rw(**kwargs), encoding="utf-8")
                if not staging.exists():
                    raise OSError(f"Failed to stage {staging}")
                wf_staged.append((skill_name, staging))
        except OSError as e:
            for _, s in wf_staged:
                try:
                    s.unlink()
                except OSError:
                    pass
            failed_agents.append((f"{agent_name}/workflow", str(e)))
            continue

        # Sweep old kluris*.md workflow files, then rename staged → final.
        if wf_dir.exists():
            for old in wf_dir.glob("kluris*.md"):
                try:
                    old.unlink()
                except OSError:
                    pass
        try:
            for skill_name, staging in wf_staged:
                target = wf_dir / f"{skill_name}.md"
                if target.exists():
                    target.unlink()
                staging.replace(target)
                total_files += 1
        except OSError as e:
            failed_agents.append((f"{agent_name}/workflow", str(e)))
            continue

    # Universal ~/.agents/skills/ slot mirrors the per-brain layout with the
    # same stage-then-rename contract.
    # NOTE: this MUST run after the per-agent installs because codex's
    # OLD_COMMAND_DIRS lists ``.agents/skills`` and the codex sweep would
    # otherwise nuke this slot.
    universal = home / ".agents" / "skills"
    u_staged: list[tuple[str, Path]] = []
    try:
        universal.mkdir(parents=True, exist_ok=True)
        for skill_name, brain_name, entry in skills_to_render:
            staging = universal / f".{skill_name}.tmp"
            if staging.exists():
                shutil.rmtree(staging)
            kwargs = _render_kwargs(brain_name, entry)
            kwargs["skill_name"] = skill_name
            files = render_commands("claude", universal, target_dir=staging, **kwargs)
            for f in files:
                if not f.exists():
                    raise OSError(f"Failed to stage {f}")
            u_staged.append((skill_name, staging))
    except OSError as e:
        for _, s in u_staged:
            try:
                shutil.rmtree(s, ignore_errors=True)
            except OSError:
                pass
        failed_agents.append(("universal", str(e)))
    else:
        _sweep_kluris(universal, [], home)
        try:
            for skill_name, staging in u_staged:
                target = universal / skill_name
                if target.exists():
                    shutil.rmtree(target)
                staging.replace(target)
                total_files += 1
        except OSError as e:
            failed_agents.append(("universal", str(e)))

    return {
        "agents": agent_count,
        "commands_per_agent": len(skills_to_render),
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
@click.option("--force", is_flag=True, help="Unregister even if the brain has uncommitted changes")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def remove(brain_name: str, force: bool, as_json: bool):
    """Unregister a brain (does not delete files).

    Refuses to unregister a brain whose git repo has uncommitted changes
    unless --force is passed. The files on disk are always preserved; this
    guard only exists to prevent an in-flight commit from getting lost in
    the user's active brain registry.
    """
    config = read_global_config()
    if brain_name not in config.brains:
        raise click.ClickException(
            f"No brain named '{brain_name}' is registered. "
            f"Run 'kluris list' to see available brains."
        )

    entry = config.brains[brain_name]
    brain_path = Path(entry.path)
    if not force and brain_path.is_dir() and is_git_repo(brain_path):
        try:
            dirty = git_status(brain_path).strip()
        except Exception:
            dirty = ""
        if dirty:
            raise click.ClickException(
                f"Brain '{brain_name}' has uncommitted changes at {brain_path}.\n"
                f"Commit or stash them first, or pass --force to unregister anyway.\n"
                f"The files on disk are preserved either way."
            )

    unregister_brain(brain_name)
    _do_install()

    if as_json:
        click.echo(json_lib.dumps({"ok": True, "name": brain_name}))
    else:
        console.print(f"Unregistered: {brain_name} (files preserved)")


@cli.command()
@click.option("--no-refresh", is_flag=True, help="Skip refreshing installed agent skills")
@click.option("--json", "as_json", is_flag=True, help="JSON output")
def doctor(as_json: bool, no_refresh: bool):
    """Check prerequisites and refresh installed agent skills.

    Verifies git is installed, Python is recent enough, and the kluris
    config directory is writable. Also re-runs ``_do_install`` to refresh
    the installed agent skills (``~/.claude/skills/kluris*``, the universal
    slot, and the Windsurf workflow files) so they reflect the currently
    registered brains and the current kluris version. This is the muscle-
    memory step after ``pipx upgrade kluris`` -- run ``kluris doctor`` and
    everything is back in sync.

    Pass ``--no-refresh`` to run only the read-only prerequisite checks.
    """
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

    # Refresh installed skills (so `kluris doctor` is also "fix what's safe to fix")
    skills_result: dict | None = None
    if not no_refresh:
        try:
            skills_result = _do_install(as_json=True)
            n_brains = len(read_global_config().brains)
            failed = skills_result.get("failed_agents", [])
            detail = (
                f"{skills_result.get('total_files', 0)} files written for "
                f"{skills_result.get('agents', 0)} agents across {n_brains} brain(s)"
            )
            if failed:
                names = ", ".join(a for a, _ in failed)
                detail += f" (failed: {names})"
                checks.append({"name": "skills", "passed": False, "detail": detail})
            else:
                checks.append({"name": "skills", "passed": True, "detail": detail})
        except Exception as e:
            checks.append({"name": "skills", "passed": False, "detail": f"refresh failed: {e}"})

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
        ("search", "Search a brain for a query string (neurons, glossary, brain.md)"),
        ("wake-up", "Compact brain snapshot for agent session bootstrap"),
        ("neuron", "Create a new neuron (--template for structured formats)"),
        ("lobe", "Create a new lobe (knowledge region)"),
        ("dream", "Regenerate maps, auto-fix safe issues, and validate links"),
        ("push", "Commit and push brain changes to git"),
        ("pull", "Pull remote changes; ask to merge origin/<default> when on another branch"),
        ("mri", "Generate interactive brain visualization and open in browser"),
        ("install-skills", "Install the /kluris skill for your AI agents"),
        ("uninstall-skills", "Remove the /kluris skill from AI agent directories"),
        ("remove", "Unregister a brain (keeps files on disk)"),
        ("templates", "List available neuron templates for the current brain"),
        ("doctor", "Check prerequisites and refresh installed agent skills (--no-refresh to skip)"),
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
