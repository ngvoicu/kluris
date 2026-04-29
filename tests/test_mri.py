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


def test_title_is_filename_stem_not_h1(tmp_path):
    """Neuron `title` is derived from the filename (hyphens → spaces,
    title-cased) regardless of the H1 the author wrote. The H1 survives
    only in `authored_title`, and only when it differs from the filename-
    derived title — so an H1 like `# emailback - architecture` doesn't
    pollute compact labels across the MRI."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\n---\n# Brain\n", encoding="utf-8")
    (brain / "projects").mkdir()
    (brain / "projects" / "emailback").mkdir()
    (brain / "projects" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# Projects\n", encoding="utf-8"
    )
    (brain / "projects" / "emailback" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# Emailback\n", encoding="utf-8"
    )
    (brain / "projects" / "emailback" / "architecture.md").write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# emailback - architecture\n\nThe service uses...\n",
        encoding="utf-8",
    )
    graph = build_graph(brain)
    node = next(n for n in graph["nodes"] if n["path"] == "projects/emailback/architecture.md")
    # Title is the filename stem, NOT the redundant H1 prefix
    assert node["title"] == "Architecture"
    # Authored H1 is preserved separately for the modal subtitle
    assert node["authored_title"] == "emailback - architecture"


def test_authored_title_hidden_when_same_as_stem(tmp_path):
    """If the H1 is the same text the filename would already produce,
    authored_title stays empty — no redundant subtitle."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\n---\n# Brain\n", encoding="utf-8")
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    (brain / "knowledge" / "auth-flow.md").write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Auth Flow\n\nHow auth works.\n",
        encoding="utf-8",
    )
    graph = build_graph(brain)
    node = next(n for n in graph["nodes"] if n["path"] == "knowledge/auth-flow.md")
    assert node["title"] == "Auth Flow"
    assert node["authored_title"] == ""


def test_map_nodes_use_h1_as_title(tmp_path):
    """map.md / brain.md / glossary.md nodes must use their H1 as the
    display title — not the filename stem, because every lobe's map
    file is literally `map.md` and the stem would collapse every lobe
    to the same label ("Map") in the left sidebar."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# My Brain\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text(
        "---\n---\n# Shared Glossary\n", encoding="utf-8"
    )
    (brain / "infrastructure").mkdir()
    (brain / "infrastructure" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# Infrastructure\n", encoding="utf-8"
    )
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# Knowledge\n", encoding="utf-8"
    )
    graph = build_graph(brain)
    by_path = {n["path"]: n for n in graph["nodes"]}
    assert by_path["brain.md"]["title"] == "My Brain"
    assert by_path["glossary.md"]["title"] == "Shared Glossary"
    assert by_path["infrastructure/map.md"]["title"] == "Infrastructure"
    assert by_path["knowledge/map.md"]["title"] == "Knowledge"
    # authored_title is not double-surfaced as a subtitle for these nodes
    assert by_path["infrastructure/map.md"]["authored_title"] == ""


def test_modal_file_tree_includes_glossary(tmp_path):
    """The modal's left file tree used to filter to `type === 'neuron'`
    only, which hid glossary.md — a real file users want to open from
    the tree. Widen the filter to include glossary. brain.md / index.md
    stay out because they're auto-generated."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\n---\n# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text(
        "---\n---\n# Glossary\n\nTerms.\n", encoding="utf-8"
    )
    (brain / "projects").mkdir()
    (brain / "projects" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# Projects\n", encoding="utf-8"
    )
    output = tmp_path / "mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Filter must include glossary; the exact string "'glossary'" appears
    # inside the buildFileTree filter.
    assert "n.type === 'neuron' || n.type === 'glossary'" in html


