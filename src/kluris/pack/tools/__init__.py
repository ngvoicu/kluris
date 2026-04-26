"""Read-only brain retrieval tools.

Exactly eight tools mirror the kluris CLI vocabulary:

- ``wake_up``, ``search``, ``read_neuron``, ``multi_read``,
  ``related``, ``recent``, ``glossary``, ``lobe_overview``

Each is a thin wrapper around :mod:`kluris_runtime.*`. The wrappers
never call write APIs — :mod:`tests.pack.test_readonly_enforcement`
greps this directory for write patterns and fails CI if any appear.
"""

from .brain import (  # noqa: F401  (re-exports)
    NotFoundError,
    SandboxError,
    TOOLS,
    glossary_tool,
    lobe_overview_tool,
    multi_read_tool,
    read_neuron_tool,
    recent_tool,
    related_tool,
    resolve_in_brain,
    search_tool,
    wake_up_tool,
)
