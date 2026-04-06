"""Tests for Pydantic data models, config read/write, and package structure."""

from pathlib import Path

import kluris
from kluris.core.config import (
    AgentsConfig,
    BrainConfig,
    BrainEntry,
    GitConfig,
    GlobalConfig,
    get_config_path,
    read_brain_config,
    read_global_config,
    register_brain,
    unregister_brain,
    write_brain_config,
    write_global_config,
)


# --- [TEST-KLU-01] Pydantic data models and package structure ---


def test_kluris_importable():
    assert hasattr(kluris, "__version__")
    assert kluris.__version__ == "1.3.7"


def test_global_config_defaults():
    cfg = GlobalConfig()
    assert cfg.brains == {}
    assert cfg.default_brain is None


def test_global_config_with_brains():
    cfg = GlobalConfig(
        default_brain="my-brain",
        brains={
            "my-brain": BrainEntry(
                path="/home/user/my-brain",
                repo="https://github.com/team/brain.git",
                description="Team brain",
                type="product-group",
            )
        },
    )
    assert cfg.default_brain == "my-brain"
    assert "my-brain" in cfg.brains
    entry = cfg.brains["my-brain"]
    assert entry.path == "/home/user/my-brain"
    assert entry.repo == "https://github.com/team/brain.git"
    assert entry.description == "Team brain"
    assert entry.type == "product-group"


def test_brain_entry_defaults():
    entry = BrainEntry(path="/x")
    assert entry.path == "/x"
    assert entry.repo is None
    assert entry.description == ""
    assert entry.type == "product-group"


def test_brain_config_defaults():
    cfg = BrainConfig(name="x")
    assert cfg.name == "x"
    assert cfg.description == ""
    assert isinstance(cfg.git, GitConfig)
    assert isinstance(cfg.agents, AgentsConfig)


def test_git_config_defaults():
    cfg = GitConfig()
    assert cfg.default_branch == "main"
    assert cfg.commit_prefix == "brain:"


def test_agents_config_defaults():
    cfg = AgentsConfig()
    assert len(cfg.commands_for) == 8
    assert "claude" in cfg.commands_for
    assert "cursor" in cfg.commands_for
    assert "windsurf" in cfg.commands_for
    assert "copilot" in cfg.commands_for
    assert "codex" in cfg.commands_for
    assert "kilocode" in cfg.commands_for
    assert "gemini" in cfg.commands_for
    assert "junie" in cfg.commands_for


# --- [TEST-KLU-03] Config read/write with KLURIS_CONFIG ---


def test_write_read_global_config(tmp_path, monkeypatch):
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))

    cfg = GlobalConfig(
        default_brain="test",
        brains={"test": BrainEntry(path="/tmp/test", description="Test")},
    )
    write_global_config(cfg)
    loaded = read_global_config()
    assert loaded.default_brain == "test"
    assert "test" in loaded.brains
    assert loaded.brains["test"].path == "/tmp/test"


def test_config_path_from_env(tmp_path, monkeypatch):
    custom_path = tmp_path / "custom" / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(custom_path))
    assert get_config_path() == custom_path


def test_config_path_default(monkeypatch):
    monkeypatch.delenv("KLURIS_CONFIG", raising=False)
    expected = Path.home() / ".kluris" / "config.yml"
    assert get_config_path() == expected


def test_write_read_brain_config(tmp_path):
    brain_path = tmp_path / "my-brain"
    brain_path.mkdir()

    cfg = BrainConfig(name="my-brain", description="Test brain")
    write_brain_config(cfg, brain_path)

    loaded = read_brain_config(brain_path)
    assert loaded.name == "my-brain"
    assert loaded.description == "Test brain"


def test_config_not_found_returns_empty(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "nonexistent.yml"))
    cfg = read_global_config()
    assert cfg.brains == {}
    assert cfg.default_brain is None


def test_register_brain(tmp_path, monkeypatch):
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))

    register_brain("my-brain", BrainEntry(path="/tmp/brain", description="A brain"))
    cfg = read_global_config()
    assert "my-brain" in cfg.brains
    assert cfg.brains["my-brain"].path == "/tmp/brain"


def test_unregister_brain(tmp_path, monkeypatch):
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))

    register_brain("my-brain", BrainEntry(path="/tmp/brain"))
    unregister_brain("my-brain")
    cfg = read_global_config()
    assert "my-brain" not in cfg.brains
