"""``kluris search`` parity against the v2.28.11 oracle.

Kluris 3 ranks with BM25F over heading chunks (research-05 §6, PAR-020), so
result order, scores, snippets and the extra hits differ from v2 by design.
What stays v2's and is checked here:

- the error envelope and messages, exit codes, and the text output's wording;
- the top-level keys (``engine`` added last) and each result's first seven
  keys and their types, with ``total`` the count after ``--limit``;
- ``--lobe`` and ``--tag``: nothing v3 returns is an item v2's filters drop;
- for each item both return: ``file_type``, ``title`` and ``deprecated``;
- the invariant: with a limit high enough, every item v2 returns (``file``
  and ``title``; glossary entries share ``glossary.md``) is also in v3's
  results, and v2's ``matched_fields`` are among v3's.

v2's side of the bulk invariant comes from ``tools/search_oracle.py`` (v2's
own ``search_brain`` in one oracle process: the CLI is that call plus
``json.dumps``); v3's from the real binary, one process per query. The CLI
of both is compared on the fixture brains and the error cases.

Brains: the fixture brains of ``crates/kluris-core/tests/fixtures/brains/``,
the search-eval brains of ``parity/fixtures/search-eval/brains/``, brains
written here for single rules, and read-only copies (without ``.git``) of the
brains registered on this machine, with the 74 tuning queries of
``KLURIS_SEARCH_EVAL_QUERIES`` (default: the spec's
``assets/search-eval-queries.jsonl`` beside this worktree) for the brains they
name.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from collections import Counter
from pathlib import Path

import pytest

from harness import IS_WINDOWS, PARITY_DIR, Sandbox, normalize, oracle_kluris, run_both, run_v2, run_v3
from runs import clear_runs, parse_exact, write_config

WORKSPACE = PARITY_DIR.parent
FIXTURES = WORKSPACE / "crates" / "kluris-core" / "tests" / "fixtures" / "brains"
EVAL_BRAINS = PARITY_DIR / "fixtures" / "search-eval" / "brains"
EVAL_QUERIES = PARITY_DIR / "fixtures" / "search-eval" / "queries.jsonl"
ORACLE_TOOL = PARITY_DIR / "tools" / "search_oracle.py"
TUNING_QUERIES = Path(
    os.environ.get("KLURIS_SEARCH_EVAL_QUERIES")
    or WORKSPACE.parent / "kluris" / ".specs" / "kluris-v3-rust" / "assets" / "search-eval-queries.jsonl"
)

V2_RESULT_KEYS = ["file", "file_type", "title", "matched_fields", "snippet", "score", "deprecated"]
V3_ADDED_KEYS = ["heading", "line", "matched_terms", "coverage", "related", "updated"]
FIELDS = ["title", "tag", "path", "heading", "body"]
EVERYTHING = 1_000_000

# Punctuation, symbols, markup and scripts the tokenizer splits or drops; v2
# matches each as a literal substring.
ODD_QUERIES = [
    "`!!!! take care`",
    "!!!!",
    "[api](../x.md)",
    "KLURIS_NO_PROMPT=1",
    "/api/v1/imports/users",
    "darwin-aarch64",
    "kluris.yml",
    "--json",
    "#",
    "## ",
    "**",
    "|",
    "a",
    "e",
    " ",
    "  spaced  out  ",
    "C++",
    "what's",
    "100%",
    "é",
    "ș",
    "prețul",
    "pretul",
    "日本",
    "\U0001F600",
    "how do I",
    "the",
    "md",
    "/",
    ".",
    "-",
    "_",
]


# --- helpers -------------------------------------------------------------------


def copy_brain(sb: Sandbox, source: Path, name: str) -> Path:
    brain = sb.work / "brains" / name
    shutil.copytree(source, brain, symlinks=True, ignore=shutil.ignore_patterns(".git"))
    return brain


def register(sb: Sandbox, brains: list[tuple[str, Path]]) -> None:
    write_config(sb, [(name, str(path), "") for name, path in brains])


def oracle_search(brain: Path, queries: list[dict]) -> dict:
    """v2's ``search_brain`` results and filtered items (``tools/search_oracle.py``)."""
    python = oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")
    out = subprocess.run(
        [str(python), "-I", str(ORACLE_TOOL)],
        input=json.dumps({"brain": str(brain), "queries": queries}),
        capture_output=True, text=True, encoding="utf-8",
    )
    assert out.returncode == 0, out.stderr
    return json.loads(out.stdout)


