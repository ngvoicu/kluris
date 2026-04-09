"""Tests for yaml-neurons feature — hash-style frontmatter block read/write.

This test file focuses on the core `kluris.core.frontmatter` yaml path.
Scanner-level tests (linker, maps, mri, search, wake-up, dream) live in
their own respective test files. End-to-end integration tests against
realistic brains live in `tests/test_yaml_neurons_integration.py`.
"""

from kluris.core.frontmatter import read_frontmatter, update_frontmatter, write_frontmatter


def test_read_yaml_neuron_with_hash_block(tmp_path):
    """A yaml file opened with read_frontmatter() must return its hash-style
    `#---` block as a metadata dict and the remaining yaml document as body.
    """
    path = tmp_path / "openapi.yml"
    path.write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# related: [./auth.md]\n"
        "# tags: [api, openapi]\n"
        "# title: Payments API\n"
        "# updated: 2026-04-09\n"
        "#---\n"
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
        "paths: {}\n",
        encoding="utf-8",
    )

    meta, body = read_frontmatter(path)

    assert meta.get("parent") == "./map.md"
    assert meta.get("related") == ["./auth.md"]
    assert meta.get("tags") == ["api", "openapi"]
    assert meta.get("title") == "Payments API"
    assert meta.get("updated") == "2026-04-09"
    # Body should start with the yaml document proper — the #--- block is stripped.
    assert body.lstrip().startswith("openapi: 3.1.0")
    assert "#---" not in body
    assert "# parent:" not in body


def test_read_yaml_neuron_without_block(tmp_path):
    """A yaml file WITHOUT a #--- block is not a kluris neuron. read_frontmatter()
    must return empty metadata and the full file content unchanged — the opt-out.
    """
    path = tmp_path / "ci-config.yml"
    original = (
        "name: ci\n"
        "on: [push, pull_request]\n"
        "jobs:\n"
        "  build:\n"
        "    runs-on: ubuntu-latest\n"
    )
    path.write_text(original, encoding="utf-8")

    meta, body = read_frontmatter(path)

    assert meta == {}
    assert body == original


def test_read_yaml_neuron_malformed_block(tmp_path):
    """A yaml neuron whose hash block contains invalid yaml must not raise.
    The reader falls back to the opt-out behaviour: empty meta, full body.
    """
    path = tmp_path / "broken.yml"
    path.write_text(
        "#---\n"
        "# parent: [this is an unclosed list\n"
        "# related: not valid yaml here either:\n"
        "#---\n"
        "openapi: 3.1.0\n",
        encoding="utf-8",
    )

    # Must not raise.
    meta, body = read_frontmatter(path)

    assert meta == {}
    # Body is the full content (fallback path returns untouched file text).
    assert "openapi: 3.1.0" in body


def test_write_yaml_neuron_preserves_body_bytes(tmp_path):
    """write_frontmatter() on a yaml file must leave the body portion
    byte-identical — no PyYAML round-trip, no comment loss, no key reordering.
    Only the #--- block at the top is replaced with the new metadata.
    """
    path = tmp_path / "openapi.yml"
    original_body = (
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  # IMPORTANT: version must be bumped on every breaking change\n"
        "  version: 1.0.0\n"
        "  description: |\n"
        "    Multi-line\n"
        "    description with trailing spaces.   \n"
        "paths:\n"
        "  /charge: {}  # placeholder\n"
    )
    path.write_text(
        "#---\n# parent: ./map.md\n# updated: 2026-01-01\n#---\n" + original_body,
        encoding="utf-8",
    )

    write_frontmatter(
        path,
        {"parent": "./map.md", "updated": "2026-04-09", "tags": ["api"]},
        original_body,
    )

    # Read back, split off the new block, and compare body byte-for-byte.
    new_content = path.read_text(encoding="utf-8")
    # Strip the leading #--- block.
    assert new_content.startswith("#---\n")
    # Find the second #--- and take everything after.
    first = new_content.index("#---")
    second = new_content.index("#---", first + 4)
    body_after = new_content[second + len("#---"):]
    # Skip the newline right after the closing sentinel.
    if body_after.startswith("\n"):
        body_after = body_after[1:]
    assert body_after == original_body, "yaml body must be preserved byte-for-byte"
    # Block must contain the new updated value. yaml.safe_dump quotes
    # date-looking strings to preserve their string type, so we accept
    # either quoted or unquoted rendering.
    block = new_content[:second]
    assert "2026-04-09" in block
    assert "tags:" in block
    assert "parent: ./map.md" in block