def test_empty_html_anchors_stripped_from_content_preview(tmp_path):
    """Glossary files use `<a id="term"></a>term` jump-target pairs so
    links like `glossary.md#term` resolve in browsers. The MRI preview
    `escapeHtml`s content before rendering, which surfaces those tags as
    literal markup noise. Strip the empty-anchor pattern from the preview
    and excerpt so only the human-readable text remains."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\n---\n# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text(
        "---\n---\n# Glossary\n\n"
        "Project-specific terms.\n\n"
        "| Term | Meaning |\n"
        "|------|---------|\n"
        '| <a id="jwt"></a>JWT | JSON Web Token. |\n'
        '| <a id="base-template"></a>base template | An `email_template` row. |\n',
        encoding="utf-8",
    )
    graph = build_graph(brain)
    node = next(n for n in graph["nodes"] if n["path"] == "glossary.md")
    assert "<a id=" not in node["content_preview"]
    assert "<a id=" not in node["content_full"]
    assert "<a id=" not in node["excerpt"]
    assert "JWT | JSON Web Token." in node["content_preview"]
    assert "base template | An" in node["content_preview"]


def test_yaml_neuron_prefers_frontmatter_title(tmp_path):
    """Yaml neurons keep using the frontmatter `title:` as the display
    title (filename stems like `openapi.yml` title-case poorly)."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    graph = build_graph(brain)
    node = next(n for n in graph["nodes"] if n["path"] == "projects/openapi.yml")
    assert node["title"] == "Payments API"
    assert node["authored_title"] == ""


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
    """The modal's body-link rewire pass must match .md, .yml, AND .yaml targets.

    After rendering markdown to HTML the modal walks every ``a.md-link``
    and rewires the ones whose href ends in a neuron suffix into a
    ``content-link`` (or ``content-link-broken`` if the target isn't in
    the graph). The behavioral assertions:

    - the suffix gate accepts .md, .yml, and .yaml
    - the rewire strips both ``#anchor`` and ``?query`` before resolving
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Suffix gate in the rewire pass.
    assert r"\.(md|ya?ml)$" in html
    # Anchor + query stripped from the href before resolving the target.
    assert "split('#')[0].split('?')[0]" in html
    # The rewire pass walks ``a.md-link[data-md-link]`` produced by the
    # markdown renderer.
    assert "a.md-link[data-md-link]" in html


def test_html_search_placeholder_mentions_yaml(tmp_path):
    """The right-panel search input placeholder must acknowledge yaml as a
    search dimension so users discover that yaml neurons are searchable."""
    brain = _make_brain_with_yaml_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "yaml" in html.lower()
    # The exact placeholder string in the new design.
    assert 'placeholder="Search neurons, path, lobe, tag, or yaml"' in html


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


def test_html_has_search_and_file_tree_ui(tmp_path):
    """The shell must include search and the left-sidebar file tree.

    The new C4 design keeps the same DOM hooks (search input, file tree,
    results panel) but the right sidebar gets restyled to the flat
    `FIND` / `LOBES` / `RECENT` / `RESULTS` layout from the mockups.
    """
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Kept: search input, lobes filter, results panel, panel tree.
    assert 'id="search-input"' in html
    assert 'id="lobes-list"' in html
    assert 'id="search-results"' in html
    assert 'id="panel-tree"' in html
    assert 'id="recent-list"' in html
    # Right-sidebar section labels (uppercase via CSS) — assert on
    # the literal HTML text in the section headers.
    assert ">Find<" in html
    assert ">Lobes<" in html
    assert ">Recent<" in html
    # Removed: every inspector / details / connections-card hook.
    assert 'id="details-panel"' not in html
    assert 'id="details-empty"' not in html
    assert 'id="nav-back"' not in html
    assert 'id="nav-forward"' not in html
    assert 'id="nav-expand"' not in html
    assert "Content preview" not in html
    assert 'class="content-preview"' not in html
    assert 'class="connection-card"' not in html
    assert 'class="details-card"' not in html
    assert "Connected nodes" not in html


def test_html_has_lobes_list_in_left_panel(tmp_path):
    """The right panel includes a Lobes filter rendered as flat 4×22 swatch rows.

    The C4 redesign drops the old gradient lobe-card; the right-sidebar
    `LOBES` section now uses flat `.lobe-row` rows with a 4×22 color swatch,
    a label, a count, and a visibility dot — matching the mockup-shell.html
    contract.
    """
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # Section + container
    assert ">Lobes<" in html
    assert 'id="lobes-list"' in html
    # New flat row style replaces .lobe-card / .lobe-swatch.
    assert ".lobe-row" in html
    assert ".swatch" in html
    # JS renderer wired up at startup
    assert "function renderLobeFilter" in html
    assert "renderLobeFilter();" in html


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


def test_lobes_act_as_visibility_toggles(tmp_path):
    """Clicking a lobe in the right panel toggles visibility for that lobe.

    The C4 redesign keeps the multi-select lobe filter (as 4×22 swatch rows
    in the right sidebar) but drops the per-sublobe collapsible tree from
    the right sidebar — sublobes are reachable by drilling into a lobe (L2)
    rather than via a tree filter.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    # The hidden-lobes set still exists and powers the L1 search dim path.
    assert "const hiddenLobes = new Set()" in html
    # The dimmed style applies to hidden lobe rows.
    assert ".lobe-row" in html
    assert ".dimmed" in html
    # Reset clears the set.
    assert "hiddenLobes.clear()" in html


