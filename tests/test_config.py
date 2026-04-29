"""Tests for Pydantic data models, config read/write, and package structure."""

from pathlib import Path

import pytest
import kluris
from kluris.core.config import (
    AgentsConfig,
    BrainConfig,
    BrainEntry,
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
    assert kluris.__version__ == "2.16.3"


def test_global_config_defaults():
    cfg = GlobalConfig()
    assert cfg.brains == {}


def test_global_config_with_brains():
    cfg = GlobalConfig(
        brains={
            "my-brain": BrainEntry(
                path="/home/user/my-brain",
                description="Team brain",
            )
        },
    )
    assert "my-brain" in cfg.brains
    entry = cfg.brains["my-brain"]
    assert entry.path == "/home/user/my-brain"
    assert entry.description == "Team brain"


def test_brain_entry_defaults():
    entry = BrainEntry(path="/x")
    assert entry.path == "/x"
    assert entry.description == ""


def test_brain_entry_model_shape():
    """BrainEntry only carries `path` and `description` as of 2.16.0."""
    assert BrainEntry.model_fields.keys() == {"path", "description"}


def test_brain_config_defaults():
    cfg = BrainConfig(name="x")
    assert cfg.name == "x"
    assert cfg.description == ""
    assert isinstance(cfg.agents, AgentsConfig)
    assert cfg.companions == []


def test_brain_config_model_shape():
    """BrainConfig drops the legacy `git` block as of 2.16.0."""
    assert BrainConfig.model_fields.keys() == {"name", "description", "agents", "companions"}


def test_git_config_class_removed():
    """`GitConfig` was deleted along with `BrainConfig.git`."""
    with pytest.raises(ImportError):
        from kluris.core.config import GitConfig  # noqa: F401


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


# --- Legacy-tolerance regressions: old YAML keys must load silently ---


def test_legacy_brain_entry_keys_are_tolerated(tmp_path, monkeypatch):
    """Loading a 2.15.x global config with `type:` and `repo:` per brain must
    succeed. The runtime model omits both fields."""
    import yaml
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))
    config_path.write_text(yaml.dump({
        "brains": {
            "foo": {
                "path": "/tmp/foo",
                "description": "x",
                "type": "product-group",     # legacy
                "repo": "git@example:t/foo", # legacy
            }
        },
    }), encoding="utf-8")
    cfg = read_global_config()
    assert "foo" in cfg.brains
    entry = cfg.brains["foo"]
    assert entry.path == "/tmp/foo"
    assert entry.description == "x"
    assert not hasattr(entry, "type")
    assert not hasattr(entry, "repo")
    assert entry.model_dump() == {"path": "/tmp/foo", "description": "x"}


def test_legacy_kluris_yml_git_block_is_tolerated(tmp_path):
    """Loading a 2.15.x kluris.yml with a `git: { commit_prefix: ... }` block
    must succeed. The runtime model has no `git` attribute."""
    brain_path = tmp_path / "old-brain"
    brain_path.mkdir()
    (brain_path / "kluris.yml").write_text(
        "name: old-brain\ndescription: y\ngit:\n  commit_prefix: 'brain:'\n",
        encoding="utf-8",
    )

    loaded = read_brain_config(brain_path)
    assert loaded.name == "old-brain"
    assert loaded.description == "y"
    assert not hasattr(loaded, "git")


def test_write_global_config_does_not_emit_legacy_keys(tmp_path, monkeypatch):
    """Round-trip: writing a fresh BrainEntry must not produce any of the
    dropped keys (`type`, `repo`) on disk."""
    config_path = tmp_path / "config.yml"
    monkeypatch.setenv("KLURIS_CONFIG", str(config_path))
    register_brain("foo", BrainEntry(path="/tmp/foo", description="x"))
    raw = config_path.read_text(encoding="utf-8")
    assert "type" not in raw
    assert "repo" not in raw


def test_write_brain_config_does_not_emit_legacy_keys(tmp_path):
    """Round-trip: writing a fresh BrainConfig must not produce a `git:` block."""
    brain_path = tmp_path / "new-brain"
    brain_path.mkdir()
    write_brain_config(BrainConfig(name="new-brain", description="y"), brain_path)
    raw = (brain_path / "kluris.yml").read_text(encoding="utf-8")
    assert "git:" not in raw
    assert "commit_prefix" not in raw


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
