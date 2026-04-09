"""Tests for synapse validation, bidirectional checks, and orphan detection."""

from kluris.core.frontmatter import read_frontmatter, update_frontmatter
from kluris.core.linker import (
    _neuron_files,
    check_frontmatter,
    detect_deprecation_issues,
    detect_orphans,
    fix_bidirectional_synapses,
    parse_markdown_links,
    validate_bidirectional,
    validate_synapses,
)


def _make_brain_with_yaml_neurons(tmp_path):
    """Small brain covering the 4 critical yaml-neurons cases:
    markdown neuron, opted-in yaml neuron (has #--- block), raw yaml (opt-out,
    no block — must be invisible), and `kluris.yml` at root (must be invisible).
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    (brain / "glossary.md").write_text("---\n---\n# Glossary\n", encoding="utf-8")
    # CRITICAL — this file must NEVER appear in any scanner result.
    (brain / "kluris.yml").write_text(
        "name: brain\ntype: product\n", encoding="utf-8"
    )

    lobe = brain / "projects"
    lobe.mkdir()
    (lobe / "map.md").write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n---\n# Projects\n",
        encoding="utf-8",
    )
    (lobe / "auth.md").write_text(
        "---\nparent: ./map.md\nrelated: [./openapi.yml]\ntags: [auth]\n"
        "created: 2026-04-01\nupdated: 2026-04-01\n---\n# Auth\n"
        "\nSee [the API](./openapi.yml) for details.\n",
        encoding="utf-8",
    )
    # Opted-in yaml neuron with hash-style block
    (lobe / "openapi.yml").write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# related: [./auth.md]\n"
        "# tags: [api, openapi]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-01\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
        "paths: {}\n",
        encoding="utf-8",
    )
    # Raw yaml file — NO #--- block. Must be invisible to scanners.
    (lobe / "ci-config.yml").write_text(
        "name: ci\non: [push]\njobs:\n  build: {}\n",
        encoding="utf-8",
    )
    return brain


def test_neuron_files_includes_opted_in_yaml(tmp_path):
    """`_neuron_files` must return markdown neurons AND opted-in yaml neurons,
    but not raw yaml without a block, not `kluris.yml` at brain root, and not
    auto-generated files (brain.md / map.md / glossary.md).
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)

    paths = sorted(f.relative_to(brain).as_posix() for f in _neuron_files(brain))

    assert "projects/auth.md" in paths
    assert "projects/openapi.yml" in paths
    # Opt-out: raw yaml without a block
    assert "projects/ci-config.yml" not in paths
    # Brain-root config must never leak
    assert "kluris.yml" not in paths
    # Auto-generated files stay excluded
    assert "brain.md" not in paths
    assert "glossary.md" not in paths
    assert "projects/map.md" not in paths


def test_neuron_files_excludes_kluris_yml_even_with_block(tmp_path):
    """Adversarial case: `kluris.yml` at the brain root with a `#---` block
    MUST still be excluded by filename (SKIP_FILES). Defense in depth — the
    opt-in block alone is not enough to protect the brain's local config.
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    # Adversarial kluris.yml with a hash block — should still be skipped.
    (brain / "kluris.yml").write_text(
        "#---\n"
        "# parent: ./brain.md\n"
        "# updated: 2026-04-09\n"
        "#---\n"
        "name: brain\ntype: product\n",
        encoding="utf-8",
    )
    lobe = brain / "projects"
    lobe.mkdir()
    (lobe / "map.md").write_text(
        "---\nparent: ../brain.md\n---\n# Projects\n", encoding="utf-8"
    )

    paths = [f.relative_to(brain).as_posix() for f in _neuron_files(brain)]
    assert "kluris.yml" not in paths


def test_validate_synapses_detects_broken_related_in_yaml_neuron(tmp_path):
    """A yaml neuron with a `related:` entry pointing to a nonexistent file
    must be flagged by `validate_synapses`.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    # Rewrite openapi.yml's block to point related at a dead file.
    openapi = brain / "projects" / "openapi.yml"
    body = (
        "openapi: 3.1.0\n"
        "info:\n  title: Payments API\n  version: 1.0.0\n"
        "paths: {}\n"
    )
    openapi.write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# related: [./deleted.md]\n"
        "# tags: [api]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-01\n"
        "#---\n" + body,
        encoding="utf-8",
    )

    broken = validate_synapses(brain)
    sources = {b["file"] for b in broken}
    assert "projects/openapi.yml" in sources


