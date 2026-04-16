"""Global and brain configuration models."""

from __future__ import annotations

import os
from pathlib import Path

import yaml
from pydantic import BaseModel


class BrainEntry(BaseModel):
    """A registered brain in the global config."""
    path: str
    repo: str | None = None
    description: str = ""
    type: str = "product-group"


class GlobalConfig(BaseModel):
    """Global kluris config at ~/.config/kluris/config.yml."""
    brains: dict[str, BrainEntry] = {}


class GitConfig(BaseModel):
    """Git settings for a brain."""
    commit_prefix: str = "brain:"


class AgentsConfig(BaseModel):
    """Agent installation settings for a brain."""
    commands_for: list[str] = [
        "claude", "cursor", "windsurf", "copilot",
        "codex", "kilocode", "gemini", "junie",
    ]


class NeuronTemplate(BaseModel):
    """A structured template for creating neurons."""
    description: str = ""
    sections: list[str] = []


class StructureNode(BaseModel):
    """Nested structure definition for brain scaffolding."""
    description: str = ""
    children: dict[str, str | StructureNode] = {}


class BrainConfig(BaseModel):
    """Local brain config stored in kluris.yml (gitignored)."""
    name: str
    description: str = ""
    git: GitConfig = GitConfig()
    agents: AgentsConfig = AgentsConfig()


# --- Config file operations ---


def get_config_path() -> Path:
    """Return the global config path, respecting KLURIS_CONFIG env var."""
    env_path = os.environ.get("KLURIS_CONFIG")
    if env_path:
        return Path(env_path)
    return Path.home() / ".kluris" / "config.yml"


def read_global_config() -> GlobalConfig:
    """Read global config from disk. Returns empty config if file doesn't exist."""
    path = get_config_path()
    if not path.exists():
        return GlobalConfig()
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    # Legacy field cleanup: kluris <= 1.6.x had a `default_brain` field that
    # was removed when per-brain skill installs replaced the single shared
    # skill. Drop it explicitly so old YAML loads cleanly even if the model
    # is later tightened to extra="forbid".
    if isinstance(data, dict):
        data.pop("default_brain", None)
    return GlobalConfig.model_validate(data)


def write_global_config(config: GlobalConfig) -> None:
    """Write global config to disk, creating parent dirs if needed."""
    path = get_config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    data = config.model_dump(exclude_none=True)
    # Convert BrainEntry models to plain dicts for YAML
    if "brains" in data:
        data["brains"] = {
            k: {kk: vv for kk, vv in v.items() if vv is not None}
            for k, v in data["brains"].items()
        }
    path.write_text(yaml.dump(data, default_flow_style=False, sort_keys=False), encoding="utf-8")


def read_brain_config(brain_path: Path) -> BrainConfig:
    """Read kluris.yml from a brain directory."""
    config_file = brain_path / "kluris.yml"
    data = yaml.safe_load(config_file.read_text(encoding="utf-8")) or {}
    return BrainConfig.model_validate(data)


def write_brain_config(config: BrainConfig, brain_path: Path) -> None:
    """Write kluris.yml to a brain directory."""
    config_file = brain_path / "kluris.yml"
    data = config.model_dump(exclude_none=True)
    config_file.write_text(yaml.dump(data, default_flow_style=False, sort_keys=False), encoding="utf-8")


def register_brain(name: str, entry: BrainEntry) -> None:
    """Add a brain to the global config."""
    config = read_global_config()
    config.brains[name] = entry
    write_global_config(config)


def unregister_brain(name: str) -> None:
    """Remove a brain from the global config."""
    config = read_global_config()
    config.brains.pop(name, None)
    write_global_config(config)
