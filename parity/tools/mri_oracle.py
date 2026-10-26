"""The MRI page assets of v2.28.11, taken from the oracle's own module.

Run with the oracle venv's Python (``-I``)::

    python -I mri_oracle.py assets OUT_DIR
    python -I mri_oracle.py graph BRAIN_DIR

``assets`` writes ``shell.html``, ``mri.css`` and ``mri.js``: ``_MRI_CSS`` and ``_MRI_JS``
as Python evaluates them, and the page ``_render_mri_html`` builds with every
interpolated value replaced by a ``{{marker}}`` (``brain_name``,
``lobe_count``, ``neuron_count``, ``edge_count``, ``css``, ``graph_json``,
``brain_name_json``, ``js``). Kluris 3 embeds the three files
(``crates/kluris-core/assets/mri/``) and fills the markers.

``graph`` prints ``json.dumps(build_graph(BRAIN_DIR), indent=2)``: v2's graph
of a brain as it is on disk (no preflight).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from kluris.core import mri

NAME = "SENTINELBRAINNAME"
GRAPH = "SENTINELGRAPHJSON"
CSS = "SENTINELCSS"
JS = "SENTINELJS"


def shell() -> str:
    # 111 lobes, 222 neurons, 333 edges: counts that occur nowhere else.
    nodes = [{"type": "neuron", "lobe": f"l{i % 111}"} for i in range(222)]
    graph = {"nodes": nodes, "edges": [{}] * 333}
    saved = mri._MRI_CSS, mri._MRI_JS
    mri._MRI_CSS, mri._MRI_JS = CSS, JS
    try:
        html = mri._render_mri_html(NAME, graph, GRAPH)
    finally:
        mri._MRI_CSS, mri._MRI_JS = saved
    assert "{{" not in html and "}}" not in html
    swaps = [
        (json.dumps(NAME), "{{brain_name_json}}", 1),
        (NAME, "{{brain_name}}", 3),
        ('id="stat-lobes">111<', 'id="stat-lobes">{{lobe_count}}<', 1),
        ('id="stat-neurons">222<', 'id="stat-neurons">{{neuron_count}}<', 1),
        ('id="stat-synapses">333<', 'id="stat-synapses">{{edge_count}}<', 1),
        (CSS, "{{css}}", 1),
        (GRAPH, "{{graph_json}}", 1),
        (JS, "{{js}}", 1),
    ]
    for old, new, count in swaps:
        assert html.count(old) == count, (old, html.count(old))
        html = html.replace(old, new)
    for marker in ("111", "222", "333"):
        assert marker not in html, marker
    return html


def main() -> None:
    if len(sys.argv) == 3 and sys.argv[1] == "graph":
        sys.stdout.buffer.write(json.dumps(mri.build_graph(Path(sys.argv[2])), indent=2).encode("utf-8"))
        return
    if len(sys.argv) != 3 or sys.argv[1] != "assets":
        sys.exit("usage: mri_oracle.py assets OUT_DIR | graph BRAIN_DIR")
    out = Path(sys.argv[2])
    out.mkdir(parents=True, exist_ok=True)
    for name, text in (("shell.html", shell()), ("mri.css", mri._MRI_CSS), ("mri.js", mri._MRI_JS)):
        (out / name).write_bytes(text.encode("utf-8"))


if __name__ == "__main__":
    main()
