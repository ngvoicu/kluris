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
    assert kluris.__version__ == "2.13.11"


def test_global_config_defaults():
    cfg = GlobalConfig()
    assert cfg.brains == {}


def test_global_config_with_brains():
    cfg = GlobalConfig(
        brains={
            "my-brain": BrainEntry(
                path="/home/user/my-brain",
                repo="https://github.com/team/brain.git",
                description="Team brain",
                type="product-group",
            )
        },
    )
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
    assert cfg.companions == []


def test_git_config_defaults():
    cfg = GitConfig()
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
        brains={"test": BrainEntry(path="/tmp/test", description="Test")},
    )
    write_global_config(cfg)
    loaded = read_global_config()
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
    assert loaded.companions == []


def test_brain_config_companions_round_trip(tmp_path):
    brain_path = tmp_path / "my-brain"
    brain_path.mkdir()

    cfg = BrainConfig(
        name="my-brain",
        description="Test brain",
        companions=["specmint-core", "specmint-tdd"],
    )
    write_brain_config(cfg, brain_path)

    loaded = read_brain_config(brain_path)
    assert loaded.companions == ["specmint-core", "specmint-tdd"]


def test_old_brain_config_without_companions_loads_with_default(tmp_path):
    brain_path = tmp_path / "old-brain"
    brain_path.mkdir()
    (brain_path / "kluris.yml").write_text(
        "name: old-brain\ndescription: Old brain\n",
        encoding="utf-8",
    )

    loaded = read_brain_config(brain_path)
    assert loaded.companions == []


def test_config_not_found_returns_empty(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "nonexistent.yml"))
    cfg = read_global_config()
    assert cfg.brains == {}


def test_legacy_default_brain_pointing_at_real_brain_is_silently_dropped(tmp_path, monkeypatch):
    """Loading a kluris<=1.6.x YAML with default_brain set to a registered brain works."""
    import yaml
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))
    config_path.write_text(yaml.dump({
        "default_brain": "my-brain",
        "brains": {"my-brain": {"path": "/tmp/my-brain", "description": "x", "type": "product-group"}},
    }), encoding="utf-8")
    cfg = read_global_config()
    assert "my-brain" in cfg.brains
    assert not hasattr(cfg, "default_brain")


def test_legacy_default_brain_pointing_at_unregistered_brain_is_silently_dropped(tmp_path, monkeypatch):
    """Loading a kluris<=1.6.x YAML with default_brain pointing at an unknown brain still works."""
    import yaml
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))
    config_path.write_text(yaml.dump({
        "default_brain": "ghost",
        "brains": {"foo": {"path": "/tmp/foo", "description": "x", "type": "product-group"}},
    }), encoding="utf-8")
    cfg = read_global_config()
    assert "foo" in cfg.brains
    assert "ghost" not in cfg.brains
    assert not hasattr(cfg, "default_brain")


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
