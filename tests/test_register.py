"""Tests for the kluris register command."""

from __future__ import annotations

import subprocess
import zipfile
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


def _zip_brain(brain_path: Path, out_path: Path, include_top_level: bool = False) -> Path:
    """Zip a brain into ``out_path``.

    - include_top_level=False: members are flat relative to brain root
      (``brain.md``, ``projects/map.md``, ...).
    - include_top_level=True: members are under ``<brain_name>/`` inside the zip.
    """
    with zipfile.ZipFile(out_path, "w") as zf:
        for item in brain_path.rglob("*"):
            if ".git" in item.parts:
                continue
            if not item.is_file():
                continue
            rel = item.relative_to(brain_path)
            arcname = f"{brain_path.name}/{rel}" if include_top_level else str(rel)
            zf.write(item, arcname)
    return out_path


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
    assert brain_config.git.commit_prefix == "custom:"


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
    # With one brain registered, the single-brain layout creates
    # ~/.claude/skills/kluris/ with SKILL.md inside.
    claude_skill = home / ".claude" / "skills" / "kluris" / "SKILL.md"
    assert claude_skill.exists(), "register must install the kluris skill"


# -----------------------------------------------------------------------------
# Git remote auto-detection
# -----------------------------------------------------------------------------


def test_register_captures_git_remote_when_set(tmp_path, monkeypatch):
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
    assert config.brains["source-brain"].repo == "git@example.com:team/source-brain.git"


def test_register_no_git_no_remote(tmp_path, monkeypatch):
    """A brain directory without a .git dir must register with repo=None."""
    source = _make_source_brain(tmp_path, monkeypatch)
    # Delete the .git dir so we look like a fresh unpacked directory.
    import shutil
    shutil.rmtree(source / ".git")

    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(source)])

    assert result.exit_code == 0, result.output
    config = read_global_config()
    assert config.brains["source-brain"].repo is None


# -----------------------------------------------------------------------------
# Zip registration
# -----------------------------------------------------------------------------


def test_register_zip_flat_layout(tmp_path, monkeypatch):
    """Zip packed from inside the brain (flat) must register correctly."""
    source = _make_source_brain(tmp_path, monkeypatch)
    zip_path = tmp_path / "source-brain.zip"
    _zip_brain(source, zip_path, include_top_level=False)

    _use_fresh_registry(tmp_path, monkeypatch)
    dest = tmp_path / "extracted-flat"
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(zip_path), "--dest", str(dest)])

    assert result.exit_code == 0, result.output
    assert (dest / "brain.md").exists()
    config = read_global_config()
    assert "source-brain" in config.brains
    assert Path(config.brains["source-brain"].path).resolve() == dest.resolve()


def test_register_zip_with_top_level_dir(tmp_path, monkeypatch):
    """Zip with a single top-level dir (GitHub-style export) must register."""
    source = _make_source_brain(tmp_path, monkeypatch)
    zip_path = tmp_path / "source-brain.zip"
    _zip_brain(source, zip_path, include_top_level=True)

    _use_fresh_registry(tmp_path, monkeypatch)
    dest = tmp_path / "extracted-nested"
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(zip_path), "--dest", str(dest)])

    assert result.exit_code == 0, result.output
    # Brain root is dest/<name>/ because the archive nested the brain one level.
    brain_root = dest / "source-brain"
    assert brain_root.exists()
    assert (brain_root / "brain.md").exists()
    config = read_global_config()
    assert "source-brain" in config.brains
    assert Path(config.brains["source-brain"].path).resolve() == brain_root.resolve()


def test_register_zip_rejects_path_traversal(tmp_path, monkeypatch):
    """Zip slip defense: a member that escapes the destination must be rejected."""
    _use_fresh_registry(tmp_path, monkeypatch)

    evil_zip = tmp_path / "evil.zip"
    with zipfile.ZipFile(evil_zip, "w") as zf:
        zf.writestr("brain.md", "---\nauto_generated: true\n---\n# evil-brain\n")
        # Attempt to write outside the extraction destination.
        zf.writestr("../escaped.txt", "pwned")

    dest = tmp_path / "sandbox"
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(evil_zip), "--dest", str(dest)])

    assert result.exit_code != 0
    assert "unsafe path" in result.output.lower()
    # The extraction dir was created but must be cleaned up on failure.
    assert not dest.exists()
    # The escape target must never exist.
    assert not (tmp_path / "escaped.txt").exists()


def test_register_zip_dest_collision(tmp_path, monkeypatch):
    """If --dest points to a non-empty dir, abort rather than overwrite."""
    source = _make_source_brain(tmp_path, monkeypatch)
    zip_path = tmp_path / "source-brain.zip"
    _zip_brain(source, zip_path, include_top_level=False)

    _use_fresh_registry(tmp_path, monkeypatch)
    dest = tmp_path / "occupied"
    dest.mkdir()
    (dest / "existing-file.txt").write_text("do not overwrite me", encoding="utf-8")

    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(zip_path), "--dest", str(dest)])

    assert result.exit_code != 0
    assert "not empty" in result.output.lower()
    # Pre-existing file must still be there.
    assert (dest / "existing-file.txt").read_text(encoding="utf-8") == "do not overwrite me"


def test_register_zip_without_brain_md(tmp_path, monkeypatch):
    """Zip that does not contain a brain.md must be rejected and extraction cleaned up."""
    _use_fresh_registry(tmp_path, monkeypatch)

    bogus_zip = tmp_path / "not-a-brain.zip"
    with zipfile.ZipFile(bogus_zip, "w") as zf:
        zf.writestr("README.md", "# not a brain\n")

    dest = tmp_path / "extract-here"
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(bogus_zip), "--dest", str(dest)])

    assert result.exit_code != 0
    assert "not a Kluris brain" in result.output or "missing brain.md" in result.output.lower()
    # Cleanup contract: we extracted, so we delete.
    assert not dest.exists()


def test_register_zip_missing_file(tmp_path, monkeypatch):
    _use_fresh_registry(tmp_path, monkeypatch)
    runner = CliRunner()
    result = runner.invoke(cli, ["register", str(tmp_path / "missing.zip")])

    assert result.exit_code != 0
    assert "not found" in result.output.lower()


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
