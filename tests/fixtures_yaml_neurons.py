"""Shared complex-brain fixture factories for the yaml-neurons spec's
Phase 7 integration tests.

Three deterministic brain factories:

- `_make_mixed_brain(tmp_path)` — 10 lobes, edge cases (empty lobes,
  yaml-only lobes, deprecated-to-yaml replacements, cross-lobe synapses).

- `_make_large_brain(tmp_path)` — 15 lobes, 6 sublobes, ~60 md neurons,
  ~15 yaml neurons, cross-lobe synapses, glossary with 20 terms. Used for
  scale tests, MRI size gates, and wake-up aggregation assertions.

- `_make_microservices_brain(tmp_path)` — 12 service lobes, each with
  `map.md`, 2 md neurons, and exactly one `openapi.yml`. Simulates a
  real monorepo where every service has its own API spec.

Each factory returns the brain `Path`. All three are intentional
deviations from the per-file fixture rule (see SPEC.md Decision Log)
because duplicating 200-line factories across 5+ Phase 7 test files
would be prohibitive.

CRITICAL INVARIANT: every fixture includes a `kluris.yml` at brain root.
Tests using these fixtures must verify that the scanner never indexes it.
"""

from __future__ import annotations

from pathlib import Path


def _write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _md(parent: str = "./map.md", related=(), tags=(), created: str = "2026-04-01",
        updated: str = "2026-04-01", title: str = "Note", body: str = "body text",
        extra_fm: str = "") -> str:
    """Build a markdown neuron file body."""
    rel_line = ""
    if related:
        rel_line = "related: [" + ", ".join(related) + "]\n"
    tags_line = ""
    if tags:
        tags_line = "tags: [" + ", ".join(tags) + "]\n"
    return (
        f"---\n"
        f"parent: {parent}\n"
        f"{rel_line}"
        f"{tags_line}"
        f"created: {created}\n"
        f"updated: {updated}\n"
        f"{extra_fm}"
        f"---\n\n"
        f"# {title}\n\n{body}\n"
    )


def _yaml_neuron(title: str = "API", related=(), tags=("api",),
                 parent: str = "./map.md", updated: str = "2026-04-01") -> str:
    """Build an opted-in yaml neuron body (OpenAPI 3.1 skeleton + #--- block)."""
    rel_line = ""
    if related:
        rel_line = "# related: [" + ", ".join(related) + "]\n"
    tags_line = ""
    if tags:
        tags_line = "# tags: [" + ", ".join(tags) + "]\n"
    return (
        "#---\n"
        f"# parent: {parent}\n"
        f"{rel_line}"
        f"{tags_line}"
        f"# title: {title}\n"
        f"# updated: {updated}\n"
        "#---\n"
        "openapi: 3.1.0\n"
        f"info:\n  title: {title}\n  version: 1.0.0\n"
        "paths:\n  /ping: {}\n"
    )


def _lobe_map(title: str) -> str:
    return (
        "---\nauto_generated: true\nparent: ../brain.md\n"
        "updated: 2026-04-01\n---\n"
        f"# {title}\n"
    )


def _sublobe_map(title: str) -> str:
    return (
        "---\nauto_generated: true\nparent: ../map.md\n"
        "updated: 2026-04-01\n---\n"
        f"# {title}\n"
    )


# --- Mixed brain: 10 lobes, edge cases --------------------------------------


MIXED_BRAIN_LOBES = [
    "architecture", "product", "standards", "projects", "runbooks",
    "decisions", "integrations", "playbooks", "api-contracts", "data",
]

# Lobes with at least one md neuron (the rest stay empty as edge cases).
MIXED_BRAIN_MD_NEURONS = {
    "architecture": ["north-star.md", "tradeoffs.md"],
    "product": ["personas.md"],
    "standards": ["naming.md", "testing.md"],
    "projects": ["alpha.md"],
    "runbooks": ["restart.md"],
    "integrations": ["stripe.md"],
    "data": ["schema-evolution.md"],
}
MIXED_BRAIN_YAML_NEURONS = {
    "api-contracts": ["public-api.yml"],
    "integrations": ["stripe-api.yml"],
    "projects": ["alpha-api.yml"],
    "runbooks": ["restart-contract.yml"],
}
MIXED_BRAIN_RAW_YAML = {
    "architecture": "ci.yml",   # no #--- block, must be invisible
    "product": "feature-flags.yml",
}
# Empty lobes (map.md only)
MIXED_BRAIN_EMPTY_LOBES = {"decisions", "playbooks"}


