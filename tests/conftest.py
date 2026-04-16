"""Shared test fixtures for kluris."""

import subprocess

import pytest
import yaml
from click.testing import CliRunner
from pathlib import Path

from kluris.cli import cli


def create_test_brain(runner, name, path, **extra_flags):
    """Non-interactive brain creation for tests. Passes all flags to skip prompts."""
    cmd = ["create", name, "--path", str(path),
           "--description", f"{name} knowledge base", "--json"]
    for k, v in extra_flags.items():
        flag = f"--{k.replace('_', '-')}"
        if isinstance(v, bool) and v:
            cmd.append(flag)
        else:
            cmd.extend([flag, str(v)])
    return runner.invoke(cli, cmd)


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
        "git": {"commit_prefix": "brain:"},
        "agents": {"commands_for": ["claude"]},
    }), encoding="utf-8")

    for lobe in ["projects", "infrastructure", "knowledge"]:
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


def create_test_brain_with_neurons(runner, name, path, count=100):
    """Scaffold a brain via the CLI and add ``count`` committed neurons.

    Used by Phase 2 tests that need a large brain to verify subprocess
    counts on dream's batch git path. Each neuron is a minimal frontmatter
    + body file under ``<brain>/projects/n_NN.md``. All N neurons are
    committed in a single git commit so the resulting brain has 2 commits
    total (the kluris create scaffold + the neurons add).
    """
    create_test_brain(runner, name, path)
    brain_path = path / name
    projects = brain_path / "projects"
    projects.mkdir(exist_ok=True)
    for i in range(count):
        (projects / f"n_{i:04d}.md").write_text(
            f"---\nparent: ./map.md\ntags: []\n"
            f"created: 2026-01-01\nupdated: 2026-04-01\n---\n"
            f"# Neuron {i}\n\nbody {i}\n",
            encoding="utf-8",
        )
    subprocess.run(["git", "add", "-A"], cwd=brain_path, capture_output=True)
    subprocess.run(
        ["git", "commit", "-m", f"add {count} neurons"],
        cwd=brain_path, capture_output=True,
    )
    return brain_path


@pytest.fixture
def counting_git_run(monkeypatch):
    """Wrap ``core.git._run`` so tests can count subprocess invocations.

    Returns a list-like counter object with ``.count`` attribute that
    increments on every ``_run`` call. Tests can monkeypatch through this
    fixture to enforce subprocess-count assertions on dream/mri batch behavior.
    """
    from kluris.core import git as git_module

    class _Counter:
        def __init__(self):
            self.count = 0
            self.calls: list[list[str]] = []

    counter = _Counter()
    original = git_module._run

    def _wrapped(*args, **kwargs):
        counter.count += 1
        if args and isinstance(args[0], list):
            counter.calls.append(args[0])
        return original(*args, **kwargs)

    monkeypatch.setattr(git_module, "_run", _wrapped)
    return counter
