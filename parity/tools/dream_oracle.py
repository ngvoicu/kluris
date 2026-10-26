"""The converged form of a file Kluris v2.28.11's ``dream`` wrote, computed with
v2's own frontmatter code. Run with the oracle venv's Python (``-I``).

Kluris 3's dream writes the same files as v2, in the form v2's dream date path
converges to, with the decided differences applied:

- every top-level date and datetime is written as its quoted ISO text
  (interview-01 #2; v2's linker path writes them unquoted);
- YAML anchors and aliases are written as plain copies (SPEC decision
  2026-10-25; v2 writes ``&id001`` / ``*id001``);
- the file keeps the line endings it had, a new file is LF (interview-01 #3);
- reverse ``related`` links come in Kluris 3's path order (PAR-008): when the
  v3 file's ``related`` list holds the same entries as v2's, v3's order is
  used.

stdin: a JSON list of ``{"name", "v2", "v3", "ending"}`` (``v2``/``v3`` the
file bytes, base64; ``ending`` ``lf`` or ``crlf``); stdout: a JSON list with,
for each, the expected v3 bytes (base64), or ``null`` when v2's file has no
frontmatter v2 can read.
"""

from __future__ import annotations

import base64
import json
import sys
import tempfile
from collections import Counter
from pathlib import Path

import frontmatter
import yaml

from kluris.core.frontmatter import read_frontmatter


def unshare(value):
    """A deep copy that shares nothing, so PyYAML writes no aliases."""
    if isinstance(value, dict):
        return {k: unshare(v) for k, v in value.items()}
    if isinstance(value, list):
        return [unshare(v) for v in value]
    return value


def read(directory: Path, name: str, data: bytes) -> tuple[dict, str]:
    path = directory / name
    path.write_bytes(data)
    return read_frontmatter(path)


def key(value) -> str:
    return json.dumps(value, sort_keys=True, default=str)


def same_entries(a, b) -> bool:
    return isinstance(a, list) and isinstance(b, list) and Counter(map(key, a)) == Counter(map(key, b))


def converge(directory: Path, item: dict) -> str | None:
    name = Path(item["name"]).name
    suffix = Path(name).suffix.lower()
    meta, body = read(directory, "v2-" + name, base64.b64decode(item["v2"]))
    v3_meta, _ = read(directory, "v3-" + name, base64.b64decode(item["v3"]))
    meta = unshare(meta)
    if "related" in meta and same_entries(meta["related"], v3_meta.get("related")):
        meta["related"] = unshare(v3_meta["related"])
    eol = "\r\n" if item["ending"] == "crlf" else "\n"
    if suffix in {".yml", ".yaml"}:
        if not meta:
            return None
        dumped = yaml.safe_dump(meta, sort_keys=True, default_flow_style=False)
        lines = [f"# {line}" if line else "#" for line in dumped.rstrip("\n").splitlines()]
        block = eol.join(["#---", *lines, "#---"]) + eol
        return base64.b64encode((block + body).encode("utf-8")).decode("ascii")
    text = frontmatter.dumps(frontmatter.Post(body, **meta)) + "\n"
    return base64.b64encode(text.replace("\n", eol).encode("utf-8")).decode("ascii")


def main() -> None:
    items = json.loads(sys.stdin.read())
    with tempfile.TemporaryDirectory() as tmp:
        out = [converge(Path(tmp), item) for item in items]
    sys.stdout.write(json.dumps(out))


if __name__ == "__main__":
    main()