def _make_mixed_brain(tmp_path: Path) -> Path:
    brain = tmp_path / "mixed-brain"
    brain.mkdir()

    _write(brain / "brain.md",
           "---\nauto_generated: true\n---\n# Mixed Brain\n\n" +
           "\n".join(f"- [{l}](./{l}/map.md)" for l in MIXED_BRAIN_LOBES) + "\n")
    _write(brain / "glossary.md",
           "---\n---\n# Glossary\n\n"
           "**OAuth** -- Open Authorization protocol.\n"
           "**Idempotency** -- Same effect on repeat.\n"
           "**SLO** -- Service-level objective.\n")
    # CRITICAL: kluris.yml at root must never be indexed.
    _write(brain / "kluris.yml", "name: mixed-brain\ntype: product\n")

    for lobe in MIXED_BRAIN_LOBES:
        _write(brain / lobe / "map.md", _lobe_map(lobe.replace("-", " ").title()))
        for md_name in MIXED_BRAIN_MD_NEURONS.get(lobe, []):
            _write(brain / lobe / md_name, _md(title=md_name.replace(".md", "").title(),
                                                tags=(lobe,)))
        for yaml_name in MIXED_BRAIN_YAML_NEURONS.get(lobe, []):
            title = yaml_name.replace(".yml", "").replace("-", " ").title()
            _write(brain / lobe / yaml_name, _yaml_neuron(title=title, tags=("api", lobe)))
        if lobe in MIXED_BRAIN_RAW_YAML:
            _write(brain / lobe / MIXED_BRAIN_RAW_YAML[lobe],
                   "# raw yaml, no kluris block\nname: raw\nvalue: 42\n")

    # Add 3 deprecated md neurons pointing at yaml replacements in other lobes.
    deprecated_cases = [
        ("architecture", "legacy-arch.md", "../api-contracts/public-api.yml"),
        ("integrations", "old-stripe.md", "./stripe-api.yml"),
        ("runbooks", "old-restart.md", "./restart-contract.yml"),
    ]
    for lobe, name, replaced_by in deprecated_cases:
        _write(brain / lobe / name,
               _md(title="Legacy", tags=("deprecated",),
                   extra_fm=f"status: deprecated\nreplaced_by: {replaced_by}\n"))

    return brain


# --- Large brain: 15 lobes, 6 sublobes, scale ------------------------------


LARGE_BRAIN_LOBES = [
    "api", "domain", "infrastructure", "security", "data",
    "ops", "release", "observability", "compliance", "docs",
    "policies", "integrations", "schemas", "contracts", "decisions",
]

# Lobes that contain a sublobe structure
LARGE_BRAIN_SUBLOBES = {
    "api": ["v1", "v2"],
    "domain": ["payments", "orders"],
    "infrastructure": ["cloud"],
    "security": ["auth"],
}

# Number of md neurons per lobe (spread 60 across 15 lobes, avg 4)
LARGE_BRAIN_MD_COUNT = {
    "api": 3, "domain": 5, "infrastructure": 5, "security": 5, "data": 4,
    "ops": 4, "release": 3, "observability": 4, "compliance": 3, "docs": 4,
    "policies": 3, "integrations": 5, "schemas": 3, "contracts": 4, "decisions": 5,
}

# Exact yaml neuron placements across 10 lobes (15 total yaml neurons)
LARGE_BRAIN_YAML_NEURONS = [
    ("api", "openapi.yml", "API Spec"),
    ("api/v1", "api-v1.yml", "API v1"),
    ("api/v2", "api-v2.yml", "API v2"),
    ("domain/payments", "payments-api.yml", "Payments API"),
    ("domain/orders", "orders-api.yml", "Orders API"),
    ("infrastructure", "terraform.yml", "Terraform Schema"),
    ("infrastructure/cloud", "cloud-provider.yml", "Cloud Provider Contract"),
    ("security", "security-policy.yml", "Security Policy"),
    ("security/auth", "auth-api.yml", "Auth API"),
    ("data", "db-schema.yml", "DB Schema"),
    ("release", "release-contract.yml", "Release Contract"),
    ("observability", "metrics-schema.yml", "Metrics Schema"),
    ("integrations", "webhook-contract.yml", "Webhook Contract"),
    ("schemas", "json-schema.yml", "JSON Schema"),
    ("contracts", "service-contract.yml", "Service Contract"),
]

# Three raw yaml files scattered in lobes (must be invisible)
LARGE_BRAIN_RAW_YAML = [
    ("ops", "prometheus.yml"),
    ("docs", "mkdocs.yml"),
    ("release", "github-workflow.yml"),
]


