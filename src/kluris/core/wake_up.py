"""Compatibility wrapper for the wake-up payload builder.

The implementation lives in :mod:`kluris_runtime.wake_up`. This module
re-exports ``build_payload`` so callers in ``kluris.core.*`` and tests
can keep importing from ``kluris.core.wake_up`` while the read-only
runtime remains the single source of truth.
"""

from __future__ import annotations

from kluris_runtime.wake_up import build_payload  # noqa: F401  (re-export)
