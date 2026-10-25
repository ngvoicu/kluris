"""The harness's own behaviour, independent of the two binaries."""

from __future__ import annotations

import os
import sys
from pathlib import Path

from harness import _run


def test_run_turns_crlf_into_lf(tmp_path: Path):
    # Python's text streams write CRLF on Windows, Rust's println! writes LF:
    # both sides are compared with LF (allowlist PAR-004).
    script = tmp_path / "say.py"
    script.write_text(
        "import sys\n"
        "sys.stdout.buffer.write(b'a\\r\\nb\\r\\nlone\\rcr\\n')\n"
        "sys.stderr.buffer.write(b'e\\r\\n')\n",
        encoding="utf-8",
    )
    r = _run(Path(sys.executable), [str(script)], dict(os.environ), None)
    assert r.code == 0, r.stderr
    assert r.stdout == "a\nb\nlone\rcr\n"
    assert r.stderr == "e\n"
