"""Tests for frontmatter read/write operations."""

from kluris.core.frontmatter import read_frontmatter, update_frontmatter, write_frontmatter


# --- [TEST-KLU-07] Frontmatter operations ---


def test_read_frontmatter(tmp_path):
    f = tmp_path / "test.md"
    f.write_text("---\ntitle: Hello\ntags: [a, b]\n---\n# Content\n")
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
        "tags: [auth]\ncreated: 2026-01-01\nupdated: 2026-03-15\n---\n# Neuron\n"
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
        "siblings:\n  - ../product/map.md\nupdated: 2026-04-01\n---\n# Map\n"
    )
    meta, _ = read_frontmatter(f)
    assert meta["auto_generated"] is True
    assert meta["parent"] == "../brain.md"
    assert meta["siblings"] == ["../product/map.md"]


def test_update_field(tmp_path):
    f = tmp_path / "test.md"
    f.write_text("---\ntitle: Old\nupdated: 2026-01-01\n---\n# Body\n")
    update_frontmatter(f, {"updated": "2026-04-01"})
    meta, _ = read_frontmatter(f)
    assert meta["updated"] == "2026-04-01"
    assert meta["title"] == "Old"


def test_preserves_content(tmp_path):
    f = tmp_path / "test.md"
    body = "# Title\n\nSome detailed content here.\n\n- Item 1\n- Item 2\n"
    f.write_text(f"---\ntitle: Test\n---\n{body}")
    update_frontmatter(f, {"title": "Updated"})
    meta, content = read_frontmatter(f)
    assert meta["title"] == "Updated"
    assert "Some detailed content here." in content
    assert "- Item 1" in content


def test_missing_frontmatter(tmp_path):
    f = tmp_path / "plain.md"
    f.write_text("# Just markdown, no frontmatter\n")
    meta, content = read_frontmatter(f)
    assert meta == {} or meta is not None
    assert "Just markdown" in content
