"""MRI visualization — build graph from brain and generate standalone HTML."""

from __future__ import annotations

import json
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter
from kluris.core.linker import LINK_PATTERN

SKIP_DIRS = {".git"}
SKIP_FILES = {".gitignore", "README.md"}


def _all_md_files(brain_path: Path) -> list[Path]:
    files = []
    for item in brain_path.rglob("*.md"):
        if any(part in SKIP_DIRS for part in item.parts):
            continue
        if item.name in SKIP_FILES:
            continue
        files.append(item)
    return files


def _extract_title_and_excerpt(path: Path, content: str) -> tuple[str, str]:
    """Extract a readable title and short excerpt from markdown content."""
    title = path.stem.replace("-", " ").title()
    excerpt = ""

    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("# ") and title == path.stem.replace("-", " ").title():
            title = line[2:].strip()
            continue
        if line.startswith(("## ", "- ", "* ", "```", "---", "up ", "sideways ")):
            continue
        excerpt = line
        break

    return title, excerpt[:220]


def _build_content_preview(content: str) -> tuple[str, bool]:
    """Build a bounded markdown body preview for the inspector panel."""
    if not content.strip():
        return "", False

    lines = content.splitlines()
    preview_lines: list[str] = []
    skipped_title = False

    for raw_line in lines:
        line = raw_line.rstrip()
        if not skipped_title and line.strip().startswith("# "):
            skipped_title = True
            continue
        if not preview_lines and not line.strip():
            continue
        preview_lines.append(line)

    preview = "\n".join(preview_lines).strip()
    if not preview:
        return "", False

    max_lines = 48
    max_chars = 2800
    truncated = len(preview_lines) > max_lines or len(preview) > max_chars
    preview = "\n".join(preview_lines[:max_lines]).strip()

    if len(preview) > max_chars:
        preview = preview[:max_chars].rstrip()
        if "\n" in preview:
            preview = preview.rsplit("\n", 1)[0].rstrip()

    if truncated:
        preview = preview.rstrip() + "\n\n..."

    return preview, truncated


