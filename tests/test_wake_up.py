"""Tests for `kluris wake-up` -- compact brain snapshot for agent bootstrap."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def _write_neuron(brain_path, rel_path, title, updated="2026-04-01", extra_fm=""):
    """Write a neuron file with frontmatter into the given brain."""
    target = brain_path / rel_path
    target.parent.mkdir(parents=True, exist_ok=True)
    content = (
        f"---\n"
        f"parent: ./map.md\n"
        f"created: 2026-01-01\n"
        f"updated: {updated}\n"
        f"{extra_fm}"
        f"---\n\n"
        f"# {title}\n\nbody\n"
    )
    target.write_text(content, encoding="utf-8")


def test_wake_up_single_brain_text(temp_brain, cli_runner):
    """Default brain wakes up with name, path, lobe list."""
    _write_neuron(temp_brain, "knowledge/use-raw-sql.md", "Use Raw SQL")
    result = cli_runner.invoke(cli, ["wake-up"])
    assert result.exit_code == 0
    assert "test-brain" in result.output
    assert "knowledge" in result.output
    assert "projects" in result.output


def test_wake_up_json_schema(temp_brain, cli_runner):
    """--json output carries ok, name, path, lobes, neurons, recent."""
    _write_neuron(temp_brain, "knowledge/use-raw-sql.md", "Use Raw SQL")
    _write_neuron(temp_brain, "projects/btb/auth.md", "Auth")

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)

    assert data["ok"] is True
    assert data["name"] == "test-brain"
    assert "is_default" not in data  # field removed in multi-brain refactor
    assert str(temp_brain) == data["path"]

    # Brain type and structure
    assert data["type"] == "product-group"
    assert isinstance(data["type_structure"], dict)
    assert "projects" in data["type_structure"]
    assert "infrastructure" in data["type_structure"]
    assert "knowledge" in data["type_structure"]

    # Lobes: list of {name, neurons} dicts
    assert isinstance(data["lobes"], list)
    lobe_names = {lobe["name"] for lobe in data["lobes"]}
    assert {"projects", "infrastructure", "knowledge"} <= lobe_names

    # Neuron count reflects the two we wrote
    assert data["total_neurons"] == 2

    # Recent list: the two neurons we wrote, newest first
    assert isinstance(data["recent"], list)
    assert len(data["recent"]) == 2


def test_wake_up_recent_sorted_newest_first(temp_brain, cli_runner):
    """Recent list sorts by frontmatter `updated` descending."""
    _write_neuron(temp_brain, "knowledge/older.md", "Older", updated="2026-01-01")
    _write_neuron(temp_brain, "knowledge/newer.md", "Newer", updated="2026-04-07")
    _write_neuron(temp_brain, "knowledge/middle.md", "Middle", updated="2026-03-15")

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    recent = data["recent"]
    assert recent[0]["path"] == "knowledge/newer.md"
    assert recent[1]["path"] == "knowledge/middle.md"
    assert recent[2]["path"] == "knowledge/older.md"


def test_wake_up_recent_limited_to_five(temp_brain, cli_runner):
    """Recent list caps at 5 entries so wake-up stays compact."""
    for i in range(10):
        _write_neuron(
            temp_brain,
            f"knowledge/neuron-{i:02d}.md",
            f"Neuron {i}",
            updated=f"2026-04-{(i + 1):02d}",
        )

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert len(data["recent"]) == 5
    # Newest (day 10) should be first
    assert data["recent"][0]["path"] == "knowledge/neuron-09.md"


def test_wake_up_type_structure_matches_brain_type(tmp_path, temp_config, cli_runner, monkeypatch):
    """type_structure reflects the brain's registered type, not just the default."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    create_test_brain(cli_runner, "research-brain", tmp_path, type="research")

    result = cli_runner.invoke(cli, ["wake-up", "--brain", "research-brain", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["type"] == "research"
    assert "literature" in data["type_structure"]
    assert "experiments" in data["type_structure"]
    assert "projects" not in data["type_structure"]


def test_wake_up_lobes_include_description(tmp_path, temp_config, cli_runner, monkeypatch):
    """Each lobe in lobes[] carries its description from map.md."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    create_test_brain(cli_runner, "desc-brain", tmp_path)

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    descs = {l["name"]: l["description"] for l in data["lobes"]}
    assert "Hosting" in descs.get("infrastructure", "")
    assert "Decisions" in descs.get("knowledge", "")
    assert "Per-project" in descs.get("projects", "")


def test_wake_up_lobes_description_legacy_body_only(temp_brain, cli_runner):
    """Lobes with legacy map.md (no frontmatter description) still surface
    a description by falling back to body parsing."""
    map_file = temp_brain / "knowledge" / "map.md"
    map_file.write_text(
        "---\nauto_generated: true\nparent: ../brain.md\nupdated: 2026-04-01\n---\n"
        "# Knowledge\n\nDecisions and learnings for the team.\n\n## Contents\n\n(empty)\n",
        encoding="utf-8",
    )
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    knowledge = next(l for l in data["lobes"] if l["name"] == "knowledge")
    assert "Decisions" in knowledge["description"]


def test_wake_up_targets_named_brain(tmp_path, temp_config, cli_runner, monkeypatch):
    """`kluris wake-up --brain NAME` targets a specific brain."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    create_test_brain(cli_runner, "brain-a", tmp_path)
    create_test_brain(cli_runner, "brain-b", tmp_path)

    result = cli_runner.invoke(cli, ["wake-up", "--brain", "brain-b", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["name"] == "brain-b"
    assert "is_default" not in data


def test_wake_up_unknown_brain_errors(temp_brain, cli_runner):
    """Unknown brain name exits non-zero with clear error."""
    result = cli_runner.invoke(cli, ["wake-up", "--brain", "nonexistent", "--json"])
    assert result.exit_code != 0
    data = json.loads(result.output)
    assert data["ok"] is False
    assert "nonexistent" in data["error"]


def test_wake_up_no_brains_errors(temp_config, cli_runner):
    """No brains registered -> error."""
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code != 0
    data = json.loads(result.output)
    assert data["ok"] is False


def test_wake_up_text_includes_recent(temp_brain, cli_runner):
    """Text output mentions at least one recent neuron path."""
    _write_neuron(temp_brain, "knowledge/use-raw-sql.md", "Use Raw SQL", updated="2026-04-07")
    result = cli_runner.invoke(cli, ["wake-up"])
    assert result.exit_code == 0
    assert "use-raw-sql" in result.output


def test_wake_up_reports_deprecation_count(temp_brain, cli_runner):
    """Wake-up payload must surface a deprecation_count so the agent can
    decide whether to dig into deprecation issues before answering."""
    # One deprecated neuron with no replacement (triggers
    # deprecated_without_replacement)
    (temp_brain / "knowledge" / "old.md").write_text(
        "---\nparent: ./map.md\nstatus: deprecated\ndeprecated_at: 2026-03-01\n"
        "tags: []\ncreated: 2026-01-01\nupdated: 2026-04-01\n---\n# Old\n",
        encoding="utf-8",
    )

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert "deprecation_count" in data
    assert data["deprecation_count"] >= 1


def test_wake_up_deprecation_count_zero_on_clean_brain(temp_brain, cli_runner):
    """Clean brain must report deprecation_count == 0, not omit the field."""
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert data["deprecation_count"] == 0


def test_wake_up_includes_brain_md_body(temp_brain, cli_runner):
    """brain.md body is returned in the snapshot so the agent doesn't re-read it."""
    # temp_brain's brain.md already contains `# Test Brain`
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert "brain_md" in data
    assert isinstance(data["brain_md"], str)
    assert "Test Brain" in data["brain_md"]
    # Frontmatter must be stripped
    assert "auto_generated" not in data["brain_md"]
    assert "---" not in data["brain_md"].split("\n")[0]


def test_wake_up_brain_md_absent_returns_empty_string(temp_brain, cli_runner):
    """Missing brain.md must not crash wake-up; return empty string."""
    (temp_brain / "brain.md").unlink()
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["brain_md"] == ""


def test_wake_up_brain_md_is_truncated_when_huge(temp_brain, cli_runner):
    """brain.md is capped so a pathological 100 KB brain.md can't blow up the bootstrap payload."""
    huge = "---\nauto_generated: true\nupdated: 2026-04-01\n---\n"
    huge += "# Test Brain\n\n"
    huge += "x" * 10000  # 10 KB of body content
    (temp_brain / "brain.md").write_text(huge, encoding="utf-8")
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    # Body capped at 4000 bytes + a small truncation marker
    assert len(data["brain_md"].encode("utf-8")) < 4200
    assert "truncated" in data["brain_md"]


def test_wake_up_includes_glossary_table_format(temp_brain, cli_runner):
    """Glossary terms in the default markdown-table format are parsed and returned."""
    (temp_brain / "glossary.md").write_text(
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n"
        "# Glossary\n\n"
        "| Term | Meaning |\n"
        "|------|---------|\n"
        "| SIT | System integration testing environment |\n"
        "| UAT | User acceptance testing environment |\n",
        encoding="utf-8",
    )
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert "glossary" in data
    terms = {e["term"]: e["definition"] for e in data["glossary"]}
    assert terms.get("SIT", "").startswith("System integration")
    assert terms.get("UAT", "").startswith("User acceptance")
    # Header/separator rows must not leak in
    assert "Term" not in terms
    assert "------" not in terms


def test_wake_up_includes_glossary_bold_dash_format(temp_brain, cli_runner):
    """Glossary terms in the SKILL.md-recommended **Term** -- Definition format are parsed."""
    (temp_brain / "glossary.md").write_text(
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n"
        "# Glossary\n\n"
        "**Synapse** -- a link between two neurons (bidirectional).\n"
        "**Lobe** -- a top-level folder in the brain.\n",
        encoding="utf-8",
    )
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    terms = {e["term"]: e["definition"] for e in data["glossary"]}
    assert "Synapse" in terms
    assert "Lobe" in terms
    assert "link between two neurons" in terms["Synapse"]


def test_wake_up_glossary_empty_when_no_entries(temp_brain, cli_runner):
    """Brain with only the scaffolded glossary (no entries) returns an empty list, not an error."""
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert "glossary" in data
    assert isinstance(data["glossary"], list)
    assert data["glossary"] == []


def test_wake_up_includes_deprecation_list(temp_brain, cli_runner):
    """deprecation_count is preserved and deprecation[] now carries the full list."""
    (temp_brain / "knowledge" / "old.md").write_text(
        "---\nparent: ./map.md\nstatus: deprecated\ndeprecated_at: 2026-03-01\n"
        "tags: []\ncreated: 2026-01-01\nupdated: 2026-04-01\n---\n# Old\n",
        encoding="utf-8",
    )
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert "deprecation" in data
    assert isinstance(data["deprecation"], list)
    assert len(data["deprecation"]) == data["deprecation_count"]
    # Each entry has at least kind + file
    assert all("kind" in e for e in data["deprecation"])
    assert any(e["kind"] == "deprecated_without_replacement" for e in data["deprecation"])


def test_wake_up_deprecation_list_empty_on_clean_brain(temp_brain, cli_runner):
    """Clean brain returns deprecation: [] AND deprecation_count: 0 (both fields present)."""
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert data["deprecation"] == []
    assert data["deprecation_count"] == 0


def test_wake_up_stale_brain_path_returns_json_error(tmp_path, temp_config, cli_runner, monkeypatch):
    """If a registered brain's filesystem path no longer exists, wake-up must
    return a structured JSON error envelope — not crash with FileNotFoundError
    and empty stdout. Agents rely on the envelope to recover gracefully."""
    import shutil
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    create_test_brain(cli_runner, "ghost-brain", tmp_path)
    # Delete the brain directory out from under the registration
    shutil.rmtree(tmp_path / "ghost-brain")

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code != 0
    data = json.loads(result.output)
    assert data["ok"] is False
    assert "ghost-brain" in data["error"] or "path" in data["error"].lower()


# --- yaml-neurons wake-up tests ---


def test_wake_up_counts_opted_in_yaml_in_lobes(temp_brain, cli_runner):
    """Wake-up's lobes[] must include opted-in yaml neurons in `neurons`
    AND expose a new `yaml_count` field per lobe.
    """
    # Add md + opted-in yaml + raw yaml (opt-out) to the knowledge lobe
    _write_neuron(temp_brain, "knowledge/note.md", "Note")
    (temp_brain / "knowledge" / "openapi.yml").write_text(
        "#---\n# title: API\n# updated: 2026-04-09\n#---\n"
        "openapi: 3.1.0\ninfo:\n  title: API\n  version: 1.0.0\npaths: {}\n",
        encoding="utf-8",
    )
    (temp_brain / "knowledge" / "ci-config.yml").write_text(
        "name: ci\n", encoding="utf-8"
    )

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)

    knowledge = next(l for l in data["lobes"] if l["name"] == "knowledge")
    # neurons total includes the yaml neuron but not the raw yaml
    assert knowledge["neurons"] == 2
    assert knowledge["yaml_count"] == 1
    # `total_yaml_neurons` is exposed at top level (new additive field)
    assert data["total_yaml_neurons"] >= 1


def test_wake_up_recent_entries_include_file_type(temp_brain, cli_runner):
    """Each entry in `recent[]` must carry a `file_type` field.
    """
    _write_neuron(temp_brain, "knowledge/note.md", "Note", updated="2026-04-10")
    (temp_brain / "knowledge" / "openapi.yml").write_text(
        "#---\n# title: API\n# updated: 2026-04-09\n#---\nopenapi: 3.1.0\n",
        encoding="utf-8",
    )

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)

    types = {r.get("file_type") for r in data["recent"]}
    # Both file types should appear in recent
    assert "markdown" in types
    assert "yaml" in types


def test_wake_up_excludes_kluris_yml(temp_brain, cli_runner):
    """Adversarial: a `kluris.yml` with a `#---` block at brain root must
    NEVER appear in the wake-up snapshot.
    """
    # Replace/augment kluris.yml with an adversarial block
    (temp_brain / "kluris.yml").write_text(
        "#---\n# updated: 2026-04-09\n#---\nname: brain\n",
        encoding="utf-8",
    )

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0, result.output
    data = json.loads(result.output)
    # kluris.yml must not appear in any lobe's contents nor in recent.
    for lobe in data["lobes"]:
        assert lobe["name"] != "kluris.yml"
    for entry in data["recent"]:
        assert entry["path"] != "kluris.yml"
