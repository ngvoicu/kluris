"""Tests for neuron template system."""

from kluris.core.brain import NEURON_TEMPLATES, lookup_template, generate_neuron_content


def test_lookup_decision():
    
    tmpl = lookup_template("decision", NEURON_TEMPLATES)
    assert tmpl is not None


def test_lookup_incident():
    
    tmpl = lookup_template("incident", NEURON_TEMPLATES)
    assert tmpl is not None


def test_lookup_runbook():
    
    tmpl = lookup_template("runbook", NEURON_TEMPLATES)
    assert tmpl is not None


def test_lookup_missing():
    
    tmpl = lookup_template("nonexistent", NEURON_TEMPLATES)
    assert tmpl is None


def test_decision_sections():
    
    tmpl = lookup_template("decision", NEURON_TEMPLATES)
    assert len(tmpl["sections"]) == 5


def test_incident_sections():
    
    tmpl = lookup_template("incident", NEURON_TEMPLATES)
    assert len(tmpl["sections"]) == 6


def test_runbook_sections():
    
    tmpl = lookup_template("runbook", NEURON_TEMPLATES)
    assert len(tmpl["sections"]) == 5


def test_generate_with_template():
    
    tmpl = NEURON_TEMPLATES["decision"]
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
    
    tmpl = NEURON_TEMPLATES["decision"]
    content = generate_neuron_content("Test", "./map.md", template_name="decision", sections=tmpl["sections"])
    assert "template: decision" in content
