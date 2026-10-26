"""v2 side of the brain-model comparison (``parity/test_brain_model.py``).

Runs in the oracle venv (Kluris v2.28.11). Prints one JSON object for the brain
at ``argv[1]``, in the shape ``cargo xtask brain-dump --path <brain>`` prints:

- ``neurons``: ``kluris_runtime.neuron_index.neuron_files`` as
  ``{"path": <POSIX relative path>, "kind": "markdown" | "yaml"}``, in v2's
  ``os.walk`` order.
- ``glossary``: ``parse_glossary_entries`` of ``glossary.md``'s body as
  ``[term, definition]`` pairs; ``[]`` without the file, ``null`` when its
  frontmatter cannot be read (wake-up and search then use no entries).
- ``deprecation``: ``detect_deprecation_issues`` over the neurons whose
  frontmatter reads, in the order of ``argv[2]`` (a JSON file holding a list of
  POSIX relative paths, e.g. v3's neuron order) or else v2's own order. Kluris 3
  skips a neuron that cannot be read instead of failing (PAR-007); v2's own
  ``preparsed=`` argument runs v2's exact rules over the rest.
- ``skipped``: the POSIX relative paths of the neurons whose frontmatter does
  not read, in that same order.
- ``deprecation_plain``: v2's own call without ``preparsed`` (its own walk), or
  ``{"error": <exception type>}`` when it raises (as it does on any neuron that
  cannot be read).

    parity/.oracle/venv/bin/python -I parity/tools/brain_model_oracle.py <brain> [order.json]
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from kluris_runtime.deprecation import detect_deprecation_issues
from kluris_runtime.frontmatter import read_frontmatter
from kluris_runtime.neuron_index import YAML_NEURON_SUFFIXES, neuron_files
from kluris_runtime.search import parse_glossary_entries


def _posix(brain: Path, path: Path) -> str:
    return path.relative_to(brain).as_posix()


def _kind(path: Path) -> str:
    return "yaml" if path.suffix in YAML_NEURON_SUFFIXES else "markdown"


def _glossary(brain: Path):
    path = brain / "glossary.md"
    if not path.exists():
        return []
    try:
        _meta, body = read_frontmatter(path)
    except Exception:
        return None
    return [[term, definition] for term, definition in parse_glossary_entries(body)]


def main() -> None:
    brain = Path(sys.argv[1]).resolve()
    walked = neuron_files(brain)
    if len(sys.argv) > 2:
        order = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
        ordered = [brain.joinpath(*rel.split("/")) for rel in order]
    else:
        ordered = walked

    preparsed = []
    skipped = []
    for neuron in ordered:
        try:
            meta, _body = read_frontmatter(neuron)
        except Exception:
            skipped.append(_posix(brain, neuron))
            continue
        preparsed.append((neuron, meta))

    try:
        plain = detect_deprecation_issues(brain)
    except Exception as exc:  # noqa: BLE001 - reported, compared by the test
        plain = {"error": type(exc).__name__}

    out = {
        "neurons": [{"path": _posix(brain, p), "kind": _kind(p)} for p in walked],
        "glossary": _glossary(brain),
        "deprecation": detect_deprecation_issues(brain, preparsed=preparsed),
        "skipped": skipped,
        "deprecation_plain": plain,
    }
    sys.stdout.write(json.dumps(out, ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