def test_fix_bidirectional_synapses_md_to_yaml(tmp_path):
    """When a markdown neuron's `related:` points at a yaml neuron that does
    NOT list the markdown neuron back, `fix_bidirectional_synapses` must add
    the reverse link INTO the yaml neuron's #--- block.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    # Remove `related: [./auth.md]` from the yaml block so it's one-way.
    openapi = brain / "projects" / "openapi.yml"
    body = (
        "openapi: 3.1.0\n"
        "info:\n  title: Payments API\n  version: 1.0.0\n"
        "paths: {}\n"
    )
    openapi.write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# tags: [api]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-01\n"
        "#---\n" + body,
        encoding="utf-8",
    )

    fixed = fix_bidirectional_synapses(brain)
    assert fixed >= 1

    meta, _ = read_frontmatter(openapi)
    related = meta.get("related") or []
    # The reverse link must now exist. It's relative to the yaml file's dir.
    assert any("auth.md" in str(r) for r in related), (
        f"expected auth.md in yaml related list, got {related}"
    )


def test_detect_orphans_flags_yaml_neuron_not_in_map(tmp_path):
    """A yaml neuron that isn't referenced from its lobe's map.md must be
    flagged as an orphan by `detect_orphans`.
    """
    brain = _make_brain_with_yaml_neurons(tmp_path)
    # The fixture's map.md doesn't mention openapi.yml at all.
    # (It's a plain header with no contents list.) So both yaml and md
    # neurons are orphans unless we add them to the map.
    orphans = detect_orphans(brain)
    orphan_paths = {str(o) for o in orphans}
    assert "projects/openapi.yml" in orphan_paths

    # Now add a link to openapi.yml from map.md and re-run.
    map_md = brain / "projects" / "map.md"
    map_md.write_text(
        "---\nparent: ../brain.md\n---\n# Projects\n\n"
        "- [api](./openapi.yml) — spec\n"
        "- [auth](./auth.md) — auth\n",
        encoding="utf-8",
    )
    orphans_after = {str(o) for o in detect_orphans(brain)}
    assert "projects/openapi.yml" not in orphans_after


def test_check_frontmatter_yaml_lighter_contract(tmp_path):
    """Yaml neurons follow a lighter frontmatter contract: only `updated` is
    required. Missing `parent` or `created` is allowed for yaml (inferred from
    filesystem position + git). Missing `updated` IS an error.
    """
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "brain.md").write_text(
        "---\nauto_generated: true\n---\n# Brain\n", encoding="utf-8"
    )
    lobe = brain / "projects"
    lobe.mkdir()
    (lobe / "map.md").write_text(
        "---\nparent: ../brain.md\n---\n# Projects\n", encoding="utf-8"
    )
    # Yaml with only `updated` — should be OK
    (lobe / "ok.yml").write_text(
        "#---\n# updated: 2026-04-09\n#---\nopenapi: 3.1.0\n",
        encoding="utf-8",
    )
    # Yaml missing `updated` — should be flagged
    (lobe / "broken.yml").write_text(
        "#---\n# parent: ./map.md\n#---\nopenapi: 3.1.0\n",
        encoding="utf-8",
    )

    issues = check_frontmatter(brain)
    by_file = {i["file"]: i.get("field") for i in issues}
    # ok.yml must have no issues (lighter contract: parent/created not required)
    assert "projects/ok.yml" not in by_file
    # broken.yml must be flagged for missing updated
    broken_issues = [i for i in issues if i["file"] == "projects/broken.yml"]
    assert any(i.get("field") == "updated" for i in broken_issues)
    # broken.yml must NOT be flagged for missing parent or created (lighter contract)
    assert not any(i.get("field") == "parent" for i in broken_issues)
    assert not any(i.get("field") == "created" for i in broken_issues)


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


def test_link_that_escapes_brain_is_broken(tmp_path):
    """A relative link that resolves outside the brain is reported broken,
    even if the target file happens to exist on disk. Brains must be
    self-contained."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    # A file OUTSIDE the brain that exists on disk
    (tmp_path / "outside.md").write_text("# Outside\n", encoding="utf-8")
    (brain / "knowledge" / "escape.md").write_text(
        "---\nparent: ./map.md\ntags: []\n"
        "created: 2026-01-01\nupdated: 2026-04-01\n---\n"
        "# Escape\n\n"
        "See [outside](../../outside.md) for details.\n",
        encoding="utf-8",
    )
    broken = validate_synapses(brain)
    assert any("escape.md" in b["file"] for b in broken)


