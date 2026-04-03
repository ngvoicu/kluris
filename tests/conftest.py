"""Shared test fixtures for kluris."""

import subprocess

import pytest
import yaml
from click.testing import CliRunner
from pathlib import Path


@pytest.fixture
def cli_runner():
    """Click CLI test runner."""
    return CliRunner()


@pytest.fixture
def temp_config(tmp_path, monkeypatch):
    """Temp global config. Sets KLURIS_CONFIG env var so all commands use it."""
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))
    return config_path


@pytest.fixture
def temp_home(tmp_path, monkeypatch):
    """Temp HOME dir so kluris install writes to tmp instead of real home."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))  # Windows compat
    return tmp_path


@pytest.fixture
def temp_brain(tmp_path, temp_config):
    """Scaffolded team brain in tmp_path with git init and config registered."""
    brain_path = tmp_path / "test-brain"
    brain_path.mkdir()

    (brain_path / "kluris.yml").write_text(yaml.dump({
        "name": "test-brain",
        "description": "Test brain",
        "type": "product-group",
        "git": {"default_branch": "main",
                "commit_prefix": "brain:"},
        "agents": {"commands_for": ["claude"]},
    }), encoding="utf-8")

    for lobe in ["architecture", "decisions", "product", "standards",
                 "services", "infrastructure", "learnings"]:
        lobe_dir = brain_path / lobe
        lobe_dir.mkdir()
        (lobe_dir / "map.md").write_text(
            f"---\nauto_generated: true\nparent: ../brain.md\n"
            f"updated: 2026-04-01\n---\n# {lobe.title()}\n", encoding="utf-8"
        )

    (brain_path / "brain.md").write_text(
        "---\nauto_generated: true\nupdated: 2026-04-01\n---\n# Test Brain\n", encoding="utf-8"
    )
    (brain_path / "glossary.md").write_text(
        "---\nauto_generated: false\nupdated: 2026-04-01\n---\n# Glossary\n", encoding="utf-8"
    )
    (brain_path / "README.md").write_text("# Test Brain\n", encoding="utf-8")

    subprocess.run(["git", "init"], cwd=brain_path, capture_output=True)
    subprocess.run(["git", "checkout", "-b", "main"], cwd=brain_path, capture_output=True)
    subprocess.run(["git", "config", "user.email", "test@test"], cwd=brain_path, capture_output=True)
    subprocess.run(["git", "config", "user.name", "test"], cwd=brain_path, capture_output=True)
    subprocess.run(["git", "add", "-A"], cwd=brain_path, capture_output=True)
    subprocess.run(
        ["git", "commit", "-m", "brain: initialize test-brain"],
        cwd=brain_path, capture_output=True,
    )

    config_data = {
        "default_brain": "test-brain",
        "brains": {
            "test-brain": {
                "path": str(brain_path),
                "description": "Test brain",
                "type": "product-group",
            }
        },
    }
    temp_config.write_text(yaml.dump(config_data), encoding="utf-8")

    return brain_path


@pytest.fixture
def bare_remote(tmp_path_factory):
    """Bare git repo for testing push/clone."""
    remote_path = tmp_path_factory.mktemp("remote")
    subprocess.run(["git", "init", "--bare"], cwd=remote_path,
                   capture_output=True)
    subprocess.run(["git", "symbolic-ref", "HEAD", "refs/heads/main"],
                   cwd=remote_path, capture_output=True)
    return remote_path
