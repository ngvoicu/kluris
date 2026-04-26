"""Delegation identity guard.

The architectural rule (spec: ``kluris_runtime delegation rule``):
``kluris_runtime/*`` is the single source of truth for read-only brain
behavior. Every symbol re-exported from ``kluris.core.*`` must be the
*same object* (``is``-equal) as the runtime version, never a parallel
copy.

If a future change accidentally redefines (rather than re-exports) one
of these symbols, this test fails — catching the silent divergence
before it reaches production.
"""

from __future__ import annotations

import kluris.core.frontmatter as core_frontmatter
import kluris.core.linker as core_linker
import kluris.core.neuron_excerpt as core_neuron_excerpt
import kluris.core.search as core_search
import kluris.core.wake_up as core_wake_up
import kluris_runtime.deprecation as rt_deprecation
import kluris_runtime.frontmatter as rt_frontmatter
import kluris_runtime.neuron_excerpt as rt_neuron_excerpt
import kluris_runtime.neuron_index as rt_neuron_index
import kluris_runtime.search as rt_search
import kluris_runtime.wake_up as rt_wake_up


def test_read_frontmatter_is_runtime_identity():
    assert core_frontmatter.read_frontmatter is rt_frontmatter.read_frontmatter


def test_yaml_suffixes_is_runtime_identity():
    assert core_frontmatter.YAML_SUFFIXES is rt_frontmatter.YAML_SUFFIXES


def test_linker_neuron_index_helpers_are_runtime_identity():
    assert core_linker._neuron_files is rt_neuron_index.neuron_files
    assert core_linker._all_neuron_files is rt_neuron_index.all_neuron_files
    assert core_linker._has_yaml_opt_in_block is rt_neuron_index.has_yaml_opt_in_block
    assert core_linker._is_within_brain is rt_neuron_index.is_within_brain
    assert core_linker.SKIP_DIRS is rt_neuron_index.SKIP_DIRS
    assert core_linker.SKIP_FILES is rt_neuron_index.SKIP_FILES
    assert core_linker.YAML_NEURON_SUFFIXES is rt_neuron_index.YAML_NEURON_SUFFIXES


def test_detect_deprecation_issues_is_runtime_identity():
    assert core_linker.detect_deprecation_issues is rt_deprecation.detect_deprecation_issues


def test_search_is_runtime_identity():
    assert core_search.search_brain is rt_search.search_brain
    assert core_search._collect_searchable is rt_search.collect_searchable
    assert core_search._parse_glossary_entries is rt_search.parse_glossary_entries
    assert core_search._score_hit is rt_search.score_hit
    assert core_search._matched_fields is rt_search.matched_fields
    assert core_search._extract_snippet is rt_search.extract_snippet


def test_wake_up_build_payload_is_runtime_identity():
    assert core_wake_up.build_payload is rt_wake_up.build_payload


def test_neuron_excerpt_extract_is_runtime_identity():
    assert core_neuron_excerpt.extract is rt_neuron_excerpt.extract