def test_sidebars_are_collapsible(tmp_path):
    """Left and right panels collapse via the panel-header buttons; floating
    expand buttons live at the screen edge and toggle them back on.
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

    # Body grid overrides for the collapsed states (left-collapsed / right-collapsed)
    assert ".body.left-collapsed" in html
    assert ".body.right-collapsed" in html

    # JS toggle wiring
    assert "function togglePanel" in html


def test_mri_starts_at_brain_root_with_lobes_and_top_files(tmp_path):
    """MRI starts at the brain root (currentPath = []) and the root view
    shows BOTH top-level lobe folders AND top-level files (glossary.md,
    brain.md). No more separate 'brain'/'lobe'/'sublobe' stage modes —
    a single path-based view drills arbitrarily deep.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Path-based navigation primitives are present
    assert "let currentPath = []" in html
    assert "function navigateTo" in html
    assert "function childrenOf" in html
    assert "function drawCurrent" in html

    # Old mode-based dispatch must be gone
    assert "let stageMode" not in html
    assert "function drawBrainMap" not in html
    assert "function drawLobeMap" not in html
    assert "function drawSublobeMap" not in html
    assert 'data-stage-mode' not in html


def test_root_includes_top_level_files_alongside_lobes(tmp_path):
    """The root view (currentPath === []) renders top-level files (e.g.
    glossary.md, brain.md) as siblings of the lobe folders. Folders and
    files coexist as children at every level.
    """
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # The childrenOf primitive walks node.path to collect direct children
    # — both folders and leaves.
    assert "function childrenOf" in html
    # Folder kicker switches between LOBE (depth 0) and SUBLOBE (deeper)
    assert "'LOBE'" in html
    assert "'SUBLOBE'" in html
    # Glossary kicker stays distinct so root-level glossary.md renders as GLOSSARY
    assert "'GLOSSARY'" in html


def test_lobe_with_direct_neurons_shows_neurons_not_empty_index(tmp_path):
    """A lobe without sublobes must still show its direct neurons.

    `map.md` is structural inside a folder, so it should not become the only
    visible INDEX card and make the level feel empty.
    """
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    assert "map.md is structural inside a folder" in html
    assert "fallback.length" in html
    assert "n.type === 'neuron' && n.path.startsWith(prefix)" in html


def test_layout_never_single_row_for_three_or_more_items(tmp_path):
    """layoutGrid must never lay >=3 items in a single horizontal row or
    switch back to a radial orbit.

    The fixture brain has 3 lobes (arch, std, product); we assert that the
    layout function explicitly branches on item count and uses an adaptive
    grid for n >= 3, rather than the 1..4 single-row cell-spread or radial
    orbit that made large folders unreadable.
    """
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # The single-row branch covers only n === 1 (centered) and n === 2
    # (horizontal pair). n >= 3 must use a multi-row grid or radial.
    assert "function layoutGrid" in html
    # Critical guards: no `n <= 4` single-row regression and no radial-orbit
    # fallback for big folders.
    assert "n <= 4" not in html
    assert "radial orbit" not in html
    assert "Math.cos(angle)" not in html
    # The adaptive grid branch points must be present.
    assert "n === 1" in html
    assert "n === 2" in html
    assert "density = n > 60 ? 'tiny'" in html
    assert "worldH" in html


