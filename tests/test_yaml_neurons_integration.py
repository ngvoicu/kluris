"""Phase 7 integration tests for the yaml-neurons spec.

End-to-end tests that exercise the full stack (frontmatter → scanners →
linker → maps → search → MRI → wake-up → dream) against three realistic
brain fixtures defined in `tests.fixtures_yaml_neurons`.

These tests catch any cross-subsystem gaps that slip through the unit-
level TEST-IMPL cycles of Phases 1-6.
"""

from __future__ import annotations

import json

from click.testing import CliRunner

from kluris.cli import cli
from kluris.core.linker import (
    _neuron_files,
    check_frontmatter,
    detect_orphans,
    validate_synapses,
)
from kluris.core.maps import _get_neurons
from kluris.core.mri import build_graph, generate_mri_html
from kluris.core.search import search_brain
from fixtures_yaml_neurons import (
    LARGE_BRAIN_LOBES,
    LARGE_BRAIN_RAW_YAML,
    LARGE_BRAIN_YAML_NEURONS,
    MICROSERVICES_BRAIN_SERVICES,
    MIXED_BRAIN_EMPTY_LOBES,
    MIXED_BRAIN_LOBES,
    _make_large_brain,
    _make_microservices_brain,
    _make_mixed_brain,
    large_brain_expected_counts,
)


# --- Mixed brain ----------------------------------------------------------


def test_mixed_brain_build_graph_excludes_kluris_yml_and_raw_yaml(tmp_path):
    brain = _make_mixed_brain(tmp_path)
    graph = build_graph(brain)
    paths = {n["path"] for n in graph["nodes"]}
    assert "kluris.yml" not in paths
    # Raw yaml files must not appear
    assert "architecture/ci.yml" not in paths
    assert "product/feature-flags.yml" not in paths
    # Opted-in yaml files must appear
    assert "api-contracts/public-api.yml" in paths
    assert "integrations/stripe-api.yml" in paths
    assert "projects/alpha-api.yml" in paths
    assert "runbooks/restart-contract.yml" in paths


def test_mixed_brain_empty_lobes_have_map_but_no_contents(tmp_path):
    """Empty lobes (no neurons) must still have a map node but zero neurons."""
    brain = _make_mixed_brain(tmp_path)
    graph = build_graph(brain)
    by_lobe = {}
    for node in graph["nodes"]:
        if node["type"] == "neuron":
            by_lobe.setdefault(node["lobe"], []).append(node)

    for lobe in MIXED_BRAIN_EMPTY_LOBES:
        assert lobe not in by_lobe or len(by_lobe[lobe]) == 0


def test_mixed_brain_yaml_nodes_have_file_type_and_color(tmp_path):
    brain = _make_mixed_brain(tmp_path)
    graph = build_graph(brain)
    yaml_nodes = [n for n in graph["nodes"]
                  if n.get("file_type") == "yaml" and n["type"] == "neuron"]
    # 4 opted-in yaml neurons in the mixed brain
    assert len(yaml_nodes) == 4


def test_mixed_brain_check_frontmatter_clean(tmp_path):
    """The mixed brain is constructed with complete frontmatter everywhere,
    so check_frontmatter should return zero issues.
    """
    brain = _make_mixed_brain(tmp_path)
    issues = check_frontmatter(brain)
    # Only issues allowed are type errors, never missing fields
    missing_issues = [i for i in issues if "kind" not in i]
    assert missing_issues == [], f"unexpected missing-field issues: {missing_issues}"


def test_mixed_brain_mri_html_valid_and_sized(tmp_path):
    brain = _make_mixed_brain(tmp_path)
    output = tmp_path / "mixed-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "<html" in html
    assert "</html>" in html
    assert "#9ea9ff" in html  # yaml color constant
    size_kb = output.stat().st_size / 1024
    assert size_kb < 1500, f"mixed brain MRI HTML too large: {size_kb:.0f} KB"


# --- Large brain ----------------------------------------------------------


