"""Tests for ``kluris_runtime.neuron_excerpt.extract``.

Covers H1 detection, filename-stem fallback, first-line excerpt
extraction (skipping subheadings/lists/fences/navigation), HTML anchor
stripping, and the 220-character cap.
"""

from __future__ import annotations

from pathlib import Path

from kluris_runtime.neuron_excerpt import extract


def _p(name: str = "auth-flow.md") -> Path:
    return Path("knowledge") / name


def test_extracts_h1_as_title():
    title, _ = extract(_p(), "# Auth Flow\n\nDetails follow.\n")
    assert title == "Auth Flow"


def test_falls_back_to_filename_stem_when_no_h1():
    title, _ = extract(_p("payments-api.md"), "Detail body.\n")
    assert title == "Payments Api"


def test_skips_blank_lines_for_excerpt():
    _title, excerpt = extract(_p(), "# Auth\n\n\nFirst real line.\n")
    assert excerpt == "First real line."


def test_skips_subheadings_lists_and_fences():
    """Lines that *start* with subheading / list / fence / nav markers
    are skipped; the first non-trivial body line wins.
    """
    body = (
        "# Auth\n"
        "\n"
        "## Section\n"
        "- a list item\n"
        "* another\n"
        "```\n"
        "Real content here.\n"
    )
    _title, excerpt = extract(_p(), body)
    assert excerpt == "Real content here."


def test_skips_navigation_lines():
    body = (
        "# Auth\n"
        "\n"
        "up brain.md\n"
        "sideways related.md\n"
        "Body line.\n"
    )
    _title, excerpt = extract(_p(), body)
    assert excerpt == "Body line."


def test_strips_empty_html_anchors():
    body = "# Term\n\n<a id=\"x\"></a>Real word.\n"
    _title, excerpt = extract(_p(), body)
    assert "<a id=" not in excerpt
    assert "Real word." in excerpt


def test_excerpt_capped_at_220_characters():
    long_line = "x" * 500
    _title, excerpt = extract(_p(), f"# T\n\n{long_line}\n")
    assert len(excerpt) == 220


def test_no_excerpt_when_only_h1():
    _title, excerpt = extract(_p(), "# Only Heading\n")
    assert excerpt == ""


def test_yaml_neuron_handling():
    """Yaml neurons have no markdown body; falls back to filename-stem title."""
    title, excerpt = extract(Path("knowledge/openapi.yml"), "openapi: 3.1.0\n")
    assert title == "Openapi"
    assert excerpt == "openapi: 3.1.0"
