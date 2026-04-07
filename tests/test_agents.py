"""Tests for agent registry and skill rendering."""

from kluris.core.agents import AGENT_REGISTRY, render_commands, render_skill


def test_registry_8_agents():
    assert len(AGENT_REGISTRY) == 8


def test_all_agents_use_skills():
    """All agents now use skills/ subdirectory with SKILL.md format."""
    for name, reg in AGENT_REGISTRY.items():
        assert reg["subdir"] == "skills", f"{name} should use skills/"


def test_render_skill_has_frontmatter():
    content = render_skill()
    assert "---" in content
    assert "name: kluris" in content
    assert "description:" in content


def test_render_skill_has_brain_info():
    brain_info = "## Your brains\n\n- **test**: `/tmp/test-brain`"
    content = render_skill(brain_info)
    assert "test-brain" in content


def test_render_creates_skill_md(tmp_path):
    files = render_commands("claude", tmp_path)
    assert len(files) == 1
    assert files[0].name == "SKILL.md"
    assert files[0].parent.name == "kluris"


def test_render_skill_content(tmp_path):
    brain_info = "## Your brains\n\n- **test**: `/tmp/test-brain`"
    files = render_commands("claude", tmp_path, brain_info=brain_info)
    content = files[0].read_text()
    assert "name: kluris" in content
    assert "test-brain" in content
    assert "How the brain is structured" in content
    assert "Intent detection" in content


def test_render_same_format_all_agents(tmp_path):
    """All agents get the same SKILL.md format."""
    for agent_name in AGENT_REGISTRY:
        agent_dir = tmp_path / agent_name
        files = render_commands(agent_name, agent_dir)
        assert len(files) == 1
        assert files[0].name == "SKILL.md"
        content = files[0].read_text()
        assert "name: kluris" in content


def test_skill_has_query_first_protocol():
    """Skill must instruct the agent to query the brain before answering."""
    content = render_skill()
    assert "Query first" in content
    assert "Never guess" in content


def test_skill_has_brain_selection_rules():
    """Skill must explain how to pick a brain when multiple are registered."""
    content = render_skill()
    assert "Brain selection" in content
    # Three-tier rule: exact name > path hint > default
    assert "names a brain" in content
    assert "current working directory" in content
    assert "(default)" in content


def test_skill_tells_agent_to_run_wake_up():
    """Skill must tell the agent to bootstrap with kluris wake-up at session start."""
    content = render_skill()
    assert "kluris wake-up" in content


def test_skill_bootstrap_instruction_is_deterministic():
    """Bootstrap instruction must be deterministic, not ambiguous.

    'at session start' is too vague — an agent reading it mid-conversation
    could decide the session already started and skip wake-up. The instruction
    must anchor to 'first /kluris call of the session' which the agent can
    observe directly.
    """
    content = render_skill()
    assert "Bootstrap" in content
    assert "first" in content.lower() and "/kluris" in content
    # Cache guidance: agent should reuse wake-up output, not re-run every turn
    assert "cache" in content.lower() or "trust" in content.lower()


def test_skill_bootstrap_lists_refresh_triggers():
    """Skill tells the agent when to re-run wake-up after the brain changes."""
    content = render_skill()
    # When the brain mutates (neuron/lobe/remember/learn/dream/push), the
    # cached snapshot is stale and wake-up must be re-run.
    lowered = content.lower()
    assert "re-run" in lowered or "refresh" in lowered
    # Must mention at least two mutation triggers so the rule is unambiguous
    triggers = ["neuron", "lobe", "dream", "push", "remember", "learn"]
    hits = sum(1 for t in triggers if t in lowered)
    assert hits >= 2