def test_related_that_escapes_brain_is_broken(tmp_path):
    """Same escape check applies to `related:` frontmatter."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    (tmp_path / "outside.md").write_text("# Outside\n", encoding="utf-8")
    _write_neuron(
        brain, "knowledge/escape.md", "Escape",
        related=["../../outside.md"],
    )
    broken = validate_synapses(brain)
    assert any(b["target"] == "../../outside.md" for b in broken)


def test_replaced_by_escaping_brain_is_flagged(tmp_path):
    """A deprecated neuron whose replaced_by resolves outside the brain is
    reported as replaced_by_missing (same envelope)."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    (tmp_path / "outside.md").write_text(
        "---\ncreated: 2026-01-01\nupdated: 2026-04-01\ntags: []\nparent: ./map.md\n---\n# Outside\n",
        encoding="utf-8",
    )
    _write_neuron(
        brain, "knowledge/old.md", "Old",
        status="deprecated",
        deprecated_at="2026-03-01",
        replaced_by="../../outside.md",
    )
    issues = detect_deprecation_issues(brain)
    assert any(i["kind"] == "replaced_by_missing" for i in issues)


def test_check_frontmatter_flags_wrong_types(tmp_path):
    """`related:` as a string, `tags:` as a string, `replaced_by:` as a list
    are all type errors that should surface, not be silently skipped."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    (brain / "knowledge" / "bad.md").write_text(
        "---\n"
        "parent: ./map.md\n"
        "related: 'just-a-string.md'\n"       # wrong: should be list
        "tags: 'also-a-string'\n"              # wrong: should be list
        "created: 2026-01-01\n"
        "updated: 2026-04-01\n"
        "---\n"
        "# Bad\n",
        encoding="utf-8",
    )
    issues = check_frontmatter(brain)
    fields_with_type_errors = [
        i["field"] for i in issues if i.get("kind") == "type"
    ]
    assert "related" in fields_with_type_errors
    assert "tags" in fields_with_type_errors


def test_check_frontmatter_flags_replaced_by_wrong_type(tmp_path):
    """`replaced_by:` must be a string; a list value is flagged."""
    brain = tmp_path / "brain"
    brain.mkdir()
    (brain / "knowledge").mkdir()
    (brain / "knowledge" / "map.md").write_text(
        "---\nauto_generated: true\n---\n# K\n", encoding="utf-8"
    )
    (brain / "knowledge" / "bad.md").write_text(
        "---\n"
        "parent: ./map.md\n"
        "status: deprecated\n"
        "replaced_by:\n  - './new.md'\n"    # wrong: should be a single string
        "created: 2026-01-01\n"
        "updated: 2026-04-01\n"
        "---\n"
        "# Bad\n",
        encoding="utf-8",
    )
    issues = check_frontmatter(brain)
    assert any(
        i.get("field") == "replaced_by" and i.get("kind") == "type"
        for i in issues
    )
