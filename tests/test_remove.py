"""Tests for kluris remove command."""

from click.testing import CliRunner
from kluris.cli import cli
from conftest import create_test_brain
from kluris.core.config import read_global_config


def test_remove_unregisters(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result = runner.invoke(cli, ["remove", "my-brain"])
    assert result.exit_code == 0
    config = read_global_config()
    assert "my-brain" not in config.brains


def test_remove_preserves_files(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    runner.invoke(cli, ["remove", "my-brain"])
    assert (tmp_path / "my-brain" / "kluris.yml").exists()


def test_remove_unknown(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    runner = CliRunner()
    result = runner.invoke(cli, ["remove", "nonexistent"])
    assert result.exit_code != 0


def test_remove_refuses_dirty_brain(tmp_path, monkeypatch):
    """Unregistering a brain with uncommitted changes must fail without --force."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    # Create uncommitted changes
    (tmp_path / "my-brain" / "projects" / "new.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# New\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["remove", "my-brain"])
    assert result.exit_code != 0
    assert "uncommitted" in result.output.lower()
    # Still registered
    assert "my-brain" in read_global_config().brains


def test_remove_force_overrides_dirty_brain(tmp_path, monkeypatch):
    """--force lets the user unregister a dirty brain anyway."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)

    (tmp_path / "my-brain" / "projects" / "new.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# New\n",
        encoding="utf-8",
    )

    result = runner.invoke(cli, ["remove", "my-brain", "--force"])
    assert result.exit_code == 0
    assert "my-brain" not in read_global_config().brains
    # Files on disk preserved
    assert (tmp_path / "my-brain" / "projects" / "new.md").exists()
