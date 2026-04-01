"""Tests for kluris mri CLI command."""

import json

from click.testing import CliRunner

from kluris.cli import cli


def test_mri_generates_html(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["mri"])
    assert result.exit_code == 0
    assert (tmp_path / "my-brain" / "brain-mri.html").exists()


def test_mri_custom_output(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    custom = tmp_path / "custom-output.html"
    result = runner.invoke(cli, ["mri", "--output", str(custom)])
    assert result.exit_code == 0
    assert custom.exists()


def test_mri_summary(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["mri"])
    assert "nodes" in result.output.lower() or "MRI" in result.output


def test_mri_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    runner.invoke(cli, ["create", str(tmp_path / "my-brain")])
    result = runner.invoke(cli, ["mri", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert "nodes" in data
    assert "edges" in data
