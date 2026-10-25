"""Python side of the float differential test (``parity/test_float_differential.py``).

Runs in the oracle venv. Reads one IEEE 754 bit pattern per stdin line (16 hex
digits) and answers one line per pattern: ``repr(x)``, a tab, ``json.dumps(x)``.
``cargo xtask float-repr`` answers the same lines from Rust.

    parity/.oracle/venv/bin/python -I parity/tools/float_oracle.py
"""

from __future__ import annotations

import json
import struct
import sys


def main() -> int:
    for line in sys.stdin:
        (x,) = struct.unpack(">d", bytes.fromhex(line.strip()))
        sys.stdout.write(f"{x!r}\t{json.dumps(x)}\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