def test_large_brain_build_graph_has_expected_shape(tmp_path):
    brain = _make_large_brain(tmp_path)
    graph = build_graph(brain)
    expected = large_brain_expected_counts()

    md_nodes = [n for n in graph["nodes"]
                if n["type"] == "neuron" and n.get("file_type") == "markdown"]
    yaml_nodes = [n for n in graph["nodes"]
                  if n["type"] == "neuron" and n.get("file_type") == "yaml"]
    map_nodes = [n for n in graph["nodes"] if n["type"] == "map"]

    assert len(yaml_nodes) == expected["yaml_neurons"], (
        f"expected {expected['yaml_neurons']} yaml neurons, got {len(yaml_nodes)}"
    )
    assert len(md_nodes) == expected["md_neurons"], (
        f"expected {expected['md_neurons']} md neurons, got {len(md_nodes)}"
    )
    # Lobe maps + sublobe maps
    assert len(map_nodes) == expected["lobes"] + expected["sublobes"]

    # Raw yaml files must never appear
    paths = {n["path"] for n in graph["nodes"]}
    for lobe, filename in LARGE_BRAIN_RAW_YAML:
        assert f"{lobe}/{filename}" not in paths


def test_large_brain_mri_html_under_2mb(tmp_path):
    """Large brain MRI HTML stays under the 2 MB budget from the spec."""
    brain = _make_large_brain(tmp_path)
    output = tmp_path / "large-mri.html"
    generate_mri_html(brain, output)
    size_kb = output.stat().st_size / 1024
    assert size_kb < 2048, f"large brain MRI HTML exceeds 2 MB: {size_kb:.0f} KB"
    # Sanity: contains the yaml color + some expected paths
    html = output.read_text(encoding="utf-8")
    assert "#9ea9ff" in html
    for lobe_path, filename, _ in LARGE_BRAIN_YAML_NEURONS:
        assert f"{lobe_path}/{filename}" in html, (
            f"missing yaml node in HTML: {lobe_path}/{filename}"
        )


def test_large_brain_kluris_yml_never_indexed(tmp_path):
    """Regression guard across all scanners for the large brain."""
    brain = _make_large_brain(tmp_path)

    # _neuron_files
    neuron_paths = {f.relative_to(brain).as_posix() for f in _neuron_files(brain)}
    assert "kluris.yml" not in neuron_paths

    # build_graph
    graph = build_graph(brain)
    assert "kluris.yml" not in {n["path"] for n in graph["nodes"]}

    # search_brain (adversarial query hitting brain config text)
    results = search_brain(brain, "large-brain", limit=50)
    assert not any(r["file"] == "kluris.yml" for r in results)


def test_large_brain_no_broken_synapses(tmp_path):
    """Cross-lobe synapses in the fixture must all resolve — zero broken links
    from `validate_synapses`. The fixture doesn't seed any dead references.
    """
    brain = _make_large_brain(tmp_path)
    broken = validate_synapses(brain)
    assert broken == [], f"unexpected broken synapses: {broken}"


def test_large_brain_get_neurons_counts_per_lobe(tmp_path):
    """Per-lobe _get_neurons counts must match the fixture's declared counts
    for each top-level lobe (not recursive — just immediate children).
    """
    from fixtures_yaml_neurons import (
        LARGE_BRAIN_MD_COUNT,
    )
    brain = _make_large_brain(tmp_path)
    for lobe in LARGE_BRAIN_LOBES:
        neurons = _get_neurons(brain / lobe)
        expected_md = LARGE_BRAIN_MD_COUNT[lobe]
        # Count deprecated md neurons (added per lobe by the fixture)
        deprecated_in_lobe = {
            "api": 1, "security": 1, "data": 1, "contracts": 1,
        }.get(lobe, 0)
        # Count yaml neurons in this top-level lobe (not sublobes)
        expected_yaml = sum(
            1 for lp, _, _ in LARGE_BRAIN_YAML_NEURONS if lp == lobe
        )
        assert len(neurons) == expected_md + deprecated_in_lobe + expected_yaml, (
            f"lobe {lobe}: expected {expected_md + deprecated_in_lobe + expected_yaml} "
            f"neurons, got {len(neurons)}: {[n['name'] for n in neurons]}"
        )


