"""Generation of brain.md, map.md, and index.md files."""

from __future__ import annotations

from datetime import date
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter, write_frontmatter
from kluris.core.linker import _has_yaml_opt_in_block

# `kluris.yml` is the brain's local config at the root; it must never be
# indexed as a neuron. Defense in depth alongside `_has_yaml_opt_in_block`.
SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", ".gitignore", "kluris.yml"}
SKIP_DIRS = {".git", ".github", ".vscode", ".idea", "node_modules", "__pycache__"}
NEURON_SUFFIXES = {".md", ".yml", ".yaml"}


def _today() -> str:
    return date.today().isoformat()


def _get_lobes(brain_path: Path) -> list[dict]:
    """Discover top-level lobe directories (dirs containing map.md)."""
    lobes = []
    for item in sorted(brain_path.iterdir()):
        if item.is_dir() and item.name not in SKIP_DIRS and not item.name.startswith("."):
            desc = _read_map_description(item / "map.md")
            lobes.append({"name": item.name, "description": desc, "path": item})
    return lobes


def _read_map_description(map_file: Path) -> str:
    """Read the persisted description for a map file, if available."""
    if not map_file.exists():
        return ""

    try:
        meta, content = read_frontmatter(map_file)
        description = meta.get("description", "")
        if isinstance(description, str) and description.strip():
            return description.strip()
    except Exception:
        try:
            content = map_file.read_text(encoding="utf-8")
        except OSError:
            return ""

    title_seen = False
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("# "):
            title_seen = True
            continue
        if not title_seen:
            continue
        if line.startswith(("up ", "sideways ", "## ", "- [" )):
            continue
        return line

    return ""


def _get_neurons(lobe_path: Path) -> list[dict]:
    """Find all neuron files in a lobe: markdown neurons plus opted-in yaml
    neurons (those with a `#---` hash frontmatter block). Excludes map.md,
    glossary.md, brain.md, README.md, and raw yaml files without the opt-in.
    """
    neurons = []
    for item in sorted(lobe_path.iterdir()):
        if not item.is_file():
            continue
        if item.name in SKIP_FILES:
            continue
        suffix = item.suffix.lower()
        if suffix not in NEURON_SUFFIXES:
            continue
        # Yaml opt-in gate
        if suffix in {".yml", ".yaml"} and not _has_yaml_opt_in_block(item):
            continue
        title = item.stem.replace("-", " ").title()
        tags = []
        updated = ""
        try:
            meta, content = read_frontmatter(item)
            if suffix == ".md":
                # Title from first markdown heading
                for line in content.split("\n"):
                    if line.startswith("# "):
                        title = line[2:].strip()
                        break
            else:
                # Yaml neuron: title comes from frontmatter `title` field
                # (written into the #--- block), else filename stem.
                fm_title = meta.get("title")
                if isinstance(fm_title, str) and fm_title.strip():
                    title = fm_title.strip()
            tags = meta.get("tags", [])
            updated = meta.get("updated", "")
        except Exception:
            pass
        neurons.append({
            "name": item.name,
            "title": title,
            "path": item,
            "tags": tags if isinstance(tags, list) else [],
            "updated": str(updated),
        })
    return neurons


def _get_sub_lobes(lobe_path: Path) -> list[dict]:
    """Find subdirectories that are nested lobes (have map.md)."""
    sub_lobes = []
    for item in sorted(lobe_path.iterdir()):
        if item.is_dir() and item.name not in SKIP_DIRS and (item / "map.md").exists():
            sub_lobes.append({"name": item.name, "path": item})
    return sub_lobes


def _get_siblings(brain_path: Path, lobe_path: Path) -> list[dict]:
    """Find sibling lobes at the same directory level."""
    parent = lobe_path.parent
    siblings = []
    for item in sorted(parent.iterdir()):
        if (item.is_dir() and item != lobe_path
                and item.name not in SKIP_DIRS
                and (item / "map.md").exists()):
            siblings.append({
                "name": item.name,
                "path": f"../{item.name}/map.md",
            })
    return siblings


def generate_brain_md(brain_path: Path, name: str, description: str) -> None:
    """Generate the root brain.md — root lobes and glossary link only."""
    lobes = _get_lobes(brain_path)

    lobe_links = "\n".join(
        f"- [{l['name']}/](./{l['name']}/map.md) — {l['description']}"
        for l in lobes
    )

    content = (
        f"# {name}\n\n{description}\n\n"
        f"## Lobes\n\n{lobe_links}\n\n"
        f"## Reference\n\n"
        f"- [glossary.md](./glossary.md) — Domain-specific terms, acronyms, and conventions\n"
    )

    metadata = {"auto_generated": True, "updated": _today()}
    write_frontmatter(brain_path / "brain.md", metadata, content)


def generate_map_md(brain_path: Path, lobe_path: Path) -> None:
    """Generate a map.md file for a lobe directory."""
    lobe_name = lobe_path.name
    neurons = _get_neurons(lobe_path)
    sub_lobes = _get_sub_lobes(lobe_path)
    siblings = _get_siblings(brain_path, lobe_path)
    description = _read_map_description(lobe_path / "map.md")

    # Determine parent
    if lobe_path.parent == brain_path:
        parent_path = "../brain.md"
        parent_name = "brain.md"
    else:
        parent_path = "../map.md"
        parent_name = lobe_path.parent.name

    # Build contents section
    contents_lines = []
    for sl in sub_lobes:
        contents_lines.append(f"- [{sl['name']}/](./{sl['name']}/map.md)")
    for n in neurons:
        contents_lines.append(f"- [{n['name']}](./{n['name']}) — {n['title']}")
    contents = "\n".join(contents_lines) if contents_lines else "(empty)"

    # Build siblings line
    sibling_links = " | ".join(
        f"[{s['name']}]({s['path']})" for s in siblings
    )

    content = (
        f"# {lobe_name.replace('-', ' ').title()}\n\n"
    )
    if description:
        content += f"{description}\n\n"
    content += f"up [{parent_name}]({parent_path})\n"
    if sibling_links:
        content += f"sideways {sibling_links}\n"
    content += f"\n## Contents\n\n{contents}\n"

    metadata = {
        "auto_generated": True,
        "parent": parent_path,
        "siblings": [s["path"] for s in siblings],
        "updated": _today(),
    }
    if description:
        metadata["description"] = description
    write_frontmatter(lobe_path / "map.md", metadata, content)
