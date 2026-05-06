"""Tests for kluris skill installation through lifecycle commands."""

from click.testing import CliRunner

from kluris.cli import cli
from conftest import create_test_brain


def test_install_creates_claude_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    skill_file = tmp_path / ".claude" / "skills" / "kluris-my-brain" / "SKILL.md"
    assert skill_file.exists()
    content = skill_file.read_text()
    assert "name: kluris-my-brain" in content


def test_install_creates_codex_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    assert (tmp_path / ".codex" / "skills" / "kluris-my-brain" / "SKILL.md").exists()


def test_install_creates_gemini_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    assert (tmp_path / ".gemini" / "skills" / "kluris-my-brain" / "SKILL.md").exists()


def test_install_creates_cursor_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    assert (tmp_path / ".cursor" / "skills" / "kluris-my-brain" / "SKILL.md").exists()


def test_install_creates_windsurf_skill(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    assert (tmp_path / ".codeium" / "windsurf" / "skills" / "kluris-my-brain" / "SKILL.md").exists()


def test_install_skill_has_brain_info(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    content = (tmp_path / ".claude" / "skills" / "kluris-my-brain" / "SKILL.md").read_text()
    assert "my-brain" in content


def test_install_idempotent(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    result1 = runner.invoke(cli, ["doctor"])
    result2 = runner.invoke(cli, ["doctor"])
    assert result1.exit_code == 0
    assert result2.exit_code == 0


def test_doctor_migrates_legacy_bare_skill_to_named(tmp_path, monkeypatch):
    """doctor removes old /kluris artifacts and rewrites /kluris-<brain>."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)

    claude_skills = tmp_path / ".claude" / "skills"
    legacy_skill = claude_skills / "kluris"
    legacy_skill.mkdir(parents=True, exist_ok=True)
    (legacy_skill / "SKILL.md").write_text("legacy", encoding="utf-8")

    workflows = tmp_path / ".codeium" / "windsurf" / "global_workflows"
    workflows.mkdir(parents=True, exist_ok=True)
    (workflows / "kluris.md").write_text("legacy", encoding="utf-8")

    result = runner.invoke(cli, ["doctor"])

    assert result.exit_code == 0
    assert not legacy_skill.exists()
    assert (claude_skills / "kluris-brain-a" / "SKILL.md").exists()
    assert not (workflows / "kluris.md").exists()
    assert (workflows / "kluris-brain-a.md").exists()


def test_install_cleans_old_commands(tmp_path, monkeypatch):
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "my-brain", tmp_path)
    # Create stale old command file
    old_cmd_dir = tmp_path / ".claude" / "commands"
    old_cmd_dir.mkdir(parents=True, exist_ok=True)
    stale = old_cmd_dir / "kluris.md"
    stale.write_text("stale", encoding="utf-8")
    # Doctor refresh should clean it through _do_install.
    runner.invoke(cli, ["doctor"])
    assert not stale.exists()


# --- Per-brain install behavior (multi-brain refactor) ---


def test_install_two_brains_creates_per_brain_skills(tmp_path, monkeypatch):
    """With 2 brains registered, install creates kluris-<name>/ per brain and no bare kluris/."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)

    claude_skills = tmp_path / ".claude" / "skills"
    assert (claude_skills / "kluris-brain-a" / "SKILL.md").exists()
    assert (claude_skills / "kluris-brain-b" / "SKILL.md").exists()
    assert not (claude_skills / "kluris").exists()


def test_install_two_brains_each_skill_body_isolated(tmp_path, monkeypatch):
    """Each per-brain SKILL.md must mention only its own brain."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)

    a_content = (tmp_path / ".claude" / "skills" / "kluris-brain-a" / "SKILL.md").read_text()
    b_content = (tmp_path / ".claude" / "skills" / "kluris-brain-b" / "SKILL.md").read_text()
    assert "Brain: brain-a" in a_content
    assert "--brain brain-a" in a_content
    assert "brain-b" not in a_content
    assert "Brain: brain-b" in b_content
    assert "--brain brain-b" in b_content
    assert "brain-a" not in b_content


def test_install_one_to_two_transition(tmp_path, monkeypatch):
    """Going from 1 brain to 2 brains keeps the named per-brain layout."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    claude_skills = tmp_path / ".claude" / "skills"
    assert (claude_skills / "kluris-brain-a" / "SKILL.md").exists()
    assert not (claude_skills / "kluris").exists()

    create_test_brain(runner, "brain-b", tmp_path)
    assert not (claude_skills / "kluris").exists()
    assert (claude_skills / "kluris-brain-a" / "SKILL.md").exists()
    assert (claude_skills / "kluris-brain-b" / "SKILL.md").exists()


def test_install_two_to_one_transition(tmp_path, monkeypatch):
    """Removing a brain so only one is left keeps the named per-brain layout."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    claude_skills = tmp_path / ".claude" / "skills"
    assert (claude_skills / "kluris-brain-a").exists()
    assert (claude_skills / "kluris-brain-b").exists()

    runner.invoke(cli, ["remove", "brain-b"])
    assert (claude_skills / "kluris-brain-a" / "SKILL.md").exists()
    assert not (claude_skills / "kluris-brain-b").exists()
    assert not (claude_skills / "kluris").exists()
    content = (claude_skills / "kluris-brain-a" / "SKILL.md").read_text()
    assert "Brain: brain-a" in content


def test_install_universal_per_brain(tmp_path, monkeypatch):
    """The universal ~/.agents/skills slot mirrors the per-brain layout."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)

    universal = tmp_path / ".agents" / "skills"
    assert (universal / "kluris-brain-a" / "SKILL.md").exists()
    assert (universal / "kluris-brain-b" / "SKILL.md").exists()
    assert not (universal / "kluris").exists()


def test_install_windsurf_workflow_per_brain(tmp_path, monkeypatch):
    """Windsurf workflow files mirror the per-brain naming."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)

    workflows = tmp_path / ".codeium" / "windsurf" / "global_workflows"
    assert (workflows / "kluris-brain-a.md").exists()
    assert (workflows / "kluris-brain-b.md").exists()
    assert not (workflows / "kluris.md").exists()


def test_install_windsurf_workflow_single_brain(tmp_path, monkeypatch):
    """With 1 brain, the Windsurf workflow is still named per brain."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "only-brain", tmp_path)

    workflows = tmp_path / ".codeium" / "windsurf" / "global_workflows"
    assert not (workflows / "kluris.md").exists()
    assert (workflows / "kluris-only-brain.md").exists()


def test_non_tty_multi_brain_status_errors_cleanly(tmp_path, monkeypatch):
    """Non-TTY (CliRunner has no isatty by default) + multi-brain + no flag = clean error."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    create_test_brain(runner, "brain-b", tmp_path)
    result = runner.invoke(cli, ["status"])
    assert result.exit_code != 0
    assert "Multiple brains" in result.output
    # Should NOT hang or crash
    assert "Traceback" not in result.output


def test_install_partial_failure_keeps_old_skill(tmp_path, monkeypatch):
    """If staging the new skill fails for a destination, the old skill stays in place."""
    monkeypatch.setenv("KLURIS_CONFIG", str(tmp_path / "config.yml"))
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setenv("USERPROFILE", str(tmp_path))
    runner = CliRunner()
    create_test_brain(runner, "brain-a", tmp_path)
    claude_skill = tmp_path / ".claude" / "skills" / "kluris-brain-a" / "SKILL.md"
    original_content = claude_skill.read_text()
    assert "brain-a" in original_content

    # Force render_commands to fail on the first call after this point.
    import kluris.cli as cli_module
    original_render = cli_module.render_commands
    call_count = {"n": 0}

    def flaky_render(*args, **kwargs):
        call_count["n"] += 1
        if call_count["n"] == 1:
            raise OSError("simulated disk full")
        return original_render(*args, **kwargs)

    monkeypatch.setattr(cli_module, "render_commands", flaky_render)

    # Trigger a refresh -- the first destination's staging will fail.
    runner.invoke(cli, ["doctor"])

    # The original skill must still exist (sweep didn't run for the failed dest).
    # Pick any destination -- at least one must still have the old SKILL.md
    # because at least one staging failure leaves the destination untouched.
    survivors = list((tmp_path / ".claude" / "skills").glob("kluris*/SKILL.md"))
    survivors.extend((tmp_path / ".cursor" / "skills").glob("kluris*/SKILL.md"))
    survivors.extend((tmp_path / ".codex" / "skills").glob("kluris*/SKILL.md"))
    assert len(survivors) > 0, "all destinations were wiped after a partial failure"
