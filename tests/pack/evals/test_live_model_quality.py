"""TEST-PACK-53 — optional live-model evals.

Skipped unless the deployer sets explicit ``KLURIS_EVAL_*`` env vars.
We don't run a real model in default CI; this file exists to document
the opt-in surface and to register the ``llm_eval`` marker.
"""

from __future__ import annotations

import os

import pytest


pytestmark = pytest.mark.llm_eval


REQUIRED_API_KEY_VARS = (
    "KLURIS_EVAL_PROVIDER_SHAPE",
    "KLURIS_EVAL_BASE_URL",
    "KLURIS_EVAL_API_KEY",
    "KLURIS_EVAL_MODEL",
)


def _env_ready() -> bool:
    return all(os.environ.get(v) for v in REQUIRED_API_KEY_VARS)


@pytest.mark.skipif(not _env_ready(), reason="KLURIS_EVAL_* env vars not set")
def test_live_model_round_trip():  # pragma: no cover (opt-in)
    """Placeholder — full implementation lives behind the env gate.

    Run with:
        KLURIS_EVAL_PROVIDER_SHAPE=anthropic \\
        KLURIS_EVAL_BASE_URL=... \\
        KLURIS_EVAL_API_KEY=... \\
        KLURIS_EVAL_MODEL=... \\
        pytest -m llm_eval tests/pack/evals
    """
    pytest.skip("live evals are documentation-only in v1")
