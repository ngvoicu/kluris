"""v2 side of ``parity/test_search.py``, run in the oracle venv (Kluris v2.28.11).

Reads one JSON object from stdin::

    {"brain": "<path>", "queries": [{"q": "...", "lobe": null, "tag": null, "limit": 10}, ...]}

and prints one JSON object::

    {"results": [<v2 search_brain results for each query>, ...],
     "items": {"[lobe, tag]": [<the searchable items those filters let through>]}}

``results`` are exactly what ``kluris search --json`` prints under
``results`` (the CLI is ``search_brain`` plus ``json.dumps``); ``items`` are
``collect_searchable`` rows (``file``, ``title``, ``kind``, ``file_type``,
``is_deprecated``) that pass ``_passes_filters``, once per filter pair (``json.dumps([lobe, tag])``), so a test can check that
Kluris 3 returns nothing v2's filters would drop. One process answers every
query, so a large query set costs one interpreter start.

    parity/.oracle/venv/bin/python -I parity/tools/search_oracle.py < request.json
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from kluris_runtime.search import _passes_filters, collect_searchable, search_brain


def main() -> None:
    request = json.loads(sys.stdin.read())
    brain = Path(request["brain"])
    items = collect_searchable(brain)
    results, allowed = [], {}
    for query in request["queries"]:
        lobe, tag = query.get("lobe"), query.get("tag")
        results.append(
            search_brain(brain, query["q"], limit=query.get("limit", 10), lobe_filter=lobe, tag_filter=tag)
        )
        key = json.dumps([lobe, tag])
        if key not in allowed:
            allowed[key] = [
                {k: item[k] for k in ("file", "title", "kind", "file_type", "is_deprecated")}
                for item in items
                if _passes_filters(item, lobe_filter=lobe, tag_filter=tag)
            ]
    sys.stdout.write(json.dumps({"results": results, "items": allowed}))


if __name__ == "__main__":
    main()
