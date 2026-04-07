"""Tests for `kluris wake-up` -- compact brain snapshot for agent bootstrap."""

import json

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def _write_neuron(brain_path, rel_path, title, updated="2026-04-01", extra_fm=""):
    """Write a neuron file with frontmatter into the given brain."""
    target = brain_path / rel_path
    target.parent.mkdir(parents=True, exist_ok=True)
    content = (
        f"---\n"
        f"parent: ./map.md\n"
        f"created: 2026-01-01\n"
        f"updated: {updated}\n"
        f"{extra_fm}"
        f"---\n\n"
        f"# {title}\n\nbody\n"
    )
    target.write_text(content, encoding="utf-8")


def test_wake_up_single_brain_text(temp_brain, cli_runner):
    """Default brain wakes up with name, path, lobe list."""
    _write_neuron(temp_brain, "knowledge/use-raw-sql.md", "Use Raw SQL")
    result = cli_runner.invoke(cli, ["wake-up"])
    assert result.exit_code == 0
    assert "test-brain" in result.output
    assert "knowledge" in result.output
    assert "projects" in result.output


def test_wake_up_json_schema(temp_brain, cli_runner):
    """--json output carries ok, name, path, lobes, neurons, recent."""
    _write_neuron(temp_brain, "knowledge/use-raw-sql.md", "Use Raw SQL")
    _write_neuron(temp_brain, "projects/btb/auth.md", "Auth")

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)

    assert data["ok"] is True
    assert data["name"] == "test-brain"
    assert data["is_default"] is True
    assert str(temp_brain) == data["path"]

    # Lobes: list of {name, neurons} dicts
    assert isinstance(data["lobes"], list)
    lobe_names = {lobe["name"] for lobe in data["lobes"]}
    assert {"projects", "infrastructure", "knowledge"} <= lobe_names

    # Neuron count reflects the two we wrote
    assert data["total_neurons"] == 2

    # Recent list: the two neurons we wrote, newest first
    assert isinstance(data["recent"], list)
    assert len(data["recent"]) == 2


def test_wake_up_recent_sorted_newest_first(temp_brain, cli_runner):
    """Recent list sorts by frontmatter `updated` descending."""
    _write_neuron(temp_brain, "knowledge/older.md", "Older", updated="2026-01-01")
    _write_neuron(temp_brain, "knowledge/newer.md", "Newer", updated="2026-04-07")
    _write_neuron(temp_brain, "knowledge/middle.md", "Middle", updated="2026-03-15")

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    recent = data["recent"]
    assert recent[0]["path"] == "knowledge/newer.md"
    assert recent[1]["path"] == "knowledge/middle.md"
    assert recent[2]["path"] == "knowledge/older.md"


def test_wake_up_recent_limited_to_five(temp_brain, cli_runner):
    """Recent list caps at 5 entries so wake-up stays compact."""
    for i in range(10):
        _write_neuron(
            temp_brain,
            f"knowledge/neuron-{i:02d}.md",
            f"Neuron {i}",
            updated=f"2026-04-{(i + 1):02d}",
        )

    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    data = json.loads(result.output)
    assert len(data["recent"]) == 5
    # Newest (day 10) should be first
    assert data["recent"][0]["path"] == "knowledge/neuron-09.md"


def test_wake_up_targets_named_brain(tmp_path, temp_config, cli_runner, monkeypatch):
    """`kluris wake-up --brain NAME` targets a non-default brain."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    create_test_brain(cli_runner, "brain-a", tmp_path)
    create_test_brain(cli_runner, "brain-b", tmp_path)

    result = cli_runner.invoke(cli, ["wake-up", "--brain", "brain-b", "--json"])
    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["name"] == "brain-b"
    assert data["is_default"] is False


def test_wake_up_unknown_brain_errors(temp_brain, cli_runner):
    """Unknown brain name exits non-zero with clear error."""
    result = cli_runner.invoke(cli, ["wake-up", "--brain", "nonexistent", "--json"])
    assert result.exit_code != 0
    data = json.loads(result.output)
    assert data["ok"] is False
    assert "nonexistent" in data["error"]


def test_wake_up_no_brains_errors(temp_config, cli_runner):
    """No brains registered -> error."""
    result = cli_runner.invoke(cli, ["wake-up", "--json"])
    assert result.exit_code != 0
    data = json.loads(result.output)
    assert data["ok"] is False


def test_wake_up_text_includes_recent(temp_brain, cli_runner):
    """Text output mentions at least one recent neuron path."""
    _write_neuron(temp_brain, "knowledge/use-raw-sql.md", "Use Raw SQL", updated="2026-04-07")
    result = cli_runner.invoke(cli, ["wake-up"])
    assert result.exit_code == 0
    assert "use-raw-sql" in result.output
