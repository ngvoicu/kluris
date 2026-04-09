"""Tests for agent registry and skill rendering."""

from kluris.core.agents import AGENT_REGISTRY, render_commands, render_skill


def _render(skill_name="kluris", brain_name="test-brain", brain_path="/tmp/test-brain",
            has_git=False, brain_description="Test brain"):
    """Convenience wrapper around render_skill with sensible defaults."""
    return render_skill(
        skill_name=skill_name,
        brain_name=brain_name,
        brain_path=brain_path,
        has_git=has_git,
        brain_description=brain_description,
    )


def _install(tmp_path, agent_name="claude", skill_name="kluris", brain_name="test-brain",
             brain_path="/tmp/test-brain", has_git=False, brain_description="Test brain"):
    """Convenience wrapper around render_commands with sensible defaults."""
    return render_commands(
        agent_name,
        tmp_path,
        skill_name=skill_name,
        brain_name=brain_name,
        brain_path=brain_path,
        has_git=has_git,
        brain_description=brain_description,
    )


def test_registry_8_agents():
    assert len(AGENT_REGISTRY) == 8


def test_all_agents_use_skills():
    """All agents now use skills/ subdirectory with SKILL.md format."""
    for name, reg in AGENT_REGISTRY.items():
        assert reg["subdir"] == "skills", f"{name} should use skills/"


def test_render_skill_has_frontmatter():
    content = _render()
    assert "---" in content
    assert "name: kluris" in content
    assert "description:" in content


def test_render_skill_has_brain_header():
    """Skill body bakes in the brain name and path."""
    content = _render(brain_name="test-brain", brain_path="/tmp/test-brain")
    assert "Brain: test-brain" in content
    assert "/tmp/test-brain" in content


def test_render_creates_skill_md(tmp_path):
    files = _install(tmp_path)
    assert len(files) == 1
    assert files[0].name == "SKILL.md"
    assert files[0].parent.name == "kluris"


def test_render_skill_content(tmp_path):
    files = _install(tmp_path, brain_name="test-brain", brain_path="/tmp/test-brain")
    content = files[0].read_text()
    assert "name: kluris" in content
    assert "test-brain" in content
    assert "How the brain is structured" in content
    assert "Intent detection" in content


def test_render_same_format_all_agents(tmp_path):
    """All agents get the same SKILL.md format."""
    for agent_name in AGENT_REGISTRY:
        agent_dir = tmp_path / agent_name
        files = _install(agent_dir, agent_name=agent_name)
        assert len(files) == 1
        assert files[0].name == "SKILL.md"
        content = files[0].read_text()
        assert "name: kluris" in content


def test_skill_has_query_first_protocol():
    """Skill must instruct the agent to query the brain before answering."""
    content = _render()
    assert "Query first" in content
    assert "Never guess" in content or "never guess" in content


def test_skill_has_no_brain_selection_section():
    """Multi-brain selection rules are gone -- each skill is bound to one brain."""
    content = _render()
    assert "Brain selection" not in content
    # The natural-language picker rules from the old body must not appear:
    assert "names a brain" not in content


def test_skill_tells_agent_to_run_wake_up():
    """Skill must tell the agent to bootstrap with kluris wake-up at session start."""
    content = _render()
    assert "kluris wake-up" in content


def test_skill_bootstrap_instruction_is_deterministic():
    """Bootstrap instruction must be deterministic, not ambiguous."""
    content = _render()
    assert "Bootstrap" in content
    assert "first" in content.lower() and "/kluris" in content
    # Cache guidance: agent should reuse wake-up output, not re-run every turn
    assert "cache" in content.lower() or "trust" in content.lower()


def test_skill_bootstrap_lists_refresh_triggers():
    """Skill tells the agent when to re-run wake-up after the brain changes."""
    content = _render()
    lowered = content.lower()
    assert "re-run" in lowered or "refresh" in lowered
    triggers = ["neuron", "lobe", "dream", "push", "remember", "learn"]
    hits = sum(1 for t in triggers if t in lowered)
    assert hits >= 2


