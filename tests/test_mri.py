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
    # Short documents: full content matches preview (sans the "..." marker).
    assert "Authentication flow details." in auth["content_full"]
    assert "Uses Keycloak." in auth["content_full"]


def test_graph_nodes_include_full_content_when_truncated(tmp_path):
    """Long documents must keep the full body in content_full so the modal can show it."""
    brain = _make_brain_with_neurons(tmp_path)
    auth = brain / "arch" / "auth.md"
    long_body = "\n\n".join(
        f"Paragraph {i} explaining a deep authentication detail." for i in range(80)
    )
    auth.write_text(
        (
            "---\nparent: ./map.md\nrelated: []\n"
            "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
            f"# Auth\n\n{long_body}\n"
        ),
        encoding="utf-8",
    )
    graph = build_graph(brain)
    auth = next(node for node in graph["nodes"] if node["path"] == "arch/auth.md")

    # Preview is bounded and marked truncated.
    assert auth["content_preview_truncated"] is True
    assert auth["content_preview"].rstrip().endswith("...")

    # Full content keeps every paragraph and never has the truncation marker appended.
    assert "Paragraph 0 explaining" in auth["content_full"]
    assert "Paragraph 79 explaining" in auth["content_full"]
    assert not auth["content_full"].rstrip().endswith("...")
    # Title line is stripped from the body, just like the preview.
    assert not auth["content_full"].lstrip().startswith("# Auth")


def test_modal_uses_full_content(tmp_path):
    """The generated HTML's modal must read content_full so 'expand' shows the whole document."""
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "node.content_full" in html


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


def test_html_has_lobes_list_in_left_panel(tmp_path):
    """The left panel must include a Lobes section that mirrors the result-card style."""
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Section title and container
    assert ">Lobes<" in html
    assert 'id="lobes-list"' in html
    # CSS hooks for the new card style
    assert ".lobe-card" in html
    assert ".lobe-swatch" in html
    # JS renderer wired up at startup
    assert "function renderLobes" in html
    assert "renderLobes();" in html


def _make_brain_with_sublobes(tmp_path):
    """Brain where the 'projects' lobe has 4 sub-lobes (the 'too busy' case)."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")

    # Top-level lobe map
    projects = brain / "projects"
    projects.mkdir()
    (projects / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n# Projects\n", encoding="utf-8"
    )

    # 4 sub-lobes under projects, each with 2 neurons
    for sub in ["alpha", "beta", "gamma", "delta"]:
        d = projects / sub
        d.mkdir()
        (d / "map.md").write_text(
            f"---\nauto_generated: true\nparent: ../map.md\n---\n# {sub.title()}\n",
            encoding="utf-8",
        )
        for name in ["one.md", "two.md"]:
            (d / name).write_text(
                "---\nparent: ./map.md\nrelated: []\ntags: []\n"
                "created: 2026-04-01\nupdated: 2026-04-01\n---\n"
                f"# {sub}/{name}\n",
                encoding="utf-8",
            )
    return brain


def test_html_has_sublobes_collapsible_tree(tmp_path):
    """The Lobes section must support sub-lobes via a collapsible tree.

    Solves the 'too busy canvas' problem when one lobe has many sub-lobes
    by giving the user a navigation aid in the left panel.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # CSS hooks for the new sub-lobe tree style.
    # Cards stay flush-left; the caret floats on the right of cards that have
    # sublobes (no left-side alignment spacer that would shift the lobes).
    assert ".lobe-group" in html
    assert ".lobe-card-wrap" in html
    assert ".lobe-caret" in html
    assert ".sublobes-list" in html
    assert ".sublobe-card" in html
    assert ".sublobe-tick" in html

    # JS state + render path for sub-lobes
    assert "expandedLobes" in html
    assert "info.sublobes" in html

    # focusOnNode must distinguish sub-lobe map nodes (zoom to sublobe members,
    # not the whole lobe) so clicking a sublobe in the list zooms tightly.
    assert "n.sublobe === node.sublobe" in html


