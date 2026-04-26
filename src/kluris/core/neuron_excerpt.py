"""Compatibility wrapper for neuron title + first-line excerpt extraction.

The implementation lives in :mod:`kluris_runtime.neuron_excerpt`. The
MRI viewer and the packed chat server's ``lobe_overview`` tool both
need the same logic; the runtime owns it and core re-exports.
"""

from __future__ import annotations

from kluris_runtime.neuron_excerpt import extract  # noqa: F401  (re-export)