def test_large_folder_layout_uses_compact_grid_and_fit(tmp_path):
    """Large brains need a flexible browser layout: compact cards, capped
    aggregate edges, no outbound strip, and a real fit-to-current-view
    helper instead of forcing everything into the viewport at scale=1.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    assert "function fitCurrentToView" in html
    assert "fitCurrentToView(true)" in html
    assert "fitCurrentToView(false)" in html
    assert "function visibleAggregateEdges" in html
    assert "itemCount > 10 ? 0" in html
    assert "const maxCols = n > 12 ? 4 : 6" in html
    assert "function drawOutboundStubs" not in html
    assert "currentOutbound" not in html


def test_header_has_back_and_forward_path_history(tmp_path):
    """The header has browser-like Back / Forward buttons for path history.

    Breadcrumbs still jump to a parent path; the dedicated buttons walk
    navigation history so users can return after drilling into a lobe.
    """
    brain = _make_brain_with_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    assert 'id="btn-back"' in html
    assert 'id="btn-forward"' in html
    assert "let pathHistory = [[]]" in html
    assert "function goPathBack" in html
    assert "function goPathForward" in html
    assert "pathHistoryIndex < pathHistory.length - 1" in html


def test_nested_sublobes_drill_arbitrarily_deep(tmp_path):
    """Drilling into a folder pushes its name onto currentPath; the same
    primitive (childrenOf + drawCurrent) renders any depth.

    Fixture: projects/backend/endpoints/* (3 levels deep).
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Path-based navigation is the only model — it works for any depth.
    assert "function navigateTo" in html
    # Click on a folder pushes its name
    assert "[...currentPath" in html
    # childrenOf walks node.path parts, agnostic to depth
    assert "function childrenOf" in html
    # path.length comparison is the depth predicate (no hardcoded depth-2 cap)
    assert "parts.length === path.length + 1" in html


def test_layered_neuron_layout_is_deterministic(tmp_path):
    """Two MRI runs over the same brain must produce byte-identical HTML.

    layoutGrid + the path-based navigation are pure functions of (graph,
    currentPath, viewport). Stable JSON encoding + deterministic ordering
    keeps snapshots stable across runs.
    """
    brain = _make_brain_with_inner_sublobes(tmp_path)
    out1 = tmp_path / "first.html"
    out2 = tmp_path / "second.html"
    generate_mri_html(brain, out1)
    generate_mri_html(brain, out2)
    h1 = out1.read_text(encoding="utf-8")
    h2 = out2.read_text(encoding="utf-8")
    assert h1 == h2, "MRI HTML must be deterministic across runs"


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
    # Search input is still present
    assert 'id="search-input"' in html


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


# ======================================================================
# C4 redesign (v2.17.0) — Phase 1..5 acceptance tests
# ======================================================================

def _make_brain_with_cross_lobe_synapses(tmp_path):
    """Brain with 3 lobes (projects, infrastructure, knowledge) and known
    cross-lobe synapse counts so the L1 brain map renders aggregate edges.

    - projects/auth.md ↔ infrastructure/keycloak.md  (related)
    - projects/auth.md → knowledge/jwt.md            (related)
    - infrastructure/keycloak.md ↔ knowledge/jwt.md  (related)
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text("---\n---\n# Brain\n", encoding="utf-8")
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")

    for lobe_name, h1 in [
        ("projects", "Projects"),
        ("infrastructure", "Infrastructure"),
        ("knowledge", "Knowledge"),
    ]:
        d = brain / lobe_name
        d.mkdir()
        (d / "map.md").write_text(
            f"---\nauto_generated: true\nparent: ../brain.md\n---\n# {h1}\n",
            encoding="utf-8",
        )

    (brain / "projects" / "auth.md").write_text(
        "---\nparent: ./map.md\n"
        "related: [../infrastructure/keycloak.md, ../knowledge/jwt.md]\n"
        "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n",
        encoding="utf-8",
    )
    (brain / "infrastructure" / "keycloak.md").write_text(
        "---\nparent: ./map.md\n"
        "related: [../knowledge/jwt.md]\n"
        "tags: [auth, identity]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Keycloak\n",
        encoding="utf-8",
    )
    (brain / "knowledge" / "jwt.md").write_text(
        "---\nparent: ./map.md\nrelated: []\n"
        "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# JWT\n",
        encoding="utf-8",
    )
    return brain


def test_root_renders_inter_child_edges_with_counts(tmp_path):
    """At the brain root (currentPath === []) the renderer must draw
    aggregate edges between child boxes using a generic aggregator.

    Children at the root are top-level lobes plus top-level files. Edges
    between any two of those siblings render as a single labeled stub
    with the synapse count.
    """
    brain = _make_brain_with_cross_lobe_synapses(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Generic aggregator + box renderer wired up
    assert "function aggregateEdgesAt" in html
    assert "function drawAggregateEdge" in html
    assert "function drawCurrent" in html
    # Edge label format with synapse count (the unicode arrow + total)
    assert "↔ " in html


def test_inside_lobe_omits_outbound_stubs(tmp_path):
    """At any non-root path, the renderer must omit outbound stub pills.

    Cross-boundary relationship pills became a noisy bottom strip in large
    brains. The canvas now stays focused on the current folder's children;
    related content remains reachable via search, recent, file tree, and
    modal links.
    """
    brain = _make_brain_with_cross_lobe_synapses(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # The unified aggregator now returns only sibling edges.
    assert "function aggregateEdgesAt" in html
    assert "function drawCurrent" in html
    assert "function drawOutboundStubs" not in html
    assert "currentOutbound" not in html
    assert "cross-boundary and outside/outside edges are omitted" in html


def test_aggregate_edges_drops_self_loops(tmp_path):
    """The aggregateEdges helper must drop self-edges (within the same lobe at
    L1, or same sublobe at L2) and exclude `parent:` edges.

    We assert by inspecting the inline JS — the function body documents both
    invariants in plain code (sk === tk skip + edge.type === 'parent' skip).
    """
    brain = _make_brain_with_cross_lobe_synapses(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Self-edge skip
    assert "if (sk === tk) continue" in html
    # parent: edges excluded
    assert "if (edge.type === 'parent') continue" in html


def test_breadcrumb_segments_are_clickable(tmp_path):
    """The breadcrumb in the header bar walks currentPath and renders each
    segment as a clickable button. Clicks jump straight to that depth via
    navigateTo(currentPath.slice(0, idx)).
    """
    brain = _make_brain_with_cross_lobe_synapses(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    assert 'id="breadcrumb"' in html
    # Breadcrumb renderer walks currentPath; segments are buttons.
    assert "function renderBreadcrumb" in html
    # Click handler invokes navigateTo with a sliced prefix of currentPath.
    assert "navigateTo(" in html


def test_mri_header_has_full_width_bar_with_back_button(tmp_path):
    """The header bar lives full-width above the three-column body and contains
    the brain title, stats, breadcrumb pill, Back/Forward buttons, and
    Fit/Reset icons. The 4-button mode-switch is GONE.
    """
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Header structure (back button replaces mode switch)
    assert 'class="mri-header"' in html
    assert 'class="brain-title"' in html
    assert 'class="stats"' in html
    assert 'class="breadcrumb"' in html
    assert 'id="btn-back"' in html
    assert 'id="btn-forward"' in html
    # Mode-switch DOM is gone — no buttons left with data-stage-mode.
    assert 'class="mode-switch"' not in html
    assert 'data-stage-mode' not in html
    # Fit + Reset icon buttons stay
    assert 'id="btn-fit"' in html
    assert 'id="btn-reset"' in html
    # Stats line numbers (lobes / neurons / synapses) stay
    assert 'id="stat-lobes"' in html
    assert 'id="stat-neurons"' in html
    assert 'id="stat-synapses"' in html


def test_mri_uses_dot_grid_background(tmp_path):
    """The stage canvas uses a flat dot-grid background (28×28) instead of
    the legacy radial gradients."""
    brain = _make_brain_with_neurons(tmp_path)
    output = tmp_path / "brain-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")

    # Dot grid background-image + size
    assert "radial-gradient(rgba(123, 167, 255, 0.08) 1px" in html
    assert "background-size: 28px 28px" in html