def v3_args(name: str, query: dict) -> list[str]:
    """The query goes after ``--`` (both CLIs end options there), so a query
    such as ``--json`` is a query."""
    args = ["search", "--brain", name, "--json", "--limit", str(query.get("limit", 10))]
    if query.get("lobe") is not None:
        args += ["--lobe", query["lobe"]]
    if query.get("tag") is not None:
        args += ["--tag", query["tag"]]
    return [*args, "--", query["q"]]


def check_shape(doc: dict, name: str, query: str, limit: int) -> None:
    """v2's keys and types, v3's added keys, ``total`` after ``--limit``;
    ``skipped`` (PAR-024) only when files were left out."""
    keys = ["ok", "brain", "query", "total", "results", "engine"]
    if "skipped" in doc:
        keys.append("skipped")
        assert doc["skipped"], "skipped is present only when non-empty"
        for s in doc["skipped"]:
            assert list(s) == ["file", "error"] and all(isinstance(v, str) for v in s.values()), s
    assert list(doc) == keys, list(doc)
    assert doc["ok"] is True and doc["brain"] == name and doc["query"] == query
    assert doc["engine"] == "bm25"
    results = doc["results"]
    assert isinstance(doc["total"], int) and doc["total"] == len(results) <= max(limit, 0)
    for r in results:
        keys = list(r)
        added = V3_ADDED_KEYS + (["replaced_by"] if r["deprecated"] else [])
        assert keys == V2_RESULT_KEYS + added, keys
        assert isinstance(r["file"], str) and "\\" not in r["file"]
        assert r["file_type"] in ("markdown", "yaml")
        assert isinstance(r["title"], str) and isinstance(r["snippet"], str)
        assert type(r["score"]) is int and r["score"] >= 1
        assert type(r["deprecated"]) is bool
        fields = r["matched_fields"]
        assert fields == [f for f in FIELDS if f in fields] and fields, fields
        assert r["heading"] is None or isinstance(r["heading"], str)
        assert r["line"] is None or (type(r["line"]) is int and r["line"] >= 1)
        assert all(isinstance(t, str) for t in r["matched_terms"])
        assert type(r["coverage"]) is float and 0.0 <= r["coverage"] <= 1.0
        assert all(isinstance(p, str) for p in r["related"]) and len(r["related"]) <= 3
        assert r["updated"] is None or isinstance(r["updated"], str)
        if r["deprecated"]:
            assert r["replaced_by"] is None or isinstance(r["replaced_by"], str)


