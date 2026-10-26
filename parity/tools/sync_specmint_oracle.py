"""Answer sync-specmint requests with Kluris v2.28.11's own script
(``v2_sync_specmint.py``, a byte-identical copy of v2.28.11's
``scripts/sync-specmint.py``), one JSON line per JSON line:

- ``{"report": {"name", "old", "new", "full_diff"}}`` (trees as
  ``{path: base64}``) -> ``{"changed": bool, "out": <what _report printed>}``;
- ``{"mentions": <text>}`` -> ``_PATH_MENTION.findall(text)``.
"""

from __future__ import annotations

import base64
import contextlib
import importlib.util
import io
import json
import sys
from pathlib import Path

_SCRIPT = Path(__file__).resolve().parent / "v2_sync_specmint.py"


def _load():
    spec = importlib.util.spec_from_file_location("v2_sync_specmint", _SCRIPT)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module  # dataclasses look the module up by name
    spec.loader.exec_module(module)
    return module


def main() -> None:
    sync = _load()
    for line in sys.stdin:
        request = json.loads(line)
        if "report" in request:
            r = request["report"]
            tree = lambda t: {k: base64.b64decode(v) for k, v in t.items()}  # noqa: E731
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                changed = sync._report(r["name"], tree(r["old"]), tree(r["new"]), full_diff=r["full_diff"])
            answer = {"changed": changed, "out": buf.getvalue()}
        else:
            answer = sync._PATH_MENTION.findall(request["mentions"])
        sys.stdout.write(json.dumps(answer) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