def test_focus_on_node_zooms_sublobe_members_only(tmp_path):
    """Sub-lobe map nodes must zoom to their own members, not the parent lobe."""
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # The fix: filter by sublobe when node.sublobe !== node.lobe
    assert "isSublobe" in html
    assert "n.sublobe === node.sublobe" in html
    assert "n.lobe === node.lobe" in html  # original branch still present for top-level lobes


def test_lobes_act_as_visibility_toggles(tmp_path):
    """Clicking a lobe in the left panel must toggle its visibility (multi-select),
    not activate a single-lobe filter. Each lobe and sub-lobe is an independent
    on/off switch, so the user can hide several at once to reduce clutter.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Independent hidden-sets for lobes and sublobes
    assert "const hiddenLobes = new Set()" in html
    assert "const hiddenSublobes = new Set()" in html
    # visibleNode() respects both sets
    assert "hiddenLobes.has(node.lobe)" in html
    assert "hiddenSublobes.has(node.sublobe)" in html
    # Dimmed visual state on hidden cards (replaces the old .active state)
    assert ".lobe-card.dimmed" in html
    assert ".sublobe-card.dimmed" in html
    # The old single-filter machinery must be gone
    assert "activeFilter" not in html
    # Reset clears both sets
    assert "hiddenLobes.clear()" in html
    assert "hiddenSublobes.clear()" in html


def test_lobes_have_anti_overlap_physics_and_auto_fit(tmp_path):
    """Lobe layout must have:
       1. A wider anchor ring (radius * 0.55 instead of the old 0.40)
       2. A pairwise lobe-centroid repulsion pass so hulls cannot overlap
       3. fitToFilteredNodes() / resetCamera() helpers wired into the
          lobe-card and sublobe-card click handlers, so applying a filter
          actually frames the result instead of leaving the user staring
          at off-screen nodes.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Bigger initial spacing
    # Elliptical anchor ring so lobes always sit inside the viewport
    assert "width * 0.36" in html
    assert "height * 0.36" in html
    # Lobe-vs-lobe centroid repulsion (the "never overlap" pass)
    assert "Push different lobes apart" in html
    assert "lobeCentroids.get(lobeKeys[i])" in html
    # Auto-fit helpers
    assert "function fitToFilteredNodes" in html
    assert "fitToFilteredNodes(true)" in html  # instant fit at startup


def test_sidebars_are_collapsible_and_long_names_dont_overflow(tmp_path):
    """Left and right panels must each have a collapse button + a matching
    floating expand button, and the lobes list must be constrained so long
    lobe/sublobe names cannot introduce a horizontal scrollbar in the panel.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Collapse + expand controls exist
    assert 'id="collapse-left"' in html
    assert 'id="collapse-right"' in html
    assert 'id="expand-left"' in html
    assert 'id="expand-right"' in html
    assert ".panel-collapse-btn" in html
    assert ".panel-expand-btn" in html

    # Grid overrides for the collapsed states
    assert ".shell.left-collapsed" in html
    assert ".shell.right-collapsed" in html

    # JS toggle wiring
    assert "function togglePanel" in html

    # Horizontal-overflow guards for long lobe / sublobe names
    assert "overflow-x: hidden" in html
    assert "grid-template-columns: minmax(0, 1fr)" in html


def test_search_chips_removed(tmp_path):
    """The glossary/neurons type-filter chips are gone -- search hits everything."""
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # All chip / type-filter machinery is removed
    assert 'id="type-filters"' not in html
    assert "renderFilters" not in html
    assert "TYPE_LABELS" not in html
    assert "activeTypes" not in html
    # Search input is still present and unchanged
    assert 'id="search-input"' in html
    assert "Search the brain" in html


def test_html_under_500kb(tmp_path):
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    size_kb = output.stat().st_size / 1024
    assert size_kb < 500