def check_against_v2(name: str, query: dict, v3_doc: dict, v2_results: list[dict], allowed: list[dict]) -> list[str]:
    """The invariant and the filters; returns the failures (empty when fine)."""
    failures = []
    label = f"{name} {query!r}"
    allowed_keys = {(i["file"], i["title"]): i for i in allowed}
    v3 = Counter((r["file"], r["title"]) for r in v3_doc["results"])
    v2 = Counter((r["file"], r["title"]) for r in v2_results)
    for key in v3:
        if key not in allowed_keys:
            failures.append(f"{label}: v3 returns {key}, which v2's items or filters do not have")
    missing = v2 - v3
    if missing and len(v3_doc["results"]) < int(query.get("limit", 10)):
        failures.append(f"{label}: v3 misses v2's {sorted(missing)}")
    by_key = {(r["file"], r["title"]): r for r in v3_doc["results"]}
    for r2 in v2_results:
        r3 = by_key.get((r2["file"], r2["title"]))
        if r3 is None:
            continue
        for k in ("file_type", "deprecated"):
            if r3[k] != r2[k]:
                failures.append(f"{label}: {r2['file']} {k} {r3[k]!r} != v2 {r2[k]!r}")
        if not set(r2["matched_fields"]) <= set(r3["matched_fields"]):
            failures.append(f"{label}: {r2['file']} fields {r3['matched_fields']} lack v2's {r2['matched_fields']}")
    for r in v3_doc["results"]:
        item = allowed_keys.get((r["file"], r["title"]))
        if item and (item["file_type"], item["is_deprecated"]) != (r["file_type"], r["deprecated"]):
            failures.append(f"{label}: {r['file']} type/deprecated differ from v2's item")
    return failures


def run_queries(sb: Sandbox, name: str, brain: Path, queries: list[dict]) -> list[str]:
    """Every query through the v3 binary and v2's runtime; the failures."""
    oracle = oracle_search(brain, queries)
    failures = []
    for query, v2_results in zip(queries, oracle["results"]):
        r = run_v3(v3_args(name, query), sb.env)
        if r.code != 0:
            failures.append(f"{name} {query!r}: exit {r.code}: {r.stdout}{r.stderr}")
            continue
        doc = parse_exact(r.stdout, sb.root, "v3")
        try:
            check_shape(doc, name, query["q"], int(query.get("limit", 10)))
        except AssertionError as exc:
            failures.append(f"{name} {query!r}: {exc}")
            continue
        allowed = oracle["items"][json.dumps([query.get("lobe"), query.get("tag")])]
        failures += check_against_v2(name, query, doc, v2_results, allowed)
    return failures


def words_of(brain: Path, limit: int = 25) -> list[str]:
    """Single-word queries: the most common title words of the brain's
    markdown files, and a few rare ones."""
    counts: Counter[str] = Counter()
    for path in sorted(brain.rglob("*.md")):
        if ".git" in path.parts:
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        for line in text.splitlines():
            if line.startswith("#"):
                counts.update(w.strip(".,:;()`*\"'").lower() for w in line.lstrip("#").split())
    words = [w for w, _ in counts.most_common() if len(w) >= 2]
    return words[:limit] + words[-5:]


def load_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


# --- the CLI against the oracle's CLI ---------------------------------------------


@pytest.fixture
def demo(sandbox) -> Sandbox:
    brain = copy_brain(sandbox, EVAL_BRAINS / "acme-sme", "acme-sme")
    register(sandbox, [("acme-sme", brain)])
    sandbox.freeze()
    return sandbox


@pytest.mark.parametrize(
    "args",
    [
        ["search", "", "--json"],
        ["search", ""],
        ["search", "--limit", "abc", "q", "--json"],
        ["search", "q", "--brain", "all", "--json"],
        ["search", "q", "--brain", "all"],
        ["search", "q", "--brain", "nope", "--json"],
        ["search", "q", "--brain", "nope"],
        ["search", "--json"],
    ],
    ids=["empty-json", "empty-text", "limit-abc-json", "all-json", "all-text", "unknown-json", "unknown-text", "no-query-json"],
)
def test_errors_match_v2(demo, args):
    r = run_both(demo, args)
    v2, v3 = r["v2"], r["v3"]
    assert v3.code == v2.code, (v2, v3)
    assert normalize(v3.stdout, demo.root) == normalize(v2.stdout, demo.root)
    if v2.stderr:
        assert v3.stderr.splitlines()[-1] == v2.stderr.splitlines()[-1], (v2.stderr, v3.stderr)


