"""Tests for kluris_runtime.snapshot — the one-walk boot snapshot.

The snapshot is the large-brain backbone: search rows, tool metadata, the
reverse related-link map, deprecation preparse, and per-lobe tag hints must
all match what the per-call walks produce, because every consumer falls back
to those walks when no snapshot is registered.
"""

from pathlib import Path

import pytest

from kluris_runtime.deprecation import detect_deprecation_issues
from kluris_runtime.search import collect_searchable
from kluris_runtime.snapshot import (
    _clear_snapshot_registry,
    build_snapshot,
    drop_snapshot,
    get_snapshot,
    register_snapshot,
)
from kluris_runtime.wake_up import build_payload


@pytest.fixture(autouse=True)
def _clean_registry():
    _clear_snapshot_registry()
    yield
    _clear_snapshot_registry()


def _write(brain: Path, rel: str, text: str) -> Path:
    target = brain / rel
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, encoding="utf-8")
    return target


def _make_brain(tmp_path: Path) -> Path:
    brain = tmp_path / "snap-brain"
    brain.mkdir()
    _write(brain, "brain.md", "# Snap Brain\n\nRoot index.\n")
    _write(brain, "glossary.md", "# Glossary\n\n**API** - interface\n")
    _write(
        brain, "alpha/auth.md",
        "---\nupdated: 2026-03-01\ntags: [security, auth]\n"
        "related:\n  - ./tokens.md\n  - ../beta/billing.md\n"
        "  - ./missing.md\n  - /etc/passwd\n---\n"
        "# Auth Flow\n\nHow auth works.\n",
    )
    _write(
        brain, "alpha/tokens.md",
        "---\nupdated: 2026-02-01\ntags: [security]\n---\n"
        "# Tokens\n\nToken rotation.\n",
    )
    _write(
        brain, "beta/billing.md",
        "---\ntags: [billing]\nstatus: deprecated\n---\n"
        "# Billing\n\nOld billing notes.\n",
    )
    _write(brain, "alpha/map.md", "# Alpha\n\nAuth lobe.\n")
    # Nested dot-dir: must be invisible everywhere (the walker-drift fix).
    _write(brain, "alpha/.archive/hidden.md", "---\nupdated: 2099-01-01\n---\n# Hidden\n")
    return brain


# --- rows: identical to collect_searchable ------------------------------------


def test_snapshot_rows_match_collect_searchable(tmp_path):
    brain = _make_brain(tmp_path)
    snap = build_snapshot(brain)
    assert snap["rows"] == collect_searchable(brain.resolve())


# --- entries -------------------------------------------------------------------


def test_snapshot_entries_sorted_and_shaped(tmp_path):
    brain = _make_brain(tmp_path)
    snap = build_snapshot(brain)
    paths = [e["path"] for e in snap["entries"]]
    assert paths == sorted(paths)
    assert "alpha/.archive/hidden.md" not in paths

    auth = next(e for e in snap["entries"] if e["path"] == "alpha/auth.md")
    assert auth["title"] == "Auth Flow"
    assert auth["label"] == "Auth"            # stem-derived Title-Case
    assert auth["excerpt"] == "How auth works."
    assert auth["tags"] == ["security", "auth"]
    assert auth["updated"] == "2026-03-01"
    assert auth["deprecated"] is False

    billing = next(e for e in snap["entries"] if e["path"] == "beta/billing.md")
    assert billing["deprecated"] is True
    assert billing["updated"] is None          # no updated: frontmatter


def test_snapshot_related_resolved_with_tool_semantics(tmp_path):
    """Outbound links: in-brain existing targets only, dedup, traversal and
    missing targets dropped — matching related_tool exactly."""
    brain = _make_brain(tmp_path)
    snap = build_snapshot(brain)
    auth = next(e for e in snap["entries"] if e["path"] == "alpha/auth.md")
    assert auth["related"] == ["alpha/tokens.md", "beta/billing.md"]


def test_snapshot_inbound_reverse_map(tmp_path):
    brain = _make_brain(tmp_path).resolve()
    snap = build_snapshot(brain)
    tokens_key = str((brain / "alpha/tokens.md").resolve())
    billing_key = str((brain / "beta/billing.md").resolve())
    assert snap["inbound"][tokens_key] == ["alpha/auth.md"]
    assert snap["inbound"][billing_key] == ["alpha/auth.md"]


# --- lobe tags -------------------------------------------------------------------


def test_snapshot_lobe_tags_frequency_then_alpha(tmp_path):
    brain = _make_brain(tmp_path)
    snap = build_snapshot(brain)
    # alpha: security ×2, auth ×1 → frequency first, then alphabetical.
    assert snap["lobe_tags"]["alpha"] == ["security", "auth"]
    assert snap["lobe_tags"]["beta"] == ["billing"]


# --- registry -------------------------------------------------------------------


def test_snapshot_registry_roundtrip(tmp_path):
    brain = _make_brain(tmp_path)
    assert get_snapshot(brain) is None
    snap = build_snapshot(brain)
    register_snapshot(brain, snap)
    assert get_snapshot(brain) is snap
    drop_snapshot(brain)
    assert get_snapshot(brain) is None


# --- walker unification (the drift fix) ------------------------------------------


def test_wake_up_and_search_agree_on_dot_dir_neurons(tmp_path):
    """A neuron nested under a dot-directory must be invisible to BOTH
    wake-up (counts + recent) and search — the walkers had drifted."""
    brain = _make_brain(tmp_path)
    payload = build_payload(brain)
    assert payload["total_neurons"] == 3  # auth, tokens, billing — not hidden
    assert all(r["path"] != "alpha/.archive/hidden.md" for r in payload["recent"])
    files = {item["file"] for item in collect_searchable(brain)}
    assert "alpha/.archive/hidden.md" not in files


