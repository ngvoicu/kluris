"""Tests for synapse validation, bidirectional checks, and orphan detection."""

from kluris.core.linker import (
    check_frontmatter,
    detect_orphans,
    parse_markdown_links,
    validate_bidirectional,
    validate_synapses,
)


def _make_linked_brain(tmp_path):
    """Brain with valid linking."""
    brain = tmp_path / "brain"
    brain.mkdir()
    arch = brain / "architecture"
    arch.mkdir()
    (arch / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n"
        "# Architecture\n\n- [auth.md](./auth.md) — Auth\n", encoding="utf-8"
    )
    (arch / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../standards/naming.md\n"
        "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    std = brain / "standards"
    std.mkdir()
    (std / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n"
        "# Standards\n\n- [naming.md](./naming.md) — Naming\n", encoding="utf-8"
    )
    (std / "naming.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../architecture/auth.md\n"
        "tags: [naming]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Naming\n", encoding="utf-8"
    )
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n\n"
        "- [architecture/](./architecture/map.md)\n"
        "- [standards/](./standards/map.md)\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")
    return brain


def test_valid_links(tmp_path):
    brain = _make_linked_brain(tmp_path)
    broken = validate_synapses(brain)
    assert len(broken) == 0


def test_broken_link(tmp_path):
    brain = _make_linked_brain(tmp_path)
    # Add a broken link
    (brain / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Auth\n\nSee [nonexistent](./nonexistent.md)\n", encoding="utf-8"
    )
    broken = validate_synapses(brain)
    assert len(broken) >= 1
    assert any("nonexistent" in b["target"] for b in broken)


def test_broken_related_synapse(tmp_path):
    brain = _make_linked_brain(tmp_path)
    (brain / "architecture" / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../standards/missing.md\n"
        "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n",
        encoding="utf-8",
    )
    broken = validate_synapses(brain)
    assert len(broken) >= 1
    assert any("missing.md" in b["target"] for b in broken)


def test_orphaned_neuron(tmp_path):
    brain = _make_linked_brain(tmp_path)
    # Add a neuron not referenced from any map
    (brain / "architecture" / "orphan.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Orphan\n", encoding="utf-8"
    )
    orphans = detect_orphans(brain)
    assert any("orphan.md" in str(o) for o in orphans)


def test_bidirectional_valid(tmp_path):
    brain = _make_linked_brain(tmp_path)
    one_way = validate_bidirectional(brain)
    assert len(one_way) == 0


def test_one_way_synapse(tmp_path):
    brain = _make_linked_brain(tmp_path)
    # Remove the reverse link from naming.md
    (brain / "standards" / "naming.md").write_text(
        "---\nparent: ./map.md\nrelated: []\n"
        "tags: [naming]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Naming\n", encoding="utf-8"
    )
    one_way = validate_bidirectional(brain)
    assert len(one_way) >= 1


def test_missing_parent(tmp_path):
    brain = _make_linked_brain(tmp_path)
    (brain / "architecture" / "no-parent.md").write_text(
        "---\ntags: [test]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# No Parent\n", encoding="utf-8"
    )
    issues = check_frontmatter(brain)
    assert any("parent" in i["field"] for i in issues)


def test_missing_created(tmp_path):
    brain = _make_linked_brain(tmp_path)
    (brain / "architecture" / "no-date.md").write_text(
        "---\nparent: ./map.md\ntags: []\n---\n# No Date\n", encoding="utf-8"
    )
    issues = check_frontmatter(brain)
    assert any("created" in i["field"] for i in issues)


def test_reachability(tmp_path):
    brain = _make_linked_brain(tmp_path)
    # All neurons in the test brain should be reachable
    orphans = detect_orphans(brain)
    # auth.md and naming.md are in maps, so they should not be orphans
    orphan_names = [str(o) for o in orphans]
    assert not any("auth.md" in o for o in orphan_names)
    assert not any("naming.md" in o for o in orphan_names)
