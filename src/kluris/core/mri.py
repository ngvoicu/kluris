"""MRI visualization — build graph from brain and generate standalone HTML."""

from __future__ import annotations

import json
import re
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

        nodes.append({
            "id": i,
            "label": f.name,
            "path": rel,
            "lobe": lobe,
            "type": ntype,
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
            parent_resolved = str((f.parent / parent).resolve().relative_to(brain_path.resolve()))
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
                r_resolved = str((f.parent / r).resolve().relative_to(brain_path.resolve()))
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

    return {"nodes": nodes, "edges": edges}


def generate_mri_html(brain_path: Path, output_path: Path) -> dict:
    """Generate a standalone HTML visualization of the brain graph."""
    graph = build_graph(brain_path)
    graph_json = json.dumps(graph, indent=2)

    html = f"""\
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Kluris Brain MRI</title>
<style>
  body {{ margin: 0; font-family: -apple-system, system-ui, sans-serif; background: #0d1117; color: #c9d1d9; }}
  #info {{ position: fixed; top: 10px; left: 10px; z-index: 10; background: #161b22; padding: 12px; border-radius: 8px; font-size: 13px; }}
  #info h3 {{ margin: 0 0 8px; color: #58a6ff; }}
  canvas {{ display: block; }}
  .legend {{ display: flex; gap: 12px; margin-top: 8px; }}
  .legend span {{ display: flex; align-items: center; gap: 4px; }}
  .dot {{ width: 10px; height: 10px; border-radius: 50%; display: inline-block; }}
</style>
</head>
<body>
<div id="info">
  <h3>Brain MRI</h3>
  <div>Nodes: {len(graph["nodes"])} | Edges: {len(graph["edges"])}</div>
  <div class="legend">
    <span><span class="dot" style="background:#58a6ff"></span> map</span>
    <span><span class="dot" style="background:#3fb950"></span> neuron</span>
    <span><span class="dot" style="background:#f0883e"></span> brain</span>
    <span><span class="dot" style="background:#8b949e"></span> other</span>
  </div>
  <div id="selected" style="margin-top:8px;"></div>
</div>
<canvas id="c"></canvas>
<script>
const graph = {graph_json};
const colors = {{brain:'#f0883e',index:'#8b949e',glossary:'#8b949e',map:'#58a6ff',neuron:'#3fb950'}};
const edgeColors = {{parent:'#30363d',related:'#58a6ff',inline:'#484f58'}};
const canvas = document.getElementById('c');
const ctx = canvas.getContext('2d');
let W, H;
function resize() {{ W = canvas.width = innerWidth; H = canvas.height = innerHeight; }}
resize(); addEventListener('resize', resize);

// Simple force layout
const nodes = graph.nodes.map((n,i) => ({{...n, x: W/2 + (Math.random()-0.5)*400, y: H/2 + (Math.random()-0.5)*400, vx:0, vy:0}}));
const edges = graph.edges;

function tick() {{
  // Repulsion
  for (let i=0;i<nodes.length;i++) for (let j=i+1;j<nodes.length;j++) {{
    let dx=nodes[j].x-nodes[i].x, dy=nodes[j].y-nodes[i].y;
    let d=Math.sqrt(dx*dx+dy*dy)||1;
    let f=200/d;
    nodes[i].vx-=dx/d*f; nodes[i].vy-=dy/d*f;
    nodes[j].vx+=dx/d*f; nodes[j].vy+=dy/d*f;
  }}
  // Attraction
  for (const e of edges) {{
    let s=nodes[e.source],t=nodes[e.target];
    let dx=t.x-s.x,dy=t.y-s.y,d=Math.sqrt(dx*dx+dy*dy)||1;
    let f=(d-80)*0.01;
    s.vx+=dx/d*f; s.vy+=dy/d*f;
    t.vx-=dx/d*f; t.vy-=dy/d*f;
  }}
  // Center gravity
  for (const n of nodes) {{
    n.vx+=(W/2-n.x)*0.001; n.vy+=(H/2-n.y)*0.001;
    n.vx*=0.9; n.vy*=0.9;
    n.x+=n.vx; n.y+=n.vy;
  }}
}}

function draw() {{
  ctx.clearRect(0,0,W,H);
  // Edges
  for (const e of edges) {{
    const s=nodes[e.source],t=nodes[e.target];
    ctx.beginPath(); ctx.moveTo(s.x,s.y); ctx.lineTo(t.x,t.y);
    ctx.strokeStyle=edgeColors[e.type]||'#30363d';
    ctx.lineWidth=e.type==='parent'?1.5:1;
    if(e.type==='related') ctx.setLineDash([4,4]); else ctx.setLineDash([]);
    ctx.stroke(); ctx.setLineDash([]);
  }}
  // Nodes
  for (const n of nodes) {{
    ctx.beginPath();
    const r = n.type==='brain'?8:n.type==='map'?6:5;
    ctx.arc(n.x,n.y,r,0,Math.PI*2);
    ctx.fillStyle=colors[n.type]||'#8b949e';
    ctx.fill();
  }}
}}

function loop() {{ for(let i=0;i<3;i++) tick(); draw(); requestAnimationFrame(loop); }}
loop();

// Click to select
canvas.addEventListener('click', e => {{
  const rect=canvas.getBoundingClientRect();
  const mx=e.clientX-rect.left, my=e.clientY-rect.top;
  let closest=null, minD=20;
  for(const n of nodes) {{
    const d=Math.sqrt((n.x-mx)**2+(n.y-my)**2);
    if(d<minD) {{ minD=d; closest=n; }}
  }}
  document.getElementById('selected').textContent = closest ? `${{closest.path}} (${{closest.type}}, ${{closest.lobe}})` : '';
}});
</script>
</body>
</html>"""

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(html)

    return {"nodes": len(graph["nodes"]), "edges": len(graph["edges"])}
