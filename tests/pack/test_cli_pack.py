"""TEST-PACK-57 — ``kluris pack`` CLI command."""

from __future__ import annotations

import json

import pytest
from click.testing import CliRunner

from kluris.cli import cli


@pytest.fixture
def cli_runner_local():
    return CliRunner()


def test_pack_single_brain_default_output(temp_brain, cli_runner_local, tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    result = cli_runner_local.invoke(cli, ["pack", "--json"])
    assert result.exit_code == 0, result.stdout + result.stderr
    data = json.loads(result.stdout)
    assert data["ok"] is True
    assert data["brain"] == "test-brain"
    assert (tmp_path / "test-brain-pack").is_dir()


def test_pack_custom_output(temp_brain, cli_runner_local, tmp_path):
    out = tmp_path / "build" / "custom"
    result = cli_runner_local.invoke(
        cli, ["pack", "--output", str(out), "--json"]
    )
    assert result.exit_code == 0, result.stdout + result.stderr
    data = json.loads(result.stdout)
    assert data["output"] == str(out.resolve())
    assert out.is_dir()


def test_pack_excludes_honored(temp_brain, cli_runner_local, tmp_path, monkeypatch):
    (temp_brain / "knowledge" / "private.md").write_text(
        "---\nupdated: 2026-04-01\n---\n# private\n", encoding="utf-8"
    )
    monkeypatch.chdir(tmp_path)
    result = cli_runner_local.invoke(
        cli,
        ["pack", "--exclude", "knowledge/private.md", "--json"],
    )
    assert result.exit_code == 0
    data = json.loads(result.stdout)
    assert "brain/knowledge/private.md" not in data["files"]


def test_pack_existing_output_errors(temp_brain, cli_runner_local, tmp_path):
    out = tmp_path / "exists"
    out.mkdir()
    result = cli_runner_local.invoke(
        cli, ["pack", "--output", str(out), "--json"]
    )
    assert result.exit_code != 0
    err = json.loads(result.stdout)
    assert err["ok"] is False
    assert "exists" in err["error"].lower()


def test_pack_help_documents_flags(cli_runner_local):
    result = cli_runner_local.invoke(cli, ["pack", "--help"])
    assert result.exit_code == 0
    for flag in ("--brain", "--output", "--exclude", "--json"):
        assert flag in result.stdout
