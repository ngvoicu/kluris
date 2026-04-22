"""Tests for MRI graph building and HTML generation."""

from kluris.core.mri import build_graph, generate_mri_html


def _make_brain_with_yaml_neurons(tmp_path):
    """Copy of the yaml-neurons fixture (per-file helper pattern)."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")
    (brain / "kluris.yml").write_text(
        "name: brain\ntype: product\n", encoding="utf-8"
    )

    lobe = brain / "projects"
    lobe.mkdir()
    (lobe / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n# Projects\n",
        encoding="utf-8",
    )
    (lobe / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated: [./openapi.yml]\ntags: [auth]\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n"
        "\nSee [the API](./openapi.yml) for details.\n",
        encoding="utf-8",
    )
    (lobe / "openapi.yml").write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# related: [./auth.md]\n"
        "# tags: [api, openapi]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-01\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
        "paths: {}\n",
        encoding="utf-8",
    )
    (lobe / "ci-config.yml").write_text(
        "name: ci\non: [push]\njobs:\n  build: {}\n",
        encoding="utf-8",
    )
    return brain


def test_build_graph_includes_opted_in_yaml_neurons(tmp_path):
    """`build_graph` must return nodes for opted-in yaml neurons with the
    `file_type: yaml` discriminator AND must exclude raw yaml + kluris.yml.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    graph = build_graph(brain)
    by_path = {n["path"]: n for n in graph["nodes"]}

    assert "projects/openapi.yml" in by_path
    openapi = by_path["projects/openapi.yml"]
    assert openapi["type"] == "neuron"
    assert openapi.get("file_type") == "yaml"
    assert openapi["title"] == "Payments API"

    assert "projects/auth.md" in by_path
    auth = by_path["projects/auth.md"]
    assert auth["type"] == "neuron"
    assert auth.get("file_type") == "markdown"

    # Opt-out and root config are invisible
    assert "projects/ci-config.yml" not in by_path
    assert "kluris.yml" not in by_path


def test_build_graph_markdown_to_yaml_creates_edge(tmp_path):
    """A markdown neuron with a link to a yaml neuron (via frontmatter
    `related:` or inline `[text](./file.yml)`) must produce an edge in
    build_graph's edge list. Dedup may collapse related+inline into one
    edge, so we assert on any edge between them, not specifically inline.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    graph = build_graph(brain)
    by_path = {n["path"]: n for n in graph["nodes"]}
    auth_id = by_path["projects/auth.md"]["id"]
    openapi_id = by_path["projects/openapi.yml"]["id"]

    edges_between = [
        e for e in graph["edges"]
        if e["source"] == auth_id and e["target"] == openapi_id
    ]
    assert len(edges_between) >= 1, (
        f"expected at least one edge auth.md -> openapi.yml, got: "
        f"{[e for e in graph['edges'] if e['source'] == auth_id or e['target'] == auth_id]}"
    )


def test_html_colors_yaml_neurons_with_periwinkle(tmp_path):
    """Generated MRI HTML must contain the yaml color constant and the
    colorForNode branch that emits it.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "#9ea9ff" in html
    assert "node.file_type === 'yaml'" in html


def test_html_connection_cards_omit_section_chip_when_same_folder(tmp_path):
    """Left-panel "Connected nodes" cards must drop the sublobe/lobe chip
    when the connection is in the same section as the currently-selected
    node — otherwise every card in a same-project cluster repeats the
    project name and hides the actual titles."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "const currentSection" in html
    assert "targetSection !== currentSection" in html


def test_html_nav_buttons_omit_parent_prefix_when_same_folder(tmp_path):
    """The modal's "Connected nodes" buttons must only prefix with the parent
    folder when it differs from the current node's parent. Otherwise every
    sibling in a single-project cluster reads "btb / X" and the prefix just
    hides the actual neuron name."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # New disambiguator check — label is prefixed only when parents differ.
    assert "parent && parent !== currentParent" in html
    # currentParent is computed from the open node's own path.
    assert "const currentParent" in html


def test_html_modal_link_regex_matches_yaml(tmp_path):
    """The modal's body-link regex must match .md, .yml, AND .yaml targets."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # The regex string in the JS source — the path portion accepts
    # .md/.yml/.yaml and may be followed by an optional #anchor or ?query.
    assert r"\.(md|yml|yaml)" in html
    assert "(#[^)" in html  # anchor group present
    assert "(\\?[^)" in html  # query group present


def test_html_search_placeholder_mentions_yaml(tmp_path):
    """The left-panel search input placeholder must acknowledge yaml as a
    search dimension."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "yaml" in html.lower()
    # Placeholder attribute specifically
    assert 'placeholder="Name, path, lobe, tag, or yaml"' in html


def test_html_searchtext_builder_includes_file_type(tmp_path):
    """The JS `searchText` builder in initializeNodes() must include
    `node.file_type` so searching "yaml" finds yaml neurons.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "node.file_type" in html


def test_html_under_500kb_with_yaml(tmp_path):
    """Adding yaml neurons should not regress the HTML-size gate."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    size_kb = output.stat().st_size / 1024
    assert size_kb < 500


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


# -- Inner sub-lobe (3-level nesting) tests ----------------------------------

