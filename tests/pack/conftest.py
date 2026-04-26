"""Shared fixtures for the pack test suite.

- ``fixture_brain``: minimal kluris brain on tmp_path (3 lobes,
  ``brain.md``, ``glossary.md``, deprecated neuron, yaml neuron with
  ``#---`` opt-in, sublobe).
- ``api_key_config`` / ``oauth_config``: pre-built :class:`Config`
  instances keyed off a ``http://test.invalid`` base URL — all tests
  that touch HTTP wrap this with ``respx``.
- ``stub_provider``: a fake :class:`LLMProvider` whose ``smoke_test``
  is a no-op; lets app-factory and route tests skip real HTTP entirely.
"""

from __future__ import annotations

from pathlib import Path
from typing import AsyncIterator, Any

import pytest

from kluris.pack.config import Config
from kluris.pack.providers.base import LLMProvider


# --- Fixture brain factory ---------------------------------------------------

_FRONTMATTER_TEMPLATE = """---
parent: ./map.md
created: 2026-01-01
updated: {updated}
tags: {tags}
related: {related}
{extra}---
"""


def _write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _neuron(meta_extra: str = "", *, updated: str = "2026-04-01",
            tags: str = "[]", related: str = "[]", body: str = "Body.") -> str:
    return _FRONTMATTER_TEMPLATE.format(
        updated=updated, tags=tags, related=related, extra=meta_extra,
    ) + body + "\n"


@pytest.fixture
def fixture_brain(tmp_path) -> Path:
    """Realistic kluris brain for retrieval-tool tests."""
    brain = tmp_path / "fixture-brain"
    brain.mkdir()

    _write(
        brain / "brain.md",
        "---\nauto_generated: true\n---\n# Fixture Brain\n\nA tiny brain.\n",
    )
    _write(
        brain / "glossary.md",
        "---\n---\n# Glossary\n\n"
        "**JWT** -- JSON Web Token used for stateless auth.\n"
        "**SIT** -- System integration testing environment.\n\n"
        "| Term | Meaning |\n"
        "|------|---------|\n"
        "| UAT | User acceptance testing environment |\n"
        "| Tenant | An isolated customer namespace inside the platform |\n",
    )

    # projects/ lobe with sub-lobe
    _write(
        brain / "projects" / "map.md",
        "---\nauto_generated: true\ndescription: Per-project notes\n---\n"
        "# Projects\n\nProject lobe.\n",
    )
    _write(
        brain / "projects" / "btb" / "map.md",
        "---\nauto_generated: true\n---\n# BTB\n\nBurnTheBurnout project.\n",
    )
    _write(
        brain / "projects" / "btb" / "auth.md",
        _neuron(
            updated="2026-04-12",
            tags='["auth", "oauth"]',
            related='["../../knowledge/jwt.md"]',
            body="# BTB Auth\n\nWe use JWT issued by Keycloak.",
        ),
    )

    # knowledge/ lobe — has the deprecated neuron and an active replacement
    _write(
        brain / "knowledge" / "map.md",
        "---\nauto_generated: true\ndescription: Decisions and learnings\n---\n"
        "# Knowledge\n\nKnowledge lobe.\n",
    )
    _write(
        brain / "knowledge" / "jwt.md",
        _neuron(
            updated="2026-04-10",
            tags='["jwt", "auth"]',
            related='["../projects/btb/auth.md"]',
            body="# JWT\n\nJSON Web Tokens are signed claims.",
        ),
    )
    _write(
        brain / "knowledge" / "raw-sql-modern.md",
        _neuron(
            updated="2026-04-15",
            tags='["sql", "decision"]',
            body="# Raw SQL Modern\n\nCurrent guidance: prefer raw SQL over JPA.",
        ),
    )
    _write(
        brain / "knowledge" / "raw-sql-old.md",
        _neuron(
            meta_extra="status: deprecated\ndeprecated_at: 2026-03-01\n"
                       "replaced_by: ./raw-sql-modern.md\n",
            updated="2026-02-01",
            tags='["sql"]',
            body="# Raw SQL Old\n\nOld guidance kept for history.",
        ),
    )

    # infrastructure/ lobe — has a yaml neuron with #--- opt-in
    _write(
        brain / "infrastructure" / "map.md",
        "---\nauto_generated: true\ndescription: Hosting and deploys\n---\n"
        "# Infrastructure\n\nInfra lobe.\n",
    )
    _write(
        brain / "infrastructure" / "openapi.yml",
        "#---\n# title: Internal API\n# updated: 2026-04-08\n#---\n"
        "openapi: 3.1.0\ninfo:\n  title: Internal API\n  version: 1.0.0\npaths: {}\n",
    )

    return brain


# --- Config fixtures ---------------------------------------------------------


@pytest.fixture
def api_key_env() -> dict:
    return {
        "KLURIS_PROVIDER_SHAPE": "anthropic",
        "KLURIS_BASE_URL": "http://test.invalid",
        "KLURIS_API_KEY": "sk-test-anthropic",
        "KLURIS_MODEL": "claude-test",
    }


@pytest.fixture
def oauth_env() -> dict:
    return {
        "KLURIS_OAUTH_TOKEN_URL": "http://idp.invalid/token",
        "KLURIS_OAUTH_API_BASE_URL": "http://api.invalid",
        "KLURIS_OAUTH_CLIENT_ID": "kluris-app",
        "KLURIS_OAUTH_CLIENT_SECRET": "oauth-secret-test",
        "KLURIS_MODEL": "internal-model",
    }


@pytest.fixture
def api_key_config(api_key_env, fixture_brain, tmp_path) -> Config:
    env = dict(
        api_key_env,
        KLURIS_BRAIN_DIR=str(fixture_brain),
        KLURIS_DATA_DIR=str(tmp_path / "data"),
    )
    (tmp_path / "data").mkdir()
    return Config.load_from_env(env)


# --- Stub provider -----------------------------------------------------------


class StubProvider(LLMProvider):
    """No-op provider for tests that don't exercise the LLM boundary."""

    model = "stub-model"

    def __init__(self, *, smoke_test_raises: Exception | None = None) -> None:
        self._smoke_raises = smoke_test_raises
        self.smoke_calls = 0

    async def smoke_test(self) -> None:
        self.smoke_calls += 1
        if self._smoke_raises is not None:
            raise self._smoke_raises

    async def complete_stream(  # type: ignore[override]
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[dict[str, Any]]:
        yield {"kind": "token", "text": "stub"}
        yield {"kind": "usage", "input": 0, "output": 0}
        yield {"kind": "end"}


@pytest.fixture
def stub_provider() -> StubProvider:
    return StubProvider()
