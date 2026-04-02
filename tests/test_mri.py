"""Tests for MRI graph building and HTML generation."""

from kluris.core.mri import build_graph, generate_mri_html


def _make_brain_with_neurons(tmp_path):
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")
    for lobe in ["arch", "std", "product"]:
        d = brain / lobe
        d.mkdir()
        (d / "map.md").write_text(f"---\nauto_generated: true\nparent: ../brain.md\n---\n# {lobe}\n", encoding="utf-8")
    # 5 neurons
    for i, (lobe, name) in enumerate([
        ("arch", "auth.md"), ("arch", "data.md"),
        ("std", "naming.md"), ("std", "review.md"),
        ("product", "roadmap.md"),
    ]):
        related = []
        if name == "auth.md":
            related = ["../std/naming.md"]
        content = (
            f"---\nparent: ./map.md\nrelated: {related}\n"
            f"tags: [t{i}]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# {name}\n"
        )
        (brain / lobe / name).write_text(content, encoding="utf-8")
    return brain


def test_graph_nodes(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    graph = build_graph(brain)
    assert len(graph["nodes"]) == 10


def test_graph_parent_edges(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    graph = build_graph(brain)
    parent_edges = [e for e in graph["edges"] if e["type"] == "parent"]
    assert len(parent_edges) >= 5  # at least neurons -> maps


def test_graph_related_edges(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    graph = build_graph(brain)
    related_edges = [e for e in graph["edges"] if e["type"] == "related"]
    assert len(related_edges) >= 1  # auth -> naming


def test_graph_inline_edges(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    # Add an inline link
    auth = brain / "arch" / "auth.md"
    auth.write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Auth\n\nSee [data flow](../arch/data.md)\n", encoding="utf-8"
    )
    graph = build_graph(brain)
    inline_edges = [e for e in graph["edges"] if e["type"] == "inline"]
    assert len(inline_edges) >= 1


def test_graph_ignores_invalid_frontmatter_links(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    auth = brain / "arch" / "auth.md"
    auth.write_text(
        "---\nparent: ../../outside/map.md\nrelated:\n  - ../../outside/other.md\n"
        "tags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n",
        encoding="utf-8",
    )

    graph = build_graph(brain)

    assert any(node["path"] == "arch/auth.md" for node in graph["nodes"])


def test_node_colors_by_lobe(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    graph = build_graph(brain)
    lobes = set(n.get("lobe", "") for n in graph["nodes"])
    assert "arch" in lobes
    assert "std" in lobes


def test_graph_nodes_include_metadata(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    auth = brain / "arch" / "auth.md"
    auth.write_text(
        (
            "---\nparent: ./map.md\nrelated: []\n"
            "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
            "# Auth\n\nAuthentication flow details.\n\nUses Keycloak.\n"
        ),
        encoding="utf-8",
    )
    graph = build_graph(brain)
    auth = next(node for node in graph["nodes"] if node["path"] == "arch/auth.md")
    assert auth["title"]
    assert isinstance(auth["tags"], list)
    assert "degree" in auth
    assert "Authentication flow details." in auth["content_preview"]
    assert auth["content_preview_truncated"] is False


def test_html_valid(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    assert output.exists()
    html = output.read_text()
    assert "<html" in html
    assert "</html>" in html


def test_html_no_cdn(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text()
    assert "unpkg.com" not in html
    assert "cdnjs" not in html


def test_html_has_search_and_details_ui(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert 'id="search-input"' in html
    assert 'id="details-panel"' in html
    assert "Search the brain" in html
    assert "Content preview" in html


def test_html_under_500kb(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    size_kb = output.stat().st_size / 1024
    assert size_kb < 500