def test_limit_that_is_not_an_integer_is_a_usage_error(demo):
    r = run_both(demo, ["search", "--limit", "abc", "q"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 2
    assert v3.stderr.splitlines()[-1] == v2.stderr.splitlines()[-1]
    assert v3.stderr.splitlines()[0].startswith("Usage: kluris search")


@pytest.mark.parametrize("limit", ["0", "-1", "-100", " 3", "+2", "1_0"])
def test_limits_as_v2(demo, limit):
    r = run_both(demo, ["search", "backup", "--limit", limit, "--json"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2, v3)
    j2, j3 = parse_exact(v2.stdout, demo.root, "v2"), parse_exact(v3.stdout, demo.root, "v3")
    n = int(limit.strip().replace("_", ""))
    check_shape(j3, "acme-sme", "backup", n)
    # v3 finds at least what v2 finds, so it fills the limit at least as far.
    assert j2["total"] <= j3["total"] <= max(n, 0), (j2, j3)
    if n <= 0:
        assert j3["results"] == j2["results"] == []


def test_multiple_brains_need_brain(sandbox):
    a = copy_brain(sandbox, EVAL_BRAINS / "acme-sme", "acme-sme")
    b = copy_brain(sandbox, EVAL_BRAINS / "clinica-sme", "clinica-sme")
    register(sandbox, [("acme-sme", a), ("clinica-sme", b)])
    sandbox.freeze()
    for args in (["search", "q", "--json"], ["search", "q"]):
        r = run_both(sandbox, args)
        assert r["v3"].code == r["v2"].code == 1
        assert normalize(r["v3"].stdout, sandbox.root) == normalize(r["v2"].stdout, sandbox.root)
        assert r["v3"].stderr.splitlines()[-1:] == r["v2"].stderr.splitlines()[-1:]
        clear_runs(sandbox)


def test_no_brains_registered(sandbox):
    write_config(sandbox, [])
    sandbox.freeze()
    r = run_both(sandbox, ["search", "q", "--json"])
    assert r["v3"].code == r["v2"].code == 1
    assert r["v3"].stdout == r["v2"].stdout


def test_text_output_wording(demo):
    r = run_both(demo, ["search", "zebra-not-there"])
    assert r["v2"].code == r["v3"].code == 0
    assert r["v3"].stdout.strip() == r["v2"].stdout.strip() == "No results for 'zebra-not-there' in acme-sme."
    clear_runs(demo)
    r = run_both(demo, ["search", "backup"])
    v2, v3 = r["v2"].stdout, r["v3"].stdout
    head = lambda text: text.strip().splitlines()[0]  # noqa: E731
    assert head(v2).endswith("result(s) for 'backup' in acme-sme") and head(v3).endswith(
        "result(s) for 'backup' in acme-sme"
    ), (v2, v3)
    assert "operations/backups.md" in v3 and "operations/backups.md" in v2


def test_search_writes_nothing(demo):
    r = run_both(demo, ["search", "backup", "--json"])
    assert r["v3"].tree == r["v2"].tree
    assert r["v3"].code == 0


def test_cli_json_shape_and_invariant_on_the_eval_brain(demo):
    # The CLI of both, byte-level: v2's own JSON next to v3's.
    brain = demo.root / "pristine" / "brains" / "acme-sme"
    for q in ["backup", "release notes", "jwt", "glossary", "DLQ", "NamedParameterJdbcTemplate", "!!!!"]:
        clear_runs(demo)
        r = run_both(demo, ["search", q, "--json", "--limit", str(EVERYTHING)])
        assert r["v2"].code == r["v3"].code == 0, (r["v2"], r["v3"])
        j2, j3 = parse_exact(r["v2"].stdout, demo.root, "v2"), parse_exact(r["v3"].stdout, demo.root, "v3")
        check_shape(j3, "acme-sme", q, EVERYTHING)
        allowed = oracle_search(brain, [{"q": q}])["items"][json.dumps([None, None])]
        query = {"q": q, "limit": EVERYTHING}
        assert check_against_v2("acme-sme", query, j3, j2["results"], allowed) == []


# --- the invariant over many queries -----------------------------------------------


def _fixture_queries(brain: Path) -> list[dict]:
    qs = [{"q": w, "limit": EVERYTHING} for w in words_of(brain)]
    qs += [{"q": q, "limit": EVERYTHING} for q in ODD_QUERIES]
    qs += [{"q": q} for q in ("a", "the", "e")]  # default limit: `total` after --limit
    return qs


@pytest.mark.parametrize("name", ["wakeup", "full", "malformed"])
def test_fixture_brain_invariant(sandbox, name):
    brain = copy_brain(sandbox, FIXTURES / name, name)
    shutil.copytree(FIXTURES / "_outside", sandbox.work / "brains" / "outside")
    register(sandbox, [(name, brain)])
    failures = run_queries(sandbox, name, brain, _fixture_queries(brain))
    assert not failures, "\n".join(failures[:40])


@pytest.mark.parametrize("name", ["acme-sme", "clinica-sme"])
def test_eval_brain_invariant(sandbox, name):
    brain = copy_brain(sandbox, EVAL_BRAINS / name, name)
    register(sandbox, [(name, brain)])
    queries = [{"q": q["q"], "limit": EVERYTHING} for q in load_jsonl(EVAL_QUERIES) if q["brain"] == name]
    queries += _fixture_queries(brain)
    failures = run_queries(sandbox, name, brain, queries)
    assert not failures, "\n".join(failures[:40])


def test_filters_as_v2(sandbox):
    brain = copy_brain(sandbox, EVAL_BRAINS / "acme-sme", "acme-sme")
    register(sandbox, [("acme-sme", brain)])
    queries = []
    for q in ("backup", "the", "postgres", "release", "a", "!!!!", "auth"):
        for lobe, tag in (
            ("operations", None),
            ("operations/", None),
            ("knowledge", "auth"),
            (None, "postgres"),
            (None, "Postgres"),
            ("nope", None),
            ("architecture/overview.md", None),
            ("", None),
        ):
            queries.append({"q": q, "lobe": lobe, "tag": tag, "limit": EVERYTHING})
    failures = run_queries(sandbox, "acme-sme", brain, queries)
    assert not failures, "\n".join(failures[:40])


def test_language_override_keeps_the_invariant(sandbox):
    # `search: {language: en}` in kluris.yml (v2 ignores the key) changes
    # v3's stemming of a Romanian brain, never what v2 would find.
    brain = copy_brain(sandbox, EVAL_BRAINS / "clinica-sme", "clinica-sme")
    (brain / "kluris.yml").write_text(
        "name: clinica-sme\ndescription: ''\ncompanions: []\nsearch:\n  language: en\n", encoding="utf-8"
    )
    register(sandbox, [("clinica-sme", brain)])
    queries = [{"q": q, "limit": EVERYTHING} for q in ("pretul", "ședințe", "mailurile", "date personale")]
    failures = run_queries(sandbox, "clinica-sme", brain, queries)
    assert not failures, "\n".join(failures)


def test_utf8_snippets(sandbox):
    brain = sandbox.work / "brains" / "utf8"
    (brain / "k").mkdir(parents=True)
    filler = "ș日\U0001F600é " * 120
    (brain / "k" / "a.md").write_text(f"---\nupdated: 2026-01-01\n---\n# A\n\n{filler}zebra{filler}\n", encoding="utf-8")
    (brain / "k" / "crlf.md").write_bytes(f"---\r\nupdated: 2026-01-01\r\n---\r\n# C\r\n\r\nline one\r\nzebra two\r\n".encode())
    register(sandbox, [("utf8", brain)])
    sandbox.freeze()
    r = run_both(sandbox, ["search", "zebra", "--json"])
    assert r["v2"].code == r["v3"].code == 0
    doc = parse_exact(r["v3"].stdout, sandbox.root, "v3")
    check_shape(doc, "utf8", "zebra", 10)
    for res in doc["results"]:
        assert "zebra" in res["snippet"] and "\r" not in res["snippet"], res
        assert "�" not in res["snippet"]


def test_deprecated_hits_keep_the_flag_and_name_their_replacement(sandbox):
    brain = copy_brain(sandbox, EVAL_BRAINS / "acme-sme", "acme-sme")
    register(sandbox, [("acme-sme", brain)])
    sandbox.freeze()
    r = run_both(sandbox, ["search", "session", "--json", "--limit", str(EVERYTHING)])
    j2, j3 = parse_exact(r["v2"].stdout, sandbox.root, "v2"), parse_exact(r["v3"].stdout, sandbox.root, "v3")
    old2 = [x for x in j2["results"] if x["file"] == "knowledge/deprecated-sessions.md"]
    old3 = [x for x in j3["results"] if x["file"] == "knowledge/deprecated-sessions.md"]
    assert old2 and old2[0]["deprecated"] is True
    assert old3 and old3[0]["deprecated"] is True
    assert old3[0]["replaced_by"] == "../architecture/auth-tokens.md"


# --- the registered brains ----------------------------------------------------------


def _registered_brains() -> list[tuple[str, Path]]:
    kluris = shutil.which("kluris")
    if not kluris:
        return []
    out = subprocess.run([kluris, "list", "--json"], capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        return []
    try:
        brains = json.loads(out.stdout).get("brains", [])
    except json.JSONDecodeError:
        return []
    return [(b["name"], Path(b["path"])) for b in brains if Path(b["path"]).is_dir()]


def test_registered_brains_invariant(tmp_path, oracle):
    brains = _registered_brains()
    if not brains:
        pytest.skip("no brains registered with an installed kluris on this machine")
    tuning = load_jsonl(TUNING_QUERIES) if TUNING_QUERIES.is_file() else []
    failures, counted = [], 0
    for name, source in brains:
        sb = Sandbox(tmp_path / name)
        copy = copy_brain(sb, source, name)
        register(sb, [(name, copy)])
        queries = [{"q": q["q"], "limit": EVERYTHING} for q in tuning if q["brain"] == name]
        queries += _fixture_queries(copy)
        counted += sum(1 for q in tuning if q["brain"] == name)
        failures += run_queries(sb, name, copy, queries)
        print(f"{name}: {len(queries)} queries")
    if tuning:
        print(f"{counted} of {len(tuning)} tuning queries ran against registered brains")
    assert not failures, "\n".join(failures[:60])


# --- T-79 review findings -------------------------------------------------------------


def test_malformed_neuron_is_skipped_and_reported(sandbox):
    # PAR-024: v2 drops the neuron silently; v3 drops it from the results too
    # and reports it (`skipped` in JSON, one stderr line in text mode).
    brain = sandbox.work / "brains" / "bad"
    (brain / "k").mkdir(parents=True)
    (brain / "k" / "bad.md").write_text("---\nrelated: [\n---\n# Bad\n\nzebra\n", encoding="utf-8")
    (brain / "k" / "good.md").write_text("---\nupdated: 2026-01-01\n---\n# Good\n\nzebra\n", encoding="utf-8")
    register(sandbox, [("bad", brain)])
    sandbox.freeze()
    r = run_both(sandbox, ["search", "zebra", "--json"])
    assert r["v2"].code == r["v3"].code == 0
    j2, j3 = parse_exact(r["v2"].stdout, sandbox.root, "v2"), parse_exact(r["v3"].stdout, sandbox.root, "v3")
    check_shape(j3, "bad", "zebra", 10)
    assert "skipped" not in j2
    assert [x["file"] for x in j2["results"]] == [x["file"] for x in j3["results"]] == ["k/good.md"]
    assert [s["file"] for s in j3["skipped"]] == ["k/bad.md"]
    assert j3["skipped"][0]["error"].startswith("invalid frontmatter: ")
    assert r["v2"].stderr == r["v3"].stderr == ""
    clear_runs(sandbox)
    r = run_both(sandbox, ["search", "zebra"])
    assert r["v2"].code == r["v3"].code == 0
    assert r["v2"].stderr == ""
    assert r["v3"].stderr.splitlines() and r["v3"].stderr.splitlines()[0].startswith(
        "Skipped k/bad.md: invalid frontmatter: "
    ), r["v3"].stderr
    assert "result(s) for 'zebra' in bad" in r["v3"].stdout


TITLE_NAMES = [
    "ΟΣ", "თბილისი", "ᾈ", "ŉ-x", "ǆemal", "straße", "ßa", "o'neil", "2nd-try", "ΣΑΣ-ΣΑΣ", "ﬁle",
    "ꭰꭱ", "ΑΣ.Β-c", "ALL-CAPS", "İstanbul", "ǰx", "ʰa", "aΣ", "𐐨𐐩",
]


def test_file_name_titles_match_v2(sandbox):
    # T-79 finding 4: a markdown neuron without an H1 and a YAML neuron
    # without a `title` are titled `stem.replace("-", " ").title()`.
    brain = sandbox.work / "brains" / "titles"
    (brain / "md").mkdir(parents=True)
    (brain / "yml").mkdir(parents=True)
    for name in TITLE_NAMES:
        (brain / "md" / f"{name}.md").write_text("---\nupdated: 2026-01-01\n---\nno heading\n", encoding="utf-8")
        (brain / "yml" / f"{name}.yml").write_text("#---\n# updated: 2026-01-01\n#---\nk: v\n", encoding="utf-8")
    register(sandbox, [("titles", brain)])
    sandbox.freeze()
    r = run_both(sandbox, ["search", "/", "--json", "--limit", str(EVERYTHING)])
    j2, j3 = parse_exact(r["v2"].stdout, sandbox.root, "v2"), parse_exact(r["v3"].stdout, sandbox.root, "v3")
    titles2 = {x["file"]: x["title"] for x in j2["results"]}
    titles3 = {x["file"]: x["title"] for x in j3["results"]}
    assert len(titles2) == 2 * len(TITLE_NAMES)
    assert titles3 == titles2
    assert titles3["md/ΟΣ.md"] == "Ος" and titles3["md/თბილისი.md"] == "თბილისი" and titles3["md/ᾈ.md"] == "ᾈ"


def test_pycase_tables_are_current():
    # The Rust case tables and the title expectations are the oracle's.
    python = oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")
    out = subprocess.run(
        [str(python), "-I", str(PARITY_DIR / "tools" / "gen_pycase.py"), "--check"],
        capture_output=True, text=True, encoding="utf-8",
    )
    assert out.returncode == 0, out.stdout + out.stderr


@pytest.mark.skipif(IS_WINDOWS, reason="a backslash is a path separator on Windows")
def test_a_backslash_file_name_is_searched(sandbox):
    # T-79 P3: on POSIX `x\y.md` is one file; v2 searches it, so v3 must.
    brain = sandbox.work / "brains" / "bs"
    (brain / "k").mkdir(parents=True)
    (brain / "k" / "x\\y.md").write_text("---\nupdated: 2026-01-01\n---\n# Odd\n\nzebra\n", encoding="utf-8")
    register(sandbox, [("bs", brain)])
    failures = run_queries(sandbox, "bs", brain, [{"q": "zebra", "limit": EVERYTHING}, {"q": "odd"}])
    assert not failures, "\n".join(failures)
    r = run_v3(["search", "zebra", "--brain", "bs", "--json"], sandbox.env)
    assert [x["file"] for x in parse_exact(r.stdout, sandbox.root, "v3")["results"]] == ["k/x/y.md"]
