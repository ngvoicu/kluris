"""Tests for frontmatter read/write operations."""

from kluris.core.frontmatter import read_frontmatter, update_frontmatter, write_frontmatter


# --- [TEST-KLU-07] Frontmatter operations ---


def test_read_frontmatter(tmp_path):
    f = tmp_path / "test.md"
    f.write_text("---\ntitle: Hello\ntags: [a, b]\n---\n# Content\n", encoding="utf-8")
    meta, content = read_frontmatter(f)
    assert meta["title"] == "Hello"
    assert meta["tags"] == ["a", "b"]
    assert "# Content" in content


def test_write_frontmatter(tmp_path):
    f = tmp_path / "test.md"
    write_frontmatter(f, {"title": "New", "tags": ["x"]}, "# Body\n")
    meta, content = read_frontmatter(f)
    assert meta["title"] == "New"
    assert meta["tags"] == ["x"]
    assert "# Body" in content


def test_read_neuron_fields(tmp_path):
    f = tmp_path / "neuron.md"
    f.write_text(
        "---\nparent: ../map.md\nrelated:\n  - ../other.md\n"
        "tags: [auth]\ncreated: 2026-01-01\nupdated: 2026-03-15\n---\n# Neuron\n", encoding="utf-8"
    )
    meta, _ = read_frontmatter(f)
    assert meta["parent"] == "../map.md"
    assert meta["related"] == ["../other.md"]
    assert meta["tags"] == ["auth"]
    assert meta["created"] == "2026-01-01"
    assert meta["updated"] == "2026-03-15"


def test_read_map_fields(tmp_path):
    f = tmp_path / "map.md"
    f.write_text(
        "---\nauto_generated: true\nparent: ../brain.md\n"
        "siblings:\n  - ../product/map.md\nupdated: 2026-04-01\n---\n# Map\n", encoding="utf-8"
    )
    meta, _ = read_frontmatter(f)
    assert meta["auto_generated"] is True
    assert meta["parent"] == "../brain.md"
    assert meta["siblings"] == ["../product/map.md"]


def test_update_field(tmp_path):
    f = tmp_path / "test.md"
    f.write_text("---\ntitle: Old\nupdated: 2026-01-01\n---\n# Body\n", encoding="utf-8")
    update_frontmatter(f, {"updated": "2026-04-01"})
    meta, _ = read_frontmatter(f)
    assert meta["updated"] == "2026-04-01"
    assert meta["title"] == "Old"


def test_preserves_content(tmp_path):
    f = tmp_path / "test.md"
    body = "# Title\n\nSome detailed content here.\n\n- Item 1\n- Item 2\n"
    f.write_text(f"---\ntitle: Test\n---\n{body}", encoding="utf-8")
    update_frontmatter(f, {"title": "Updated"})
    meta, content = read_frontmatter(f)
    assert meta["title"] == "Updated"
    assert "Some detailed content here." in content
    assert "- Item 1" in content


def test_missing_frontmatter(tmp_path):
    f = tmp_path / "plain.md"
    f.write_text("# Just markdown, no frontmatter\n", encoding="utf-8")
    meta, content = read_frontmatter(f)
    assert meta == {} or meta is not None
    assert "Just markdown" in content


# --- preloaded shortcut (Phase 2) ---


def test_update_frontmatter_preloaded_skips_disk_read(tmp_path, monkeypatch):
    """When `preloaded=(meta, body)` is passed, update_frontmatter must NOT
    call frontmatter.load — it uses the supplied tuple directly. This eliminates
    the hidden 2x read cost in `_sync_brain_state`."""
    import frontmatter

    f = tmp_path / "test.md"
    f.write_text(
        "---\ntitle: Original\nupdated: 2026-01-01\n---\n# Body\n\nSome content.\n",
        encoding="utf-8",
    )

    # Read once via the public API
    meta, body = read_frontmatter(f)

    # Now monkeypatch frontmatter.load to raise — if update_frontmatter
    # tries to re-read the file, this assertion fires.
    def _explode(*args, **kwargs):
        raise AssertionError("frontmatter.load was called even though preloaded was passed")
    monkeypatch.setattr(frontmatter, "load", _explode)

    # The preloaded path must NOT call frontmatter.load
    update_frontmatter(f, {"updated": "2026-04-07"}, preloaded=(meta, body))

    # Un-patch to verify the file was actually updated
    monkeypatch.undo()
    new_meta, new_body = read_frontmatter(f)
    assert new_meta["updated"] == "2026-04-07"
    assert new_meta["title"] == "Original"  # other fields preserved
    assert "Some content." in new_body  # body preserved


def test_update_frontmatter_legacy_path_still_works(tmp_path):
    """The non-preloaded call signature still works (backward compatibility)."""
    f = tmp_path / "test.md"
    f.write_text(
        "---\ntitle: Old\nupdated: 2026-01-01\n---\n# Body\n",
        encoding="utf-8",
    )
    update_frontmatter(f, {"updated": "2026-04-07"})
    meta, _ = read_frontmatter(f)
    assert meta["updated"] == "2026-04-07"
    assert meta["title"] == "Old"


def test_update_frontmatter_preloaded_does_not_mutate_caller_dict(tmp_path):
    """update_frontmatter must defensively copy the preloaded meta so the
    caller's dict is not modified."""
    f = tmp_path / "test.md"
    f.write_text(
        "---\ntitle: T\nupdated: 2026-01-01\n---\n# B\n",
        encoding="utf-8",
    )
    meta, body = read_frontmatter(f)
    original_title = meta.get("title")
    update_frontmatter(f, {"updated": "2026-04-07", "title": "NEW"}, preloaded=(meta, body))
    # The caller's dict must not have been mutated
    assert meta["title"] == original_title
    assert "updated" in meta  # the original updated value is still there
    assert meta["updated"] == "2026-01-01"
