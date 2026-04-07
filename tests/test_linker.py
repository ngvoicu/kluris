"""Tests for synapse validation, bidirectional checks, and orphan detection."""

from kluris.core.linker import (
    check_frontmatter,
    detect_deprecation_issues,
    detect_orphans,
    parse_markdown_links,
    validate_bidirectional,
    validate_synapses,
)


def _make_linked_brain(tmp_path):
    """Brain with valid linking."""
    brain = tmp_path / "brain"
    brain.mkdir()
    arch = brain / "projects"
    arch.mkdir()
    (arch / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n"
        "# Architecture\n\n- [auth.md](./auth.md) — Auth\n", encoding="utf-8"
    )
    (arch / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../knowledge/naming.md\n"
        "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n", encoding="utf-8"
    )
    std = brain / "knowledge"
    std.mkdir()
    (std / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n"
        "# Standards\n\n- [naming.md](./naming.md) — Naming\n", encoding="utf-8"
    )
    (std / "naming.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../projects/auth.md\n"
        "tags: [naming]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Naming\n", encoding="utf-8"
    )
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n\n"
        "- [architecture/](./projects/map.md)\n"
        "- [standards/](./knowledge/map.md)\n", encoding="utf-8"
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
    (brain / "projects" / "auth.md").write_text(
        "---\nparent: ./map.md\ntags: []\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n"
        "# Auth\n\nSee [nonexistent](./nonexistent.md)\n", encoding="utf-8"
    )
    broken = validate_synapses(brain)
    assert len(broken) >= 1
    assert any("nonexistent" in b["target"] for b in broken)


def test_broken_related_synapse(tmp_path):
    brain = _make_linked_brain(tmp_path)
    (brain / "projects" / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated:\n  - ../knowledge/missing.md\n"
        "tags: [auth]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n",
        encoding="utf-8",
    )
    broken = validate_synapses(brain)
    assert len(broken) >= 1
    assert any("missing.md" in b["target"] for b in broken)


def test_orphaned_neuron(tmp_path):
    brain = _make_linked_brain(tmp_path)
    # Add a neuron not referenced from any map
    (brain / "projects" / "orphan.md").write_text(
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
    (brain / "knowledge" / "naming.md").write_text(
        "---\nparent: ./map.md\nrelated: []\n"
        "tags: [naming]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# Naming\n", encoding="utf-8"
    )
    one_way = validate_bidirectional(brain)
    assert len(one_way) >= 1


def test_missing_parent(tmp_path):
    brain = _make_linked_brain(tmp_path)
    (brain / "projects" / "no-parent.md").write_text(
        "---\ntags: [test]\ncreated: 2026-04-01\nupdated: 2026-04-01\n---\n# No Parent\n", encoding="utf-8"
    )
    issues = check_frontmatter(brain)
    assert any("parent" in i["field"] for i in issues)


def test_missing_created(tmp_path):
    brain = _make_linked_brain(tmp_path)
    (brain / "projects" / "no-date.md").write_text(
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


# --- deprecation detection ---


def _write_neuron(
    brain,
    rel_path,
    title,
    status=None,
    replaced_by=None,
    related=None,
    deprecated_at=None,
):
    """Write a neuron file with optional deprecation frontmatter."""
    from pathlib import Path
    target = brain / rel_path
    target.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "---",
        "parent: ./map.md",
        "created: 2026-01-01",
        "updated: 2026-04-01",
    ]
    if status:
        lines.append(f"status: {status}")
    if deprecated_at:
        lines.append(f"deprecated_at: {deprecated_at}")
    if replaced_by:
        lines.append(f"replaced_by: {replaced_by}")
    if related:
        lines.append("related:")
        for r in related:
            lines.append(f"  - {r}")
    lines.append("---")
    lines.append("")
    lines.append(f"# {title}")
    target.write_text("\n".join(lines) + "\n", encoding="utf-8")


def test_no_deprecation_issues_clean(tmp_path):
    """Brain without any deprecated neurons returns no issues."""
    brain = _make_linked_brain(tmp_path)
    issues = detect_deprecation_issues(brain)
    assert issues == []


def test_active_neuron_links_to_deprecated(tmp_path):
    """Active neuron with `related:` pointing at a deprecated neuron is flagged."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    _write_neuron(
        brain, "knowledge/new-decision.md", "New",
        related=["./old-decision.md"],
    )
    _write_neuron(
        brain, "knowledge/old-decision.md", "Old",
        status="deprecated",
        deprecated_at="2026-03-01",
        replaced_by="./new-decision.md",
    )
    issues = detect_deprecation_issues(brain)
    assert any(i["kind"] == "active_links_to_deprecated" for i in issues)
    msg = next(i for i in issues if i["kind"] == "active_links_to_deprecated")
    assert "new-decision.md" in msg["source"]
    assert "old-decision.md" in msg["target"]


def test_deprecated_missing_replaced_by(tmp_path):
    """Deprecated neuron without `replaced_by` is flagged."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    _write_neuron(
        brain, "knowledge/old.md", "Old",
        status="deprecated",
        deprecated_at="2026-03-01",
    )
    issues = detect_deprecation_issues(brain)
    assert any(i["kind"] == "deprecated_without_replacement" for i in issues)


