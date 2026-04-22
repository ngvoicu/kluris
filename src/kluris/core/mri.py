"""MRI visualization — build graph from brain and generate standalone HTML."""

from __future__ import annotations

import json
import re
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter
from kluris.core.linker import LINK_PATTERN, _has_yaml_opt_in_block

# Empty HTML anchor tags used as jump targets (`<a id="foo"></a>term`) show
# up as literal markup in the MRI preview because content is `escapeHtml`'d.
# Strip them from the preview text — the jump-target role is a markdown
# feature, not something a human reading the preview needs to see.
_EMPTY_HTML_ANCHOR = re.compile(r'<a\s+id=["\'][^"\']*["\']\s*>\s*</a>', re.IGNORECASE)


def _strip_empty_html_anchors(text: str) -> str:
    return _EMPTY_HTML_ANCHOR.sub("", text)

SKIP_DIRS = {".git"}
# `kluris.yml` at brain root is the local config; never index it as a node.
SKIP_FILES = {".gitignore", "README.md", "kluris.yml"}
YAML_NEURON_SUFFIXES = {".yml", ".yaml"}


def _all_neuron_files(brain_path: Path) -> list[Path]:
    """Collect markdown neurons plus opted-in yaml neurons.

    MRI has its own narrower SKIP_FILES (it keeps `glossary.md`, `index.md`,
    `brain.md`, `map.md` as visible graph nodes — unlike linker/maps which
    hide them). The yaml opt-in gate is the same.
    """
    files: list[Path] = []
    for item in brain_path.rglob("*.md"):
        if any(part in SKIP_DIRS for part in item.parts):
            continue
        if item.name in SKIP_FILES:
            continue
        files.append(item)
    for suffix in ("*.yml", "*.yaml"):
        for item in brain_path.rglob(suffix):
            if any(part in SKIP_DIRS for part in item.parts):
                continue
            if item.name in SKIP_FILES:
                continue
            if not _has_yaml_opt_in_block(item):
                continue
            files.append(item)
    return files


# Backward-compat alias for any external caller.
_all_md_files = _all_neuron_files


def _extract_title_and_excerpt(path: Path, content: str) -> tuple[str, str, str]:
    """Extract a display title, an authored H1 subtitle, and a short excerpt.

    The display title is always the filename stem (hyphens → spaces,
    title-cased) so compact labels everywhere in the MRI read consistently
    — no matter what H1 the author wrote. The authored H1 is returned
    separately so the modal can still surface it as a subtitle when it
    adds information beyond the filename.
    """
    display_title = path.stem.replace("-", " ").title()
    authored_title = ""
    excerpt = ""

    for raw_line in content.splitlines():
        line = _strip_empty_html_anchors(raw_line).strip()
        if not line:
            continue
        if line.startswith("# ") and not authored_title:
            authored_title = line[2:].strip()
            continue
        if line.startswith(("## ", "- ", "* ", "```", "---", "up ", "sideways ")):
            continue
        excerpt = line
        break

    return display_title, authored_title, excerpt[:220]


def _build_content_preview(content: str) -> tuple[str, str, bool]:
    """Return (full_body, preview, truncated) for a markdown document.

    The preview is bounded for the inspector panel; the full body has the same
    title-stripping but no length cap so the modal can show the whole document.
    """
    if not content.strip():
        return "", "", False

    body_lines: list[str] = []
    skipped_title = False

    for raw_line in content.splitlines():
        line = _strip_empty_html_anchors(raw_line).rstrip()
        if not skipped_title and line.strip().startswith("# "):
            skipped_title = True
            continue
        if not body_lines and not line.strip():
            continue
        body_lines.append(line)

    full_body = "\n".join(body_lines).strip()
    if not full_body:
        return "", "", False

    max_lines = 48
    max_chars = 2800
    truncated = len(body_lines) > max_lines or len(full_body) > max_chars
    preview = "\n".join(body_lines[:max_lines]).strip()

    if len(preview) > max_chars:
        preview = preview[:max_chars].rstrip()
        if "\n" in preview:
            preview = preview.rsplit("\n", 1)[0].rstrip()

    if truncated:
        preview = preview.rstrip() + "\n\n..."

    return full_body, preview, truncated


def build_graph(brain_path: Path) -> dict:
    """Build a graph of nodes and edges from a brain directory."""
    nodes = []
    edges = []
    node_ids: dict[str, int] = {}

    files = _all_md_files(brain_path)

    # Create nodes
    for i, f in enumerate(files):
        rel = f.relative_to(brain_path).as_posix()
        node_ids[rel] = i

        # Determine lobe and sublobe
        parts = f.relative_to(brain_path).parts
        lobe = parts[0] if len(parts) > 1 else "root"
        sublobe = "/".join(parts[:-1]) if len(parts) > 2 else lobe

        # Determine type
        if f.name == "brain.md":
            ntype = "brain"
        elif f.name == "index.md":
            ntype = "index"
        elif f.name == "glossary.md":
            ntype = "glossary"
        elif f.name == "map.md":
            ntype = "map"
        else:
            ntype = "neuron"

        meta = {}
        content = ""
        try:
            meta, content = read_frontmatter(f)
        except Exception:
            pass

        # File flavor: yaml neurons get file_type "yaml", everything else
        # (including auto-generated brain.md / map.md / glossary.md / index.md)
        # is "markdown".
        is_yaml = f.suffix.lower() in YAML_NEURON_SUFFIXES
        file_type = "yaml" if is_yaml else "markdown"

        title, authored_title, excerpt = _extract_title_and_excerpt(f, content)
        # Non-neuron nodes (map, brain, glossary, index) use the H1 as their
        # title because their filenames are always the same across every lobe
        # — `map.md`, `brain.md`, `glossary.md` — so the stem collapses every
        # lobe's map node to "Map" in sidebars. The H1 carries the real name
        # (the lobe name) for these nodes.
        if ntype != "neuron" and authored_title:
            title = authored_title
            authored_title = ""
        # Yaml neurons: prefer frontmatter `title` as the display title.
        # Stems like `openapi.yml` title-case poorly ("Openapi"), and the
        # frontmatter title is the author's canonical name for the spec.
        if is_yaml:
            fm_title = meta.get("title")
            if isinstance(fm_title, str) and fm_title.strip():
                title = fm_title.strip()
                authored_title = ""

        content_full, content_preview, preview_truncated = _build_content_preview(content)
        tags = meta.get("tags", [])
        related = meta.get("related", [])

        # authored_title is surfaced in the modal header only when it genuinely
        # differs from the filename-derived display title — so compact labels
        # stay uniform while the author's H1 still shows up for context when
        # it adds something.
        authored_subtitle = authored_title if authored_title and authored_title != title else ""

        nodes.append({
            "id": i,
            "label": title,
            "path": rel,
            "lobe": lobe,
            "sublobe": sublobe,
            "type": ntype,
            "file_type": file_type,
            "file_name": f.name,
            "title": title,
            "authored_title": authored_subtitle,
            "excerpt": excerpt,
            "content_preview": content_preview,
            "content_preview_truncated": preview_truncated,
            "content_full": content_full,
            "tags": tags if isinstance(tags, list) else [],
            "created": str(meta.get("created", "")),
            "updated": str(meta.get("updated", "")),
            "parent": str(meta.get("parent", "")),
            "related": related if isinstance(related, list) else [],
        })

    # Create edges from frontmatter and inline links
    for f in files:
        rel = f.relative_to(brain_path).as_posix()
        source_id = node_ids.get(rel)
        if source_id is None:
            continue

        try:
            meta, content = read_frontmatter(f)
        except Exception:
            continue

        # Parent edge
        parent = meta.get("parent")
        if parent:
            try:
                parent_resolved = (f.parent / parent).resolve().relative_to(brain_path.resolve()).as_posix()
            except (ValueError, OSError):
                parent_resolved = ""
            if parent_resolved in node_ids:
                edges.append({
                    "source": source_id,
                    "target": node_ids[parent_resolved],
                    "type": "parent",
                })

        # Related edges
        related = meta.get("related", [])
        if isinstance(related, list):
            for r in related:
                try:
                    r_resolved = (f.parent / r).resolve().relative_to(brain_path.resolve()).as_posix()
                except (ValueError, OSError):
                    continue
                if r_resolved in node_ids:
                    edges.append({
                        "source": source_id,
                        "target": node_ids[r_resolved],
                        "type": "related",
                    })

        # Inline link edges
        for match in LINK_PATTERN.finditer(content):
            target = match.group(2)
            if target.startswith("http"):
                continue
            # Strip #anchor / ?query so `glossary.md#jwt` still resolves to glossary.md.
            target_path = target.split("#", 1)[0].split("?", 1)[0]
            if not target_path:
                continue
            try:
                t_resolved = (f.parent / target_path).resolve().relative_to(brain_path.resolve()).as_posix()
                if t_resolved in node_ids and t_resolved != rel:
                    # Avoid duplicating parent/related edges
                    existing = {(e["source"], e["target"]) for e in edges}
                    if (source_id, node_ids[t_resolved]) not in existing:
                        edges.append({
                            "source": source_id,
                            "target": node_ids[t_resolved],
                            "type": "inline",
                        })
            except (ValueError, OSError):
                continue

    neighbors: dict[int, set[int]] = {node["id"]: set() for node in nodes}
    edge_counts = {"parent": 0, "related": 0, "inline": 0}
    for edge in edges:
        neighbors[edge["source"]].add(edge["target"])
        neighbors[edge["target"]].add(edge["source"])
        edge_counts[edge["type"]] = edge_counts.get(edge["type"], 0) + 1

    for node in nodes:
        node["degree"] = len(neighbors[node["id"]])

    return {"nodes": nodes, "edges": edges, "meta": {"edge_counts": edge_counts}}