def _make_brain_with_inner_sublobes(tmp_path):
    """Brain with 3-level nesting: projects/backend/endpoints/*.md.

    Mimics btb-sme layout where endpoints is an inner lobe inside a project
    sub-lobe, alongside sibling neurons (overview.md, database-schema.md).
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")

    # Top-level lobe
    projects = brain / "projects"
    projects.mkdir()
    (projects / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n# Projects\n",
        encoding="utf-8",
    )

    # 2nd-level sub-lobe: projects/backend
    backend = projects / "backend"
    backend.mkdir()
    (backend / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../map.md\n---\n# Backend\n",
        encoding="utf-8",
    )
    (backend / "overview.md").write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n# Backend Overview\n",
        encoding="utf-8",
    )
    (backend / "database-schema.md").write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n# Database Schema\n",
        encoding="utf-8",
    )

    # 3rd-level inner lobe: projects/backend/endpoints
    endpoints = backend / "endpoints"
    endpoints.mkdir()
    (endpoints / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../map.md\n---\n# Endpoints\n",
        encoding="utf-8",
    )
    for name in ["get-users.md", "post-users.md", "get-orders.md"]:
        (endpoints / name).write_text(
            "---\nparent: ./map.md\nrelated: []\ntags: [api]\n"
            "created: 2026-04-01\nupdated: 2026-04-01\n---\n"
            f"# {name.replace('.md', '').replace('-', ' ').title()}\n",
            encoding="utf-8",
        )
    return brain


def test_inner_sublobe_gets_distinct_sublobe_key(tmp_path):
    """Neurons at depth 3 (projects/backend/endpoints/) must get a different
    sublobe than neurons at depth 2 (projects/backend/).
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    graph = build_graph(brain)
    by_path = {n["path"]: n for n in graph["nodes"]}

    overview = by_path["projects/backend/overview.md"]
    get_users = by_path["projects/backend/endpoints/get-users.md"]

    assert overview["sublobe"] == "projects/backend"
    assert get_users["sublobe"] == "projects/backend/endpoints"
    assert overview["sublobe"] != get_users["sublobe"]


def test_inner_sublobe_map_has_own_sublobe(tmp_path):
    """The endpoints/map.md must have sublobe 'projects/backend/endpoints',
    not the same sublobe as backend/map.md. This prevents the title-overwrite
    bug where 'Endpoints' would replace 'Backend' as the sublobe name.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    graph = build_graph(brain)
    by_path = {n["path"]: n for n in graph["nodes"]}

    backend_map = by_path["projects/backend/map.md"]
    endpoints_map = by_path["projects/backend/endpoints/map.md"]

    assert backend_map["sublobe"] == "projects/backend"
    assert endpoints_map["sublobe"] == "projects/backend/endpoints"
    assert backend_map["sublobe"] != endpoints_map["sublobe"]


def test_inner_sublobe_html_generation_succeeds(tmp_path):
    """MRI HTML generation must not crash or regress with 3-level nesting."""
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    assert output.exists()
    html = output.read_text(encoding="utf-8")
    assert "<html" in html
    assert "</html>" in html
    size_kb = output.stat().st_size / 1024
    assert size_kb < 500


def test_inner_sublobe_sidebar_has_nested_tree(tmp_path):
    """Inner sublobes must render nested under their parent in the sidebar,
    not as flat siblings. The JS must use expandedSublobes state and the
    recursive renderSubTree helper.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # expandedSublobes state exists alongside expandedLobes
    assert "const expandedSublobes = new Set()" in html
    # Recursive tree builder
    assert "renderSubTree" in html
    # CSS hooks for nested sublobe groups
    assert ".sublobe-group" in html
    assert ".sublobe-card-wrap" in html


def test_inner_sublobe_visibility_cascades(tmp_path):
    """Hiding a parent sublobe must cascade to its inner sublobes.
    The click handler must use startsWith to toggle descendants.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Cascade logic: startsWith(key + '/') for toggling descendants
    assert "startsWith(child.key + '/')" in html or "startsWith(sub.key + '/')" in html


def test_inner_sublobe_reset_clears_expanded_sublobes(tmp_path):
    """The reset-view handler must clear expandedSublobes alongside the
    other state sets.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    assert "expandedSublobes.clear()" in html


def test_connections_collapsed_with_show_all(tmp_path):
    """The detail panel must show only a few connections by default and
    offer a 'Show all N' button when there are more.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # The limit constant and Show all button builder
    assert "CONN_LIMIT" in html
    assert "conn-show-all" in html
    # CSS for the button
    assert ".conn-show-all" in html


def test_inner_sublobe_no_canvas_hull(tmp_path):
    """3rd-level sublobes must NOT get their own hull or label on the canvas.
    They stay inside their parent 2nd-level sublobe hull. The skip is based
    on sublobe key depth (more than 2 segments).
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # The skip guard for inner sublobes in the canvas hull pass
    assert "sl.split('/').length > 2" in html


def test_inner_sublobe_included_in_parent_hull(tmp_path):
    """2nd-level sublobe hulls must include descendant nodes (3rd-level+)
    via startsWith matching. Physics must merge inner sublobes into their
    parent group so they cluster together.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Hull member filter uses startsWith to include descendants
    assert "n.sublobe.startsWith(sl + '/')" in html
    # Physics merges inner sublobes into parent group
    assert "physicsSublobeKey" in html


def test_modal_nav_collapsed_with_toggle(tmp_path):
    """Modal nav buttons must be collapsed to one row by default with a
    toggle button to expand when there are many connections.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # CSS + JS for toggle button
    assert ".modal-nav-toggle" in html
    # JS: NAV_COLLAPSE constant and renderNav toggle logic
    assert "NAV_COLLAPSE" in html
    assert "renderNav" in html
