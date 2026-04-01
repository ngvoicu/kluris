"""Generation of brain.md, map.md, and index.md files."""

from __future__ import annotations

from datetime import date
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter, write_frontmatter

SKIP_FILES = {"map.md", "brain.md", "index.md", "glossary.md", "README.md", ".gitignore"}
SKIP_DIRS = {".git"}


def _today() -> str:
    return date.today().isoformat()


def _get_lobes(brain_path: Path) -> list[dict]:
    """Discover top-level lobe directories (dirs containing map.md)."""
    lobes = []
    for item in sorted(brain_path.iterdir()):
        if item.is_dir() and item.name not in SKIP_DIRS:
            desc = ""
            map_file = item / "map.md"
            if map_file.exists():
                # Try to extract description from map.md content
                content = map_file.read_text(encoding="utf-8")
                lines = content.split("\n")
                for line in lines:
                    if line.strip() and not line.startswith("#") and not line.startswith("---"):
                        desc = line.strip()
                        break
            lobes.append({"name": item.name, "description": desc, "path": item})
    return lobes


def _get_neurons(lobe_path: Path) -> list[dict]:
    """Find all .md files in a lobe (excluding map.md and auto-generated files)."""
    neurons = []
    for item in sorted(lobe_path.iterdir()):
        if item.is_file() and item.suffix == ".md" and item.name not in SKIP_FILES:
            title = item.stem.replace("-", " ").title()
            tags = []
            updated = ""
            try:
                meta, content = read_frontmatter(item)
                # Extract first heading as title
                for line in content.split("\n"):
                    if line.startswith("# "):
                        title = line[2:].strip()
                        break
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


def _get_recent_changes(brain_path: Path, lobe_path: Path, limit: int = 5) -> list[dict]:
    """Get recent git changes for a lobe directory."""
    try:
        from kluris.core.git import _run
        result = _run(
            ["git", "log", f"-{limit}", "--format=%aI|%s", "--", str(lobe_path)],
            cwd=brain_path,
        )
        changes = []
        for line in result.stdout.strip().splitlines():
            if "|" in line:
                date_str, msg = line.split("|", 1)
                changes.append({"date": date_str[:10], "action": msg})
        return changes
    except Exception:
        return []


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
        f"up [{parent_name}]({parent_path})\n"
    )
    if sibling_links:
        content += f"sideways {sibling_links}\n"
    content += f"\n## Contents\n\n{contents}\n"

    metadata = {
        "auto_generated": True,
        "parent": parent_path,
        "siblings": [s["path"] for s in siblings],
        "updated": _today(),
    }
    write_frontmatter(lobe_path / "map.md", metadata, content)


def generate_index_md(brain_path: Path) -> None:
    """Deprecated — index is now part of brain.md. No-op for backward compat."""
    pass
