"""Tests for kluris dream command."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from kluris.core.frontmatter import read_frontmatter


def test_dream_regenerates_maps(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    # Add a neuron manually
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream"])
    map_content = (tmp_path / "my-brain" / "architecture" / "map.md").read_text().lower()
    assert "auth" in map_content


def test_dream_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    assert "healthy" in data
    assert "broken_synapses" in data


def test_dream_exit_0_healthy(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0


def test_dream_regenerates_brain_md(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    # Add a lobe manually
    (tmp_path / "my-brain" / "experiments").mkdir()
    runner.invoke(cli, ["dream"])
    brain_md = (tmp_path / "my-brain" / "brain.md").read_text()
    assert "experiments" in brain_md


def test_dream_updates_map_with_neuron(tmp_path, monkeypatch):
    """After adding a neuron, dream should update the lobe's map.md (not brain.md)."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    runner.invoke(cli, ["dream"])
    map_content = (tmp_path / "my-brain" / "architecture" / "map.md").read_text()
    assert "auth" in map_content.lower()


def test_dream_preserves_lobe_descriptions(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])

    runner.invoke(cli, ["dream"])
    runner.invoke(cli, ["dream"])

    brain_md = (tmp_path / "my-brain" / "brain.md").read_text(encoding="utf-8")
    assert "- [architecture/](./architecture/map.md) — System design, technical patterns" in brain_md
    assert "— auto_generated: true" not in brain_md


def test_dream_reports_broken_links(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "bad.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Bad\n\n[broken](./nonexistent.md)\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    assert data["broken_synapses"] >= 1


def test_dream_fixes_one_way_synapse(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "a.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../standards/b.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# A\n", encoding="utf-8"
    )
    (tmp_path / "my-brain" / "standards" / "b.md").write_text(
        "---\nparent: ./map.md\nrelated: []\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# B\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    meta, _ = read_frontmatter(tmp_path / "my-brain" / "standards" / "b.md")
    assert result.exit_code == 0
    assert data["one_way_synapses"] == 0
    assert "../architecture/a.md" in meta["related"]


def test_dream_adds_missing_parent_frontmatter(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    neuron = tmp_path / "my-brain" / "architecture" / "no-parent.md"
    neuron.write_text(
        "---\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# No Parent\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    meta, _ = read_frontmatter(neuron)

    assert result.exit_code == 0
    assert data["frontmatter_issues"] == 0
    assert meta["parent"] == "./map.md"


def test_dream_fixes_orphans_by_regenerating_parent_map(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    neuron = tmp_path / "my-brain" / "architecture" / "orphan.md"
    neuron.write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Orphan\n",
        encoding="utf-8",
    )
    (tmp_path / "my-brain" / "architecture" / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\nupdated: 2026-04-01\n---\n# Architecture\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)
    map_content = (tmp_path / "my-brain" / "architecture" / "map.md").read_text(encoding="utf-8")

    assert result.exit_code == 0
    assert data["orphans"] == 0
    assert "orphan.md" in map_content


def test_dream_reports_broken_related_synapse(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "a.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../standards/missing.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# A\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["dream", "--json"])
    data = json.loads(result.output)

    assert result.exit_code == 1
    assert data["broken_synapses"] >= 1


def test_dream_exit_1_issues(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "bad.md").write_text(
        "---\nparent: ./map.md\n---\n# Bad\n\n[broken](./nope.md)\n", encoding="utf-8"
    )
    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 1


def test_dream_generates_nested_maps(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "services" / "api").mkdir(parents=True)

    result = runner.invoke(cli, ["dream"])

    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "services" / "api" / "map.md").exists()
