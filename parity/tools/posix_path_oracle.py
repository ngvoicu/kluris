"""v2 side of ``parity/test_skill_install.py``'s path differential, run in the
oracle venv (Kluris v2.28.11, Python 3.11): one JSON string per stdin line,
answered with the JSON string ``kluris.core.agents._posix_path`` gives for it
(the form SKILL.md writes brain and companion paths in).

    parity/.oracle/venv/bin/python -I parity/tools/posix_path_oracle.py
"""

from __future__ import annotations

import json
import sys

from kluris.core.agents import _posix_path


def main() -> None:
    for line in sys.stdin:
        sys.stdout.write(json.dumps(_posix_path(json.loads(line))) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
