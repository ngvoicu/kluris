"""Tagged JSON: YAML values of the Kluris 3 value model as JSON, shared by the
YAML differential test, its PyYAML oracle (``yaml_oracle.py``) and
``cargo xtask yaml-emit`` / ``yaml-load``.

Null, bools, strings, arrays and objects are themselves. An int is a JSON
integer or ``{"$int": "<digits>"}``; a float is ``{"$float": "<repr>"}``
(``inf``, ``-inf``, ``nan`` included); a date is ``{"$date": "YYYY-MM-DD"}``;
a datetime is ``{"$datetime": "<isoformat>"}`` (naive, or with a ``+HH:MM`` or
``+HH:MM:SS`` offset); a mapping whose only key starts with ``$`` is
``{"$map": [[key, value], ...]}``.
"""

from __future__ import annotations

import datetime
import math


def decode(value: object) -> object:
    if isinstance(value, list):
        return [decode(v) for v in value]
    if isinstance(value, dict):
        if len(value) == 1:
            ((key, inner),) = value.items()
            if key == "$int":
                return int(inner)
            if key == "$float":
                return float(inner)
            if key == "$date":
                return datetime.date.fromisoformat(inner)
            if key == "$datetime":
                return datetime.datetime.fromisoformat(inner)
            if key == "$map":
                return {k: decode(v) for k, v in inner}
        return {k: decode(v) for k, v in value.items()}
    return value


def encode(value: object) -> object:
    if isinstance(value, bool) or value is None or isinstance(value, str):
        return value
    if isinstance(value, int):
        return value if -(2**63) <= value < 2**63 else {"$int": str(value)}
    if isinstance(value, float):
        return {"$float": repr(value)}
    if isinstance(value, datetime.datetime):
        return {"$datetime": value.isoformat()}
    if isinstance(value, datetime.date):
        return {"$date": value.isoformat()}
    if isinstance(value, list):
        return [encode(v) for v in value]
    if isinstance(value, dict):
        if not all(isinstance(k, str) for k in value):
            raise TypeError(f"non-string key in {value!r}")
        if len(value) == 1 and next(iter(value)).startswith("$"):
            return {"$map": [[k, encode(v)] for k, v in value.items()]}
        return {k: encode(v) for k, v in value.items()}
    raise TypeError(f"outside the value model: {value!r}")


def same(a: object, b: object) -> bool:
    """Exact equality: same types (``True`` is not ``1``), map keys in the same
    order, NaN equals NaN, ``-0.0`` is not ``0.0``, datetimes by fields and
    offset rather than by instant."""
    if type(a) is not type(b):
        return False
    if isinstance(a, float):
        assert isinstance(b, float)
        if math.isnan(a) or math.isnan(b):
            return math.isnan(a) and math.isnan(b)
        return a == b and math.copysign(1.0, a) == math.copysign(1.0, b)
    if isinstance(a, list):
        assert isinstance(b, list)
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, dict):
        assert isinstance(b, dict)
        return list(a) == list(b) and all(same(a[k], b[k]) for k in a)
    if isinstance(a, datetime.datetime):
        assert isinstance(b, datetime.datetime)
        return a.replace(tzinfo=None) == b.replace(tzinfo=None) and a.utcoffset() == b.utcoffset()
    return a == b