def test_update_frontmatter_yaml_adds_block_when_missing(tmp_path):
    """update_frontmatter() on a raw yaml file (no #--- block) must add a
    block at the top with the patched metadata, leaving the existing body
    unchanged below. This is the "bootstrap an existing yaml into a kluris
    neuron" path used by dream's _sync_brain_state.
    """
    path = tmp_path / "openapi.yml"
    original_body = (
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
        "paths: {}\n"
    )
    path.write_text(original_body, encoding="utf-8")

    update_frontmatter(path, {"updated": "2026-04-09", "parent": "./map.md"})

    meta, body = read_frontmatter(path)
    assert meta.get("updated") == "2026-04-09"
    assert meta.get("parent") == "./map.md"
    assert body == original_body


def test_read_yaml_neuron_preserves_crlf_body(tmp_path):
    """A yaml file with CRLF line endings must be readable without Python's
    universal-newlines mode normalizing `\\r\\n` to `\\n`. The body returned
    by read_frontmatter() must still contain CRLF if the file on disk does.
    """
    path = tmp_path / "openapi.yml"
    # Write raw bytes with CRLF throughout — block + body.
    path.write_bytes(
        b"#---\r\n# parent: ./map.md\r\n# updated: 2026-01-01\r\n#---\r\n"
        b"openapi: 3.1.0\r\ninfo:\r\n  title: API\r\n"
    )

    meta, body = read_frontmatter(path)

    # Metadata was parsed correctly
    assert meta.get("parent") == "./map.md"
    # Body must still have CRLF — universal newlines would have silently
    # lost this information.
    assert "\r\n" in body, (
        f"expected CRLF preserved in body, got repr: {body!r}"
    )


def test_round_trip_crlf_yaml_body_preserved(tmp_path):
    """Full round trip: read a CRLF yaml file, call update_frontmatter()
    with a patch, then read it again and verify the body STILL has CRLF.
    This catches the combined read_text + write_text normalization bug.
    """
    path = tmp_path / "openapi.yml"
    original = (
        b"#---\r\n# parent: ./map.md\r\n# updated: 2026-01-01\r\n#---\r\n"
        b"openapi: 3.1.0\r\n"
        b"info:\r\n"
        b"  title: API\r\n"
        b"  version: 1.0.0\r\n"
    )
    path.write_bytes(original)

    update_frontmatter(path, {"updated": "2026-04-09"})

    # The body portion (after the closing sentinel) must still be CRLF.
    raw = path.read_bytes()
    # Find the closing #--- (last one in the block)
    # Simple heuristic: split on `#---` and the last part is the body
    parts = raw.split(b"#---")
    body_part = parts[-1].lstrip(b"\r\n")
    assert b"openapi: 3.1.0\r\n" in body_part, (
        f"expected CRLF body after round trip, got: {body_part[:100]!r}"
    )


def test_update_frontmatter_yaml_mutates_existing_block(tmp_path):
    """update_frontmatter() on a yaml file that already has a #--- block must
    merge the patch into the existing block, preserve unrelated fields, and
    leave the body byte-identical.
    """
    path = tmp_path / "openapi.yml"
    original_body = (
        "openapi: 3.1.0\n"
        "info:\n"
        "  title: Payments API\n"
        "  version: 1.0.0\n"
    )
    path.write_text(
        "#---\n"
        "# parent: ./map.md\n"
        "# tags: [api]\n"
        "# updated: 2026-01-01\n"
        "#---\n" + original_body,
        encoding="utf-8",
    )

    update_frontmatter(path, {"updated": "2026-04-09"})

    meta, body = read_frontmatter(path)
    assert meta.get("updated") == "2026-04-09"
    assert meta.get("parent") == "./map.md"  # preserved
    assert meta.get("tags") == ["api"]  # preserved
    assert body == original_body