def _make_large_brain(tmp_path: Path) -> Path:
    brain = tmp_path / "large-brain"
    brain.mkdir()

    # brain.md + glossary.md + kluris.yml at root
    _write(brain / "brain.md",
           "---\nauto_generated: true\n---\n# Large Brain\n\n" +
           "\n".join(f"- [{l}](./{l}/map.md)" for l in LARGE_BRAIN_LOBES) + "\n")
    glossary_body = "# Glossary\n\n" + "\n".join(
        f"**Term{i}** -- Definition number {i}." for i in range(20)
    ) + "\n"
    _write(brain / "glossary.md", f"---\n---\n{glossary_body}")
    _write(brain / "kluris.yml", "name: large-brain\ntype: product\n")

    # Lobes + their map.md
    for lobe in LARGE_BRAIN_LOBES:
        _write(brain / lobe / "map.md", _lobe_map(lobe.title()))

    # Sublobes + their map.md
    for parent_lobe, sub_names in LARGE_BRAIN_SUBLOBES.items():
        for sub in sub_names:
            _write(brain / parent_lobe / sub / "map.md", _sublobe_map(sub.title()))

    # Md neurons spread across lobes (deterministic names)
    total_md = 0
    for lobe, count in LARGE_BRAIN_MD_COUNT.items():
        for i in range(count):
            name = f"topic-{i + 1}.md"
            _write(brain / lobe / name,
                   _md(title=f"{lobe.title()} topic {i + 1}",
                       tags=(lobe, f"topic-{i + 1}"),
                       body=f"Notes about {lobe} topic {i + 1}."))
            total_md += 1

    # Add 3-5 neurons per sublobe
    sublobe_md_count = 0
    for parent_lobe, sub_names in LARGE_BRAIN_SUBLOBES.items():
        for sub in sub_names:
            for i in range(3):
                name = f"sub-{i + 1}.md"
                _write(brain / parent_lobe / sub / name,
                       _md(title=f"{sub.title()} topic {i + 1}",
                           tags=(sub,),
                           body=f"Nested notes in {parent_lobe}/{sub}."))
                sublobe_md_count += 1

    # Yaml neurons (opted-in)
    for lobe_path, filename, title in LARGE_BRAIN_YAML_NEURONS:
        _write(brain / lobe_path / filename,
               _yaml_neuron(title=title, tags=("api", lobe_path.split("/")[0])))

    # Raw yaml files (opt-out, must be invisible)
    for lobe, filename in LARGE_BRAIN_RAW_YAML:
        _write(brain / lobe / filename,
               "# raw yaml, not a neuron\nkey: value\nlist: [a, b, c]\n")

    # Add 5 deprecated md neurons pointing at yaml replacements
    deprecated_cases = [
        ("api", "old-api.md", "./openapi.yml"),
        ("domain/payments", "old-payments.md", "./payments-api.yml"),
        ("security", "old-policy.md", "./security-policy.yml"),
        ("data", "old-schema.md", "./db-schema.yml"),
        ("contracts", "old-contract.md", "./service-contract.yml"),
    ]
    for lobe, name, replaced_by in deprecated_cases:
        _write(brain / lobe / name,
               _md(title="Legacy", tags=("deprecated",),
                   extra_fm=f"status: deprecated\nreplaced_by: {replaced_by}\n"))

    return brain


def large_brain_expected_counts() -> dict:
    """Deterministic counts for assertions."""
    sublobe_count = sum(len(subs) for subs in LARGE_BRAIN_SUBLOBES.values())
    sublobe_md = sublobe_count * 3  # 3 neurons per sublobe
    lobe_md = sum(LARGE_BRAIN_MD_COUNT.values())
    deprecated_count = 5
    yaml_count = len(LARGE_BRAIN_YAML_NEURONS)
    return {
        "lobes": len(LARGE_BRAIN_LOBES),
        "sublobes": sublobe_count,
        "md_neurons": lobe_md + sublobe_md + deprecated_count,
        "yaml_neurons": yaml_count,
        "raw_yaml_excluded": len(LARGE_BRAIN_RAW_YAML),
    }


# --- Microservices brain: 12 services, each with openapi.yml ---------------


MICROSERVICES_BRAIN_SERVICES = [
    "payments", "orders", "inventory", "shipping", "users", "auth",
    "catalog", "recommendations", "notifications", "billing", "search", "reviews",
]


def _make_microservices_brain(tmp_path: Path) -> Path:
    brain = tmp_path / "microservices-brain"
    brain.mkdir()

    _write(brain / "brain.md",
           "---\nauto_generated: true\n---\n# Microservices Brain\n\n" +
           "\n".join(f"- [{s}](./{s}/map.md)" for s in MICROSERVICES_BRAIN_SERVICES) + "\n")
    _write(brain / "glossary.md",
           "---\n---\n# Glossary\n\n"
           "**API** -- Application Programming Interface.\n"
           "**OpenAPI** -- API spec standard, v3.1.\n")
    _write(brain / "kluris.yml", "name: microservices-brain\ntype: product\n")

    for svc in MICROSERVICES_BRAIN_SERVICES:
        _write(brain / svc / "map.md", _lobe_map(svc.title()))
        _write(brain / svc / "README.md", f"# {svc} service (skipped)\n")
        # 2 md neurons per service
        _write(brain / svc / "architecture.md",
               _md(title=f"{svc.title()} architecture",
                   tags=("architecture", svc),
                   body=f"Architecture notes for {svc}."))
        _write(brain / svc / "runbook.md",
               _md(title=f"{svc.title()} runbook",
                   tags=("runbook", svc),
                   body=f"Runbook for {svc}."))
        # openapi.yml per service, all tagged with "api"
        _write(brain / svc / "openapi.yml",
               _yaml_neuron(title=f"{svc.title()} API",
                             tags=("api", svc)))

    return brain
