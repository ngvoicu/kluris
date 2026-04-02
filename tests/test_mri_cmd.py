"""Tests for kluris mri CLI command."""

import json

from click.testing import CliRunner

from kluris.cli import cli


def test_mri_generates_html(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["mri"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "brain-mri.html").exists()


def test_mri_custom_output(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    custom = tmp_path / "custom-output.html"
    result = runner.invoke(cli, ["mri", "--output", str(custom)])
    assert result.exit_code == 0
    assert custom.exists()


def test_mri_summary(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["mri"])
    assert "nodes" in result.output.lower() or "MRI" in result.output


def test_mri_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    result = runner.invoke(cli, ["mri", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "nodes" in data
    assert "edges" in data
    assert "preflight_fixes" in data


def test_mri_runs_dream_preflight(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    neuron = tmp_path / "my-brain" / "architecture" / "orphan.md"
    neuron.write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Orphan\n",
        encoding="utf-8",
    )
    result = runner.invoke(cli, ["mri", "--json"])
    data = json.loads(result.output)
    map_content = (tmp_path / "my-brain" / "architecture" / "map.md").read_text(encoding="utf-8")

    assert result.exit_code == 0
    assert data["preflight_fixes"]["orphan_references_added"] >= 1
    assert "orphan.md" in map_content