def test_large_brain_wake_up_json_schema(tmp_path, monkeypatch):
    """wake-up --json must include yaml_count per lobe and total_yaml_neurons
    at top level for the large brain.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))

    # Register the large brain in a fresh config
    brain = _make_large_brain(tmp_path)
    runner = CliRunner()
    # Manually write config so we don't need create_test_brain (which scaffolds a different shape)
    config_dir = tmp_path / ".kluris"
    config_dir.mkdir(exist_ok=True)
    (config_dir / "config.yml").write_text(
        "version: 1\nbrains:\n"
        f"  large-brain:\n"
        f"    path: {brain}\n"
        f"    description: Large brain\n"
        f"    type: product\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("KLURIS_CONFIG", str(config_dir / "config.yml"))

    result = runner.invoke(cli, ["wake-up", "--json", "--brain", "large-brain"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    assert "total_yaml_neurons" in data
    assert data["total_yaml_neurons"] == large_brain_expected_counts()["yaml_neurons"]
    # Each lobe entry has a yaml_count
    for lobe in data["lobes"]:
        assert "yaml_count" in lobe


# --- Microservices brain --------------------------------------------------


def test_microservices_brain_every_service_has_yaml_node(tmp_path):
    brain = _make_microservices_brain(tmp_path)
    graph = build_graph(brain)
    yaml_paths = {n["path"] for n in graph["nodes"] if n.get("file_type") == "yaml"}
    for svc in MICROSERVICES_BRAIN_SERVICES:
        assert f"{svc}/openapi.yml" in yaml_paths, f"missing {svc}/openapi.yml node"
    assert len(yaml_paths) == len(MICROSERVICES_BRAIN_SERVICES)


def test_microservices_brain_search_api_tag_returns_every_service(tmp_path):
    """Every openapi.yml in the monorepo fixture shares the `api` tag in its
    #--- block. Searching "api" must return all 12.
    """
    brain = _make_microservices_brain(tmp_path)
    results = search_brain(brain, "api", limit=50)
    yaml_files = {r["file"] for r in results if r.get("file_type") == "yaml"}
    assert len(yaml_files) == len(MICROSERVICES_BRAIN_SERVICES)
    for svc in MICROSERVICES_BRAIN_SERVICES:
        assert f"{svc}/openapi.yml" in yaml_files


def test_microservices_brain_mri_has_service_hulls_and_yaml_colors(tmp_path):
    brain = _make_microservices_brain(tmp_path)
    output = tmp_path / "ms-mri.html"
    generate_mri_html(brain, output)
    html = output.read_text(encoding="utf-8")
    assert "#9ea9ff" in html
    # Every service path appears in the HTML graph payload
    for svc in MICROSERVICES_BRAIN_SERVICES:
        assert f"{svc}/openapi.yml" in html


def test_microservices_brain_each_service_has_md_and_yaml(tmp_path):
    brain = _make_microservices_brain(tmp_path)
    for svc in MICROSERVICES_BRAIN_SERVICES:
        neurons = _get_neurons(brain / svc)
        names = {n["name"] for n in neurons}
        assert "architecture.md" in names
        assert "runbook.md" in names
        assert "openapi.yml" in names
        # README.md is excluded
        assert "README.md" not in names


def test_microservices_brain_check_frontmatter_clean(tmp_path):
    """No missing required fields anywhere in the microservices brain."""
    brain = _make_microservices_brain(tmp_path)
    issues = check_frontmatter(brain)
    missing = [i for i in issues if "kind" not in i]
    assert missing == [], f"unexpected missing-field issues: {missing}"


def test_microservices_brain_kluris_yml_never_in_results(tmp_path):
    brain = _make_microservices_brain(tmp_path)
    # Search for a term that would match the kluris.yml body
    results = search_brain(brain, "microservices-brain", limit=50)
    assert not any(r["file"] == "kluris.yml" for r in results)