# --- deprecation preparse ---------------------------------------------------------


def test_deprecation_preparsed_matches_fresh_walk(tmp_path):
    brain = _make_brain(tmp_path)
    snap = build_snapshot(brain)
    fresh = detect_deprecation_issues(brain)
    via_snapshot = detect_deprecation_issues(brain, preparsed=snap["preparsed"])
    assert via_snapshot == fresh
    # Sanity: the fixture actually produces issues (active → deprecated link,
    # deprecated without replacement), so the equality is non-vacuous.
    kinds = {i["kind"] for i in fresh}
    assert "active_links_to_deprecated" in kinds
    assert "deprecated_without_replacement" in kinds


# --- wake_up payload from snapshot -------------------------------------------------


def test_build_payload_with_snapshot_matches_fresh(tmp_path):
    """The snapshot-fed payload must equal the walk-fed payload except for
    the additive per-lobe top_tags enrichment."""
    brain = _make_brain(tmp_path)
    snap = build_snapshot(brain)
    fresh = build_payload(brain)
    fed = build_payload(brain, snapshot=snap)

    fed_lobes = [
        {k: v for k, v in lobe.items() if k != "top_tags"}
        for lobe in fed["lobes"]
    ]
    assert fed_lobes == fresh["lobes"]
    assert fed["recent"] == fresh["recent"]
    assert fed["total_neurons"] == fresh["total_neurons"]
    assert fed["deprecation"] == fresh["deprecation"]
    assert fed["deprecation_count"] == fresh["deprecation_count"]

    alpha = next(lobe for lobe in fed["lobes"] if lobe["name"] == "alpha")
    assert alpha["top_tags"] == ["security", "auth"]


def test_build_payload_with_snapshot_keeps_empty_lobes(tmp_path):
    """A lobe directory with no neurons still appears with count 0."""
    brain = _make_brain(tmp_path)
    (brain / "empty-lobe").mkdir()
    snap = build_snapshot(brain)
    fed = build_payload(brain, snapshot=snap)
    empty = next(lobe for lobe in fed["lobes"] if lobe["name"] == "empty-lobe")
    assert empty["neurons"] == 0
    assert empty["top_tags"] == []


# --- symlink escape: search must not surface what the read sandbox refuses ------


def test_snapshot_excludes_symlink_escaping_brain(tmp_path):
    """A neuron symlinked to a file OUTSIDE the brain root must not be walked,
    indexed, or have its body land in search rows — mirroring the read tools'
    sandbox so search can't leak what read_neuron rejects."""
    brain = _make_brain(tmp_path)
    outside = tmp_path / "outside"
    outside.mkdir()
    secret = outside / "secret.md"
    secret.write_text("---\n---\nESCAPEDSECRET\n", encoding="utf-8")
    link = brain / "alpha" / "leak.md"
    try:
        link.symlink_to(secret)
    except (OSError, NotImplementedError):  # pragma: no cover (Windows w/o priv)
        pytest.skip("symlinks not supported on this platform/privilege level")

    snap = build_snapshot(brain)
    paths = {e["path"] for e in snap["entries"]}
    assert "alpha/leak.md" not in paths
    assert "alpha/auth.md" in paths  # real neurons still indexed
    bodies = " ".join(r.get("body", "") for r in snap["rows"])
    assert "ESCAPEDSECRET" not in bodies


def test_snapshot_survives_symlink_loop(tmp_path):
    """A self-referential symlink must not abort the boot walk: resolve()
    raises RuntimeError (<=3.12) or yields an unreadable path (3.13+). Either
    way the neuron is dropped and the snapshot still builds."""
    brain = _make_brain(tmp_path)
    loop = brain / "alpha" / "loop.md"
    try:
        loop.symlink_to(loop)
    except (OSError, NotImplementedError):  # pragma: no cover (Windows w/o priv)
        pytest.skip("symlinks not supported on this platform/privilege level")

    snap = build_snapshot(brain)  # must not raise
    paths = {e["path"] for e in snap["entries"]}
    assert "alpha/loop.md" not in paths
    assert "alpha/auth.md" in paths


# --- parse errors: malformed frontmatter is dropped, but COUNTED --------------


def test_snapshot_tallies_malformed_frontmatter(tmp_path):
    """A neuron whose frontmatter won't parse is dropped from every surface —
    so the snapshot reports the count (and a bounded sample) instead of
    silently under-counting the brain."""
    brain = _make_brain(tmp_path)
    # Unbalanced flow sequence in YAML frontmatter → parse error.
    _write(brain, "alpha/bad.md", "---\ntitle: [unclosed\n---\noops\n")

    snap = build_snapshot(brain)
    assert snap["parse_errors"] == 1
    assert "alpha/bad.md" in snap["parse_error_sample"]
    # The malformed neuron is absent; the good ones remain.
    paths = {e["path"] for e in snap["entries"]}
    assert "alpha/bad.md" not in paths
    assert "alpha/auth.md" in paths


def test_wake_up_payload_surfaces_parse_errors(tmp_path):
    """build_payload threads the snapshot's parse-error count into the wake-up
    payload so an operator can see neurons were dropped."""
    brain = _make_brain(tmp_path)
    _write(brain, "alpha/bad.md", "---\ntitle: [unclosed\n---\noops\n")
    snap = build_snapshot(brain)
    fed = build_payload(brain, snapshot=snap)
    assert fed["parse_errors"] == 1
    # No snapshot ⇒ the per-call walk doesn't tally, so the field is 0.
    assert build_payload(brain)["parse_errors"] == 0