def test_render_skill_per_brain_has_brain_flag_hint():
    """A kluris-<X> skill must instruct the agent to pass --brain X on every CLI call."""
    content = _render(skill_name="kluris-foo", brain_name="foo")
    assert "--brain foo" in content
    assert "kluris-foo" in content
    # The frontmatter name must match the skill_name
    assert "name: kluris-foo" in content


def test_render_skill_single_brain_no_brain_flag_hint():
    """The kluris (single-brain) skill must NOT mention --brain anywhere."""
    content = _render(skill_name="kluris", brain_name="foo")
    assert "--brain" not in content
    # The placeholder must be substituted to empty, not left as-is
    assert "{brain_flag_hint}" not in content
    assert "{brain_flag_hint_inline}" not in content


def test_render_skill_isolation():
    """Rendering two skills back to back must not mention the other brain."""
    a = _render(skill_name="kluris-brain-a", brain_name="brain-a", brain_path="/tmp/a")
    b = _render(skill_name="kluris-brain-b", brain_name="brain-b", brain_path="/tmp/b")
    assert "brain-a" in a
    assert "brain-b" not in a
    assert "brain-b" in b
    assert "brain-a" not in b


def test_render_skill_name_matches_dir(tmp_path):
    """The frontmatter `name:` field must match the directory name created by render_commands."""
    files = _install(tmp_path, skill_name="kluris-foo", brain_name="foo")
    assert files[0].parent.name == "kluris-foo"
    content = files[0].read_text()
    assert "name: kluris-foo" in content


def test_render_skill_substitutes_all_placeholders():
    """No raw `{...}` placeholders should leak into the rendered output."""
    content = _render(skill_name="kluris", brain_name="foo", brain_path="/tmp/foo",
                      has_git=True, brain_description="A test brain")
    placeholders = ["{skill_name}", "{brain_name}", "{brain_path}", "{git_label}",
                    "{brain_description}", "{brain_flag_hint}", "{brain_flag_hint_inline}"]
    for ph in placeholders:
        assert ph not in content, f"{ph} was not substituted"


def test_render_skill_git_label():
    """has_git toggles between 'git' and 'no git' in the body."""
    git_content = _render(has_git=True)
    no_git_content = _render(has_git=False)
    assert "(git)" in git_content
    assert "(no git)" in no_git_content


def test_render_skill_windows_path_is_posix_form():
    """Windows paths must be baked into SKILL.md as forward-slash POSIX form
    so bash on Windows (Git Bash / WSL) handles them correctly. A raw
    ``C:\\Users\\...`` path makes bash interpret ``\\U`` etc as escapes,
    producing ``C:Users...`` and a 'No such file or directory' error."""
    content = _render(brain_path="C:\\Users\\Gabriel_Voicu\\Projects\\brain")
    # The path should appear in forward-slash form, preserving the drive letter
    assert "C:/Users/Gabriel_Voicu/Projects/brain" in content
    # The raw Windows path form must NOT appear anywhere in the rendered body
    assert "C:\\Users\\Gabriel_Voicu" not in content
    assert "C:\\Users" not in content


def test_render_skill_posix_path_passthrough():
    """POSIX paths pass through unchanged (no mangling on macOS/Linux)."""
    content = _render(brain_path="/Users/gv/Projects/brain")
    assert "/Users/gv/Projects/brain" in content
    # No accidental double slashes or drive letters inserted
    assert "//" not in content.replace("://", "")


# --- yaml-neurons SKILL.md template tests ---


def test_skill_body_mentions_yaml_neurons():
    """Rendered SKILL.md must include a yaml-neurons section telling agents
    how to write opted-in yaml files.
    """
    content = _render()
    lower = content.lower()
    assert "yaml neuron" in lower
    assert "openapi.yml" in content
    assert "#---" in content
    assert "frontmatter" in lower or "hash" in lower


def test_skill_body_yaml_template_has_frontmatter_fields():
    """The yaml template in SKILL.md must show the expected frontmatter
    fields and a minimal OpenAPI skeleton.
    """
    content = _render()
    assert "parent:" in content
    assert "related:" in content
    assert "tags:" in content
    assert "title:" in content
    assert "updated:" in content
    assert "openapi: 3.1.0" in content
    assert "info:" in content
