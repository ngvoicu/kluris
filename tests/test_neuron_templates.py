"""Tests for neuron template system."""

from kluris.core.brain import get_type_defaults, lookup_template, generate_neuron_content


def test_lookup_decision():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("decision", defaults.get("neuron_templates", {}))
    assert tmpl is not None


def test_lookup_incident():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("incident", defaults.get("neuron_templates", {}))
    assert tmpl is not None


def test_lookup_runbook():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("runbook", defaults.get("neuron_templates", {}))
    assert tmpl is not None


def test_lookup_missing():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("nonexistent", defaults.get("neuron_templates", {}))
    assert tmpl is None


def test_decision_sections():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("decision", defaults["neuron_templates"])
    assert len(tmpl["sections"]) == 5


def test_incident_sections():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("incident", defaults["neuron_templates"])
    assert len(tmpl["sections"]) == 6


def test_runbook_sections():
    defaults = get_type_defaults("team")
    tmpl = lookup_template("runbook", defaults["neuron_templates"])
    assert len(tmpl["sections"]) == 5


def test_generate_with_template():
    defaults = get_type_defaults("team")
    tmpl = defaults["neuron_templates"]["decision"]
    content = generate_neuron_content("Auth Migration", "./map.md", template_name="decision", sections=tmpl["sections"])
    assert "## Context" in content
    assert "## Decision" in content
    assert "## Rationale" in content
    assert "parent: ./map.md" in content
    assert "template: decision" in content


def test_generate_without_template():
    content = generate_neuron_content("Simple Note", "./map.md")
    assert "# Simple Note" in content
    assert "parent: ./map.md" in content
    assert "## Context" not in content


def test_frontmatter_template_field():
    defaults = get_type_defaults("team")
    tmpl = defaults["neuron_templates"]["decision"]
    content = generate_neuron_content("Test", "./map.md", template_name="decision", sections=tmpl["sections"])
    assert "template: decision" in content
