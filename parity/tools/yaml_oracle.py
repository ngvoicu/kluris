"""PyYAML side of the YAML differential test (``parity/test_yaml_differential.py``).

Runs in the oracle venv (PyYAML 6.0.3 with libyaml). Reads one request per
stdin line, ``{"profile": "md" | "yml" | "config", "value": <tagged JSON>}``,
and answers one JSON line: ``{"yaml": <what v2 writes>, "reread": <tagged
JSON>}``, or ``{"yaml": ..., "reread_error": <message>}`` when PyYAML cannot
read its own output back. ``reread`` is the output loaded with the loader v2
uses for that file (python-frontmatter's ``CSafeLoader`` for markdown,
``SafeLoader`` for YAML neurons and configs).

Values travel as tagged JSON (``tagged_json.py``).

    parity/.oracle/venv/bin/python -I parity/tools/yaml_oracle.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import yaml

# -I leaves the script's directory off sys.path; the shared codec lives there.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from tagged_json import decode, encode  # noqa: E402

assert yaml.__version__ == "6.0.3", yaml.__version__
assert yaml.__with_libyaml__, "the md profile needs PyYAML built with libyaml"


def dump(profile: str, value: dict) -> tuple[str, type]:
    if profile == "md":
        text = yaml.dump(value, Dumper=yaml.CSafeDumper, default_flow_style=False, allow_unicode=True)
        return text, yaml.CSafeLoader
    if profile == "yml":
        return yaml.safe_dump(value, sort_keys=True, default_flow_style=False), yaml.SafeLoader
    if profile == "config":
        return yaml.dump(value, default_flow_style=False, sort_keys=False), yaml.SafeLoader
    raise ValueError(f"unknown profile {profile!r}")


def main() -> int:
    for line in sys.stdin:
        request = json.loads(line)
        text, loader = dump(request["profile"], decode(request["value"]))
        answer: dict[str, object] = {"yaml": text}
        try:
            answer["reread"] = encode(yaml.load(text, Loader=loader))
        except Exception as exc:  # noqa: BLE001 -- any failure is the answer
            answer["reread_error"] = f"{type(exc).__name__}: {exc}"
        sys.stdout.write(json.dumps(answer) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
