"""Brain-model parity: v3's walker, neuron index, glossary parsing and
deprecation detection (``cargo xtask brain-dump``) against v2.28.11's runtime
functions (``tools/brain_model_oracle.py``), on the fixture brains of
``crates/kluris-core/tests/fixtures/brains/`` and on copies of the brains
registered on this machine.

What is compared:
- the neuron list as a set (v2 keeps ``os.walk`` order, research-03 §3.2), and
  v3's own order pinned: markdown then YAML, each by path bytes;
- the glossary entries, exactly (``null`` when ``glossary.md`` cannot be read);
- the deprecation issues, exactly, against v2's own rules run in v3's neuron
  order over the neurons that read (PAR-007), and as a multiset against v2's
  unmodified call wherever that call does not raise;
- the neurons skipped because they cannot be read.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from harness import IS_WINDOWS, PARITY_DIR, oracle_kluris

WORKSPACE = PARITY_DIR.parent
FIXTURES = WORKSPACE / "crates" / "kluris-core" / "tests" / "fixtures" / "brains"
ORACLE_TOOL = PARITY_DIR / "tools" / "brain_model_oracle.py"


@pytest.fixture(scope="module")
def dump():
    venv = oracle_kluris().parent
    python = venv / ("python.exe" if IS_WINDOWS else "python")
    build = subprocess.run(
        ["cargo", "build", "--quiet", "--package", "xtask"], cwd=WORKSPACE, capture_output=True, text=True
    )
    assert build.returncode == 0, build.stderr

    def run(brain: Path, scratch: Path) -> tuple[dict, dict]:
        v3 = subprocess.run(
            ["cargo", "--quiet", "xtask", "brain-dump", "--path", str(brain)],
            cwd=WORKSPACE, capture_output=True, text=True, encoding="utf-8",
        )
        assert v3.returncode == 0, v3.stderr
        v3_out = json.loads(v3.stdout)
        order = scratch / "v3-order.json"
        order.write_text(json.dumps([n["path"] for n in v3_out["neurons"]]), encoding="utf-8")
        v2 = subprocess.run(
            [str(python), "-I", str(ORACLE_TOOL), str(brain), str(order)],
            capture_output=True, text=True, encoding="utf-8",
        )
        assert v2.returncode == 0, v2.stderr
        return v3_out, json.loads(v2.stdout)

    return run


def _issue_key(issue: dict) -> str:
    return json.dumps(issue, sort_keys=True, ensure_ascii=False)


def assert_same_model(v3: dict, v2: dict) -> None:
    def neuron_set(out):
        return sorted((n["path"], n["kind"]) for n in out["neurons"])

    assert neuron_set(v3) == neuron_set(v2)

    def bytes_order(kind):
        return sorted((n["path"] for n in v3["neurons"] if n["kind"] == kind), key=lambda p: p.encode())

    assert [n["path"] for n in v3["neurons"]] == bytes_order("markdown") + bytes_order("yaml")

    assert v3["glossary"] == v2["glossary"]
    assert v3["deprecation"] == v2["deprecation"]
    assert v3["skipped"] == v2["skipped"]

    plain = v2["deprecation_plain"]
    if isinstance(plain, dict):
        # v2's own call raises on a neuron it cannot read; v3 skips it.
        assert v3["skipped"], f"v2 raised {plain['error']} with no unreadable neuron"
    else:
        assert sorted(map(_issue_key, v3["deprecation"])) == sorted(map(_issue_key, plain))


def _materialize(name: str, base: Path) -> Path:
    """Copy a fixture brain to ``base/<name>`` with ``outside/`` and
    ``outside-the-brain.md`` beside it, and make the symlinks of
    ``links.symlinks`` for the ``links`` brain."""
    base.mkdir(parents=True, exist_ok=True)
    base = base.resolve()
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


@pytest.mark.parametrize("name", ["full", "malformed", "links"])
def test_fixture_brain_model_matches_v2(name, tmp_path, dump):
    brain = _materialize(name, tmp_path / "brains")
    v3, v2 = dump(brain, tmp_path)
    assert_same_model(v3, v2)


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


def test_registered_brains_model_matches_v2(tmp_path, dump):
    brains = _registered_brains()
    if not brains:
        pytest.skip("no brains registered with an installed kluris on this machine")
    failures = []
    for name, source in brains:
        copy = tmp_path / "brains" / name
        shutil.copytree(source, copy, symlinks=True, ignore=shutil.ignore_patterns(".git"))
        v3, v2 = dump(copy.resolve(), tmp_path)
        try:
            assert_same_model(v3, v2)
        except AssertionError as exc:
            failures.append(f"{name}: {exc}")
        print(
            f"{name}: {len(v3['neurons'])} neurons, {len(v3['glossary'] or [])} glossary entries, "
            f"{len(v3['deprecation'])} deprecation issues, {len(v3['skipped'])} skipped"
        )
    assert not failures, "\n".join(failures)
