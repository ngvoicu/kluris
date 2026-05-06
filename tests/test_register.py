"""Tests for the kluris register command."""

from __future__ import annotations

import subprocess
from pathlib import Path

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.config import read_brain_config, read_global_config


def _make_source_brain(tmp_path: Path, monkeypatch, name: str = "source-brain") -> Path:
    """Scaffold a fully-formed brain using the existing create command.

    Registers it into a throwaway config so the later register test can use a
    fresh KLURIS_CONFIG without conflicting registrations carrying over.
    """
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "source-config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path / "source-home"))
    (tmp_path / "source-home").mkdir(exist_ok=True)
    runner = CliRunner()
    create_test_brain(runner, name, tmp_path)
    return tmp_path / name


def _use_fresh_registry(tmp_path: Path, monkeypatch) -> None:
    """Point KLURIS_CONFIG and HOME at a fresh empty registry for the register call."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "register-config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path / "register-home"))
    (tmp_path / "register-home").mkdir(exist_ok=True)


# -----------------------------------------------------------------------------
# Directory registration
# -----------------------------------------------------------------------------


def test_register_dir_succeeds(tmp_path, monkeypatch):
    source = _make_source_brain(tmp_path, monkeypatch)
    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    config = read_global_config()
    assert "source-brain" in config.brains
    assert Path(config.brains["source-brain"].path).resolve() == source.resolve()


def test_register_wizard_prompt_specmint(tmp_path, monkeypatch):
    source = _make_source_brain(tmp_path, monkeypatch)
    _use_fresh_registry(tmp_path, monkeypatch)
    monkeypatch.setattr("kluris.cli._is_interactive", lambda: True)

    runner = CliRunner()
    result = runner.invoke(cli, ["register"], input=f"{source}\n2\n")

    assert result.exit_code == 0, result.output
    assert read_brain_config(source).companions == ["specmint-tdd"]
    home = tmp_path / "register-home"
    assert (home / ".kluris" / "companions" / "specmint-tdd" / "SKILL.md").exists()


def test_register_flag_driven_skips_companion_prompt(tmp_path, monkeypatch):
    source = _make_source_brain(tmp_path, monkeypatch)
    _use_fresh_registry(tmp_path, monkeypatch)
    monkeypatch.setattr("kluris.cli._is_interactive", lambda: True)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    assert "Install specmint companions" not in result.output
    assert read_brain_config(source).companions == []


def test_register_dir_in_place_does_not_move_source(tmp_path, monkeypatch):
    """register must never move or delete the source directory."""
    source = _make_source_brain(tmp_path, monkeypatch)
    source_marker = source / "projects" / "marker.md"
    source_marker.write_text("# marker\n", encoding="utf-8")

    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    assert source.exists()
    assert (source / "brain.md").exists()
    assert source_marker.exists()


def test_register_keeps_existing_kluris_yml(tmp_path, monkeypatch):
    """When the source already has kluris.yml, we must not clobber it."""
    source = _make_source_brain(tmp_path, monkeypatch)
    # Edit the existing kluris.yml so we can detect whether we overwrote it.
    # The legacy `git: { commit_prefix: ... }` block is intentionally included
    # to cover the read-tolerance regression: a 2.15.x kluris.yml must load
    # cleanly under 2.16.0 without raising.
    (source / "kluris.yml").write_text(
        "name: source-brain\ndescription: custom tag\ngit:\n  commit_prefix: 'custom:'\n",
        encoding="utf-8",
    )

    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    brain_config = read_brain_config(source)
    assert brain_config.description == "custom tag"
    # Legacy `git:` block on disk is ignored at runtime; no `git` attribute on the model.
    assert not hasattr(brain_config, "git")


def test_register_writes_kluris_yml_when_missing(tmp_path, monkeypatch):
    source = _make_source_brain(tmp_path, monkeypatch)
    (source / "kluris.yml").unlink()

    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    assert (source / "kluris.yml").exists()
    brain_config = read_brain_config(source)
    assert brain_config.name == "source-brain"


def test_register_rejects_non_brain(tmp_path, monkeypatch):
    """Directory without brain.md must be rejected with a helpful message."""
    _use_fresh_registry(tmp_path, monkeypatch)
    not_brain = tmp_path / "not-brain"
    not_brain.mkdir()
    (not_brain / "readme.md").write_text("# not a brain\n", encoding="utf-8")

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(not_brain)])

    assert result.exit_code != 0
    assert "not a Kluris brain" in result.output
    # Source must not be touched.
    assert not_brain.exists()
    assert (not_brain / "readme.md").exists()


def test_register_rejects_missing_path(tmp_path, monkeypatch):
    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(tmp_path / "does-not-exist")])

    assert result.exit_code != 0
    assert "not found" in result.output.lower()


# -----------------------------------------------------------------------------
# Idempotency & duplicate handling
# -----------------------------------------------------------------------------


def test_register_idempotent_same_path_same_name(tmp_path, monkeypatch):
    """Re-registering the same brain at the same path must be a no-op success."""
    source = _make_source_brain(tmp_path, monkeypatch)
    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    first = runner.invoke(cli, ["register", str(source)])
    assert first.exit_code == 0, first.output

    second = runner.invoke(cli, ["register", str(source)])
    assert second.exit_code == 0, second.output
    assert "already registered" in second.output.lower()

    config = read_global_config()
    assert list(config.brains.keys()) == ["source-brain"]


def test_register_rejects_same_name_different_path(tmp_path, monkeypatch):
    """Registering a brain whose canonical name collides with an existing one
    at a DIFFERENT path must fail with a clear error."""
    source_a = _make_source_brain(tmp_path, monkeypatch, name="source-brain")

    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    assert runner.invoke(cli, ["register", str(source_a)]).exit_code == 0

    # Create a second brain with the same canonical name but at a different path.
    other_parent = tmp_path / "other-copy"
    other_parent.mkdir()
    # Copy source-brain into the new location.
    import shutil
    shutil.copytree(source_a, other_parent / "source-brain", dirs_exist_ok=True)

    result = runner.invoke(cli, ["register", str(other_parent / "source-brain")])
    assert result.exit_code != 0
    assert "already registered" in result.output.lower()


def test_register_rejects_path_registered_under_different_name(tmp_path, monkeypatch):
    """If the same directory is already registered under a different name,
    register must tell the user to remove the existing registration first."""
    source = _make_source_brain(tmp_path, monkeypatch)

    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    assert runner.invoke(cli, ["register", str(source)]).exit_code == 0

    # Change the brain's identity (H1) so a second register would try a
    # different canonical name, but the path is already registered.
    brain_md = source / "brain.md"
    brain_md.write_text(
        "---\nauto_generated: true\nupdated: 2026-04-01\n---\n# renamed-brain\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["register", str(source)])
    assert result.exit_code != 0
    assert "already registered" in result.output.lower()
    assert "kluris remove" in result.output.lower()


# -----------------------------------------------------------------------------
# Install integration
# -----------------------------------------------------------------------------


def test_register_runs_install(tmp_path, monkeypatch):
    """register must trigger _do_install so agent skills are present."""
    source = _make_source_brain(tmp_path, monkeypatch)
    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    home = tmp_path / "register-home"
    # With one brain registered, the named layout still creates
    # ~/.claude/skills/kluris-<brain>/ with SKILL.md inside.
    claude_skill = home / ".claude" / "skills" / "kluris-source-brain" / "SKILL.md"
    assert claude_skill.exists(), "register must install the named kluris skill"


# -----------------------------------------------------------------------------
# Persisted shape — no `type`, no `repo`
# -----------------------------------------------------------------------------


def test_register_does_not_persist_repo(tmp_path, monkeypatch):
    """Even if the brain has a git origin remote, the persisted BrainEntry
    must not include a `repo` field — kluris no longer tracks remote URLs."""
    source = _make_source_brain(tmp_path, monkeypatch)
    # Wire up a fake origin remote. It does not need to be reachable.
    subprocess.run(
        ["git", "remote", "add", "origin", "git@example.com:team/source-brain.git"],
        cwd=source,
        capture_output=True,
        check=True,
    )

    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    config = read_global_config()
    entry = config.brains["source-brain"]
    assert not hasattr(entry, "repo")


def test_register_does_not_persist_type(tmp_path, monkeypatch):
    """The persisted BrainEntry must have no `type` field after register."""
    source = _make_source_brain(tmp_path, monkeypatch)

    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    config = read_global_config()
    entry = config.brains["source-brain"]
    assert not hasattr(entry, "type")


# -----------------------------------------------------------------------------
# Zip rejection (zip path removed in 2.16.0)
# -----------------------------------------------------------------------------


def test_register_rejects_zip_with_clear_hint(tmp_path, monkeypatch):
    """Passing a .zip path must error with a hint to unzip + git clone."""
    _use_fresh_registry(tmp_path, monkeypatch)
    zip_path = tmp_path / "old-brain.zip"
    zip_path.write_bytes(b"PK\x03\x04")  # minimal bytes — we never read it

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(zip_path)])

    assert result.exit_code != 0
    output = result.output.lower()
    assert "no longer accepts .zip" in output
    assert "unzip first" in output
    assert "git clone" in output


def test_register_no_dest_flag(tmp_path, monkeypatch):
    """--dest was removed when zip support was removed."""
    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    result = runner.invoke(cli, ["register", "--dest", "/tmp/foo", "/tmp/bar"])
    assert result.exit_code != 0
    # Click reports "no such option" for an unknown flag.
    assert "no such option" in result.output.lower() or "--dest" in result.output


# -----------------------------------------------------------------------------
# JSON output
# -----------------------------------------------------------------------------


def test_register_json_output(tmp_path, monkeypatch):
    import json as json_lib

    source = _make_source_brain(tmp_path, monkeypatch)
    _use_fresh_registry(tmp_path, monkeypatch)

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source), "--json"])

    assert result.exit_code == 0, result.output
    payload = json_lib.loads(result.output.strip())
    assert payload["ok"] is True
    assert payload["name"] == "source-brain"
    assert Path(payload["path"]).resolve() == source.resolve()
    # JSON must not advertise the dropped fields.
    assert "remote" not in payload
