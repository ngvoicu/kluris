"""Tests for kluris doctor command."""

import json
from click.testing import CliRunner
from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.config import read_brain_config, write_brain_config


def test_doctor_all_pass(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor"])
    assert result.exit_code == 0


def test_doctor_json(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    result = runner.invoke(cli, ["doctor", "--json"])
    data = json.loads(result.output)
    assert data["ok"] is True
    assert len(data["checks"]) >= 4  # git, python, config_dir, skills
    assert "companions" in data


def test_doctor_refreshes_installed_skills(tmp_path, monkeypatch):
    """`kluris doctor` re-runs _do_install so post-upgrade users get fresh
    SKILL.md files without needing a separate skill install command."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    skill_file = tmp_path / ".claude" / "skills" / "kluris-my-brain" / "SKILL.md"
    assert skill_file.exists()  # written by `create`

    # Sabotage the installed skill (simulate it being stale post-upgrade)
    skill_file.write_text("OUTDATED_PLACEHOLDER", encoding="utf-8")

    result = runner.invoke(cli, ["doctor"])
    assert result.exit_code == 0
    # The skill should have been rewritten with real content
    assert "OUTDATED_PLACEHOLDER" not in skill_file.read_text()
    assert "name: kluris-my-brain" in skill_file.read_text()


def test_doctor_no_refresh_skips_skill_install(tmp_path, monkeypatch):
    """--no-refresh keeps doctor read-only (skips _do_install)."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    skill_file = tmp_path / ".claude" / "skills" / "kluris-my-brain" / "SKILL.md"
    skill_file.write_text("OUTDATED_PLACEHOLDER", encoding="utf-8")

    result = runner.invoke(cli, ["doctor", "--no-refresh"])
    assert result.exit_code == 0
    # The placeholder should still be there (no refresh ran)
    assert "OUTDATED_PLACEHOLDER" in skill_file.read_text()


def test_doctor_json_includes_skills_check(tmp_path, monkeypatch):
    """The skills refresh appears in the JSON checks list with detail."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    result = runner.invoke(cli, ["doctor", "--json"])
    data = json.loads(result.output)
    skill_check = next((c for c in data["checks"] if c["name"] == "skills"), None)
    assert skill_check is not None
    assert skill_check["passed"] is True
    assert "1 brain" in skill_check["detail"]


def test_doctor_refreshes_referenced_companion(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"
    cfg = read_brain_config(brain)
    cfg.companions = ["specmint-core"]
    write_brain_config(cfg, brain)

    companion_dir = tmp_path / ".kluris" / "companions" / "specmint-core"
    assert not companion_dir.exists()

    result = runner.invoke(cli, ["doctor", "--json"])

    data = json.loads(result.output)
    assert result.exit_code == 0, result.output
    assert (companion_dir / "SKILL.md").exists()
    assert data["companions"] == [{"name": "specmint-core", "refreshed": True}]


def test_doctor_no_refresh_skips_companion_refresh(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    brain = tmp_path / "my-brain"
    cfg = read_brain_config(brain)
    cfg.companions = ["specmint-core"]
    write_brain_config(cfg, brain)

    result = runner.invoke(cli, ["doctor", "--no-refresh", "--json"])

    data = json.loads(result.output)
    assert result.exit_code == 0, result.output
    assert data["companions"] == []
    assert not (tmp_path / ".kluris" / "companions" / "specmint-core").exists()
