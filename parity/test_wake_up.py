"""``kluris wake-up`` parity against the v2.28.11 oracle: the JSON payload
(research-03 §1.6, §2.7) byte-exact apart from ``deprecation[]``, which is
compared as a multiset (PAR-008), and the allowlisted differences each test
names; the text output by its wording.

Brains: the fixture brains of ``crates/kluris-core/tests/fixtures/brains/``,
brains generated here for single rules (odd ``updated`` values, brain.md
truncation, CRLF, stored path forms, a BOM), and copies of the brains
registered on this machine. ``recent`` comes from each neuron's ``updated``
frontmatter only (v2 runs no git for wake-up), so no git history is needed.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from harness import IS_WINDOWS, PARITY_DIR, Sandbox, normalize, oracle_kluris, run_both
from runs import assert_wake_up_equal, clear_runs, parse_exact, same_words, write_config

WORKSPACE = PARITY_DIR.parent
FIXTURES = WORKSPACE / "crates" / "kluris-core" / "tests" / "fixtures" / "brains"
ORACLE_TOOL = PARITY_DIR / "tools" / "brain_model_oracle.py"


def materialize(sb: Sandbox, name: str) -> Path:
    """Copy a fixture brain into the sandbox with ``outside/`` beside it (and
    the symlinks of ``links.symlinks`` for ``links``)."""
    base = sb.work / "brains"
    base.mkdir(parents=True, exist_ok=True)
    brain = base / name
    shutil.copytree(FIXTURES / name, brain)
    shutil.copytree(FIXTURES / "_outside", base / "outside")
    (base / "outside-the-brain.md").write_text("# outside\n", encoding="utf-8")
    if name == "links":
        for line in (FIXTURES / "links.symlinks").read_text(encoding="utf-8").splitlines():
            fields = line.split()
            if line.startswith("#") or len(fields) != 3:
                continue
            kind, link, target = fields
            try:
                os.symlink(target.replace("/", os.sep), brain / link, target_is_directory=kind == "dir")
            except OSError as exc:
                pytest.skip(f"cannot create symlinks on this machine: {exc}")
    return brain


def register(sb: Sandbox, brain: Path, name: str | None = None, description: str = "Fixture brain") -> None:
    write_config(sb, [(name or brain.name, str(brain), description)])
    sb.freeze()


def v2_rules(brain: Path) -> dict:
    """v2's deprecation rules over the neurons that read, and those that do
    not (``tools/brain_model_oracle.py``)."""
    python = oracle_kluris().parent / ("python.exe" if IS_WINDOWS else "python")
    out = subprocess.run(
        [str(python), "-I", str(ORACLE_TOOL), str(brain)], capture_output=True, text=True, encoding="utf-8"
    )
    assert out.returncode == 0, out.stderr
    return json.loads(out.stdout)


def wake_up_both(sb: Sandbox, *extra: str):
    r = run_both(sb, ["wake-up", "--json", *extra])
    return r["v2"], r["v3"]


def compare(sb: Sandbox, brain: Path) -> dict:
    """Run ``wake-up --json`` on both and compare, with the neurons v2's own
    reader cannot read either (``tools/brain_model_oracle.py``): v2 then
    reports no deprecation for the brain and ``parse_errors: 0``, v3 the
    issues of the rest (PAR-007) and their count (PAR-009)."""
    frozen = sb.root / "pristine" / brain.relative_to(sb.work)
    rules = v2_rules(frozen if frozen.is_dir() else brain)
    v2, v3 = wake_up_both(sb)
    return assert_wake_up_equal(
        v2, v3, sb.root, skipped=rules["skipped"] or None, v3_deprecation=rules["deprecation"]
    )


@pytest.mark.parametrize("name", ["wakeup", "full", "links"])
def test_fixture_brain(sandbox, name):
    brain = materialize(sandbox, name)
    register(sandbox, brain)
    compare(sandbox, brain)


def test_malformed_fixture_brain(sandbox):
    # v2's deprecation check raises on the unreadable neurons (v2 reports
    # nothing for the brain); v3 skips them (PAR-007) and counts them in
    # `parse_errors` (PAR-009).
    brain = materialize(sandbox, "malformed")
    assert v2_rules(brain)["skipped"], "the fixture has unreadable neurons"
    register(sandbox, brain)
    compare(sandbox, brain)


@pytest.mark.parametrize("name", ["wakeup", "full"])
def test_text_output_wording(sandbox, name):
    brain = materialize(sandbox, name)
    register(sandbox, brain, description="A described brain")
    r = run_both(sandbox, ["wake-up"])
    v2, v3 = r["v2"], r["v3"]
    assert v2.code == v3.code == 0, (v2.stderr, v3.stderr)
    same_words(v2.stdout, v3.stdout, sandbox.root)
    assert "Deprecation warnings:" in v3.stdout and "Glossary (" in v3.stdout
    assert v3.stderr == v2.stderr == ""


def test_text_output_of_an_empty_brain(sandbox):
    brain = sandbox.work / "brains" / "empty"
    brain.mkdir(parents=True)
    register(sandbox, brain, description="")
    r = run_both(sandbox, ["wake-up"])
    assert r["v2"].code == r["v3"].code == 0
    same_words(r["v2"].stdout, r["v3"].stdout, sandbox.root)
    assert "Recently updated" not in r["v3"].stdout


def test_empty_brain_json(sandbox):
    brain = sandbox.work / "brains" / "empty"
    brain.mkdir(parents=True)
    register(sandbox, brain)
    compare(sandbox, brain)


ODD_UPDATED = [
    "[2026-01-01, x]",
    "true",
    "1.5",
    "{a: 1, 'b''c': [null, 2026-01-01 10:00:00]}",
    "[2026-01-01 10:00:00+01:00, 2026-01-01 10:00:00-05:30, 2026-01-01 10:00:00Z]",
    "\"it's\"",
    "[\"it's\", 'say \"hi\"', \"tab\\there\", \"\\x7f\\x85\\xa0\\u2028\\U0001F600\\u00e9\"]",
    "1e3",
    "-0.0",
    ".inf",
    ".nan",
    "0x1F",
    "123456789012345678901234567890",
    "[]",
    "{}",
    "2026-01-01 10:00:05.000123",
    "2026-01-01 10:00:00+05:45",
    "''",
    "yesterday",
    "No",
    "-17",
    "2026-W10",
    "20260101xx",
    "2026-02-30",
    "\"2026-13-01\"",
    "[2026-01-01 10:00:00.000001]",
    "\"\\u00e9t\\u00e9 2026\"",
    "\"2026-01-0\\u00e9\"",
    "!!str 2026-01-01",
    "2026-01-01t10:00:00z",
]


@pytest.mark.parametrize("batch", range(0, len(ODD_UPDATED), 5))
def test_odd_updated_values(sandbox, batch):
    # Five per brain, so every value is in `recent` (top 5).
    brain = sandbox.work / "brains" / "odd"
    (brain / "lobe").mkdir(parents=True)
    for i, value in enumerate(ODD_UPDATED[batch : batch + 5]):
        (brain / "lobe" / f"n{i}.md").write_text(f"---\nupdated: {value}\n---\n# n{i}\n", encoding="utf-8")
    register(sandbox, brain)
    payload = compare(sandbox, brain)
    # `2026-02-30` is a timestamp PyYAML cannot construct: unreadable in both.
    readable = [v for v in ODD_UPDATED[batch : batch + 5] if v != "2026-02-30"]
    assert len(payload["recent"]) == len(readable)


@pytest.mark.parametrize(
    "body",
    [
        "a" * 4000,
        "a" * 4001,
        "a" * 3998 + "日" + "tail",
        "a" * 3999 + "é" * 3,
        ("line é日\U0001F600\n" * 400),
    ],
    ids=["exact", "one-over", "cut-3-byte", "cut-2-byte", "many-lines"],
)
def test_brain_md_truncation(sandbox, body):
    brain = sandbox.work / "brains" / "big"
    brain.mkdir(parents=True)
    (brain / "brain.md").write_text(f"---\nupdated: 2026-01-01\n---\n\n{body}\n", encoding="utf-8")
    register(sandbox, brain)
    compare(sandbox, brain)


def test_crlf_brain(sandbox):
    brain = sandbox.work / "brains" / "crlf"
    (brain / "lobe").mkdir(parents=True)
    files = {
        "brain.md": "---\nupdated: 2026-01-01\n---\n\n# crlf\n\nbody line\n\n",
        "glossary.md": "# Glossary\n\n**A** - first\n| B | second |\n",
        "lobe/map.md": "# Lobe\n\nup [x](x)\n\n  The lobe.  \n",
        "lobe/n.md": "---\nupdated: 2026-02-02\n---\n# n\n",
        "lobe/y.yml": "#---\n# updated: 2026-02-03\n#---\nk: v\n",
        "lobe/cr.md": "---\rupdated: 2026-02-04\r---\r# cr\r",
    }
    for rel, text in files.items():
        (brain / rel).write_bytes(text.replace("\n", "\r\n").encode("utf-8"))
    register(sandbox, brain)
    compare(sandbox, brain)


@pytest.mark.parametrize("suffix", ["/", "/./", "//"], ids=["trailing", "dot", "double"])
def test_stored_path_is_printed_as_python_path(sandbox, suffix):
    brain = sandbox.work / "brains" / "p"
    brain.mkdir(parents=True)
    write_config(sandbox, [("p", str(brain) + suffix.replace("/", os.sep), "")])
    sandbox.freeze()
    payload = compare(sandbox, brain)
    assert payload["path"] == "<ROOT>/work/brains/p".replace("/", os.sep)
    clear_runs(sandbox)
    r = run_both(sandbox, ["wake-up"])
    same_words(r["v2"].stdout, r["v3"].stdout, sandbox.root)


def test_description_comes_from_the_config(sandbox):
    brain = materialize(sandbox, "wakeup")
    register(sandbox, brain, name="renamed", description="déjà vu — ok")
    v2, v3 = wake_up_both(sandbox)
    payload = assert_wake_up_equal(v2, v3, sandbox.root)
    assert payload["name"] == "renamed" and payload["description"] == "déjà vu — ok"


def test_bom_frontmatter_is_read(sandbox):
    # PAR-010: v2 (python-frontmatter) sees no frontmatter behind a UTF-8 BOM;
    # v3 reads it. Everything else is the same.
    brain = sandbox.work / "brains" / "bom"
    (brain / "lobe").mkdir(parents=True)
    bom = "﻿"
    (brain / "brain.md").write_text(f"{bom}---\nupdated: 2026-01-01\n---\n\n# bom\n", encoding="utf-8")
    (brain / "lobe" / "map.md").write_text(f"{bom}---\ndescription: From frontmatter\n---\n# Lobe\n\nbody line\n", encoding="utf-8")
    (brain / "lobe" / "n.md").write_text(f"{bom}---\nupdated: 2026-03-03\n---\n# n\n", encoding="utf-8")
    (brain / "lobe" / "plain.md").write_text("---\nupdated: 2026-01-01\n---\n# plain\n", encoding="utf-8")
    register(sandbox, brain)
    v2, v3 = wake_up_both(sandbox)
    assert v2.code == v3.code == 0
    j2, j3 = parse_exact(v2.stdout, sandbox.root, "v2"), parse_exact(v3.stdout, sandbox.root, "v3")
    assert j2["brain_md"] == "﻿---\nupdated: 2026-01-01\n---\n\n# bom"
    assert j3["brain_md"] == "# bom"
    assert [lobe["description"] for lobe in j2["lobes"]] == ["body line"]
    assert [lobe["description"] for lobe in j3["lobes"]] == ["From frontmatter"]
    assert [r["path"] for r in j2["recent"]] == ["lobe/plain.md"]
    assert [r["path"] for r in j3["recent"]] == ["lobe/n.md", "lobe/plain.md"]
    for doc in (j2, j3):
        del doc["brain_md"], doc["recent"]
        doc["lobes"][0]["description"] = ""
    assert j3 == j2


def test_map_md_that_is_not_utf8(sandbox):
    # PAR-011: v2's lobe-description fallback raises UnicodeDecodeError (a
    # traceback, exit 1); v3 gives the lobe an empty description.
    brain = sandbox.work / "brains" / "bad-map"
    (brain / "lobe").mkdir(parents=True)
    (brain / "lobe" / "map.md").write_bytes(b"# Lobe\n\n\xff\xfe description\n")
    (brain / "lobe" / "n.md").write_text("---\nupdated: 2026-01-01\n---\n", encoding="utf-8")
    register(sandbox, brain)
    v2, v3 = wake_up_both(sandbox)
    assert v2.code == 1 and "UnicodeDecodeError" in v2.stderr and v2.stdout == ""
    assert v3.code == 0, v3.stderr
    j3 = parse_exact(v3.stdout, sandbox.root, "v3")
    assert j3["lobes"] == [{"name": "lobe", "description": "", "neurons": 1, "yaml_count": 0}]


def test_frontmatter_v3_refuses(sandbox):
    # PAR-009: frontmatter v2 reads but v3's reader refuses as an error value
    # (a `<<` merge key, `!!binary`, `!!set`; Kluris 3 decision TEST-K3-03):
    # v3 leaves those neurons out of `recent` and `deprecation` and counts
    # them in `parse_errors`. A non-string key is unreadable in both (v2's
    # python-frontmatter cannot build its post).
    brain = sandbox.work / "brains" / "refused"
    (brain / "lobe").mkdir(parents=True)
    files = {
        "merge.md": "---\nbase: &b {a: 1}\nm:\n  <<: *b\nupdated: 2026-01-02\n---\n",
        "binary.md": "---\nb: !!binary aGk=\nupdated: 2026-01-03\n---\n",
        "set.md": "---\ns: !!set {a: null}\nupdated: 2026-01-04\n---\n",
        "int-key.md": "---\n1: x\nupdated: 2026-01-05\n---\n",
        "plain.md": "---\nupdated: 2026-01-01\n---\n",
    }
    for name, text in files.items():
        (brain / "lobe" / name).write_text(text, encoding="utf-8")
    register(sandbox, brain)
    v2, v3 = wake_up_both(sandbox)
    assert v2.code == v3.code == 0
    j2, j3 = parse_exact(v2.stdout, sandbox.root, "v2"), parse_exact(v3.stdout, sandbox.root, "v3")
    assert [r["path"] for r in j2["recent"]] == ["lobe/set.md", "lobe/binary.md", "lobe/merge.md", "lobe/plain.md"]
    assert [r["path"] for r in j3["recent"]] == ["lobe/plain.md"]
    assert (j2["parse_errors"], j3["parse_errors"]) == (0, 4)
    for doc in (j2, j3):
        del doc["recent"], doc["parse_errors"]
    assert j3 == j2


def _registered_brains() -> list[tuple[str, Path, str]]:
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
    return [(b["name"], Path(b["path"]), b.get("description", "")) for b in brains if Path(b["path"]).is_dir()]


def test_registered_brains(tmp_path, oracle):
    brains = _registered_brains()
    if not brains:
        pytest.skip("no brains registered with an installed kluris on this machine")
    failures = []
    for name, source, description in brains:
        sb = Sandbox(tmp_path / name)
        copy = sb.work / "brains" / name
        shutil.copytree(source, copy, symlinks=True, ignore=shutil.ignore_patterns(".git"))
        rules = v2_rules(copy)
        register(sb, copy, name=name, description=description)
        try:
            payload = compare(sb, copy)
        except AssertionError as exc:
            failures.append(f"{name}: {exc}")
            continue
        print(
            f"{name}: {payload['total_neurons']} lobe neurons, {len(payload['lobes'])} lobes, "
            f"{len(payload['glossary'])} glossary entries, {len(rules['deprecation'])} deprecation issues, "
            f"{len(rules['skipped'])} skipped"
        )
    assert not failures, "\n".join(failures)


def test_wake_up_writes_nothing(sandbox):
    brain = materialize(sandbox, "wakeup")
    register(sandbox, brain)
    r = run_both(sandbox, ["wake-up", "--json"])
    assert r["v3"].tree == r["v2"].tree
    assert normalize(r["v3"].stdout, sandbox.root) != ""
