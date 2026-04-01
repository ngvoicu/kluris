"""Tests for kluris dream command."""

import json
from click.testing import CliRunner
from kluris.cli import cli


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


def test_dream_regenerates_index(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    (tmp_path / "my-brain" / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    runner.invoke(cli, ["dream"])
    index = (tmp_path / "my-brain" / "index.md").read_text()
    assert "auth" in index.lower()


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


def test_dream_reports_one_way_synapse(tmp_path, monkeypatch):
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
    assert data["one_way_synapses"] >= 1


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
