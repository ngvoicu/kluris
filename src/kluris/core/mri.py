"""MRI visualization — build graph from brain and generate standalone HTML."""

from __future__ import annotations

import json
from pathlib import Path

from kluris.core.frontmatter import read_frontmatter
from kluris.core.linker import LINK_PATTERN

SKIP_DIRS = {".git"}
SKIP_FILES = {".gitignore"}


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

        # Determine lobe
        parts = f.relative_to(brain_path).parts
        lobe = parts[0] if len(parts) > 1 else "root"

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
    graph_json = json.dumps(graph, indent=2)

    html = f"""\
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Kluris Brain MRI</title>
<style>
  :root {{
    --bg: #06111f;
    --panel: rgba(8, 15, 32, 0.86);
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
      <p class="eyebrow">Constellation Navigation</p>
      <h1>Brain MRI</h1>
      <p class="subhead">
        Search by node title, path, lobe, or tags. Click any node to inspect metadata,
        connected knowledge, and the local neighborhood instantly.
      </p>
      <div class="stats">
        <div class="stat">
          <div class="stat-label">Nodes</div>
          <div class="stat-value" id="stat-nodes">{len(graph["nodes"])}</div>
        </div>
        <div class="stat">
          <div class="stat-label">Edges</div>
          <div class="stat-value" id="stat-edges">{len(graph["edges"])}</div>
        </div>
        <div class="stat">
          <div class="stat-label">Visible</div>
          <div class="stat-value" id="stat-visible">{len(graph["nodes"])}</div>
        </div>
        <div class="stat">
          <div class="stat-label">Selected</div>
          <div class="stat-value" id="stat-selected">None</div>
        </div>
      </div>
      <div class="search-wrap">
        <label for="search-input">Search the brain</label>
        <div class="search-row">
          <input id="search-input" type="search" placeholder="Search nodes, paths, lobes, tags, or excerpts" autocomplete="off">
          <button class="button" id="reset-view" type="button">Reset</button>
        </div>
        <div class="filters" id="type-filters"></div>
      </div>
      <div class="section-title">Results</div>
      <div id="result-count" class="subhead">Showing every node in the graph.</div>
      <div class="results" id="search-results"></div>
      <div class="legend">
        <div class="legend-item"><span class="legend-swatch" style="background:#ffb86b"></span>Brain / glossary anchors</div>
        <div class="legend-item"><span class="legend-swatch" style="background:#7bf7ff"></span>Map nodes for navigation hubs</div>
        <div class="legend-item"><span class="legend-swatch" style="background:#7df7b4"></span>Neuron nodes for authored knowledge</div>
        <div class="legend-item"><span class="legend-line parent"></span>Parent relationships</div>
        <div class="legend-item"><span class="legend-line related"></span>Related synapses</div>
        <div class="legend-item"><span class="legend-line inline"></span>Inline markdown links</div>
      </div>
    </div>
  </aside>

  <main class="stage">
    <div class="stage-hud">
      <div class="stage-pill">Drag nodes, pan empty space, zoom with the wheel, press <strong>/</strong> to focus search.</div>
      <div class="stage-pill" id="stage-focus">No node selected</div>
    </div>
    <canvas id="mri-canvas"></canvas>
  </main>

  <aside class="panel panel-right">
    <div class="panel-inner">
      <p class="eyebrow">Signal Readout</p>
      <h2>Node Inspector</h2>
      <p class="subhead">Click a node or a search result to open the full metadata panel.</p>
      <div class="details-empty" id="details-empty">
        Nothing selected yet. Pick a node to inspect its path, tags, timestamps,
        excerpt, and connected neighbors.
      </div>
      <div id="details-panel"></div>
    </div>
  </aside>
</div>
<script>
const graph = {graph_json};
const TYPE_COLORS = {{
  brain: '#ffb86b',
  index: '#ffd28e',
  glossary: '#ffc6f4',
  map: '#7bf7ff',
  neuron: '#7df7b4',
}};
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
const statSelected = document.getElementById('stat-selected');
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

const lobePalette = ['#7bf7ff', '#ff8bd8', '#f8c76d', '#7df7b4', '#9ea9ff', '#ffa06f', '#b8f0c1', '#f2a8ff'];
const uniqueLobes = [...new Set(graph.nodes.map(node => node.lobe))];
const lobeAnchors = new Map();

function colorForNode(node) {{
  if (node.type !== 'neuron' && node.type !== 'map') return TYPE_COLORS[node.type] || '#d0d8f3';
  const index = uniqueLobes.indexOf(node.lobe);
  return lobePalette[(index >= 0 ? index : 0) % lobePalette.length];
}}

function nodeRadius(node) {{
  if (node.type === 'brain') return 16;
  if (node.type === 'glossary' || node.type === 'index') return 11;
  if (node.type === 'map') return 10;
  return Math.max(7, 6 + Math.min(node.degree || 0, 6) * 0.55);
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
  const radius = Math.min(width, height) * 0.28;
  uniqueLobes.forEach((lobe, index) => {{
    if (lobe === 'root') {{
      lobeAnchors.set(lobe, {{ x: cx, y: cy - radius * 0.1 }});
      return;
    }}
    const angle = (-Math.PI / 2) + (index / Math.max(1, uniqueLobes.length)) * Math.PI * 2;
    lobeAnchors.set(lobe, {{
      x: cx + Math.cos(angle) * radius,
      y: cy + Math.sin(angle) * radius,
    }});
  }});
}}

function initializeNodes() {{
  const dimensions = canvas.parentElement.getBoundingClientRect();
  const lobeCounters = new Map();
  return graph.nodes.map((node, index) => {{
    const anchor = lobeAnchors.get(node.lobe) || {{ x: dimensions.width / 2, y: dimensions.height / 2 }};
    const count = lobeCounters.get(node.lobe) || 0;
    lobeCounters.set(node.lobe, count + 1);
    const orbit = 42 + count * 14;
    const angle = count * 0.9 + index * 0.17;
    let targetX = anchor.x + Math.cos(angle) * orbit;
    let targetY = anchor.y + Math.sin(angle) * orbit;
    if (node.type === 'brain') {{
      targetX = dimensions.width / 2;
      targetY = dimensions.height / 2;
    }} else if (node.type === 'glossary') {{
      targetX = dimensions.width / 2;
      targetY = dimensions.height * 0.16;
    }} else if (node.type === 'index') {{
      targetX = dimensions.width * 0.18;
      targetY = dimensions.height * 0.18;
    }} else if (node.type === 'map') {{
      targetX = anchor.x;
      targetY = anchor.y;
    }}
    const searchText = [
      node.title,
      node.path,
      node.file_name,
      node.lobe,
      node.type,
      ...(node.tags || []),
      node.excerpt || '',
      node.content_preview || '',
    ].join(' ').toLowerCase();
    return {{
      ...node,
      searchText,
      color: colorForNode(node),
      radius: nodeRadius(node),
      x: targetX + (Math.random() - 0.5) * 20,
      y: targetY + (Math.random() - 0.5) * 20,
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
  if (!activeTypes.has(node.type)) return false;
  const query = searchInput.value.trim().toLowerCase();
  if (!query) return true;
  return node.searchText.includes(query);
}}

function refreshVisibility() {{
  filteredNodes = nodes.filter(visibleNode);
  statVisible.textContent = String(filteredNodes.length);
  resultCountEl.textContent = searchInput.value.trim()
    ? `Found ${{filteredNodes.length}} matching nodes.`
    : `Showing all ${{filteredNodes.length}} nodes in the graph.`;
  renderResults();
}}

function renderFilters() {{
  const counts = graph.nodes.reduce((acc, node) => {{
    acc[node.type] = (acc[node.type] || 0) + 1;
    return acc;
  }}, {{}});
  typeFiltersEl.innerHTML = '';
  for (const type of ['brain', 'glossary', 'index', 'map', 'neuron']) {{
    if (!counts[type]) continue;
    const button = document.createElement('button');
    button.type = 'button';
    button.className = `chip${{activeTypes.has(type) ? ' active' : ''}}`;
    button.textContent = `${{type}} (${{counts[type]}})`;
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
      <div class="result-meta">${{escapeHtml(node.type)}} • ${{escapeHtml(node.lobe)}} • degree ${{node.degree}}</div>
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
    statSelected.textContent = 'None';
    stageFocus.textContent = 'No node selected';
    return;
  }}

  detailsEmpty.style.display = 'none';
  statSelected.textContent = node.title;
  stageFocus.textContent = `${{node.title}} • ${{node.path}}`;
  const connected = [...(neighbors.get(node.id) || [])]
    .map(id => nodes.find(item => item.id === id))
    .filter(Boolean)
    .sort((a, b) => a.title.localeCompare(b.title));
  const tags = (node.tags || []).map(tag => `<span class="tag">${{escapeHtml(tag)}}</span>`).join('');
  const contentPreview = escapeHtml(node.content_preview || 'No content preview available for this node.');
  const previewNote = node.content_preview_truncated
    ? '<div class="content-preview-note">Preview truncated for readability. Open the source file for the full document.</div>'
    : '';
  const connections = connected.length
    ? connected.map(target => `
        <button type="button" class="connection-card" data-node-id="${{target.id}}">
          <div class="result-title">${{escapeHtml(target.title)}}</div>
          <div class="result-meta">${{escapeHtml(target.type)}} • ${{escapeHtml(target.lobe)}}</div>
          <div class="result-path">${{escapeHtml(target.path)}}</div>
        </button>
      `).join('')
    : `<div class="details-empty">No connected nodes found for this selection.</div>`;

  detailsPanel.innerHTML = `
    <div class="details-card">
      <div class="details-title">${{escapeHtml(node.title)}}</div>
      <div class="details-path">${{escapeHtml(node.path)}}</div>
      <div class="meta-grid">
        <div class="meta-card"><span class="label">Type</span><span class="value">${{escapeHtml(node.type)}}</span></div>
        <div class="meta-card"><span class="label">Lobe</span><span class="value">${{escapeHtml(node.lobe)}}</span></div>
        <div class="meta-card"><span class="label">Updated</span><span class="value">${{escapeHtml(node.updated || '—')}}</span></div>
        <div class="meta-card"><span class="label">Created</span><span class="value">${{escapeHtml(node.created || '—')}}</span></div>
        <div class="meta-card"><span class="label">Template</span><span class="value">${{escapeHtml(node.template || '—')}}</span></div>
        <div class="meta-card"><span class="label">Connections</span><span class="value">${{connected.length}}</span></div>
      </div>
      ${{tags ? `<div class="tag-row">${{tags}}</div>` : ''}}
      <div class="section-title">Excerpt</div>
      <div class="details-copy">${{escapeHtml(node.excerpt || 'No excerpt available for this node.')}}</div>
      <div class="section-title">Content preview</div>
      <pre class="content-preview">${{contentPreview}}</pre>
      ${{previewNote}}
      <div class="section-title">Frontmatter links</div>
      <div class="details-copy">
        Parent: <strong>${{escapeHtml(node.parent || '—')}}</strong><br>
        Related entries: <strong>${{(node.related || []).length}}</strong>
      </div>
      <div class="section-title">Connected nodes</div>
      <div class="results">${{connections}}</div>
    </div>
  `;
  for (const button of detailsPanel.querySelectorAll('[data-node-id]')) {{
    button.addEventListener('click', () => selectNode(Number(button.dataset.nodeId), true));
  }}
}}

function selectNode(id, recenter = false) {{
  selectedId = id;
  updateDetails();
  if (recenter) focusOnNode(id);
}}

function focusOnNode(id) {{
  const node = nodes.find(item => item.id === id);
  if (!node) return;
  const rect = canvas.parentElement.getBoundingClientRect();
  camera.x = rect.width / 2 - node.x * camera.scale;
  camera.y = rect.height / 2 - node.y * camera.scale;
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
  const visibleIds = new Set(filteredNodes.map(node => node.id));
  for (let i = 0; i < filteredNodes.length; i++) {{
    for (let j = i + 1; j < filteredNodes.length; j++) {{
      const a = filteredNodes[i];
      const b = filteredNodes[j];
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const distance = Math.max(24, Math.hypot(dx, dy));
      const force = 1100 / (distance * distance);
      const ux = dx / distance;
      const uy = dy / distance;
      a.vx -= ux * force;
      a.vy -= uy * force;
      b.vx += ux * force;
      b.vy += uy * force;
    }}
  }}

  for (const edge of graph.edges) {{
    if (!visibleIds.has(edge.source) || !visibleIds.has(edge.target)) continue;
    const source = nodes[edge.source];
    const target = nodes[edge.target];
    const dx = target.x - source.x;
    const dy = target.y - source.y;
    const distance = Math.max(30, Math.hypot(dx, dy));
    const ideal = edge.type === 'parent' ? 110 : edge.type === 'related' ? 160 : 145;
    const force = (distance - ideal) * 0.0026;
    const ux = dx / distance;
    const uy = dy / distance;
    source.vx += ux * force;
    source.vy += uy * force;
    target.vx -= ux * force;
    target.vy -= uy * force;
  }}

  for (const node of filteredNodes) {{
    const anchorPull = node.type === 'brain' ? 0.013 : node.type === 'map' ? 0.012 : 0.006;
    node.vx += (node.targetX - node.x) * anchorPull;
    node.vy += (node.targetY - node.y) * anchorPull;
    if (draggingNodeId === node.id) {{
      node.vx = 0;
      node.vy = 0;
      continue;
    }}
    node.vx *= 0.9;
    node.vy *= 0.9;
    node.x += node.vx;
    node.y += node.vy;
  }}
}}

function draw() {{
  const rect = canvas.getBoundingClientRect();
  ctx.clearRect(0, 0, rect.width, rect.height);
  ctx.save();
  ctx.translate(camera.x, camera.y);
  ctx.scale(camera.scale, camera.scale);

  const visibleIds = new Set(filteredNodes.map(node => node.id));
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

  for (const node of filteredNodes) {{
    const isSelected = node.id === selectedId;
    const isHovered = node.id === hoveredId;
    const alpha = searchInput.value.trim() && !node.searchText.includes(searchInput.value.trim().toLowerCase()) ? 0.28 : 1;
    ctx.globalAlpha = alpha;
    ctx.beginPath();
    ctx.fillStyle = node.color;
    ctx.shadowColor = node.color;
    ctx.shadowBlur = isSelected ? 34 : isHovered ? 24 : 16;
    ctx.arc(node.x, node.y, node.radius, 0, Math.PI * 2);
    ctx.fill();
    ctx.shadowBlur = 0;
    if (isSelected || isHovered || node.type === 'brain' || node.type === 'map') {{
      ctx.strokeStyle = isSelected ? '#ffffff' : 'rgba(255,255,255,0.36)';
      ctx.lineWidth = isSelected ? 2.2 : 1.1;
      ctx.stroke();
      const label = node.title.length > 24 ? node.title.slice(0, 24) + '…' : node.title;
      ctx.fillStyle = 'rgba(233, 241, 255, 0.92)';
      ctx.font = `${{node.type === 'brain' ? 13 : 11}}px "Avenir Next", sans-serif`;
      ctx.fillText(label, node.x + node.radius + 8, node.y - node.radius - 2);
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

document.addEventListener('keydown', event => {{
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