def generate_mri_html(brain_path: Path, output_path: Path) -> dict:
    """Generate a standalone HTML visualization of the brain graph."""
    graph = build_graph(brain_path)
    brain_name = brain_path.name
    graph_json = json.dumps(graph, indent=2)

    html = f"""\
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{brain_name} Brain MRI</title>
<style>
  :root {{
    --bg: #0a0f1a;
    --panel: rgba(10, 16, 30, 0.92);
    --panel-strong: rgba(12, 21, 44, 0.96);
    --line: rgba(123, 167, 255, 0.18);
    --text: #e9f1ff;
    --muted: #8ba7d1;
    --accent: #7bf7ff;
    --accent-2: #ff8bd8;
    --accent-3: #f8c76d;
    --success: #7df7b4;
    --shadow: 0 30px 80px rgba(0, 0, 0, 0.45);
    --radius: 22px;
    --mono: "SFMono-Regular", "SF Mono", "Monaco", "Cascadia Code", monospace;
    --sans: "Avenir Next", "Segoe UI", sans-serif;
  }}
  * {{ box-sizing: border-box; }}
  html, body {{ height: 100%; }}
  body {{
    margin: 0;
    font-family: var(--sans);
    color: var(--text);
    background:
      radial-gradient(circle at 20% 20%, rgba(123, 247, 255, 0.20), transparent 28%),
      radial-gradient(circle at 78% 12%, rgba(255, 139, 216, 0.18), transparent 24%),
      radial-gradient(circle at 68% 74%, rgba(248, 199, 109, 0.16), transparent 26%),
      linear-gradient(145deg, #050913 0%, #06111f 38%, #0a1731 100%);
    overflow: hidden;
  }}
  body::before {{
    content: "";
    position: fixed;
    inset: 0;
    background:
      linear-gradient(rgba(255,255,255,0.03) 1px, transparent 1px),
      linear-gradient(90deg, rgba(255,255,255,0.03) 1px, transparent 1px);
    background-size: 36px 36px;
    mask-image: radial-gradient(circle at center, black 35%, transparent 88%);
    pointer-events: none;
  }}
  .shell {{
    position: relative;
    display: grid;
    grid-template-columns: minmax(300px, 360px) minmax(0, 1fr) minmax(320px, 400px);
    height: 100vh;
    gap: 18px;
    padding: 18px;
  }}
  .shell.left-collapsed {{
    grid-template-columns: minmax(0, 1fr) minmax(320px, 400px);
  }}
  .shell.right-collapsed {{
    grid-template-columns: minmax(300px, 360px) minmax(0, 1fr);
  }}
  .shell.left-collapsed.right-collapsed {{
    grid-template-columns: minmax(0, 1fr);
  }}
  .shell.left-collapsed .panel-left,
  .shell.right-collapsed .panel-right {{
    display: none;
  }}
  .panel-collapse-btn {{
    appearance: none;
    position: absolute;
    top: 14px;
    right: 14px;
    width: 24px;
    height: 24px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.12);
    background: rgba(8,15,32,0.70);
    color: var(--muted);
    cursor: pointer;
    font-size: 0.85rem;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 10;
    transition: border-color 160ms ease, background 160ms ease, color 160ms ease;
  }}
  .panel-collapse-btn:hover {{
    color: var(--text);
    border-color: rgba(123,247,255,0.40);
    background: rgba(123,247,255,0.12);
  }}
  .panel-expand-btn {{
    appearance: none;
    position: fixed;
    top: 50%;
    transform: translateY(-50%);
    width: 22px;
    height: 56px;
    border: 1px solid rgba(255,255,255,0.12);
    background: rgba(12,21,44,0.88);
    color: var(--muted);
    cursor: pointer;
    font-size: 0.9rem;
    line-height: 1;
    display: none;
    align-items: center;
    justify-content: center;
    z-index: 25;
    backdrop-filter: blur(10px);
    transition: border-color 160ms ease, background 160ms ease, color 160ms ease;
  }}
  .panel-expand-btn:hover {{
    color: var(--text);
    border-color: rgba(123,247,255,0.45);
    background: rgba(123,247,255,0.16);
  }}
  .panel-expand-btn.panel-expand-left {{
    left: 0;
    border-radius: 0 12px 12px 0;
    border-left: none;
  }}
  .panel-expand-btn.panel-expand-right {{
    right: 0;
    border-radius: 12px 0 0 12px;
    border-right: none;
  }}
  .panel-expand-btn.visible {{
    display: flex;
  }}
  .panel {{
    position: relative;
    z-index: 5;
    background: var(--panel);
    border: 1px solid rgba(255, 255, 255, 0.09);
    border-radius: var(--radius);
    box-shadow: var(--shadow);
    backdrop-filter: blur(18px);
    overflow: hidden;
  }}
  .panel::after {{
    content: "";
    position: absolute;
    inset: 0;
    border-radius: inherit;
    border: 1px solid rgba(255, 255, 255, 0.04);
    pointer-events: none;
  }}
  .panel-inner {{
    position: relative;
    padding: 18px 18px 20px;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
  }}
  .eyebrow {{
    margin: 0 0 10px;
    font-size: 11px;
    letter-spacing: 0.32em;
    text-transform: uppercase;
    color: var(--accent);
  }}
  h1, h2, h3, p {{ margin: 0; }}
  h1 {{
    font-size: clamp(1.5rem, 2.6vw, 2.2rem);
    line-height: 1.04;
    letter-spacing: -0.04em;
    text-transform: uppercase;
    overflow-wrap: anywhere;
    word-break: break-word;
  }}
  .subhead {{
    margin-top: 12px;
    font-size: 0.96rem;
    line-height: 1.5;
    color: var(--muted);
  }}
  .stats {{
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin-top: 18px;
  }}
  .stat {{
    padding: 12px 14px;
    border-radius: 18px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }}
  .stat-label {{
    font-size: 0.72rem;
    text-transform: uppercase;
    letter-spacing: 0.18em;
    color: var(--muted);
  }}
  .stat-value {{
    margin-top: 8px;
    font-size: 1.2rem;
    font-weight: 700;
  }}
  .search-wrap {{
    margin-top: 18px;
    padding: 14px;
    border-radius: 18px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.06);
  }}
  .search-wrap label {{
    display: block;
    font-size: 0.74rem;
    text-transform: uppercase;
    letter-spacing: 0.18em;
    color: var(--muted);
    margin-bottom: 10px;
  }}
  .search-row {{
    display: flex;
    gap: 10px;
  }}
  #search-input {{
    width: 100%;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 14px;
    background: rgba(6, 12, 27, 0.82);
    color: var(--text);
    padding: 12px 14px;
    outline: none;
    font: inherit;
  }}
  #search-input:focus {{
    border-color: rgba(123, 247, 255, 0.55);
    box-shadow: 0 0 0 3px rgba(123, 247, 255, 0.12);
  }}
  .button {{
    appearance: none;
    border: 1px solid rgba(255,255,255,0.08);
    background: linear-gradient(180deg, rgba(123,247,255,0.16), rgba(123,247,255,0.06));
    color: var(--text);
    border-radius: 14px;
    padding: 0 14px;
    font: inherit;
    cursor: pointer;
    transition: transform 160ms ease, border-color 160ms ease, background 160ms ease;
  }}
  .button:hover {{ transform: translateY(-1px); border-color: rgba(123,247,255,0.32); }}
  .results {{
    margin-top: 16px;
    display: grid;
    gap: 10px;
  }}
  .result-card, .connection-card {{
    width: 100%;
    text-align: left;
    border: 1px solid rgba(255,255,255,0.07);
    background: rgba(255,255,255,0.04);
    color: var(--text);
    border-radius: 16px;
    padding: 12px 14px;
    cursor: pointer;
  }}
  .result-card:hover, .connection-card:hover {{
    border-color: rgba(123,247,255,0.35);
    background: rgba(123,247,255,0.08);
  }}
  .conn-show-all {{
    appearance: none;
    width: 100%;
    padding: 8px 14px;
    border-radius: 12px;
    border: 1px dashed rgba(123,247,255,0.25);
    background: rgba(123,247,255,0.04);
    color: var(--accent);
    font: inherit;
    font-size: 0.82rem;
    font-weight: 600;
    cursor: pointer;
    transition: border-color 160ms ease, background 160ms ease;
  }}
  .conn-show-all:hover {{
    border-color: rgba(123,247,255,0.50);
    background: rgba(123,247,255,0.10);
  }}
  .result-title {{
    font-weight: 700;
    font-size: 0.98rem;
  }}
  .result-meta {{
    margin-top: 4px;
    color: var(--muted);
    font-size: 0.84rem;
  }}
  .result-path {{
    margin-top: 8px;
    color: var(--accent);
    font-family: var(--mono);
    font-size: 0.78rem;
    word-break: break-all;
  }}
  .lobes-list {{
    margin-top: 4px;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 8px;
  }}
  .lobe-group {{
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
    min-width: 0;
  }}
  .lobe-card-wrap {{
    position: relative;
    min-width: 0;
  }}
  .lobe-card-wrap.has-caret .lobe-card {{
    padding-right: 40px;
  }}
  .lobe-caret {{
    appearance: none;
    position: absolute;
    top: 50%;
    right: 8px;
    transform: translateY(-50%);
    width: 24px;
    height: 24px;
    border: 1px solid rgba(255,255,255,0.10);
    border-radius: 8px;
    background: rgba(8,15,32,0.7);
    color: var(--muted);
    font-size: 0.78rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2;
    transition: border-color 160ms ease, background 160ms ease, color 160ms ease;
  }}
  .lobe-caret:hover {{
    color: var(--text);
    border-color: rgba(123,247,255,0.45);
    background: rgba(123,247,255,0.14);
  }}
  .sublobes-list {{
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
    margin-left: 14px;
    padding-left: 10px;
    border-left: 1px dashed rgba(255,255,255,0.10);
    min-width: 0;
  }}
  .sublobe-group {{
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
    min-width: 0;
  }}
  .sublobe-card-wrap {{
    position: relative;
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
  }}
  .sublobe-card-wrap.has-caret > button.sublobe-card {{
    padding-right: 30px;
  }}
  .sublobe-card-wrap > .lobe-caret {{
    position: absolute;
    top: 50%;
    right: 6px;
    transform: translateY(-50%);
  }}
  .sublobe-card {{
    appearance: none;
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-width: 0;
    text-align: left;
    padding: 7px 10px;
    border-radius: 12px;
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.06);
    color: var(--text);
    font: inherit;
    cursor: pointer;
    transition: border-color 160ms ease, background 160ms ease, transform 160ms ease;
  }}
  .sublobe-card:hover {{
    border-color: rgba(123,247,255,0.30);
    background: rgba(123,247,255,0.06);
    transform: translateX(2px);
  }}
  .sublobe-card.dimmed {{
    opacity: 0.42;
    background: rgba(255,255,255,0.02);
    transform: none;
  }}
  .sublobe-card.dimmed:hover {{
    opacity: 0.75;
  }}
  .sublobe-card[disabled] {{
    cursor: default;
    opacity: 0.55;
    transform: none;
  }}
  .sublobe-tick {{
    width: 3px;
    height: 22px;
    border-radius: 999px;
    flex-shrink: 0;
  }}
  .sublobe-name {{
    font-weight: 600;
    font-size: 0.70rem;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .lobe-card {{
    appearance: none;
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-width: 0;
    text-align: left;
    padding: 9px 11px;
    border-radius: 14px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.07);
    color: var(--text);
    font: inherit;
    cursor: pointer;
    transition: border-color 160ms ease, background 160ms ease, transform 160ms ease;
  }}
  .lobe-card:hover {{
    border-color: rgba(123,247,255,0.35);
    background: rgba(123,247,255,0.08);
    transform: translateY(-1px);
  }}
  .lobe-card.dimmed {{
    opacity: 0.42;
    background: rgba(255,255,255,0.02);
  }}
  .lobe-card.dimmed:hover {{
    opacity: 0.75;
  }}
  .lobe-card[disabled] {{
    cursor: default;
    opacity: 0.55;
    transform: none;
  }}
  .lobe-swatch {{
    width: 12px;
    height: 12px;
    border-radius: 50%;
    flex-shrink: 0;
  }}
  .lobe-body {{
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }}
  .lobe-name {{
    font-weight: 700;
    font-size: 0.78rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .lobe-meta {{
    color: var(--muted);
    font-size: 0.70rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .stage {{
    position: relative;
    border-radius: 28px;
    overflow: hidden;
    border: 1px solid rgba(255,255,255,0.08);
    box-shadow: var(--shadow);
    background:
      radial-gradient(circle at 50% 45%, rgba(123,247,255,0.08), transparent 34%),
      radial-gradient(circle at 50% 50%, rgba(255,255,255,0.05), transparent 68%);
  }}
  canvas {{
    display: block;
    width: 100%;
    height: 100%;
    cursor: grab;
  }}
  canvas.dragging {{ cursor: grabbing; }}
  .stage-hud {{
    position: absolute;
    top: 16px;
    left: 16px;
    right: 16px;
    display: flex;
    justify-content: space-between;
    gap: 12px;
    pointer-events: none;
  }}
  .stage-pill {{
    pointer-events: auto;
    padding: 10px 14px;
    border-radius: 999px;
    background: rgba(8,15,32,0.72);
    border: 1px solid rgba(255,255,255,0.08);
    color: var(--muted);
    font-size: 0.84rem;
    backdrop-filter: blur(10px);
  }}
  .details-card {{
    margin-top: 18px;
    padding: 18px;
    border-radius: 20px;
    background: linear-gradient(180deg, rgba(255,255,255,0.04), rgba(255,255,255,0.02));
    border: 1px solid rgba(255,255,255,0.08);
  }}
  .details-empty {{
    margin-top: 18px;
    padding: 18px;
    border-radius: 20px;
    border: 1px dashed rgba(255,255,255,0.12);
    color: var(--muted);
    line-height: 1.6;
  }}
  .details-title {{
    font-size: 1.35rem;
    line-height: 1.05;
  }}
  .details-subtitle {{
    margin-top: 4px;
    color: var(--muted);
    font-size: 0.85rem;
    font-style: italic;
    line-height: 1.2;
  }}
  .details-path {{
    margin-top: 10px;
    color: var(--accent);
    font-family: var(--mono);
    font-size: 0.82rem;
    word-break: break-all;
  }}
  .meta-grid {{
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    margin-top: 16px;
  }}
  .meta-card {{
    padding: 12px;
    border-radius: 16px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.06);
  }}
  .meta-card .label {{
    display: block;
    margin-bottom: 6px;
    color: var(--muted);
    font-size: 0.74rem;
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }}
  .meta-card .value {{
    font-size: 0.95rem;
    line-height: 1.45;
    word-break: break-word;
  }}
  .tag-row {{
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 14px;
  }}
  .tag {{
    padding: 6px 10px;
    border-radius: 999px;
    background: rgba(255,255,255,0.06);
    color: var(--text);
    font-size: 0.8rem;
    border: 1px solid rgba(255,255,255,0.08);
  }}
  .details-copy {{
    margin-top: 14px;
    line-height: 1.7;
    color: #d7e3fb;
  }}
  .content-preview {{
    margin-top: 14px;
    padding: 16px;
    border-radius: 18px;
    background: rgba(6, 12, 27, 0.86);
    border: 1px solid rgba(255,255,255,0.08);
    color: #eef4ff;
    font-family: var(--mono);
    font-size: 0.82rem;
    line-height: 1.65;
    white-space: pre-wrap;
    word-break: break-word;
    overflow-wrap: anywhere;
  }}
  .content-preview-note {{
    margin-top: 10px;
    color: var(--muted);
    font-size: 0.82rem;
  }}
  .section-title {{
    margin-top: 20px;
    margin-bottom: 10px;
    font-size: 0.76rem;
    letter-spacing: 0.22em;
    text-transform: uppercase;
    color: var(--muted);
  }}
  .legend {{
    display: grid;
    gap: 10px;
    margin-top: 18px;
    padding-top: 18px;
    border-top: 1px solid rgba(255,255,255,0.08);
  }}
  .legend-item {{
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--muted);
    font-size: 0.86rem;
  }}
  .legend-swatch {{
    width: 14px;
    height: 14px;
    border-radius: 50%;
    display: inline-block;
    box-shadow: 0 0 18px rgba(255,255,255,0.18);
  }}
  .legend-line {{
    width: 26px;
    height: 2px;
    display: inline-block;
    border-radius: 999px;
    background: rgba(255,255,255,0.5);
  }}
  .legend-line.related {{ background: linear-gradient(90deg, var(--accent), rgba(123,247,255,0.18)); }}
  .legend-line.parent {{ background: linear-gradient(90deg, var(--accent-3), rgba(248,199,109,0.18)); }}
  .legend-line.inline {{ background: linear-gradient(90deg, var(--accent-2), rgba(255,139,216,0.18)); }}
  .breadcrumbs {{
    margin-top: 8px;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
    font-family: var(--mono);
    font-size: 0.82rem;
  }}
  .breadcrumb-link {{
    appearance: none;
    background: none;
    border: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    padding: 2px 0;
  }}
  .breadcrumb-link:hover {{ text-decoration: underline; }}
  .breadcrumb-current {{ color: var(--text); padding: 2px 0; }}
  .breadcrumb-sep {{ color: var(--muted); padding: 0 4px; }}
  .expand-btn {{
    appearance: none;
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.1);
    color: var(--muted);
    border-radius: 8px;
    padding: 4px 8px;
    font-size: 0.76rem;
    cursor: pointer;
    margin-left: 8px;
  }}
  .expand-btn:hover {{ color: var(--text); border-color: rgba(123,247,255,0.4); }}
  .modal-overlay {{
    position: fixed;
    inset: 0;
    z-index: 100;
    background: rgba(0,0,0,0.7);
    backdrop-filter: blur(6px);
    display: flex;
    align-items: center;
    justify-content: center;
  }}
  .modal-box {{
    width: 90vw;
    max-height: 90vh;
    background: var(--panel-strong);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: var(--radius);
    box-shadow: 0 40px 100px rgba(0,0,0,0.6);
    display: grid;
    grid-template-columns: 280px minmax(0, 1fr);
    grid-template-rows: auto 1fr;
    overflow: hidden;
  }}
  .modal-header {{
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 22px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
  }}
  .modal-tree {{
    grid-row: 2;
    grid-column: 1;
    overflow: auto;
    border-right: 1px solid rgba(255,255,255,0.08);
    padding: 14px 12px;
    background: rgba(0,0,0,0.18);
    font-family: var(--mono);
    font-size: 0.82rem;
  }}
  .modal-tree-folder {{
    margin: 2px 0;
  }}
  .modal-tree-folder-label {{
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    color: var(--muted);
    cursor: pointer;
    border-radius: 4px;
    user-select: none;
  }}
  .modal-tree-folder-label:hover {{
    background: rgba(255,255,255,0.06);
    color: var(--text);
  }}
  .modal-tree-folder-label .caret {{
    display: inline-block;
    width: 10px;
    text-align: center;
    color: var(--muted);
    transition: transform 0.1s;
  }}
  .modal-tree-folder.collapsed > .modal-tree-children {{ display: none; }}
  .modal-tree-folder.collapsed > .modal-tree-folder-label .caret {{
    transform: rotate(-90deg);
  }}
  .modal-tree-children {{
    margin-left: 14px;
    border-left: 1px solid rgba(255,255,255,0.06);
    padding-left: 6px;
  }}
  .modal-tree-file {{
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
    color: var(--text);
    cursor: pointer;
    border-radius: 4px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }}
  .modal-tree-file:hover {{
    background: rgba(123,247,255,0.08);
    color: var(--accent);
  }}
  .modal-tree-file.active {{
    background: rgba(123,247,255,0.18);
    color: var(--accent);
  }}
  .modal-tree-file .icon,
  .modal-tree-folder-label .icon {{
    opacity: 0.6;
    flex-shrink: 0;
  }}
  .modal-title {{
    font-size: 1.1rem;
    font-weight: 700;
    color: var(--text);
  }}
  .modal-close {{
    appearance: none;
    background: none;
    border: none;
    color: var(--muted);
    font-size: 1.6rem;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }}
  .modal-close:hover {{ color: var(--text); }}
  .modal-main {{
    grid-row: 2;
    grid-column: 2;
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: hidden;
  }}
  .modal-nav {{
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 12px 22px;
    border-bottom: 1px solid rgba(255,255,255,0.06);
  }}
  .modal-nav:empty {{ display: none; }}
  .modal-nav-toggle {{
    appearance: none;
    background: rgba(123,247,255,0.04);
    border: 1px dashed rgba(123,247,255,0.25);
    color: var(--accent);
    border-radius: 999px;
    padding: 5px 12px;
    font-size: 0.78rem;
    font-family: var(--mono);
    font-weight: 600;
    cursor: pointer;
    flex-shrink: 0;
  }}
  .modal-nav-toggle:hover {{ background: rgba(123,247,255,0.10); border-color: rgba(123,247,255,0.50); }}
  .modal-nav-btn {{
    appearance: none;
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.1);
    color: var(--accent);
    border-radius: 999px;
    padding: 5px 12px;
    font-size: 0.78rem;
    font-family: var(--mono);
    cursor: pointer;
  }}
  .modal-nav-btn:hover {{ background: rgba(123,247,255,0.12); border-color: rgba(123,247,255,0.4); }}
  .content-link {{
    appearance: none;
    background: none;
    border: none;
    color: var(--accent);
    font: inherit;
    cursor: pointer;
    padding: 0;
    text-decoration: underline;
    text-decoration-color: rgba(123,247,255,0.3);
    text-underline-offset: 2px;
  }}
  .content-link:hover {{ text-decoration-color: var(--accent); }}
  .content-link-broken {{
    color: #ff8a8a;
    text-decoration: line-through;
    text-decoration-color: rgba(255, 138, 138, 0.55);
    cursor: help;
  }}
  .modal-content {{
    flex: 1;
    overflow: auto;
    margin: 0;
    padding: 22px;
    color: #eef4ff;
    font-family: var(--mono);
    font-size: 0.88rem;
    line-height: 1.7;
    white-space: pre-wrap;
    word-break: break-word;
  }}
  @media (max-width: 1200px) {{
    .shell {{ grid-template-columns: 300px minmax(0,1fr); }}
    .panel-right {{ display: none; }}
  }}
  @media (max-width: 860px) {{
    .shell {{
      grid-template-columns: 1fr;
      grid-template-rows: auto minmax(320px, 50vh) auto;
      height: auto;
      min-height: 100vh;
    }}
    .stage {{ min-height: 52vh; }}
    .panel-right {{ display: block; }}
  }}
</style>
</head>
<body>
<div class="shell">
  <aside class="panel panel-left">
    <button type="button" class="panel-collapse-btn" id="collapse-left" title="Collapse left panel" aria-label="Collapse left panel">&lsaquo;</button>
    <div class="panel-inner">
      <p class="eyebrow">Brain MRI</p>
      <h1>{brain_name}</h1>
      <div class="stats">
        <div class="stat">
          <div class="stat-label">Neurons</div>
          <div class="stat-value" id="stat-nodes">{sum(1 for n in graph["nodes"] if n["type"] == "neuron")}</div>
        </div>
        <div class="stat">
          <div class="stat-label">Links</div>
          <div class="stat-value" id="stat-edges">{len(graph["edges"])}</div>
        </div>
      </div>
      <div class="search-wrap">
        <label for="search-input">Search the brain</label>
        <div class="search-row">
          <input id="search-input" type="search" placeholder="Name, path, lobe, tag, or yaml" autocomplete="off">
          <button class="button" id="reset-view" type="button">Reset</button>
        </div>
      </div>
      <div class="section-title">Lobes</div>
      <div class="lobes-list" id="lobes-list"></div>
      <div class="section-title">Results</div>
      <div id="result-count" class="subhead"></div>
      <div class="results" id="search-results"></div>
    </div>
  </aside>

  <main class="stage">
    <div class="stage-hud">
      <div class="stage-pill">Drag, pan, scroll to zoom, <strong>/</strong> to search</div>
      <div class="stage-pill" id="stage-focus"></div>
    </div>
    <canvas id="mri-canvas"></canvas>
  </main>

  <aside class="panel panel-right">
    <button type="button" class="panel-collapse-btn" id="collapse-right" title="Collapse right panel" aria-label="Collapse right panel">&rsaquo;</button>
    <div class="panel-inner">
      <div style="display:flex;align-items:center;justify-content:space-between;padding-right:32px">
        <div><p class="eyebrow" style="margin:0">Inspector</p></div>
        <div style="display:flex;gap:6px">
          <button type="button" class="expand-btn" id="nav-back" title="Back">&larr;</button>
          <button type="button" class="expand-btn" id="nav-forward" title="Forward">&rarr;</button>
        </div>
      </div>
      <h2>Details</h2>
      <div class="details-empty" id="details-empty">
        Click a neuron to see its content and connections.
      </div>
      <div id="details-panel"></div>
    </div>
  </aside>
</div>
<button type="button" class="panel-expand-btn panel-expand-left" id="expand-left" title="Show left panel" aria-label="Show left panel">&rsaquo;</button>
<button type="button" class="panel-expand-btn panel-expand-right" id="expand-right" title="Show right panel" aria-label="Show right panel">&lsaquo;</button>
<div id="content-modal" class="modal-overlay" style="display:none">
  <div class="modal-box">
    <div class="modal-header">
      <div style="display:flex;align-items:center;gap:8px">
        <button type="button" class="expand-btn" id="modal-back" title="Back">&larr;</button>
        <button type="button" class="expand-btn" id="modal-forward" title="Forward">&rarr;</button>
        <div class="modal-title" id="modal-title"></div>
      </div>
      <button type="button" class="modal-close" id="modal-close">&times;</button>
    </div>
    <nav class="modal-tree" id="modal-tree" aria-label="Brain files"></nav>
    <div class="modal-main">
      <div class="modal-nav" id="modal-nav"></div>
      <pre class="modal-content" id="modal-content"></pre>
    </div>
  </div>
</div>
<script>
const graph = {graph_json};
const EDGE_COLORS = {{
  parent: 'rgba(248, 199, 109, 0.42)',
  related: 'rgba(123, 247, 255, 0.44)',
  inline: 'rgba(255, 139, 216, 0.34)',
}};
const canvas = document.getElementById('mri-canvas');
const ctx = canvas.getContext('2d');
const searchInput = document.getElementById('search-input');
const resultsEl = document.getElementById('search-results');
const resultCountEl = document.getElementById('result-count');
const detailsPanel = document.getElementById('details-panel');
const detailsEmpty = document.getElementById('details-empty');
const stageFocus = document.getElementById('stage-focus');
const lobesListEl = document.getElementById('lobes-list');
const neighbors = new Map();
for (const node of graph.nodes) neighbors.set(node.id, new Set());
for (const edge of graph.edges) {{
  neighbors.get(edge.source)?.add(edge.target);
  neighbors.get(edge.target)?.add(edge.source);
}}

let W = 0;
let H = 0;
const camera = {{ x: 0, y: 0, scale: 1 }};
let pointer = {{ x: 0, y: 0 }};
let selectedId = null;
let hoveredId = null;
let draggingNodeId = null;
let isPanning = false;
let dragMoved = false;
let dragOffset = {{ x: 0, y: 0 }};
let lastPointer = {{ x: 0, y: 0 }};
// Multi-select visibility: each lobe and sub-lobe can be independently
// hidden from the canvas. Click a lobe / sub-lobe card to toggle it.
// Default (empty sets) means "show everything".
const hiddenLobes = new Set();
const hiddenSublobes = new Set();
const expandedLobes = new Set();
const expandedSublobes = new Set();

// --- Color system: two-tier palette ---
const lobePalette = ['#7bf7ff','#ff8bd8','#f8c76d','#7df7b4','#9ea9ff','#ffa06f','#b8f0c1','#f2a8ff'];
const uniqueLobes = [...new Set(graph.nodes.map(n => n.lobe))];
const lobeAnchors = new Map();

function hexToRgb(hex) {{
  const r = parseInt(hex.slice(1,3), 16);
  const g = parseInt(hex.slice(3,5), 16);
  const b = parseInt(hex.slice(5,7), 16);
  return [r, g, b];
}}

function lobeColor(lobe) {{
  const idx = uniqueLobes.indexOf(lobe);
  return lobePalette[(idx >= 0 ? idx : 0) % lobePalette.length];
}}

function desaturate(hex, amount) {{
  const [r, g, b] = hexToRgb(hex);
  const gray = (r + g + b) / 3;
  const nr = Math.round(r + (gray - r) * amount);
  const ng = Math.round(g + (gray - g) * amount);
  const nb = Math.round(b + (gray - b) * amount);
  return `rgb(${{nr}},${{ng}},${{nb}})`;
}}

function rgbaFromHex(hex, alpha) {{
  const [r, g, b] = hexToRgb(hex);
  return `rgba(${{r}},${{g}},${{b}},${{alpha}})`;
}}

// --- Tree reconstruction ---
function buildTree() {{
  const parentEdges = graph.edges.filter(e => e.type === 'parent');
  const depthMap = new Map();
  const treeParent = new Map();
  // brain.md is depth 0
  for (const n of graph.nodes) {{
    if (n.type === 'brain') depthMap.set(n.id, 0);
  }}
  // BFS from brain to assign depths
  let frontier = [...depthMap.keys()];
  while (frontier.length) {{
    const next = [];
    for (const pid of frontier) {{
      for (const e of parentEdges) {{
        if (e.target === pid && !depthMap.has(e.source)) {{
          depthMap.set(e.source, depthMap.get(pid) + 1);
          treeParent.set(e.source, pid);
          next.push(e.source);
        }}
      }}
    }}
    frontier = next;
  }}
  // Assign remaining (unlinked) nodes
  for (const n of graph.nodes) {{
    if (!depthMap.has(n.id)) {{
      depthMap.set(n.id, n.type === 'map' ? 1 : 2);
    }}
  }}
  return {{ depthMap, treeParent }};
}}

const {{ depthMap, treeParent }} = buildTree();

function colorForNode(node) {{
  if (node.type === 'brain') return '#ffffff';
  if (node.type === 'glossary') return '#ffc6f4';
  if (node.type === 'index') return '#ffd28e';
  if (node.type === 'map') return lobeColor(node.lobe);
  // Yaml neurons get a distinct periwinkle so they read as "structured spec"
  // at a glance — independent of lobe color.
  if (node.file_type === 'yaml') return '#9ea9ff';
  // markdown neuron: desaturated lobe color
  return desaturate(lobeColor(node.lobe), 0.3);
}}

function nodeRadius(node) {{
  if (node.type === 'brain') return 22;
  if (node.type === 'glossary') return 13;
  if (node.type === 'index') return 11;
  if (node.type === 'map') {{
    const depth = depthMap.get(node.id) || 1;
    return depth <= 1 ? 30 : 22; // visual radius for hit-testing
  }}
  return Math.max(6, 5 + Math.min(node.degree || 0, 6) * 0.6);
}}

function resize() {{
  const rect = canvas.parentElement.getBoundingClientRect();
  W = canvas.width = Math.floor(rect.width * devicePixelRatio);
  H = canvas.height = Math.floor(rect.height * devicePixelRatio);
  canvas.style.width = `${{rect.width}}px`;
  canvas.style.height = `${{rect.height}}px`;
  ctx.setTransform(devicePixelRatio, 0, 0, devicePixelRatio, 0, 0);
  buildAnchors(rect.width, rect.height);
}}

function buildAnchors(width, height) {{
  const cx = width / 2;
  const cy = height / 2;
  // Elliptical anchor ring so lobes always sit inside the viewport, no
  // matter the aspect ratio. With a landscape stage the old min-based
  // radius pushed top/bottom lobes off-canvas. 0.36 leaves ~14% margin
  // on each side for orbit + hull spread.
  const rx = width * 0.36;
  const ry = height * 0.36;
  const nonRootLobes = uniqueLobes.filter(l => l !== 'root');
  lobeAnchors.set('root', {{ x: cx, y: cy * 0.86 }});
  nonRootLobes.forEach((lobe, i) => {{
    const angle = -Math.PI / 2 + (i / Math.max(1, nonRootLobes.length)) * Math.PI * 2;
    lobeAnchors.set(lobe, {{ x: cx + Math.cos(angle) * rx, y: cy + Math.sin(angle) * ry }});
  }});
}}

function initializeNodes() {{
  const dim = canvas.parentElement.getBoundingClientRect();
  const lobeCounters = new Map();
  return graph.nodes.map((node, index) => {{
    const anchor = lobeAnchors.get(node.lobe) || lobeAnchors.get('root') || {{ x: dim.width / 2, y: dim.height / 2 }};
    let targetX, targetY;
    const depth = depthMap.get(node.id) || 0;
    if (node.type === 'brain') {{
      targetX = dim.width / 2;
      targetY = dim.height / 2;
    }} else if (node.type === 'glossary') {{
      targetX = dim.width / 2;
      targetY = dim.height * 0.12;
    }} else if (node.type === 'index') {{
      targetX = dim.width * 0.14;
      targetY = dim.height * 0.14;
    }} else if (node.type === 'map') {{
      if (depth <= 1) {{
        targetX = anchor.x;
        targetY = anchor.y;
      }} else {{
        // Sub-lobe: offset from lobe anchor
        const count = lobeCounters.get('sublobe_' + node.lobe) || 0;
        lobeCounters.set('sublobe_' + node.lobe, count + 1);
        const subAngle = -Math.PI / 2 + (count / 3) * Math.PI * 2;
        targetX = anchor.x + Math.cos(subAngle) * 65;
        targetY = anchor.y + Math.sin(subAngle) * 65;
      }}
    }} else {{
      const count = lobeCounters.get(node.lobe) || 0;
      lobeCounters.set(node.lobe, count + 1);
      const orbitRadius = 70 + count * 22;
      const angle = count * 0.85 + index * 0.13;
      targetX = anchor.x + Math.cos(angle) * orbitRadius;
      targetY = anchor.y + Math.sin(angle) * orbitRadius;
    }}
    const searchText = [
      node.title, node.path, node.file_name, node.lobe,
      node.type, node.file_type || '', ...(node.tags || []),
      node.excerpt || '', node.content_preview || '',
    ].join(' ').toLowerCase();
    return {{
      ...node,
      searchText,
      color: colorForNode(node),
      radius: nodeRadius(node),
      depth,
      x: targetX + (Math.random() - 0.5) * 6,
      y: targetY + (Math.random() - 0.5) * 6,
      vx: 0,
      vy: 0,
      targetX,
      targetY,
    }};
  }});
}}

let nodes = [];
let filteredNodes = [];

function visibleNode(node) {{
  if (node.type === 'brain') return false;
  // Hide all map nodes -- hull labels show lobe/project names
  if (node.type === 'map') return false;
  // Multi-select visibility: hidden lobes and sub-lobes are excluded.
  if (hiddenLobes.has(node.lobe)) return false;
  if (node.sublobe && node.sublobe !== node.lobe && hiddenSublobes.has(node.sublobe)) return false;
  const query = searchInput.value.trim().toLowerCase();
  if (!query) return true;
  return node.searchText.includes(query);
}}

function refreshVisibility() {{
  filteredNodes = nodes.filter(visibleNode);
  const query = searchInput.value.trim();
  const total = nodes.filter(n => n.type === 'neuron' || n.type === 'glossary' || n.type === 'index').length;
  const anyHidden = hiddenLobes.size > 0 || hiddenSublobes.size > 0;
  if (query) {{
    resultCountEl.textContent = `Found ${{filteredNodes.length}} result${{filteredNodes.length === 1 ? '' : 's'}}.`;
  }} else if (anyHidden) {{
    resultCountEl.textContent = `Showing ${{filteredNodes.length}} of ${{total}} neurons.`;
  }} else {{
    resultCountEl.textContent = `Showing all ${{filteredNodes.length}} neurons.`;
  }}
  renderResults();
}}

function renderResults() {{
  resultsEl.innerHTML = '';
  const limited = filteredNodes.slice(0, 14);
  if (!limited.length) {{
    const empty = document.createElement('div');
    empty.className = 'details-empty';
    empty.textContent = 'No nodes match the current search. Try a path fragment, a lobe name, or a tag.';
    resultsEl.appendChild(empty);
    return;
  }}
  for (const node of limited) {{
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'result-card';
    button.innerHTML = `
      <div class="result-title">${{escapeHtml(node.title)}}</div>
      <div class="result-meta">${{node.type === 'map' ? 'lobe' : escapeHtml(node.type)}} • ${{escapeHtml(node.sublobe || node.lobe)}}</div>
      <div class="result-path">${{escapeHtml(node.path)}}</div>
    `;
    button.addEventListener('click', () => selectNode(node.id, true));
    resultsEl.appendChild(button);
  }}
}}

function renderLobes() {{
  lobesListEl.innerHTML = '';
  // Aggregate top-level lobes and their sub-lobes from the source graph.
  // Each lobe collects: map node id, title, neuron count (incl. sub-lobes),
  // color, and a Map of sub-lobes keyed by sublobe path. Sub-lobes track
  // their own map node id, leaf title, and neuron count. The synthetic
  // 'root' bucket (loose files in brain root) is skipped.
  const lobeInfo = new Map();
  function ensureLobe(lobeKey) {{
    if (!lobeInfo.has(lobeKey)) {{
      lobeInfo.set(lobeKey, {{
        mapNodeId: null,
        title: lobeKey,
        neuronCount: 0,
        color: lobeColor(lobeKey),
        sublobes: new Map(),
      }});
    }}
    return lobeInfo.get(lobeKey);
  }}
  function ensureSublobe(info, sublobeKey) {{
    if (!info.sublobes.has(sublobeKey)) {{
      info.sublobes.set(sublobeKey, {{
        key: sublobeKey,
        mapNodeId: null,
        // Default to the leaf segment (e.g. "projects/foo" -> "foo") until
        // a real map.md title is found.
        title: sublobeKey.split('/').pop() || sublobeKey,
        neuronCount: 0,
      }});
    }}
    return info.sublobes.get(sublobeKey);
  }}
  for (const node of nodes) {{
    if (!node.lobe || node.lobe === 'root') continue;
    const info = ensureLobe(node.lobe);
    const isSublobe = node.sublobe && node.sublobe !== node.lobe;
    if (node.type === 'map' && !isSublobe) {{
      info.mapNodeId = node.id;
      info.title = node.title || node.lobe;
    }} else if (node.type === 'map' && isSublobe) {{
      const sub = ensureSublobe(info, node.sublobe);
      sub.mapNodeId = node.id;
      sub.title = node.title || sub.title;
    }} else if (node.type === 'neuron') {{
      info.neuronCount += 1;
      if (isSublobe) {{
        ensureSublobe(info, node.sublobe).neuronCount += 1;
      }}
    }}
  }}
  if (!lobeInfo.size) {{
    const empty = document.createElement('div');
    empty.className = 'details-empty';
    empty.textContent = 'No lobes in this brain yet. Ask your agent to create one, then run kluris dream.';
    lobesListEl.appendChild(empty);
    return;
  }}
  const sortedLobes = [...lobeInfo.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  for (const [lobeKey, info] of sortedLobes) {{
    const group = document.createElement('div');
    group.className = 'lobe-group';

    const hasSublobes = info.sublobes.size > 0;
    const isExpanded = expandedLobes.has(lobeKey);

    // The card stays flush-left like every other result-card; the caret
    // (when present) floats over the card's right edge so cards never get
    // shifted by an alignment spacer.
    const wrap = document.createElement('div');
    wrap.className = hasSublobes ? 'lobe-card-wrap has-caret' : 'lobe-card-wrap';

    const isLobeHidden = hiddenLobes.has(lobeKey);

    const card = document.createElement('button');
    card.type = 'button';
    card.className = isLobeHidden ? 'lobe-card dimmed' : 'lobe-card';
    card.title = isLobeHidden ? 'Click to show this lobe' : 'Click to hide this lobe';
    const swatchShadow = `${{info.color}}55`;
    const countLabel = `${{info.neuronCount}} neuron${{info.neuronCount === 1 ? '' : 's'}}`;
    const subCountLabel = hasSublobes
      ? ` • ${{info.sublobes.size}} sublobe${{info.sublobes.size === 1 ? '' : 's'}}`
      : '';
    card.innerHTML = `
      <span class="lobe-swatch" style="background:${{info.color}};box-shadow:0 0 14px ${{swatchShadow}}"></span>
      <span class="lobe-body">
        <span class="lobe-name">${{escapeHtml(String(info.title).toUpperCase())}}</span>
        <span class="lobe-meta">${{countLabel}}${{subCountLabel}}</span>
      </span>
    `;
    card.addEventListener('click', () => {{
      // Toggle this lobe's visibility. Multi-select: other hidden lobes
      // stay hidden, other visible lobes stay visible.
      if (hiddenLobes.has(lobeKey)) hiddenLobes.delete(lobeKey);
      else hiddenLobes.add(lobeKey);
      renderLobes();
      refreshVisibility();
      fitToFilteredNodes();
    }});
    wrap.appendChild(card);

    if (hasSublobes) {{
      const caret = document.createElement('button');
      caret.type = 'button';
      caret.className = 'lobe-caret';
      caret.textContent = isExpanded ? '▾' : '▸';
      caret.setAttribute('aria-label', isExpanded ? 'Collapse sublobes' : 'Expand sublobes');
      caret.addEventListener('click', event => {{
        event.stopPropagation();
        if (expandedLobes.has(lobeKey)) expandedLobes.delete(lobeKey);
        else expandedLobes.add(lobeKey);
        renderLobes();
      }});
      wrap.appendChild(caret);
    }}

    group.appendChild(wrap);

    if (hasSublobes && isExpanded) {{
      const sortedSubs = [...info.sublobes.values()].sort((a, b) => a.key.localeCompare(b.key));
      // Build tree: separate root sublobes (depth 2, e.g. "projects/foo")
      // from inner sublobes (depth 3+, e.g. "projects/foo/bar").
      function renderSubTree(container, parentKey, allSubs, color) {{
        const children = allSubs.filter(s => {{
          if (!s.key.startsWith(parentKey + '/')) return false;
          // Only direct children: no further '/' after parentKey + '/'
          const rest = s.key.slice(parentKey.length + 1);
          return !rest.includes('/');
        }});
        if (!children.length) return;
        const childList = document.createElement('div');
        childList.className = 'sublobes-list';
        for (const child of children) {{
          const isChildHidden = hiddenSublobes.has(child.key);
          const hasInner = allSubs.some(s => s.key.startsWith(child.key + '/'));
          const isChildExpanded = expandedSublobes.has(child.key);
          const childWrap = document.createElement('div');
          childWrap.className = 'sublobe-group';
          const childCard = document.createElement('button');
          childCard.type = 'button';
          childCard.className = isChildHidden ? 'sublobe-card dimmed' : 'sublobe-card';
          childCard.title = isChildHidden ? 'Click to show this sublobe' : 'Click to hide this sublobe';
          const childCount = `${{child.neuronCount}} neuron${{child.neuronCount === 1 ? '' : 's'}}`;
          childCard.innerHTML = `
            <span class="sublobe-tick" style="background:${{color}}"></span>
            <span class="lobe-body">
              <span class="sublobe-name">${{escapeHtml(String(child.title))}}</span>
              <span class="lobe-meta">${{childCount}}</span>
            </span>
          `;
          childCard.addEventListener('click', event => {{
            event.stopPropagation();
            if (hiddenSublobes.has(child.key)) {{
              hiddenSublobes.delete(child.key);
              for (const s of allSubs) {{
                if (s.key.startsWith(child.key + '/')) hiddenSublobes.delete(s.key);
              }}
            }} else {{
              hiddenSublobes.add(child.key);
              for (const s of allSubs) {{
                if (s.key.startsWith(child.key + '/')) hiddenSublobes.add(s.key);
              }}
            }}
            renderLobes();
            refreshVisibility();
            fitToFilteredNodes();
          }});
          if (hasInner) {{
            const innerWrap = document.createElement('div');
            innerWrap.className = 'sublobe-card-wrap has-caret';
            innerWrap.appendChild(childCard);
            const caret = document.createElement('button');
            caret.type = 'button';
            caret.className = 'lobe-caret';
            caret.textContent = isChildExpanded ? '▾' : '▸';
            caret.setAttribute('aria-label', isChildExpanded ? 'Collapse inner lobes' : 'Expand inner lobes');
            caret.addEventListener('click', event => {{
              event.stopPropagation();
              if (expandedSublobes.has(child.key)) expandedSublobes.delete(child.key);
              else expandedSublobes.add(child.key);
              renderLobes();
            }});
            innerWrap.appendChild(caret);
            childWrap.appendChild(innerWrap);
            if (isChildExpanded) {{
              renderSubTree(childWrap, child.key, allSubs, color);
            }}
          }} else {{
            childWrap.appendChild(childCard);
          }}
          childList.appendChild(childWrap);
        }}
        container.appendChild(childList);
      }}
      // Render root-level sublobes (direct children of the lobe)
      const rootSubs = sortedSubs.filter(s => {{
        const rest = s.key.slice(lobeKey.length + 1);
        return !rest.includes('/');
      }});
      const subList = document.createElement('div');
      subList.className = 'sublobes-list';
      for (const sub of rootSubs) {{
        const isSubHidden = hiddenSublobes.has(sub.key);
        const hasInner = sortedSubs.some(s => s.key.startsWith(sub.key + '/'));
        const isSubExpanded = expandedSublobes.has(sub.key);
        const subWrap = document.createElement('div');
        subWrap.className = 'sublobe-group';
        const subCard = document.createElement('button');
        subCard.type = 'button';
        subCard.className = isSubHidden ? 'sublobe-card dimmed' : 'sublobe-card';
        subCard.title = isSubHidden ? 'Click to show this sublobe' : 'Click to hide this sublobe';
        const subCount = `${{sub.neuronCount}} neuron${{sub.neuronCount === 1 ? '' : 's'}}`;
        subCard.innerHTML = `
          <span class="sublobe-tick" style="background:${{info.color}}"></span>
          <span class="lobe-body">
            <span class="sublobe-name">${{escapeHtml(String(sub.title))}}</span>
            <span class="lobe-meta">${{subCount}}</span>
          </span>
        `;
        subCard.addEventListener('click', event => {{
          event.stopPropagation();
          if (hiddenSublobes.has(sub.key)) {{
            hiddenSublobes.delete(sub.key);
            for (const s of sortedSubs) {{
              if (s.key.startsWith(sub.key + '/')) hiddenSublobes.delete(s.key);
            }}
          }} else {{
            hiddenSublobes.add(sub.key);
            for (const s of sortedSubs) {{
              if (s.key.startsWith(sub.key + '/')) hiddenSublobes.add(s.key);
            }}
          }}
          renderLobes();
          refreshVisibility();
          fitToFilteredNodes();
        }});
        if (hasInner) {{
          const innerWrap = document.createElement('div');
          innerWrap.className = 'sublobe-card-wrap has-caret';
          innerWrap.appendChild(subCard);
          const caret = document.createElement('button');
          caret.type = 'button';
          caret.className = 'lobe-caret';
          caret.textContent = isSubExpanded ? '▾' : '▸';
          caret.setAttribute('aria-label', isSubExpanded ? 'Collapse inner lobes' : 'Expand inner lobes');
          caret.addEventListener('click', event => {{
            event.stopPropagation();
            if (expandedSublobes.has(sub.key)) expandedSublobes.delete(sub.key);
            else expandedSublobes.add(sub.key);
            renderLobes();
          }});
          innerWrap.appendChild(caret);
          subWrap.appendChild(innerWrap);
          if (isSubExpanded) {{
            renderSubTree(subWrap, sub.key, sortedSubs, info.color);
          }}
        }} else {{
          subWrap.appendChild(subCard);
        }}
        subList.appendChild(subWrap);
      }}
      group.appendChild(subList);
    }}

    lobesListEl.appendChild(group);
  }}
}}

function escapeHtml(value) {{
  return String(value || '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}}

function updateDetails() {{
  const node = nodes.find(item => item.id === selectedId);
  if (!node) {{
    detailsPanel.innerHTML = '';
    detailsEmpty.style.display = 'block';
    stageFocus.textContent = '';
    return;
  }}

  detailsEmpty.style.display = 'none';
  stageFocus.textContent = `${{node.title}} • ${{node.path}}`;
  const connected = [...(neighbors.get(node.id) || [])]
    .map(id => nodes.find(item => item.id === id))
    .filter(Boolean)
    .sort((a, b) => a.title.localeCompare(b.title));
  const tags = [...new Set(node.tags || [])].map(tag => `<span class="tag">${{escapeHtml(tag)}}</span>`).join('');
  const contentPreview = escapeHtml(node.content_preview || 'No content preview available for this node.');
  const previewNote = node.content_preview_truncated
    ? '<div class="content-preview-note">Preview truncated for readability. Click expand to read the full document.</div>'
    : '';
  const CONN_LIMIT = 3;
  // Meta line dedupe: drop the sublobe/lobe chip when the connected neuron
  // lives in the same section as the currently-selected node — otherwise
  // every card in a same-project cluster repeats "neuron • btb" and the
  // useful info (the title) competes with boilerplate.
  const currentSection = node.sublobe || node.lobe || '';
  const connectionCards = connected.map(target => {{
    const targetSection = target.sublobe || target.lobe || '';
    const typeLabel = target.type === 'map' ? 'lobe' : target.type;
    const sectionChip = targetSection && targetSection !== currentSection
      ? ` • ${{escapeHtml(targetSection)}}`
      : '';
    return `
        <button type="button" class="connection-card" data-node-id="${{target.id}}">
          <div class="result-title">${{escapeHtml(target.title)}}</div>
          <div class="result-meta">${{escapeHtml(typeLabel)}}${{sectionChip}}</div>
          <div class="result-path">${{escapeHtml(target.path)}}</div>
        </button>
      `;
  }});
  const connections = connected.length
    ? connectionCards.slice(0, CONN_LIMIT).join('')
      + (connected.length > CONN_LIMIT
        ? `<button type="button" class="conn-show-all" id="conn-show-all">Show all ${{connected.length}} connections</button>`
        : '')
    : `<div class="details-empty">No connected nodes found for this selection.</div>`;
  const connectionsAll = connectionCards.join('');

  // Build breadcrumbs from path
  const pathParts = node.path.split('/');
  const crumbs = pathParts.map((part, i) => {{
    const partPath = pathParts.slice(0, i + 1).join('/');
    // Find a node matching this path (map.md for directories, or the file itself)
    const isLast = i === pathParts.length - 1;
    const targetPath = isLast ? partPath : partPath + '/map.md';
    const target = nodes.find(n => n.path === targetPath);
    const label = part.replace(/\.(md|yml|yaml)$/, '');
    if (target && target.id !== node.id) {{
      return `<button type="button" class="breadcrumb-link" data-node-id="${{target.id}}">${{escapeHtml(label)}}</button>`;
    }}
    return `<span class="breadcrumb-current">${{escapeHtml(label)}}</span>`;
  }}).join('<span class="breadcrumb-sep">/</span>');

  const detailsAuthoredSub = node.authored_title
    ? `<div class="details-subtitle" title="From H1 in the file">${{escapeHtml(node.authored_title)}}</div>`
    : '';
  detailsPanel.innerHTML = `
    <div class="details-card">
      <div class="details-title">${{escapeHtml(node.title)}}</div>
      ${{detailsAuthoredSub}}
      <div class="breadcrumbs">${{crumbs}}</div>
      <div class="meta-grid">
        <div class="meta-card"><span class="label">Type</span><span class="value">${{node.type === 'map' ? 'lobe' : node.type === 'neuron' ? 'neuron' : escapeHtml(node.type)}}</span></div>
        <div class="meta-card"><span class="label">Section</span><span class="value">${{escapeHtml(node.sublobe || node.lobe)}}</span></div>
        <div class="meta-card"><span class="label">Updated</span><span class="value">${{escapeHtml(node.updated || '—')}}</span></div>
        <div class="meta-card"><span class="label">Created</span><span class="value">${{escapeHtml(node.created || '—')}}</span></div>
        <div class="meta-card"><span class="label">Connections</span><span class="value">${{connected.length}}</span></div>
      </div>
      ${{tags ? `<div class="tag-row">${{tags}}</div>` : ''}}
      <div class="section-title">Excerpt</div>
      <div class="details-copy">${{escapeHtml(node.excerpt || 'No excerpt available for this node.')}}</div>
      <div class="section-title">Content preview <button type="button" class="expand-btn" id="expand-preview">expand</button></div>
      <pre class="content-preview">${{contentPreview}}</pre>
      ${{previewNote}}
      <div class="section-title">Connected nodes</div>
      <div class="results">${{connections}}</div>
    </div>
  `;
  for (const button of detailsPanel.querySelectorAll('[data-node-id]')) {{
    button.addEventListener('click', () => selectNode(Number(button.dataset.nodeId), true));
  }}
  for (const crumb of detailsPanel.querySelectorAll('.breadcrumb-link')) {{
    crumb.addEventListener('click', () => selectNode(Number(crumb.dataset.nodeId), true));
  }}
  const expandBtn = document.getElementById('expand-preview');
  if (expandBtn) {{
    expandBtn.addEventListener('click', () => openModal(node));
  }}
  const showAllBtn = document.getElementById('conn-show-all');
  if (showAllBtn) {{
    showAllBtn.addEventListener('click', () => {{
      const container = showAllBtn.parentElement;
      container.innerHTML = connectionsAll;
      for (const btn of container.querySelectorAll('[data-node-id]')) {{
        btn.addEventListener('click', () => selectNode(Number(btn.dataset.nodeId), true));
      }}
    }});
  }}
}}

// --- File browser tree (left sidebar of the expand modal) ---
//
// Built once from the set of neuron nodes, then reused on every openModal
// call. Folders toggle via the caret. Clicking a file opens that node.
const treeRoot = {{ folders: new Map(), files: [] }};
let treeBuilt = false;
let collapsedTreePaths = new Set();

function buildFileTree() {{
  treeRoot.folders = new Map();
  treeRoot.files = [];
  // Include neurons + the glossary so the modal's left file tree mirrors
  // what's actually on disk — glossary.md is a real file users want to
  // click to open. brain.md and index.md stay hidden (they're
  // auto-generated and don't carry hand-written content worth navigating
  // to from this tree).
  const entries = nodes
    .filter(n => (n.type === 'neuron' || n.type === 'glossary') && n.path)
    .sort((a, b) => a.path.localeCompare(b.path));
  for (const node of entries) {{
    const parts = node.path.split('/');
    let cursor = treeRoot;
    for (let i = 0; i < parts.length - 1; i++) {{
      const part = parts[i];
      if (!cursor.folders.has(part)) {{
        cursor.folders.set(part, {{ folders: new Map(), files: [] }});
      }}
      cursor = cursor.folders.get(part);
    }}
    cursor.files.push(node);
  }}
  treeBuilt = true;
}}

function renderTreeFolder(name, folder, pathSoFar) {{
  const full = pathSoFar ? `${{pathSoFar}}/${{name}}` : name;
  const collapsed = collapsedTreePaths.has(full);
  const caret = collapsed ? '▸' : '▾';
  const subFolderHtml = [...folder.folders.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([n, f]) => renderTreeFolder(n, f, full))
    .join('');
  const fileHtml = folder.files
    .sort((a, b) => a.title.localeCompare(b.title))
    .map(node => `<div class="modal-tree-file" data-tree-node="${{node.id}}" title="${{escapeHtml(node.path)}}"><span class="icon">📄</span>${{escapeHtml(node.title)}}</div>`)
    .join('');
  return (
    `<div class="modal-tree-folder${{collapsed ? ' collapsed' : ''}}" data-tree-folder="${{escapeHtml(full)}}">` +
      `<div class="modal-tree-folder-label"><span class="caret">${{caret}}</span><span class="icon">📁</span>${{escapeHtml(name)}}</div>` +
      `<div class="modal-tree-children">${{subFolderHtml}}${{fileHtml}}</div>` +
    `</div>`
  );
}}

function renderFileTree(activeNodeId) {{
  if (!treeBuilt) buildFileTree();
  const treeEl = document.getElementById('modal-tree');
  if (!treeEl) return;
  const topFolders = [...treeRoot.folders.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([n, f]) => renderTreeFolder(n, f, ''))
    .join('');
  const topFiles = treeRoot.files
    .sort((a, b) => a.title.localeCompare(b.title))
    .map(node => `<div class="modal-tree-file" data-tree-node="${{node.id}}" title="${{escapeHtml(node.path)}}"><span class="icon">📄</span>${{escapeHtml(node.title)}}</div>`)
    .join('');
  treeEl.innerHTML = topFolders + topFiles;

  // Highlight the active file and auto-expand its ancestor folders
  if (activeNodeId != null) {{
    for (const el of treeEl.querySelectorAll('.modal-tree-file')) {{
      if (Number(el.dataset.treeNode) === activeNodeId) el.classList.add('active');
    }}
  }}

  // Wire folder caret clicks
  for (const el of treeEl.querySelectorAll('.modal-tree-folder-label')) {{
    el.addEventListener('click', () => {{
      const folder = el.closest('.modal-tree-folder');
      const path = folder?.dataset.treeFolder;
      if (!path) return;
      if (collapsedTreePaths.has(path)) {{
        collapsedTreePaths.delete(path);
        folder.classList.remove('collapsed');
      }} else {{
        collapsedTreePaths.add(path);
        folder.classList.add('collapsed');
      }}
    }});
  }}

  // Wire file clicks to open that node in the modal
  for (const el of treeEl.querySelectorAll('.modal-tree-file')) {{
    el.addEventListener('click', () => {{
      const target = nodes.find(n => n.id === Number(el.dataset.treeNode));
      if (target) {{ selectNode(target.id, true); openModal(target); }}
    }});
  }}
}}

function openModal(node) {{
  const modal = document.getElementById('content-modal');
  const breadcrumb = node.path.split('/').map(p => p.replace(/\.(md|yml|yaml)$/, '')).join(' / ');
  const authoredSub = node.authored_title
    ? ` <span style="color:var(--muted);font-size:0.7em;font-weight:400;margin-left:10px" title="From H1 in the file">· ${{escapeHtml(node.authored_title)}}</span>`
    : '';
  document.getElementById('modal-title').innerHTML = `${{escapeHtml(node.title)}}${{authoredSub}} <span style="color:var(--muted);font-size:0.8em;font-weight:400;margin-left:8px">${{escapeHtml(breadcrumb)}}</span>`;
  renderFileTree(node.id);
  // Render content with clickable markdown links
  // Run regex on raw content BEFORE escaping, then escape text parts individually
  // Prefer the untruncated body so the modal shows the full document; fall back to the preview.
  const raw = node.content_full || node.content_preview || 'No content.';
  const nodePath = node.path.replace(/[^/]+$/, '');
  // Match [text](path.md|.yml|.yaml[#anchor][?query]) markdown links —
  // yaml neurons can be link targets from markdown body text too, and
  // glossary links routinely carry a trailing #anchor (strip it before
  // resolving so the target node is still found).
  const linkRe = /\[([^\]]+)\]\(([^)\s]+?\.(md|yml|yaml))(#[^)\s]*)?(\?[^)\s]*)?\)/g;
  let linkedContent = '';
  let lastIdx = 0;
  let m;
  while ((m = linkRe.exec(raw)) !== null) {{
    // Escape text before this match
    linkedContent += escapeHtml(raw.slice(lastIdx, m.index));
    const text = m[1] || '';
    const pathPart = m[2] || '';
    const anchor = m[4] || '';
    const href = pathPart + anchor + (m[5] || '');
    if (!pathPart || pathPart.startsWith('http')) {{
      linkedContent += escapeHtml(m[0]);
    }} else {{
      const parts = (nodePath + pathPart).split('/');
      const resolved = [];
      for (const p of parts) {{
        if (p === '..') resolved.pop();
        else if (p && p !== '.') resolved.push(p);
      }}
      const resolvedPath = resolved.join('/');
      const target = nodes.find(n => n.path === resolvedPath);
      if (target) {{
        linkedContent += `<button type="button" class="content-link" data-modal-nav="${{target.id}}" title="${{escapeHtml(target.path + anchor)}}">${{escapeHtml(text)}}</button>`;
      }} else {{
        linkedContent += `<span class="content-link-broken" title="broken link: ${{escapeHtml(href)}}">${{escapeHtml(text)}}</span>`;
      }}
    }}
    lastIdx = m.index + m[0].length;
  }}
  linkedContent += escapeHtml(raw.slice(lastIdx));
  document.getElementById('modal-content').innerHTML = linkedContent;
  for (const btn of document.getElementById('modal-content').querySelectorAll('[data-modal-nav]')) {{
    btn.addEventListener('click', () => {{
      const target = nodes.find(n => n.id === Number(btn.dataset.modalNav));
      if (target) {{ selectNode(target.id, true); openModal(target); }}
    }});
  }}
  // Build nav buttons for connected nodes — show first N, expand on demand
  const navEl = document.getElementById('modal-nav');
  const connected = [...(neighbors.get(node.id) || [])]
    .map(id => nodes.find(n => n.id === id))
    .filter(n => n && n.type === 'neuron')
    .sort((a, b) => a.title.localeCompare(b.title));
  const NAV_COLLAPSE = 6;
  // Disambiguator: only prefix a nav button with its parent folder when that
  // parent differs from the current node's parent. In a same-project cluster
  // every button would otherwise read "btb / X" and the repeated prefix
  // hides the actual neuron name.
  const currentParts = node.path.split('/');
  const currentParent = currentParts.length >= 2 ? currentParts[currentParts.length - 2] : '';
  const allNavHtml = connected.map(n => {{
    const parts = n.path.split('/');
    const parent = parts.length >= 2 ? parts[parts.length - 2] : '';
    const label = parent && parent !== currentParent ? `${{parent}} / ${{n.title}}` : n.title;
    return `<button type="button" class="modal-nav-btn" data-modal-nav="${{n.id}}" title="${{escapeHtml(n.path)}}">${{escapeHtml(label)}}</button>`;
  }});
  const hasOverflow = connected.length > NAV_COLLAPSE;
  function renderNav(expanded) {{
    const btns = expanded ? allNavHtml.join('') : allNavHtml.slice(0, NAV_COLLAPSE).join('');
    const toggle = hasOverflow
      ? `<button type="button" class="modal-nav-toggle" id="modal-nav-toggle">${{expanded ? 'show less' : '+' + (connected.length - NAV_COLLAPSE) + ' more'}}</button>`
      : '';
    navEl.innerHTML = btns + toggle;
    for (const btn of navEl.querySelectorAll('[data-modal-nav]')) {{
      btn.addEventListener('click', () => {{
        const target = nodes.find(n => n.id === Number(btn.dataset.modalNav));
        if (target) {{ selectNode(target.id, true); openModal(target); }}
      }});
    }}
    const tog = document.getElementById('modal-nav-toggle');
    if (tog) tog.addEventListener('click', () => renderNav(!expanded));
  }}
  renderNav(false);
  modal.style.display = 'flex';
}}

const navHistory = [];
let navIndex = -1;

function selectNode(id, recenter = false, fromNav = false) {{
  if (id !== selectedId && id != null && !fromNav) {{
    // Truncate forward history when navigating to a new node
    navHistory.splice(navIndex + 1);
    navHistory.push(id);
    navIndex = navHistory.length - 1;
  }}
  selectedId = id;
  updateDetails();
  if (recenter) focusOnNode(id);
}}

function navBack() {{
  if (navIndex > 0) {{
    navIndex--;
    selectNode(navHistory[navIndex], true, true);
  }}
}}

function navForward() {{
  if (navIndex < navHistory.length - 1) {{
    navIndex++;
    selectNode(navHistory[navIndex], true, true);
  }}
}}

let cameraAnim = null;
function animateCamera(tx, ty, ts, duration) {{
  const sx = camera.x, sy = camera.y, ss = camera.scale;
  const start = performance.now();
  if (cameraAnim) cancelAnimationFrame(cameraAnim);
  function step(now) {{
    const t = Math.min(1, (now - start) / duration);
    const ease = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    camera.x = sx + (tx - sx) * ease;
    camera.y = sy + (ty - sy) * ease;
    camera.scale = ss + (ts - ss) * ease;
    if (t < 1) cameraAnim = requestAnimationFrame(step);
    else cameraAnim = null;
  }}
  cameraAnim = requestAnimationFrame(step);
}}

function fitToFilteredNodes(instant = false) {{
  // Frame the camera around whatever is currently visible. Used after a
  // visibility toggle (and at startup with instant=true) so the user
  // actually sees the result.
  //
  // Key trick: we use each node's stable targetX/targetY (the anchor-based
  // layout coordinates) instead of live x/y. Physics has not settled at the
  // moment the click handler fires, so live positions are transient. Target
  // coords are deterministic and match where the nodes will drift toward,
  // so the fit is correct regardless of mid-physics state.
  if (!filteredNodes.length) return;
  const rect = canvas.parentElement.getBoundingClientRect();
  const padding = 140;
  const xs = filteredNodes.map(n => (n.targetX != null ? n.targetX : n.x));
  const ys = filteredNodes.map(n => (n.targetY != null ? n.targetY : n.y));
  const minX = Math.min(...xs) - padding;
  const maxX = Math.max(...xs) + padding;
  const minY = Math.min(...ys) - padding;
  const maxY = Math.max(...ys) + padding;
  const w = Math.max(1, maxX - minX);
  const h = Math.max(1, maxY - minY);
  const scale = Math.min(rect.width / w, rect.height / h) * 0.80;
  const clampedScale = Math.min(2.4, Math.max(0.42, scale));
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  const tx = rect.width / 2 - cx * clampedScale;
  const ty = rect.height / 2 - cy * clampedScale;
  if (instant) {{
    if (cameraAnim) {{ cancelAnimationFrame(cameraAnim); cameraAnim = null; }}
    camera.x = tx;
    camera.y = ty;
    camera.scale = clampedScale;
  }} else {{
    animateCamera(tx, ty, clampedScale, 320);
  }}
}}

function resetCamera() {{
  // Smoothly snap the camera back to the default unfiltered view.
  animateCamera(0, 0, 1, 320);
}}

function focusOnNode(id) {{
  const node = nodes.find(item => item.id === id);
  if (!node) return;
  const rect = canvas.parentElement.getBoundingClientRect();
  if (node.type === 'map') {{
    // Zoom to frame the lobe (or sub-lobe). Sub-lobe map nodes have
    // sublobe !== lobe (e.g. lobe="projects", sublobe="projects/foo");
    // those should zoom to just their sub-lobe members, not the whole lobe.
    const isSublobe = node.sublobe && node.sublobe !== node.lobe;
    const members = isSublobe
      ? filteredNodes.filter(n => n.sublobe === node.sublobe || n.sublobe.startsWith(node.sublobe + '/'))
      : filteredNodes.filter(n => n.lobe === node.lobe);
    if (members.length > 1) {{
      const minX = Math.min(...members.map(n => n.x)) - 60;
      const maxX = Math.max(...members.map(n => n.x)) + 60;
      const minY = Math.min(...members.map(n => n.y)) - 60;
      const maxY = Math.max(...members.map(n => n.y)) + 60;
      const lobeW = maxX - minX;
      const lobeH = maxY - minY;
      const scale = Math.min(rect.width / lobeW, rect.height / lobeH) * 0.85;
      const cx = (minX + maxX) / 2;
      const cy = (minY + maxY) / 2;
      animateCamera(rect.width / 2 - cx * scale, rect.height / 2 - cy * scale, Math.min(2.4, Math.max(0.5, scale)), 300);
      return;
    }}
  }}
  const tx = rect.width / 2 - node.x * camera.scale;
  const ty = rect.height / 2 - node.y * camera.scale;
  animateCamera(tx, ty, camera.scale, 300);
}}

function toWorld(clientX, clientY) {{
  const rect = canvas.getBoundingClientRect();
  return {{
    x: (clientX - rect.left - camera.x) / camera.scale,
    y: (clientY - rect.top - camera.y) / camera.scale,
  }};
}}

function hitTest(worldX, worldY) {{
  let candidate = null;
  let minDistance = Infinity;
  for (const node of filteredNodes) {{
    const distance = Math.hypot(node.x - worldX, node.y - worldY);
    if (distance <= node.radius + 8 && distance < minDistance) {{
      minDistance = distance;
      candidate = node;
    }}
  }}
  return candidate;
}}

function tick() {{
  const visibleIds = new Set(filteredNodes.map(n => n.id));
  // Repulsion with cross-lobe boost
  for (let i = 0; i < filteredNodes.length; i++) {{
    for (let j = i + 1; j < filteredNodes.length; j++) {{
      const a = filteredNodes[i];
      const b = filteredNodes[j];
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const distance = Math.max(24, Math.hypot(dx, dy));
      const crossLobe = a.lobe !== b.lobe ? 2.6 : 1.0;
      const force = (1200 * crossLobe) / (distance * distance);
      const ux = dx / distance;
      const uy = dy / distance;
      a.vx -= ux * force;
      a.vy -= uy * force;
      b.vx += ux * force;
      b.vy += uy * force;
    }}
  }}
  // Same-lobe cohesion. Track members on the centroid record so the
  // pairwise lobe-vs-lobe push below can reuse them without re-filtering.
  const lobeCentroids = new Map();
  for (const lobe of uniqueLobes) {{
    const members = filteredNodes.filter(n => n.lobe === lobe);
    if (!members.length) continue;
    const cx = members.reduce((s, n) => s + n.x, 0) / members.length;
    const cy = members.reduce((s, n) => s + n.y, 0) / members.length;
    lobeCentroids.set(lobe, {{ x: cx, y: cy, members }});
    for (const n of members) {{
      if (n.type !== 'brain') {{
        n.vx += (cx - n.x) * 0.002;
        n.vy += (cy - n.y) * 0.002;
      }}
    }}
  }}
  // Push different lobes apart at the centroid level so their hulls
  // never overlap. minDist controls the breathing room; if two lobe
  // centroids drift closer than that, every member of each lobe gets a
  // shove away from the other lobe's centroid.
  const lobeKeys = [...lobeCentroids.keys()];
  for (let i = 0; i < lobeKeys.length; i++) {{
    for (let j = i + 1; j < lobeKeys.length; j++) {{
      const a = lobeCentroids.get(lobeKeys[i]);
      const b = lobeCentroids.get(lobeKeys[j]);
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.max(60, Math.hypot(dx, dy));
      // Scale minDist with each lobe's spread so big lobes claim more space.
      const aSpread = Math.max(80, Math.sqrt(a.members.length) * 50);
      const bSpread = Math.max(80, Math.sqrt(b.members.length) * 50);
      const minDist = aSpread + bSpread + 220;
      if (dist < minDist) {{
        const push = (minDist - dist) * 0.030;
        const ux = dx / dist;
        const uy = dy / dist;
        for (const n of a.members) {{ n.vx -= ux * push; n.vy -= uy * push; }}
        for (const n of b.members) {{ n.vx += ux * push; n.vy += uy * push; }}
      }}
    }}
  }}
  // Sub-lobe cohesion + cross-sub-lobe repulsion
  // Merge inner sublobes (depth 3+) into their 2nd-level parent for physics
  function physicsSublobeKey(sl) {{
    const parts = sl.split('/');
    return parts.length > 2 ? parts.slice(0, 2).join('/') : sl;
  }}
  const sublobeGroups = new Map();
  for (const n of filteredNodes) {{
    const key = physicsSublobeKey(n.sublobe);
    if (!sublobeGroups.has(key)) sublobeGroups.set(key, []);
    sublobeGroups.get(key).push(n);
  }}
  const sublobeCentroids = new Map();
  for (const [sl, members] of sublobeGroups) {{
    if (members.length < 2 || sl === members[0].lobe) continue;
    const cx = members.reduce((s, n) => s + n.x, 0) / members.length;
    const cy = members.reduce((s, n) => s + n.y, 0) / members.length;
    sublobeCentroids.set(sl, {{ x: cx, y: cy, lobe: members[0].lobe }});
    // Pull members toward their sub-lobe centroid
    for (const n of members) {{
      n.vx += (cx - n.x) * 0.004;
      n.vy += (cy - n.y) * 0.004;
    }}
  }}
  // Push different sub-lobes within the same lobe apart
  const slKeys = [...sublobeCentroids.keys()];
  for (let i = 0; i < slKeys.length; i++) {{
    for (let j = i + 1; j < slKeys.length; j++) {{
      const a = sublobeCentroids.get(slKeys[i]);
      const b = sublobeCentroids.get(slKeys[j]);
      if (a.lobe !== b.lobe) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.max(40, Math.hypot(dx, dy));
      const minDist = 400;
      if (dist < minDist) {{
        const push = (minDist - dist) * 0.02;
        const ux = dx / dist;
        const uy = dy / dist;
        const membersA = sublobeGroups.get(slKeys[i]);
        const membersB = sublobeGroups.get(slKeys[j]);
        for (const n of membersA) {{ n.vx -= ux * push; n.vy -= uy * push; }}
        for (const n of membersB) {{ n.vx += ux * push; n.vy += uy * push; }}
      }}
    }}
  }}
  // Edge springs
  for (const edge of graph.edges) {{
    if (!visibleIds.has(edge.source) || !visibleIds.has(edge.target)) continue;
    const source = nodes[edge.source];
    const target = nodes[edge.target];
    const dx = target.x - source.x;
    const dy = target.y - source.y;
    const distance = Math.max(30, Math.hypot(dx, dy));
    const ideal = edge.type === 'parent' ? 80 : edge.type === 'related' ? 160 : 145;
    const force = (distance - ideal) * 0.0026;
    const ux = dx / distance;
    const uy = dy / distance;
    source.vx += ux * force;
    source.vy += uy * force;
    target.vx -= ux * force;
    target.vy -= uy * force;
  }}
  // Anchor pull + damping
  for (const node of filteredNodes) {{
    const anchorPull = node.type === 'brain' ? 0.015 : node.type === 'map' ? 0.014 : 0.006;
    node.vx += (node.targetX - node.x) * anchorPull;
    node.vy += (node.targetY - node.y) * anchorPull;
    if (draggingNodeId === node.id) {{ node.vx = 0; node.vy = 0; continue; }}
    node.vx *= 0.88;
    node.vy *= 0.88;
    node.x += node.vx;
    node.y += node.vy;
  }}
}}

// --- Convex hull for lobe backgrounds ---
function convexHull(points) {{
  if (points.length < 3) return points.slice();
  points = points.slice().sort((a, b) => a.x - b.x || a.y - b.y);
  const cross = (O, A, B) => (A.x - O.x) * (B.y - O.y) - (A.y - O.y) * (B.x - O.x);
  const lower = [];
  for (const p of points) {{ while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) lower.pop(); lower.push(p); }}
  const upper = [];
  for (let i = points.length - 1; i >= 0; i--) {{ const p = points[i]; while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) upper.pop(); upper.push(p); }}
  lower.pop();
  upper.pop();
  return lower.concat(upper);
}}

function expandHull(hull, pad) {{
  if (hull.length < 2) return hull;
  const cx = hull.reduce((s, p) => s + p.x, 0) / hull.length;
  const cy = hull.reduce((s, p) => s + p.y, 0) / hull.length;
  return hull.map(p => {{
    const dx = p.x - cx;
    const dy = p.y - cy;
    const dist = Math.max(1, Math.hypot(dx, dy));
    return {{ x: p.x + (dx / dist) * pad, y: p.y + (dy / dist) * pad }};
  }});
}}

function drawLabel(text, x, y, fontSize, bold) {{
  const maxLen = 22;
  const label = text.length > maxLen ? text.slice(0, maxLen) + '...' : text;
  ctx.font = `${{bold ? 'bold ' : ''}}${{fontSize}}px "Avenir Next", "Segoe UI", sans-serif`;
  const metrics = ctx.measureText(label);
  const pw = 6;
  const ph = 3;
  const lw = metrics.width;
  const lh = fontSize;
  // Background pill
  ctx.fillStyle = 'rgba(10, 15, 26, 0.75)';
  const rx = x - pw;
  const ry = y - lh - ph;
  const rw = lw + pw * 2;
  const rh = lh + ph * 2;
  ctx.beginPath();
  ctx.roundRect(rx, ry, rw, rh, 4);
  ctx.fill();
  // Text
  ctx.fillStyle = 'rgba(233, 241, 255, 0.94)';
  ctx.fillText(label, x, y);
}}

function draw() {{
  const rect = canvas.getBoundingClientRect();
  ctx.clearRect(0, 0, rect.width, rect.height);
  ctx.save();
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  const visibleIds = new Set(filteredNodes.map(n => n.id));
  const query = searchInput.value.trim().toLowerCase();

  // --- Pass 1: Lobe hull backgrounds ---
  for (const lobe of uniqueLobes) {{
    if (lobe === 'root') continue;
    const members = filteredNodes.filter(n => n.lobe === lobe);
    const color = lobeColor(lobe);
    if (members.length < 1) {{
      // Empty lobe: draw circle at anchor with label
      const anchor = lobeAnchors.get(lobe);
      if (anchor) {{
        ctx.beginPath();
        ctx.arc(anchor.x, anchor.y, 50, 0, Math.PI * 2);
        ctx.fillStyle = rgbaFromHex(color, 0.04);
        ctx.fill();
        ctx.strokeStyle = rgbaFromHex(color, 0.1);
        ctx.lineWidth = 1;
        ctx.setLineDash([6, 6]);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.fillStyle = rgbaFromHex(color, 0.2);
        ctx.font = 'bold 16px "Avenir Next", "Segoe UI", sans-serif';
        ctx.textAlign = 'center';
        ctx.fillText(lobe.toUpperCase(), anchor.x, anchor.y - 58);
        ctx.fillStyle = rgbaFromHex(color, 0.12);
        ctx.font = '11px "Avenir Next", "Segoe UI", sans-serif';
        ctx.fillText('(empty)', anchor.x, anchor.y + 4);
        ctx.textAlign = 'start';
      }}
      continue;
    }}
    const points = members.map(n => ({{ x: n.x, y: n.y }}));
    if (points.length === 1) {{
      // Single node: draw circle
      ctx.beginPath();
      ctx.arc(points[0].x, points[0].y, 50, 0, Math.PI * 2);
      ctx.fillStyle = rgbaFromHex(color, 0.06);
      ctx.fill();
      ctx.strokeStyle = rgbaFromHex(color, 0.12);
      ctx.lineWidth = 1;
      ctx.stroke();
    }} else if (points.length === 2) {{
      // Two nodes: draw ellipse between them
      const mx = (points[0].x + points[1].x) / 2;
      const my = (points[0].y + points[1].y) / 2;
      ctx.beginPath();
      ctx.ellipse(mx, my, Math.hypot(points[1].x - points[0].x, points[1].y - points[0].y) / 2 + 40, 40, Math.atan2(points[1].y - points[0].y, points[1].x - points[0].x), 0, Math.PI * 2);
      ctx.fillStyle = rgbaFromHex(color, 0.06);
      ctx.fill();
      ctx.strokeStyle = rgbaFromHex(color, 0.12);
      ctx.lineWidth = 1;
      ctx.stroke();
    }} else {{
      const hull = expandHull(convexHull(points), 40);
      ctx.beginPath();
      ctx.moveTo(hull[0].x, hull[0].y);
      for (let i = 1; i < hull.length; i++) ctx.lineTo(hull[i].x, hull[i].y);
      ctx.closePath();
      ctx.fillStyle = rgbaFromHex(color, 0.06);
      ctx.fill();
      ctx.strokeStyle = rgbaFromHex(color, 0.12);
      ctx.lineWidth = 1;
      ctx.stroke();
    }}
    // Hull label
    const cx = members.reduce((s, n) => s + n.x, 0) / members.length;
    const cy = members.reduce((s, n) => s + n.y, 0) / members.length;
    const minY = Math.min(...members.map(n => n.y));
    ctx.fillStyle = rgbaFromHex(color, 0.2);
    ctx.font = 'bold 16px "Avenir Next", "Segoe UI", sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText(lobe.toUpperCase(), cx, minY - 50);
    ctx.textAlign = 'start';
  }}

  // --- Pass 1b: Sub-lobe group backgrounds ---
  // Only draw hulls for 2nd-level sublobes (2 segments, e.g. "projects/foo").
  // Include descendant nodes (3rd-level+) inside the parent hull.
  const sublobes = [...new Set(filteredNodes.map(n => n.sublobe))];
  for (const sl of sublobes) {{
    if (sl === 'root') continue;
    const topLobe = sl.split('/')[0];
    if (sl === topLobe) continue; // top-level lobe, already drawn above
    // Skip inner sublobes (depth 3+): they are included in their parent hull
    if (sl.split('/').length > 2) continue;
    // Include descendants: exact match OR starts with sl + '/'
    const members = filteredNodes.filter(n => n.sublobe === sl || n.sublobe.startsWith(sl + '/'));
    if (members.length < 2) continue;
    const color = lobeColor(topLobe);
    const points = members.map(n => ({{ x: n.x, y: n.y }}));
    if (points.length === 2) {{
      const mx = (points[0].x + points[1].x) / 2;
      const my = (points[0].y + points[1].y) / 2;
      ctx.beginPath();
      ctx.ellipse(mx, my, Math.hypot(points[1].x - points[0].x, points[1].y - points[0].y) / 2 + 30, 30, Math.atan2(points[1].y - points[0].y, points[1].x - points[0].x), 0, Math.PI * 2);
      ctx.fillStyle = rgbaFromHex(color, 0.04);
      ctx.fill();
      ctx.strokeStyle = rgbaFromHex(color, 0.08);
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 4]);
      ctx.stroke();
      ctx.setLineDash([]);
    }} else {{
      const hull = expandHull(convexHull(points), 28);
      ctx.beginPath();
      ctx.moveTo(hull[0].x, hull[0].y);
      for (let i = 1; i < hull.length; i++) ctx.lineTo(hull[i].x, hull[i].y);
      ctx.closePath();
      ctx.fillStyle = rgbaFromHex(color, 0.04);
      ctx.fill();
      ctx.strokeStyle = rgbaFromHex(color, 0.08);
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 4]);
      ctx.stroke();
      ctx.setLineDash([]);
    }}
    // Sub-lobe label
    const slName = sl.split('/').pop() || sl;
    const scx = members.reduce((s, n) => s + n.x, 0) / members.length;
    const sMinY = Math.min(...members.map(n => n.y));
    ctx.fillStyle = rgbaFromHex(color, 0.16);
    ctx.font = 'bold 12px "Avenir Next", "Segoe UI", sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText(slName, scx, sMinY - 34);
    ctx.textAlign = 'start';
  }}

  // --- Pass 2: Edges ---
  for (const edge of graph.edges) {{
    if (!visibleIds.has(edge.source) || !visibleIds.has(edge.target)) continue;
    const source = nodes[edge.source];
    const target = nodes[edge.target];
    const selected = selectedId != null && (edge.source === selectedId || edge.target === selectedId);
    ctx.beginPath();
    ctx.moveTo(source.x, source.y);
    const mx = (source.x + target.x) / 2;
    const my = (source.y + target.y) / 2;
    const curve = edge.type === 'related' ? 18 : edge.type === 'inline' ? -12 : 0;
    ctx.quadraticCurveTo(mx + curve, my - curve, target.x, target.y);
    ctx.strokeStyle = EDGE_COLORS[edge.type] || 'rgba(255,255,255,0.14)';
    ctx.lineWidth = selected ? 2.7 : edge.type === 'parent' ? 1.8 : 1.2;
    if (edge.type === 'related') ctx.setLineDash([8, 8]);
    else if (edge.type === 'inline') ctx.setLineDash([2, 7]);
    else ctx.setLineDash([]);
    ctx.stroke();
  }}
  ctx.setLineDash([]);

  // --- Pass 3: Nodes ---
  for (const node of filteredNodes) {{
    if (node.type === 'brain') continue; // brain name is in the page title
    const isSelected = node.id === selectedId;
    const isHovered = node.id === hoveredId;
    const dimmed = query && !node.searchText.includes(query);
    ctx.globalAlpha = dimmed ? 0.22 : 1;
    ctx.shadowColor = node.color;
    ctx.shadowBlur = isSelected ? 28 : isHovered ? 18 : 10;

    if (node.type === 'map') {{
      // Skip top-level lobe maps -- the hull label already shows the lobe name
      // Top-level: path is "lobe/map.md" (2 parts). Sub-lobe: "lobe/sub/map.md" (3+ parts)
      const pathDepth = node.path.split('/').length;
      if (pathDepth <= 2) {{ ctx.shadowBlur = 0; continue; }}
      const w = 110;
      const h = 28;
      const rx = node.x - w / 2;
      const ry = node.y - h / 2;
      const lobeCol = lobeColor(node.lobe);
      ctx.beginPath();
      ctx.roundRect(rx, ry, w, h, 10);
      ctx.fillStyle = rgbaFromHex(lobeCol, 0.14);
      ctx.fill();
      ctx.strokeStyle = isSelected ? '#ffffff' : rgbaFromHex(lobeCol, 0.6);
      ctx.lineWidth = isSelected ? 2.5 : 1.5;
      ctx.stroke();
      ctx.shadowBlur = 0;
      // Label: show parent/name for context (e.g. "projects / specmint")
      const pathParts = node.path.split('/');
      const dirName = pathParts.length >= 2 ? pathParts[pathParts.length - 2] : node.lobe;
      const label = dirName.length > 20 ? dirName.slice(0, 20) + '...' : dirName;
      ctx.fillStyle = 'rgba(233, 241, 255, 0.95)';
      ctx.font = 'bold 11px "Avenir Next", "Segoe UI", sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText(label, node.x, node.y + 4);
      ctx.textAlign = 'start';

    }} else if (node.type === 'glossary' || node.type === 'index') {{
      // Diamond shape
      const r = node.radius;
      ctx.beginPath();
      ctx.moveTo(node.x, node.y - r);
      ctx.lineTo(node.x + r, node.y);
      ctx.lineTo(node.x, node.y + r);
      ctx.lineTo(node.x - r, node.y);
      ctx.closePath();
      ctx.fillStyle = node.color;
      ctx.fill();
      ctx.shadowBlur = 0;
      if (isSelected) {{ ctx.strokeStyle = '#ffffff'; ctx.lineWidth = 2; ctx.stroke(); }}
      // Always show label
      drawLabel(node.title, node.x + r + 6, node.y + 4, 11, false);

    }} else if (node.type === 'brain') {{
      // Large white circle with double ring
      ctx.beginPath();
      ctx.arc(node.x, node.y, node.radius, 0, Math.PI * 2);
      ctx.fillStyle = '#ffffff';
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.strokeStyle = 'rgba(255,255,255,0.5)';
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(node.x, node.y, node.radius + 5, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(255,255,255,0.2)';
      ctx.lineWidth = 1;
      ctx.stroke();
      drawLabel(node.title, node.x + node.radius + 10, node.y + 5, 15, true);

    }} else {{
      // Neuron: circle
      ctx.beginPath();
      ctx.arc(node.x, node.y, node.radius, 0, Math.PI * 2);
      ctx.fillStyle = node.color;
      ctx.fill();
      ctx.shadowBlur = 0;
      if (isSelected) {{ ctx.strokeStyle = '#ffffff'; ctx.lineWidth = 2; ctx.stroke(); }}
      else if (isHovered) {{ ctx.strokeStyle = 'rgba(255,255,255,0.4)'; ctx.lineWidth = 1; ctx.stroke(); }}

      // Zoom-adaptive labels for neurons
      const showLabel = isSelected || isHovered
        || (camera.scale >= 1.2)
        || (camera.scale >= 0.8 && (node.degree || 0) >= 3);
      if (showLabel) {{
        drawLabel(node.title, node.x + node.radius + 5, node.y + 3, 10, false);
      }}
    }}
  }}
  ctx.globalAlpha = 1;
  ctx.restore();
}}

function loop() {{
  for (let i = 0; i < 2; i++) tick();
  draw();
  requestAnimationFrame(loop);
}}

canvas.addEventListener('pointerdown', event => {{
  canvas.setPointerCapture(event.pointerId);
  dragMoved = false;
  lastPointer = {{ x: event.clientX, y: event.clientY }};
  const world = toWorld(event.clientX, event.clientY);
  const hit = hitTest(world.x, world.y);
  if (hit) {{
    draggingNodeId = hit.id;
    dragOffset = {{ x: world.x - hit.x, y: world.y - hit.y }};
    selectNode(hit.id, false);
  }} else {{
    isPanning = true;
    canvas.classList.add('dragging');
  }}
}});

canvas.addEventListener('pointermove', event => {{
  pointer = {{ x: event.clientX, y: event.clientY }};
  const world = toWorld(event.clientX, event.clientY);
  const hit = hitTest(world.x, world.y);
  hoveredId = hit ? hit.id : null;
  const dx = event.clientX - lastPointer.x;
  const dy = event.clientY - lastPointer.y;
  if (Math.abs(dx) > 1 || Math.abs(dy) > 1) dragMoved = true;
  lastPointer = {{ x: event.clientX, y: event.clientY }};
  if (draggingNodeId != null) {{
    const node = nodes.find(item => item.id === draggingNodeId);
    if (node) {{
      node.x = world.x - dragOffset.x;
      node.y = world.y - dragOffset.y;
      node.targetX = node.x;
      node.targetY = node.y;
    }}
  }} else if (isPanning) {{
    camera.x += dx;
    camera.y += dy;
  }}
}});

canvas.addEventListener('pointerup', event => {{
  if (!dragMoved && draggingNodeId == null && !isPanning) {{
    const world = toWorld(event.clientX, event.clientY);
    const hit = hitTest(world.x, world.y);
    if (hit) selectNode(hit.id, false);
    else selectNode(null, false);
  }}
  draggingNodeId = null;
  isPanning = false;
  canvas.classList.remove('dragging');
}});

canvas.addEventListener('pointerleave', () => {{
  hoveredId = null;
  draggingNodeId = null;
  isPanning = false;
  canvas.classList.remove('dragging');
}});

canvas.addEventListener('wheel', event => {{
  event.preventDefault();
  const rect = canvas.getBoundingClientRect();
  const mouseX = event.clientX - rect.left;
  const mouseY = event.clientY - rect.top;
  const worldX = (mouseX - camera.x) / camera.scale;
  const worldY = (mouseY - camera.y) / camera.scale;
  const nextScale = Math.min(2.6, Math.max(0.42, camera.scale * (event.deltaY < 0 ? 1.08 : 0.92)));
  camera.x = mouseX - worldX * nextScale;
  camera.y = mouseY - worldY * nextScale;
  camera.scale = nextScale;
}}, {{ passive: false }});

searchInput.addEventListener('input', refreshVisibility);
document.getElementById('reset-view').addEventListener('click', () => {{
  const rect = canvas.parentElement.getBoundingClientRect();
  searchInput.value = '';
  hiddenLobes.clear();
  hiddenSublobes.clear();
  expandedLobes.clear();
  expandedSublobes.clear();
  renderLobes();
  refreshVisibility();
  selectNode(null, false);
  buildAnchors(rect.width, rect.height);
  nodes = initializeNodes();
  refreshVisibility();
  fitToFilteredNodes(true);
}});

// --- Collapsible side panels ---
function togglePanel(side) {{
  const shell = document.querySelector('.shell');
  const cls = side + '-collapsed';
  const isCollapsed = shell.classList.toggle(cls);
  const expandBtn = document.getElementById('expand-' + side);
  if (expandBtn) expandBtn.classList.toggle('visible', isCollapsed);
  // The stage column changes size; give the canvas a chance to re-measure
  // on the next frame so the physics + hull rendering stay centered.
  requestAnimationFrame(() => {{
    resize();
    nodes = initializeNodes();
    refreshVisibility();
    updateDetails();
  }});
}}
document.getElementById('collapse-left').addEventListener('click', () => togglePanel('left'));
document.getElementById('collapse-right').addEventListener('click', () => togglePanel('right'));
document.getElementById('expand-left').addEventListener('click', () => togglePanel('left'));
document.getElementById('expand-right').addEventListener('click', () => togglePanel('right'));

document.getElementById('nav-back').addEventListener('click', navBack);
document.getElementById('nav-forward').addEventListener('click', navForward);
document.getElementById('modal-back').addEventListener('click', () => {{
  navBack();
  const node = nodes.find(n => n.id === selectedId);
  if (node && document.getElementById('content-modal').style.display !== 'none') openModal(node);
}});
document.getElementById('modal-forward').addEventListener('click', () => {{
  navForward();
  const node = nodes.find(n => n.id === selectedId);
  if (node && document.getElementById('content-modal').style.display !== 'none') openModal(node);
}});
document.getElementById('modal-close').addEventListener('click', () => {{
  document.getElementById('content-modal').style.display = 'none';
}});
document.getElementById('content-modal').addEventListener('click', event => {{
  if (event.target === event.currentTarget) event.currentTarget.style.display = 'none';
}});
document.addEventListener('keydown', event => {{
  if (event.key === 'Escape') {{
    document.getElementById('content-modal').style.display = 'none';
  }}
  if (event.key === '/') {{
    event.preventDefault();
    searchInput.focus();
    searchInput.select();
  }}
}});

resize();
addEventListener('resize', () => {{
  resize();
  nodes = initializeNodes();
  refreshVisibility();
  updateDetails();
}});
nodes = initializeNodes();
renderLobes();
refreshVisibility();
updateDetails();
// Frame the initial layout so the brain is centered regardless of viewport
// aspect ratio. Uses targetX/targetY so the fit is deterministic.
fitToFilteredNodes(true);
loop();
</script>
</body>
</html>"""

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(html, encoding="utf-8")

    return {"nodes": len(graph["nodes"]), "edges": len(graph["edges"])}
