"""PyYAML side of the raw-YAML reader differential
(``parity/test_yaml_load_differential.py``).

Runs in the oracle venv (PyYAML 6.0.3 with libyaml). Reads one JSON string per
stdin line, a YAML text, and answers one JSON line with what each loader v2
uses makes of it: ``{"safe": <outcome>, "c": <outcome>}`` for
``yaml.safe_load`` and ``yaml.load(Loader=CSafeLoader)``, where an outcome is
``{"value": <tagged JSON>}`` or ``{"error": <message>}``. A value outside the
Kluris 3 value model (a non-string key, bytes, a set, ...) is an error: v3
refuses it, as is a string with a lone surrogate. So is a merge key (``<<``) where PyYAML would merge: v3 refuses
merge keys (decided in the Phase 1 brief), so both loaders run with a
``flatten_mapping`` that raises on one and is otherwise PyYAML's.

    parity/.oracle/venv/bin/python -I parity/tools/yaml_load_oracle.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import yaml

# -I leaves the script's directory off sys.path; the shared codec lives there.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from tagged_json import encode  # noqa: E402

assert yaml.__version__ == "6.0.3", yaml.__version__
assert yaml.__with_libyaml__, "the comparison needs PyYAML built with libyaml"


def refusing_merges(base: type) -> type:
    class Loader(base):
        def flatten_mapping(self, node):
            for key_node, _ in node.value:
                if key_node.tag == "tag:yaml.org,2002:merge":
                    raise yaml.constructor.ConstructorError(
                        None, None, "merge keys are not supported", key_node.start_mark
                    )
            super().flatten_mapping(node)

    Loader.__name__ = base.__name__
    return Loader


SAFE = refusing_merges(yaml.SafeLoader)
CSAFE = refusing_merges(yaml.CSafeLoader)


def outcome(text: str, loader: type) -> dict[str, object]:
    try:
        value = yaml.load(text, Loader=loader)
    except Exception as exc:  # noqa: BLE001 -- any failure is the answer
        return {"error": f"{type(exc).__name__}: {exc}"}
    # Writing out an int PyYAML built must not fail on Python's 4,300-digit
    # str() limit (loading above ran with the default limit).
    sys.set_int_max_str_digits(0)
    try:
        encoded = encode(value)
    except (TypeError, RecursionError) as exc:
        return {"error": f"outside the value model: {exc}"}
    finally:
        sys.set_int_max_str_digits(sys.int_info.default_max_str_digits)
    if has_surrogate(value):
        return {"error": "outside the value model: a lone surrogate"}
    return {"value": encoded}


def has_surrogate(value: object) -> bool:
    """A string holding a lone surrogate (from a `\\ud800` escape): a Rust
    string cannot hold one."""
    if isinstance(value, str):
        return any(0xD800 <= ord(ch) <= 0xDFFF for ch in value)
    if isinstance(value, list):
        return any(has_surrogate(v) for v in value)
    if isinstance(value, dict):
        return any(has_surrogate(k) or has_surrogate(v) for k, v in value.items())
    return False


def main() -> int:
    for line in sys.stdin:
        text = json.loads(line)
        answer = {"safe": outcome(text, SAFE), "c": outcome(text, CSAFE)}
        sys.stdout.write(json.dumps(answer) + "\n")
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