def build_graph(brain_path: Path) -> dict:
    """Build a graph of nodes and edges from a brain directory."""
    nodes = []
    edges = []
    node_ids: dict[str, int] = {}

    files = _all_md_files(brain_path)

    # Create nodes
    for i, f in enumerate(files):
        rel = str(f.relative_to(brain_path))
        node_ids[rel] = i

        # Determine lobe and sublobe
        parts = f.relative_to(brain_path).parts
        lobe = parts[0] if len(parts) > 1 else "root"
        sublobe = "/".join(parts[:2]) if len(parts) > 2 else lobe

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

        title, excerpt = _extract_title_and_excerpt(f, content)
        content_preview, preview_truncated = _build_content_preview(content)
        tags = meta.get("tags", [])
        related = meta.get("related", [])

        nodes.append({
            "id": i,
            "label": title,
            "path": rel,
            "lobe": lobe,
            "sublobe": sublobe,
            "type": ntype,
            "file_name": f.name,
            "title": title,
            "excerpt": excerpt,
            "content_preview": content_preview,
            "content_preview_truncated": preview_truncated,
            "tags": tags if isinstance(tags, list) else [],
            "created": str(meta.get("created", "")),
            "updated": str(meta.get("updated", "")),
            "template": str(meta.get("template", "")),
            "parent": str(meta.get("parent", "")),
            "related": related if isinstance(related, list) else [],
        })

    # Create edges from frontmatter and inline links
    for f in files:
        rel = str(f.relative_to(brain_path))
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
                parent_resolved = str((f.parent / parent).resolve().relative_to(brain_path.resolve()))
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
                    r_resolved = str((f.parent / r).resolve().relative_to(brain_path.resolve()))
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
            try:
                t_resolved = str((f.parent / target).resolve().relative_to(brain_path.resolve()))
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
    overflow: auto;
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
    font-size: clamp(1.8rem, 3vw, 2.6rem);
    line-height: 1.04;
    letter-spacing: -0.04em;
    text-transform: uppercase;
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
  .filters {{
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 14px;
  }}
  .chip {{
    padding: 8px 12px;
    border-radius: 999px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    color: var(--muted);
    cursor: pointer;
    user-select: none;
    font-size: 0.88rem;
  }}
  .chip.active {{
    color: var(--text);
    border-color: rgba(123,247,255,0.4);
    background: rgba(123,247,255,0.12);
  }}
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
    width: min(90vw, 800px);
    max-height: 85vh;
    background: var(--panel-strong);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: var(--radius);
    box-shadow: 0 40px 100px rgba(0,0,0,0.6);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }}
  .modal-header {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 22px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
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
  .modal-nav {{
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 12px 22px;
    border-bottom: 1px solid rgba(255,255,255,0.06);
  }}
  .modal-nav:empty {{ display: none; }}
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
        <div class="stat">
          <div class="stat-label">Visible</div>
          <div class="stat-value" id="stat-visible">{len(graph["nodes"])}</div>
        </div>
      </div>
      <div class="search-wrap">
        <label for="search-input">Search the brain</label>
        <div class="search-row">
          <input id="search-input" type="search" placeholder="Name, path, lobe, or tag" autocomplete="off">
          <button class="button" id="reset-view" type="button">Reset</button>
        </div>
        <div class="filters" id="type-filters"></div>
      </div>
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
    <div class="panel-inner">
      <div style="display:flex;align-items:center;justify-content:space-between">
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
    <div class="modal-nav" id="modal-nav"></div>
    <pre class="modal-content" id="modal-content"></pre>
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
const typeFiltersEl = document.getElementById('type-filters');
const statVisible = document.getElementById('stat-visible');
// statSelected removed from left panel
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
let activeTypes = new Set(['brain', 'index', 'glossary', 'map', 'neuron']);

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
  // neuron: desaturated lobe color
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
  const radius = Math.min(width, height) * 0.40;
  const nonRootLobes = uniqueLobes.filter(l => l !== 'root');
  lobeAnchors.set('root', {{ x: cx, y: cy * 0.86 }});
  nonRootLobes.forEach((lobe, i) => {{
    const angle = -Math.PI / 2 + (i / Math.max(1, nonRootLobes.length)) * Math.PI * 2;
    lobeAnchors.set(lobe, {{ x: cx + Math.cos(angle) * radius, y: cy + Math.sin(angle) * radius }});
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
      node.type, ...(node.tags || []), node.excerpt || '', node.content_preview || '',
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
  // Hide top-level lobe maps -- hull label shows the lobe name
  if (node.type === 'map' && node.path.split('/').length <= 2) return false;
  if (!activeTypes.has(node.type)) return false;
  const query = searchInput.value.trim().toLowerCase();
  if (!query) return true;
  return node.searchText.includes(query);
}}

function refreshVisibility() {{
  filteredNodes = nodes.filter(visibleNode);
  statVisible.textContent = String(filteredNodes.length);
  resultCountEl.textContent = searchInput.value.trim()
    ? `Found ${{filteredNodes.length}} results.`
    : `Showing all ${{filteredNodes.length}} neurons.`;
  renderResults();
}}

const TYPE_LABELS = {{ glossary: 'glossary', map: 'lobes', neuron: 'neurons' }};
function renderFilters() {{
  const counts = graph.nodes.reduce((acc, node) => {{
    acc[node.type] = (acc[node.type] || 0) + 1;
    return acc;
  }}, {{}});
  typeFiltersEl.innerHTML = '';
  for (const type of ['glossary', 'map', 'neuron']) {{
    if (!counts[type]) continue;
    const label = TYPE_LABELS[type] || type;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `chip${{activeTypes.has(type) ? ' active' : ''}}`;
    button.textContent = `${{label}} (${{counts[type]}})`;
    button.addEventListener('click', () => {{
      if (activeTypes.has(type) && activeTypes.size > 1) activeTypes.delete(type);
      else activeTypes.add(type);
      renderFilters();
      refreshVisibility();
    }});
    typeFiltersEl.appendChild(button);
  }}
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
    ? '<div class="content-preview-note">Preview truncated for readability. Open the source file for the full document.</div>'
    : '';
  const connections = connected.length
    ? connected.map(target => `
        <button type="button" class="connection-card" data-node-id="${{target.id}}">
          <div class="result-title">${{escapeHtml(target.title)}}</div>
          <div class="result-meta">${{target.type === 'map' ? 'lobe' : escapeHtml(target.type)}} • ${{escapeHtml(target.sublobe || target.lobe)}}</div>
          <div class="result-path">${{escapeHtml(target.path)}}</div>
        </button>
      `).join('')
    : `<div class="details-empty">No connected nodes found for this selection.</div>`;

  // Build breadcrumbs from path
  const pathParts = node.path.split('/');
  const crumbs = pathParts.map((part, i) => {{
    const partPath = pathParts.slice(0, i + 1).join('/');
    // Find a node matching this path (map.md for directories, or the file itself)
    const isLast = i === pathParts.length - 1;
    const targetPath = isLast ? partPath : partPath + '/map.md';
    const target = nodes.find(n => n.path === targetPath);
    const label = part.replace('.md', '');
    if (target && target.id !== node.id) {{
      return `<button type="button" class="breadcrumb-link" data-node-id="${{target.id}}">${{escapeHtml(label)}}</button>`;
    }}
    return `<span class="breadcrumb-current">${{escapeHtml(label)}}</span>`;
  }}).join('<span class="breadcrumb-sep">/</span>');

  detailsPanel.innerHTML = `
    <div class="details-card">
      <div class="details-title">${{escapeHtml(node.title)}}</div>
      <div class="breadcrumbs">${{crumbs}}</div>
      <div class="meta-grid">
        <div class="meta-card"><span class="label">Type</span><span class="value">${{node.type === 'map' ? 'lobe' : node.type === 'neuron' ? 'neuron' : escapeHtml(node.type)}}</span></div>
        <div class="meta-card"><span class="label">Section</span><span class="value">${{escapeHtml(node.sublobe || node.lobe)}}</span></div>
        <div class="meta-card"><span class="label">Updated</span><span class="value">${{escapeHtml(node.updated || '—')}}</span></div>
        <div class="meta-card"><span class="label">Created</span><span class="value">${{escapeHtml(node.created || '—')}}</span></div>
        <div class="meta-card"><span class="label">Template</span><span class="value">${{escapeHtml(node.template || '—')}}</span></div>
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
}}

function openModal(node) {{
  const modal = document.getElementById('content-modal');
  const breadcrumb = node.path.split('/').map(p => p.replace('.md', '')).join(' / ');
  document.getElementById('modal-title').innerHTML = `${{escapeHtml(node.title)}} <span style="color:var(--muted);font-size:0.8em;font-weight:400;margin-left:8px">${{escapeHtml(breadcrumb)}}</span>`;
  // Render content with clickable markdown links
  const rawContent = escapeHtml(node.content_preview || 'No content.');
  const nodePath = node.path.replace(/[^/]+$/, ''); // directory of current node
  const linkedContent = rawContent.replace(
    /\[([^\]]*)\]\(([^)]+)\)/g,
    (match, text, href) => {{
      if (href.startsWith('http')) return match;
      // Resolve relative path from node's directory
      const parts = (nodePath + href).split('/');
      const resolved = [];
      for (const p of parts) {{
        if (p === '..') resolved.pop();
        else if (p && p !== '.') resolved.push(p);
      }}
      const resolvedPath = resolved.join('/');
      const target = nodes.find(n => n.path === resolvedPath);
      if (target) {{
        return `<button type="button" class="modal-nav-btn" data-modal-nav="${{target.id}}" style="display:inline;padding:2px 6px;font-size:inherit">${{text}}</button>`;
      }}
      return `${{text}} (${{href}})`;
    }}
  );
  document.getElementById('modal-content').innerHTML = linkedContent;
  for (const btn of document.getElementById('modal-content').querySelectorAll('[data-modal-nav]')) {{
    btn.addEventListener('click', () => {{
      const target = nodes.find(n => n.id === Number(btn.dataset.modalNav));
      if (target) {{ selectNode(target.id, true); openModal(target); }}
    }});
  }}
  // Build nav buttons for connected nodes
  const navEl = document.getElementById('modal-nav');
  const connected = [...(neighbors.get(node.id) || [])]
    .map(id => nodes.find(n => n.id === id))
    .filter(n => n && n.type === 'neuron')
    .sort((a, b) => a.title.localeCompare(b.title));
  navEl.innerHTML = connected.map(n => {{
    const parts = n.path.split('/');
    const parent = parts.length >= 2 ? parts[parts.length - 2] : '';
    const label = parent ? `${{parent}} / ${{n.title}}` : n.title;
    return `<button type="button" class="modal-nav-btn" data-modal-nav="${{n.id}}">${{escapeHtml(label)}}</button>`;
  }}).join('');
  for (const btn of navEl.querySelectorAll('[data-modal-nav]')) {{
    btn.addEventListener('click', () => {{
      const target = nodes.find(n => n.id === Number(btn.dataset.modalNav));
      if (target) {{
        selectNode(target.id, true);
        openModal(target);
      }}
    }});
  }}
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

function focusOnNode(id) {{
  const node = nodes.find(item => item.id === id);
  if (!node) return;
  const rect = canvas.parentElement.getBoundingClientRect();
  if (node.type === 'map') {{
    // Zoom to frame the lobe
    const members = filteredNodes.filter(n => n.lobe === node.lobe);
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
      const crossLobe = a.lobe !== b.lobe ? 1.6 : 1.0;
      const force = (1200 * crossLobe) / (distance * distance);
      const ux = dx / distance;
      const uy = dy / distance;
      a.vx -= ux * force;
      a.vy -= uy * force;
      b.vx += ux * force;
      b.vy += uy * force;
    }}
  }}
  // Same-lobe cohesion
  const lobeCentroids = new Map();
  for (const lobe of uniqueLobes) {{
    const members = filteredNodes.filter(n => n.lobe === lobe);
    if (!members.length) continue;
    const cx = members.reduce((s, n) => s + n.x, 0) / members.length;
    const cy = members.reduce((s, n) => s + n.y, 0) / members.length;
    lobeCentroids.set(lobe, {{ x: cx, y: cy }});
    for (const n of members) {{
      if (n.type !== 'brain') {{
        n.vx += (cx - n.x) * 0.002;
        n.vy += (cy - n.y) * 0.002;
      }}
    }}
  }}
  // Sub-lobe cohesion + cross-sub-lobe repulsion
  const sublobeGroups = new Map();
  for (const n of filteredNodes) {{
    if (!sublobeGroups.has(n.sublobe)) sublobeGroups.set(n.sublobe, []);
    sublobeGroups.get(n.sublobe).push(n);
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
  const sublobes = [...new Set(filteredNodes.map(n => n.sublobe))];
  for (const sl of sublobes) {{
    if (sl === 'root') continue;
    // Only draw sub-lobe hulls when sublobe differs from lobe (i.e. nested)
    const members = filteredNodes.filter(n => n.sublobe === sl);
    if (members.length < 2) continue;
    const topLobe = members[0].lobe;
    if (sl === topLobe) continue; // top-level lobe, already drawn above
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
      const w = 140;
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
      const parentName = pathParts.length >= 3 ? pathParts[pathParts.length - 3] : '';
      const fullLabel = parentName ? `${{parentName}} / ${{dirName}}` : dirName;
      const label = fullLabel.length > 24 ? fullLabel.slice(0, 24) + '...' : fullLabel;
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
  camera.scale = 1;
  camera.x = 0;
  camera.y = 0;
  searchInput.value = '';
  activeTypes = new Set(['brain', 'index', 'glossary', 'map', 'neuron']);
  renderFilters();
  refreshVisibility();
  selectNode(null, false);
  buildAnchors(rect.width, rect.height);
  nodes = initializeNodes();
}});

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
renderFilters();
refreshVisibility();
updateDetails();
loop();
</script>
</body>
</html>"""

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(html, encoding="utf-8")

    return {"nodes": len(graph["nodes"]), "edges": len(graph["edges"])}
