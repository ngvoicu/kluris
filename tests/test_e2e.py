"""End-to-end workflow tests."""

import json
from click.testing import CliRunner
from kluris.cli import cli
from kluris.core.config import read_global_config


def test_full_workflow(tmp_path, monkeypatch):
    """Create brain -> add neurons -> dream -> verify -> mri -> push."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()

    # 1. Create brain
    result = runner.invoke(cli, ["create", "my-brain", "--path", str(tmp_path)])
    assert result.exit_code == 0

    brain = tmp_path / "my-brain"

    # 2. Add neurons
    runner.invoke(cli, ["neuron", "auth.md", "--lobe", "architecture"])
    runner.invoke(cli, ["neuron", "naming.md", "--lobe", "standards"])
    runner.invoke(cli, ["neuron", "deploy.md", "--lobe", "infrastructure"])

    # 3. Add neuron with decision template
    result = runner.invoke(cli, ["neuron", "use-raw-sql.md", "--lobe", "decisions", "--template", "decision"])
    assert result.exit_code == 0
    content = (brain / "decisions" / "use-raw-sql.md").read_text()
    assert "## Context" in content

    # 4. Dream — regenerate maps and validate
    result = runner.invoke(cli, ["dream"])
    assert result.exit_code == 0

    # 5. Verify maps list neurons
    arch_map = (brain / "architecture" / "map.md").read_text()
    assert "auth.md" in arch_map

    # 6. Verify brain.md has lobes (neurons are in map.md, not brain.md)
    brain_content = (brain / "brain.md").read_text()
    assert "architecture" in brain_content

    # 7. MRI
    result = runner.invoke(cli, ["mri"])
    assert (brain / "brain-mri.html").exists()

    # 8. Push (local only, no remote)
    result = runner.invoke(cli, ["push", "-m", "add neurons"])
    # Should succeed even without remote (commits locally)
    assert result.exit_code == 0


def test_multi_brain(tmp_path, monkeypatch):
    """Create 2 brains -> list -> remove 1 -> list."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    runner = CliRunner()

    runner.invoke(cli, ["create", "brain-a", "--path", str(tmp_path)])
    runner.invoke(cli, ["create", "brain-b", "--path", str(tmp_path)])

    result = runner.invoke(cli, ["list", "--json"])
    data = json.loads(result.output)
    assert len(data["brains"]) == 2

    runner.invoke(cli, ["remove", "brain-a"])

    result = runner.invoke(cli, ["list", "--json"])
    data = json.loads(result.output)
    assert len(data["brains"]) == 1
    assert data["brains"][0]["name"] == "brain-b"
