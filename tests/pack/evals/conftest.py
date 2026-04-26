"""Eval-suite fixtures."""

from __future__ import annotations

from pathlib import Path

import pytest

from kluris.pack.config import Config


@pytest.fixture
def eval_config(fixture_brain: Path, tmp_path) -> Config:
    """Config scoped to the shared fixture brain + a fresh data dir."""
    env = {
        "KLURIS_PROVIDER_SHAPE": "anthropic",
        "KLURIS_BASE_URL": "http://api.test",
        "KLURIS_API_KEY": "sk-eval",
        "KLURIS_MODEL": "scripted-eval",
        "KLURIS_BRAIN_DIR": str(fixture_brain),
        "KLURIS_DATA_DIR": str(tmp_path / "data"),
    }
    (tmp_path / "data").mkdir()
    return Config.load_from_env(env)
