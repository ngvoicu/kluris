"""``kluris mri`` parity against the v2.28.11 oracle (research-03 §1.11, §2.8).

Each scenario runs both binaries on identical copies of a brain and compares:

- the exit code, the JSON (``{ok, brains: [{name, output_path,
  preflight_fixes, nodes, edges}]}``) byte for byte, and the text by its
  wording;
- the brain tree after the dream preflight, as ``test_dream`` compares it
  (the decided differences of PAR-033..036 and PAR-008), the generated pages
  left out;
- the graph embedded in v3's page against v2's ``build_graph`` run on v3's
  tree as it is after the run (``tools/mri_oracle.py graph``): nodes as a set
  keyed by path (ids and order follow the walk, PAR-008), edges as a multiset
  of ``(source path, target path, type)``, ``meta`` exactly. v2 graphs files in
  directories the walker prunes and symlinks that leave the brain; those are
  taken out of v2's graph first (PAR-042);
- the page around the graph: v2's shell, CSS and JS byte for byte
  (``crates/kluris-core/assets/mri/`` equal ``tools/mri_oracle.py assets``),
  with the registered brain name (PAR-044) and ``</`` / ``<!--`` escaped in
  the graph JSON (PAR-043). On Windows v2 writes the page CRLF (PAR-045).
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from collections import Counter
from pathlib import Path

import pytest

from harness import IS_WINDOWS, PARITY_DIR, Sandbox, normalize, oracle_kluris, snapshot_tree
from runs import clear_runs, parse_exact, same_words, write_config
from test_dream import (
    D2, Step, assert_v2_traceback, commit, compare_trees, frozen, healthy_brain, init, is_pruned, neuron, register, rich_brain, run_steps,
    write, _registered_brains, changed,
)

MRI_ORACLE = PARITY_DIR / "tools" / "mri_oracle.py"
ASSETS = PARITY_DIR.parent / "crates" / "kluris-core" / "assets" / "mri"
PAGE = "brain-mri.html"


def oracle_python() -> Path:
    return oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")


def is_wsl(env: dict[str, str]) -> bool:
    """v2's ``_is_wsl`` (cli.py:1525) as the binaries run it: ``WSL_DISTRO_NAME``
    set in their environment, or "microsoft" in ``/proc/version``."""
    if env.get("WSL_DISTRO_NAME"):
        return True
    try:
        with open("/proc/version", encoding="utf-8") as f:
            return "microsoft" in f.read().lower()
    except OSError:
        return False


def open_link(stdout: str) -> str:
    """The link of the ``Open:`` line of ``kluris mri``'s text, up to the
    ``N nodes, M edges`` line (v2's Rich may wrap it over several lines)."""
    match = re.search(r"Open: (.*?)\n\s*\d+ nodes, \d+ edges", stdout, re.DOTALL)
    assert match, f"no Open line in {stdout!r}"
    return match.group(1).strip()


# --- the page -----------------------------------------------------------------


def split_page(html: str) -> tuple[str, str]:
    """The page with the graph JSON taken out, and that JSON."""
    start = html.index("const graph = ") + len("const graph = ")
    end = html.index(";\nconst BRAIN_NAME = ")
    return html[:start] + html[end:], html[start:end]


def escaped(text: str) -> str:
    return text.replace("</", "<\\/").replace("<!--", "\\u003c!--")


def counts(graph: dict) -> tuple[int, int, int]:
    lobes = len({n["lobe"] for n in graph["nodes"] if n["lobe"] and n["lobe"] != "root"})
    neurons = sum(1 for n in graph["nodes"] if n["type"] == "neuron")
    return lobes, neurons, len(graph["edges"])


def expected_shell(name: str, graph: dict) -> str:
    """v2's page without its graph JSON, for ``name`` and ``graph``, from the
    embedded assets."""
    shell = (ASSETS / "shell.html").read_text(encoding="utf-8")
    lobes, neurons, edges = counts(graph)
    values = {
        "brain_name": name, "lobe_count": str(lobes), "neuron_count": str(neurons), "edge_count": str(edges),
        "css": (ASSETS / "mri.css").read_text(encoding="utf-8"), "graph_json": "",
        "brain_name_json": escaped(json.dumps(name)), "js": (ASSETS / "mri.js").read_text(encoding="utf-8"),
    }
    return re.sub(r"\{\{(\w+)\}\}", lambda m: values[m.group(1)], shell)


def v3_page(html: bytes, name: str) -> dict:
    """Check v3's page around its graph and return the graph."""
    text = html.decode("utf-8")
    assert "\r" not in text, "v3 writes the page LF"
    rest, raw = split_page(text)
    graph = json.loads(raw)
    assert raw == escaped(json.dumps(graph, indent=2)), "v3's graph JSON is not json.dumps(indent=2) of its parse"
    assert rest == expected_shell(name, graph), "v3's page differs from v2's shell, CSS and JS"
    assert text.count("</script>") == 1
    return graph


def v2_page(html: bytes) -> tuple[str, dict]:
    text = html.decode("utf-8")
    if IS_WINDOWS:
        text = text.replace("\r\n", "\n")  # PAR-045
    rest, raw = split_page(text)
    return rest, json.loads(raw)


def oracle_graph(brain: Path) -> dict:
    out = subprocess.run([str(oracle_python()), "-I", str(MRI_ORACLE), "graph", str(brain)], capture_output=True)
    assert out.returncode == 0, out.stderr.decode("utf-8", "replace")
    return json.loads(out.stdout.decode("utf-8"))


def escapes_brain(brain: Path, rel: str) -> bool:
    try:
        (brain / rel).resolve().relative_to(brain.resolve())
        return False
    except ValueError:
        return True


def pruned_graph(graph: dict, brain: Path) -> dict:
    """v2's graph without the files the shared walker does not see (PAR-042):
    those under pruned directories and symlinks that resolve outside the
    brain; their edges go too, and degrees and edge counts follow."""
    keep = [n for n in graph["nodes"] if not is_pruned(n["path"]) and not escapes_brain(brain, n["path"])]
    ids = {n["id"] for n in keep}
    edges = [e for e in graph["edges"] if e["source"] in ids and e["target"] in ids]
    neighbours = {i: set() for i in ids}
    edge_counts = {"parent": 0, "related": 0, "inline": 0, "replaced_by": 0}
    for e in edges:
        neighbours[e["source"]].add(e["target"])
        neighbours[e["target"]].add(e["source"])
        edge_counts[e["type"]] += 1
    renumber = {n["id"]: i for i, n in enumerate(keep)}
    nodes = [{**n, "id": renumber[n["id"]], "degree": len(neighbours[n["id"]])} for n in keep]
    edges = [{**e, "source": renumber[e["source"]], "target": renumber[e["target"]]} for e in edges]
    return {"nodes": nodes, "edges": edges, "meta": {"edge_counts": edge_counts}}


def graph_key(graph: dict) -> tuple:
    paths = {n["id"]: n["path"] for n in graph["nodes"]}
    assert [n["id"] for n in graph["nodes"]] == list(range(len(graph["nodes"]))), "ids are list indexes"
    nodes = {n["path"]: {k: v for k, v in n.items() if k != "id"} for n in graph["nodes"]}
    assert len(nodes) == len(graph["nodes"]), "one node per path"
    edges = Counter((paths[e["source"]], paths[e["target"]], e["type"]) for e in graph["edges"])
    assert all(list(e) == ["source", "target", "type"] for e in graph["edges"])
    return nodes, edges, graph["meta"]


def assert_graphs(v3: dict, v2: dict) -> None:
    n3, e3, m3 = graph_key(v3)
    n2, e2, m2 = graph_key(v2)
    assert sorted(n3) == sorted(n2), f"node sets differ: only v3 {sorted(set(n3) - set(n2))}, only v2 {sorted(set(n2) - set(n3))}"
    bad = [f"{p}:\n  v3 {json.dumps(n3[p])[:1500]}\n  v2 {json.dumps(n2[p])[:1500]}" for p in n3 if n3[p] != n2[p]]
    assert not bad, "nodes differ:\n" + "\n".join(bad[:10])
    for p in n3:
        assert list(n3[p]) == list(n2[p]), f"{p}: key order differs"
    assert e3 == e2, f"edges differ: only v3 {e3 - e2}, only v2 {e2 - e3}"
    assert json.dumps(m3) == json.dumps(m2)


# --- running ------------------------------------------------------------------


def outputs(tree: dict[str, bytes]) -> dict[str, bytes]:
    return {rel: data for rel, data in tree.items() if rel.endswith(".html")}


def without_outputs(tree: dict[str, bytes]) -> dict[str, bytes]:
    return {rel: data for rel, data in tree.items() if not rel.endswith(".html")}


def check_run(sb: Sandbox, out: dict, i: int, brains: dict[str, str], *, json_out: bool = True,
              pruned_ok: bool = False, page_rel: dict[str, str] | None = None) -> dict:
    """Compare run ``i`` of ``out``: ``brains`` maps registered names to their
    work-relative dirs. Returns v3's JSON (or ``{}`` for text)."""
    o2, o3 = out["v2"][i], out["v3"][i]
    r2, r3 = o2.result, o3.result
    assert r3.code == r2.code == 0, f"exit v3 {r3.code} v2 {r2.code}\nv3: {r3.stdout}{r3.stderr}\nv2: {r2.stdout}{r2.stderr}"
    assert "Traceback" not in r2.stderr
    assert r3.stderr == r2.stderr == ""
    before = frozen(sb)
    compare_trees(without_outputs(r2.tree), without_outputs(r3.tree), without_outputs(before), pruned_ok=pruned_ok)
    v3_root = sb.root / "v3"
    expected_counts = {}
    for name, rel in brains.items():
        page = (page_rel or {}).get(name, f"{rel}/{PAGE}")
        assert page in r2.tree and page in r3.tree, f"no {page}"
        graph = v3_page(r3.tree[page], name)
        brain = v3_root / rel
        assert_graphs(graph, pruned_graph(oracle_graph(brain), brain))
        expected_counts[name] = (len(graph["nodes"]), len(graph["edges"]))
        # v2's own page: the same shell with v2's name; its graph, pruned, has
        # v3's counts.
        rest2, graph2 = v2_page(r2.tree[page])
        dir_name = Path(rel).name
        g2 = pruned_graph(graph2, sb.root / "v2" / rel)
        assert len(g2["nodes"]) == len(graph["nodes"]) and len(g2["edges"]) == len(graph["edges"])
        assert rest2 == expected_shell(dir_name, graph2).replace(escaped(json.dumps(dir_name)), json.dumps(dir_name))
    if not json_out:
        same_words(r2.stdout, r3.stdout, sb.root)
        return {}
    j2, j3 = parse_exact(r2.stdout, sb.root, "v2"), parse_exact(r3.stdout, sb.root, "v3")
    for entry in j2["brains"]:
        if entry["name"] in expected_counts:
            entry["nodes"], entry["edges"] = expected_counts[entry["name"]]
    assert json.dumps(j3) == json.dumps(j2), f"mri JSON differs\nv3 {json.dumps(j3)}\nv2 {json.dumps(j2)}"
    return j3


MRI = Step(["mri", "--json"])


def mri_files() -> dict[str, str]:
    """What the MRI shows beyond the dream fixture: an index, long and
    non-ASCII text, HTML that must not end the script, anchors, a deep
    neuron, inline links of every kind."""
    long_body = "\n".join(f"Line {i} of a long neuron." for i in range(1, 70))
    return {
        "index.md": "# Index\n\nSee [projects](./projects/map.md).\n",
        "projects/script.md": neuron(
            "Script", body="Inline </script><script>alert(1)</script> and <!--<script> and </style> here.",
        ),
        "projects/long-one.md": neuron("Long one", body=long_body + "\n\n" + "x" * 3000),
        # Dated as committed: v2 never syncs a non-ASCII name's dates (PAR-013).
        "projects/unicode-ăîș.md": neuron("Șir de caractere — ünïcødé 🚀", updated="'2026-02-03'",
                                          body="Ăla e un text cu diacritice."),
        "projects/anchors.md": neuron(
            "Anchors", body='<a id="top"></a>First <A ID=\'b\'> </a>line.\n\n## Sub\n\n<a name="n"></a>Kept.',
        ),
        "projects/links.md": neuron(
            "Links",
            extra="related:\n- ./script.md\n- ./script.md\n- ../nowhere.md\nreplaced_by: ./anchors.md\n",
            body="[s](./script.md) [l](./long-one.md#part) [q](long-one.md?x=1) [self](./links.md) "
                 "[web](https://example.com) [g](../glossary.md#jwt) [up](../index.md)",
        ),
        "projects/api/deeper/leaf.md": neuron("Leaf", parent="../map.md"),
    }


def mri_brain(sb: Sandbox, name: str = "rich") -> Path:
    brain = rich_brain(sb, name)
    write(brain, mri_files())
    commit(sb, brain, "mri files", D2)
    return brain


# --- assets -------------------------------------------------------------------


def test_assets_are_v2s(tmp_path, oracle):
    """The embedded shell, CSS and JS are exactly what v2.28.11's module
    holds (``_MRI_CSS``, ``_MRI_JS`` as Python evaluates them, and
    ``_render_mri_html`` with its values replaced by markers)."""
    out = subprocess.run([str(oracle_python()), "-I", str(MRI_ORACLE), "assets", str(tmp_path)], capture_output=True)
    assert out.returncode == 0, out.stderr
    for name in ("shell.html", "mri.css", "mri.js"):
        assert (ASSETS / name).read_bytes() == (tmp_path / name).read_bytes(), name


# --- scenarios ----------------------------------------------------------------


def test_mri_rich_brain(sandbox):
    brain = mri_brain(sandbox)
    register(sandbox, [brain])
    out = run_steps(sandbox, ["brains/rich"], [MRI, MRI, Step(["mri"])])
    first = check_run(sandbox, out, 0, {"rich": "brains/rich"})
    entry = first["brains"][0]
    assert entry["preflight_fixes"]["total"] > 0 and entry["nodes"] > 30
    check_run(sandbox, out, 1, {"rich": "brains/rich"})
    check_run(sandbox, out, 2, {"rich": "brains/rich"}, json_out=False)
    text = out["v3"][2].result.stdout
    assert "MRI complete — " in text and "  Open: file://" in text and " nodes, " in text
    page = out["v3"][0].result.tree["brains/rich/brain-mri.html"].decode("utf-8")
    v2_page_text = out["v2"][0].result.tree["brains/rich/brain-mri.html"].decode("utf-8")
    # The neuron's `</script>` ends v2's script early; it cannot end v3's.
    assert page.count("</script>") == 1 and v2_page_text.count("</script>") > 1
    assert "<!--" not in split_page(page)[1]


def test_mri_on_a_healthy_brain_matches_v2_byte_for_byte(sandbox):
    """Directory name = registered name and nothing to fix: the two pages
    differ only by the order of the graph's nodes and edges (PAR-008)."""
    brain = healthy_brain(sandbox)
    register(sandbox, [brain])
    out = run_steps(sandbox, ["brains/calm"], [MRI])
    j3 = check_run(sandbox, out, 0, {"calm": "brains/calm"})
    assert j3["brains"][0]["preflight_fixes"]["total"] > 0  # the first dream writes maps
    rest2, g2 = v2_page(out["v2"][0].result.tree["brains/calm/brain-mri.html"])
    rest3, raw3 = split_page(out["v3"][0].result.tree["brains/calm/brain-mri.html"].decode("utf-8"))
    assert rest3 == rest2
    assert_graphs(json.loads(raw3), g2)


def test_page_is_named_after_the_registered_brain(sandbox):
    """PAR-044: v2 names the page after the directory."""
    brain = healthy_brain(sandbox, name="Some Folder")
    register_as = "calm-brain"
    write_config(sandbox, [(register_as, str(brain), "fixture")])
    sandbox.freeze()
    out = run_steps(sandbox, ["brains/Some Folder"], [MRI])
    check_run(sandbox, out, 0, {register_as: "brains/Some Folder"})
    v2 = out["v2"][0].result.tree["brains/Some Folder/brain-mri.html"].decode("utf-8")
    v3 = out["v3"][0].result.tree["brains/Some Folder/brain-mri.html"].decode("utf-8")
    assert "<title>Some Folder Brain MRI</title>" in v2
    assert "<title>calm-brain Brain MRI</title>" in v3
    assert 'const BRAIN_NAME = "calm-brain";' in v3 and '<h1 class="brain-title">calm-brain</h1>' in v3


def test_output_relative_to_cwd_with_missing_parents(sandbox):
    brain = healthy_brain(sandbox)
    register(sandbox, [brain])
    target = "out dir/./deep//é page.html"
    out = run_steps(sandbox, ["brains/calm"], [Step(["mri", "--json", "--output", target]),
                                               Step(["mri", "--output", target])])
    page = "out dir/deep/é page.html"
    j3 = check_run(sandbox, out, 0, {"calm": "brains/calm"}, page_rel={"calm": page})
    assert j3["brains"][0]["output_path"] == str(Path(target))
    assert "brains/calm/brain-mri.html" not in out["v3"][0].result.tree
    check_run(sandbox, out, 1, {"calm": "brains/calm"}, json_out=False, page_rel={"calm": page})
    if is_wsl(sandbox.env):
        # Under WSL v2 links the page through `wslpath -w` as
        # file://wsl.localhost/<distro>/<path>, unencoded (cli.py:1587-1597),
        # and v3 does the same (mri/uri.rs). Both ran in the same `work/`, so
        # the links are equal; v2's Rich wraps long lines, so they are
        # compared without whitespace, and v3's own line exactly.
        v2_open, v3_open = open_link(out["v2"][1].result.stdout), open_link(out["v3"][1].result.stdout)
        assert v3_open.startswith("file://wsl.localhost/"), v3_open
        assert v3_open.endswith("/work/out dir/deep/é page.html"), v3_open
        assert re.sub(r"\s+", "", v3_open) == re.sub(r"\s+", "", v2_open), f"v3: {v3_open!r}\nv2: {v2_open!r}"
    else:
        assert "/work/out%20dir/deep/%C3%A9%20page.html" in out["v3"][1].result.stdout.replace("\n", "")


def test_brain_all(sandbox):
    rich = mri_brain(sandbox)
    calm = healthy_brain(sandbox)
    register(sandbox, [rich, calm])
    out = run_steps(sandbox, ["brains/rich", "brains/calm"], [Step(["mri", "--json", "--brain", "all"]),
                                                              Step(["mri", "--brain", "all"])])
    j3 = check_run(sandbox, out, 0, {"rich": "brains/rich", "calm": "brains/calm"})
    assert [b["name"] for b in j3["brains"]] == ["rich", "calm"]
    check_run(sandbox, out, 1, {"rich": "brains/rich", "calm": "brains/calm"}, json_out=False)


def test_one_brain_of_two_by_name(sandbox):
    rich = mri_brain(sandbox)
    calm = healthy_brain(sandbox)
    register(sandbox, [rich, calm])
    out = run_steps(sandbox, ["brains/rich", "brains/calm"], [Step(["mri", "--json", "--brain", "calm"])])
    j3 = check_run(sandbox, out, 0, {"calm": "brains/calm"})
    assert [b["name"] for b in j3["brains"]] == ["calm"]
    assert "brains/rich/brain-mri.html" not in out["v3"][0].result.tree


@pytest.mark.parametrize("json_flag", [True, False])
def test_output_with_brain_all_is_refused(sandbox, json_flag):
    rich = healthy_brain(sandbox, name="one")
    calm = healthy_brain(sandbox, name="two")
    register(sandbox, [rich, calm])
    args = ["mri", "--brain", "all", "--output", "x.html", *(["--json"] if json_flag else [])]
    out = run_steps(sandbox, ["brains/one", "brains/two"], [Step(args)])
    r2, r3 = out["v2"][0].result, out["v3"][0].result
    assert (r3.code, r3.stdout, r3.stderr) == (r2.code, r2.stdout, r2.stderr)
    assert r3.code == 1 and "--output cannot be combined with multi-brain fan-out" in r3.stdout + r3.stderr
    assert r3.tree == frozen(sandbox)


def test_output_with_one_brain_and_brain_all_is_allowed(sandbox):
    brain = healthy_brain(sandbox)
    register(sandbox, [brain])
    out = run_steps(sandbox, ["brains/calm"], [Step(["mri", "--json", "--brain", "all", "--output", "one.html"])])
    check_run(sandbox, out, 0, {"calm": "brains/calm"}, page_rel={"calm": "one.html"})


def test_no_brains(sandbox):
    write_config(sandbox, [])
    sandbox.freeze()
    out = run_steps(sandbox, [], [Step(["mri", "--json"]), Step(["mri"])])
    for i in range(2):
        r2, r3 = out["v2"][i].result, out["v3"][i].result
        assert (r3.code, r3.stdout, r3.stderr) == (r2.code, r2.stdout, r2.stderr)
        assert r3.code == 1


def test_pruned_directories_and_escaping_symlinks(sandbox, tmp_path):
    """PAR-042: v2 graphs dot directories, ``node_modules`` and symlinks that
    leave the brain; v3 does not (and its preflight writes no map.md there,
    PAR-036)."""
    brain = healthy_brain(sandbox)
    write(brain, {
        ".archive/old.md": neuron("Old archived", body="[gamma](../knowledge/gamma.md)"),
        "node_modules/pkg/readme-like.md": "# pkg\n",
        "projects/.hidden/note.md": "# Hidden\n",
        "projects/linked.md": neuron("Linked", body="[archived](../.archive/old.md)"),
    })
    if not IS_WINDOWS:
        outside = tmp_path / "outside"
        outside.mkdir()
        (outside / "secret.md").write_text("# Secret\n", encoding="utf-8")
        os.symlink(outside / "secret.md", brain / "projects" / "leak.md")
        os.symlink("linked.md", brain / "projects" / "alias.md")
    commit(sandbox, brain, "pruned", D2)
    register(sandbox, [brain])
    out = run_steps(sandbox, ["brains/calm"], [MRI])
    check_run(sandbox, out, 0, {"calm": "brains/calm"}, pruned_ok=True)
    g3 = json.loads(split_page(out["v3"][0].result.tree["brains/calm/brain-mri.html"].decode("utf-8"))[1])
    _, g2 = v2_page(out["v2"][0].result.tree["brains/calm/brain-mri.html"])
    p3, p2 = {n["path"] for n in g3["nodes"]}, {n["path"] for n in g2["nodes"]}
    assert ".archive/old.md" in p2 and ".archive/old.md" not in p3
    assert "node_modules/pkg/readme-like.md" in p2 and "node_modules/pkg/readme-like.md" not in p3
    if not IS_WINDOWS:
        assert "projects/leak.md" in p2 and "projects/leak.md" not in p3
        assert "projects/alias.md" in p3


def test_wsl_prints_the_windows_link(sandbox, tmp_path):
    """Under WSL (``WSL_DISTRO_NAME``) both ask ``wslpath -w`` for the page and
    print a ``file://wsl.localhost/...`` link; when it fails they fall back to
    the ``file:///`` link."""
    if IS_WINDOWS:
        pytest.skip("wslpath is a Linux tool")
    brain = healthy_brain(sandbox)
    register(sandbox, [brain])
    bin_dir = sandbox.root / "fakebin"
    bin_dir.mkdir()
    fake = bin_dir / "wslpath"
    fake.write_text(r"""#!/bin/sh
[ "$1" = "-w" ] || exit 2
[ -n "$FAIL_WSLPATH" ] && exit 1
printf '%s\n' '\\wsl.localhost\Ubuntu\home\me\brain-mri.html'
""", encoding="utf-8")
    fake.chmod(0o755)
    path = f"{bin_dir}{os.pathsep}{sandbox.env['PATH']}"
    for env, expected in (({"WSL_DISTRO_NAME": "Ubuntu", "PATH": path}, "Open: file://wsl.localhost/Ubuntu/home/me/brain-mri.html"),
                          ({"WSL_DISTRO_NAME": "Ubuntu", "PATH": path, "FAIL_WSLPATH": "1"}, "Open: file:///")):
        out = run_steps(sandbox, ["brains/calm"], [Step(["mri"])], env=env)
        check_run(sandbox, out, 0, {"calm": "brains/calm"}, json_out=False)
        assert expected in out["v3"][0].result.stdout, out["v3"][0].result.stdout
        clear_runs(sandbox)


def test_mri_help_wording(sandbox):
    write_config(sandbox, [])
    sandbox.freeze()
    out = run_steps(sandbox, [], [Step(["mri", "--help"])])
    v2, v3 = out["v2"][0].result, out["v3"][0].result
    assert v2.code == v3.code == 0
    for text in ("Usage: kluris mri [OPTIONS]", "Generate interactive brain visualization.", "--brain", "Specific brain",
                 "--output", "Output HTML file path", "--json", "JSON output"):
        assert text in v2.stdout, f"v2 no longer says {text!r}"
        assert text in v3.stdout, text


def test_mri_is_in_the_root_help(sandbox):
    """PAR-025: ``kluris --help`` lists ``mri`` once it is ported."""
    write_config(sandbox, [])
    sandbox.freeze()
    out = run_steps(sandbox, [], [Step(["--help"])])
    line = next(l for l in out["v2"][0].result.stdout.splitlines() if l.strip().startswith("mri "))
    assert re.sub(r"\s+", " ", line).strip() in re.sub(r"[ \t]+", " ", out["v3"][0].result.stdout)


# --- the brains registered on this machine -----------------------------------------


def test_registered_brains(tmp_path, oracle):
    """Copies (with ``.git``) of every registered brain: the preflight writes
    what v2's would but for the decided differences, and v3's graph is v2's
    ``build_graph`` of the result without the pruned files. Prints, per
    brain, the files each preflight changed and the graph sizes."""
    brains = _registered_brains()
    if not brains:
        pytest.skip("no brains registered with an installed kluris on this machine")
    failures, report = [], []
    for name, source, description in brains:
        sb = Sandbox(tmp_path / name)
        copy = sb.work / "brains" / name
        shutil.copytree(source, copy, symlinks=True)
        write_config(sb, [(name, str(copy), description)])
        sb.freeze()
        before = frozen(sb)
        rel = f"brains/{name}"
        out = run_steps(sb, [rel], [MRI])
        r2, r3 = out["v2"][0].result, out["v3"][0].result
        n2 = changed(without_outputs(before), without_outputs(r2.tree))
        n3 = changed(without_outputs(before), without_outputs(r3.tree))
        try:
            j3 = check_run(sb, out, 0, {name: rel}, pruned_ok=True)
            entry = j3["brains"][0]
            _, g2 = v2_page(r2.tree[f"{rel}/{PAGE}"])
            report.append(f"{name}: preflight changed v2 {len(n2)} / v3 {len(n3)} files; graph v3 "
                          f"{entry['nodes']} nodes {entry['edges']} edges, v2 {len(g2['nodes'])} nodes "
                          f"{len(g2['edges'])} edges")
        except AssertionError as exc:
            failures.append(f"{name}: {exc}")
    print("\n" + "\n".join(report))
    assert not failures, "\n".join(failures)


# --- write safety (review T-103) -------------------------------------------------


def test_preflight_skipped_files_are_reported(sandbox):
    """PAR-065 (with PAR-046): v2's preflight raises on a neuron it cannot
    read; v3 leaves it alone, writes the page and reports it as dream does:
    ``skipped`` after v2's keys in its brain's entry, and dream's stderr line.
    A brain with nothing skipped keeps v2's shape."""
    calm = healthy_brain(sandbox)
    write(calm, {"projects/bad.md": "---\ntitle: Foo: bar\n---\n\n# Bad\n"})
    commit(sandbox, calm, "bad", D2)
    rich = mri_brain(sandbox)
    register(sandbox, [calm, rich])
    before = frozen(sandbox)
    out = run_steps(sandbox, ["brains/calm", "brains/rich"], [Step(["mri", "--json", "--brain", "all"]),
                                                              Step(["mri", "--brain", "all"])])
    assert_v2_traceback(out["v2"][0], "line 2, column 11")
    r3 = out["v3"][0].result
    assert r3.code == 0, r3.stderr
    assert r3.tree["brains/calm/projects/bad.md"] == before["brains/calm/projects/bad.md"]
    doc = parse_exact(r3.stdout, sandbox.root, "v3")
    calm_entry, rich_entry = doc["brains"]
    assert list(calm_entry) == ["name", "output_path", "preflight_fixes", "nodes", "edges", "skipped"]
    assert [list(s) for s in calm_entry["skipped"]] == [["brain", "file", "error"]]
    assert calm_entry["skipped"][0]["file"] == "projects/bad.md"
    assert calm_entry["skipped"][0]["error"].startswith("invalid frontmatter: ")
    assert list(rich_entry) == ["name", "output_path", "preflight_fixes", "nodes", "edges"]
    text = out["v3"][1].result
    assert text.code == 0
    assert text.stderr.startswith("Skipped calm: projects/bad.md: invalid frontmatter: "), text.stderr
    assert len(text.stderr.splitlines()) == 1