def test_replaced_by_missing_file(tmp_path):
    """`replaced_by` pointing at a nonexistent file is flagged."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    _write_neuron(
        brain, "knowledge/old.md", "Old",
        status="deprecated",
        deprecated_at="2026-03-01",
        replaced_by="./ghost.md",
    )
    issues = detect_deprecation_issues(brain)
    assert any(i["kind"] == "replaced_by_missing" for i in issues)


def test_deprecated_referenced_only_from_map_is_ok(tmp_path):
    """A deprecated neuron referenced only from map.md files is not flagged
    as 'active_links_to_deprecated' — maps are auto-generated indexes, not
    editorial endorsements."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n\n- [old.md](./old.md) — Old\n",
        encoding="utf-8",
    )
    _write_neuron(
        brain, "knowledge/old.md", "Old",
        status="deprecated",
        deprecated_at="2026-03-01",
        replaced_by="./new.md",
    )
    _write_neuron(brain, "knowledge/new.md", "New")
    issues = detect_deprecation_issues(brain)
    assert not any(i["kind"] == "active_links_to_deprecated" for i in issues)


def test_active_status_is_ok_without_frontmatter(tmp_path):
    """Neurons without a `status` field default to active — no issues."""
    brain = _make_linked_brain(tmp_path)
    # Existing brain has no `status` frontmatter at all
    issues = detect_deprecation_issues(brain)
    # None of the existing neurons should cause deprecation issues
    assert all(i["kind"] not in ("deprecated_without_replacement", "replaced_by_missing")
               for i in issues)


def test_replaced_by_pointing_to_deprecated_is_flagged(tmp_path):
    """Chain: A deprecated → B, but B itself is deprecated → C. A's migration
    target is dead, readers end up on another deprecated page."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    _write_neuron(
        brain, "knowledge/a.md", "A",
        status="deprecated",
        deprecated_at="2026-02-01",
        replaced_by="./b.md",
    )
    _write_neuron(
        brain, "knowledge/b.md", "B",
        status="deprecated",
        deprecated_at="2026-03-01",
        replaced_by="./c.md",
    )
    _write_neuron(brain, "knowledge/c.md", "C")

    issues = detect_deprecation_issues(brain)
    kinds = [i["kind"] for i in issues]
    # A is the one with a dead chain — B's deprecation is tracked via its own
    # replaced_by (which is fine: C is active).
    assert "replaced_by_not_active" in kinds
    a_issue = next(i for i in issues if i["kind"] == "replaced_by_not_active"
                   and "a.md" in i["file"])
    assert "b.md" in a_issue["target"]


def test_replaced_by_pointing_to_non_neuron_is_flagged(tmp_path):
    """`replaced_by: ./map.md` points readers to a generated index instead of
    a replacement record. That's not a migration path — it's noise."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    _write_neuron(
        brain, "knowledge/old.md", "Old",
        status="deprecated",
        deprecated_at="2026-03-01",
        replaced_by="./map.md",
    )
    issues = detect_deprecation_issues(brain)
    kinds = [i["kind"] for i in issues]
    assert "replaced_by_not_active" in kinds
