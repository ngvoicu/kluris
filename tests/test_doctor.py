"""Tests for kluris doctor command."""

import json
from click.testing import CliRunner
from kluris.cli import cli


def test_doctor_all_pass(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor"])
    assert result.exit_code == 0


def test_doctor_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert len(data["checks"]) >= 3
