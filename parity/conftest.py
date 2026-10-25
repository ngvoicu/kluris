"""Fixtures for the parity harness; the helpers live in ``harness.py``."""

from __future__ import annotations

import pytest

from harness import ORACLE_STATS, Sandbox, oracle_kluris


@pytest.fixture(scope="session")
def oracle():
    """The oracle's kluris executable, built (or reused) once per session."""
    return oracle_kluris()


@pytest.fixture
def sandbox(tmp_path, oracle) -> Sandbox:
    """A fresh isolated environment (HOME, config, git, locale) per test."""
    return Sandbox(tmp_path)


def pytest_terminal_summary(terminalreporter):
    if ORACLE_STATS:
        how = "reused" if ORACLE_STATS["reused"] else "built"
        terminalreporter.write_line(
            f"parity oracle: {how} in {ORACLE_STATS['seconds']:.1f}s at {ORACLE_STATS['venv']}"
        )
