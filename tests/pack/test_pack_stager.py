"""TEST-PACK-55 — stager produces the expected pack directory."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

from kluris.core import pack as pack_module
from kluris.core.pack import stage_pack


@pytest.fixture
def staged_brain(fixture_brain: Path, tmp_path) -> tuple[Path, dict]:
    out = tmp_path / "fixture-brain-pack"
    manifest = stage_pack(fixture_brain, out, brain_name="fixture-brain")
    return out, manifest


def test_writes_expected_top_level_files(staged_brain):
    out, _ = staged_brain
    expected = {
        "Dockerfile",
        "docker-compose.yml",
        ".dockerignore",
        ".gitignore",
        ".env.example",
        "README.md",
    }
    actual = {p.name for p in out.iterdir() if p.is_file()}
    assert expected <= actual
    # `.env` is NOT generated — the deployer copies it from `.env.example`
    # so they get the documented defaults instead of an all-commented stub
    # whose uncommented values diverged from the example.
    assert not (out / ".env").exists()


def test_writes_expected_top_level_dirs(staged_brain):
    out, _ = staged_brain
    dirs = {p.name for p in out.iterdir() if p.is_dir()}
    assert {"app", "kluris_runtime", "brain"} <= dirs


def test_pack_templates_fallback_to_package_resources(monkeypatch, tmp_path):
    monkeypatch.setattr(
        pack_module,
        "_SOURCE_PACKAGING_ROOT",
        tmp_path / "missing-source-packaging",
    )

    text = pack_module._read_template("Dockerfile.template")
    assert "FROM python:3.12-slim" in text


def test_packaged_templates_match_source_templates():
    source_dir = Path(__file__).resolve().parent.parent.parent / "packaging"
    packaged_dir = (
        Path(__file__).resolve().parent.parent.parent
        / "src"
        / "kluris"
        / "_packaging"
    )
    source_files = {p.name for p in source_dir.iterdir() if p.is_file()}
    packaged_files = {
        p.name for p in packaged_dir.iterdir()
        if p.is_file() and p.name != "__init__.py"
    }
    assert packaged_files == source_files
    for name in source_files:
        assert (packaged_dir / name).read_text(encoding="utf-8") == (
            source_dir / name
        ).read_text(encoding="utf-8")


def test_env_example_uses_obvious_placeholders(staged_brain):
    """`.env.example` is what the deployer copies to `.env`. Any uncommented
    secret-bearing var must use a placeholder marker so an accidental commit
    of a freshly-staged pack can't leak a real key.
    """
    out, _ = staged_brain
    env_text = (out / ".env.example").read_text(encoding="utf-8")
    api_key_line = next(
        (l for l in env_text.splitlines()
         if l.startswith("KLURIS_API_KEY=")),
        None,
    )
    assert api_key_line is not None
    assert "replace-me" in api_key_line.lower(), (
        f"KLURIS_API_KEY must use a placeholder marker, got: {api_key_line!r}"
    )
    # OAuth secret must not be live — only commented examples are allowed.
    for line in env_text.splitlines():
        assert not line.startswith("KLURIS_OAUTH_CLIENT_SECRET="), (
            f"KLURIS_OAUTH_CLIENT_SECRET must stay commented, got: {line!r}"
        )


def test_gitignore_protects_env_and_local_artifacts(staged_brain):
    out, _ = staged_brain
    gi = (out / ".gitignore").read_text(encoding="utf-8")
    assert ".env" in gi
    assert "!.env.example" in gi
    assert "*.db" in gi
    assert "__pycache__/" in gi
    assert ".DS_Store" in gi


def test_brain_excludes_default_garbage(fixture_brain, tmp_path):
    # Add garbage to the brain
    (fixture_brain / "kluris.yml").write_text("name: x\n", encoding="utf-8")
    (fixture_brain / "brain-mri.html").write_text("<html></html>", encoding="utf-8")
    (fixture_brain / "knowledge" / "__pycache__").mkdir(exist_ok=True)
    (fixture_brain / "knowledge" / "__pycache__" / "a.pyc").write_bytes(b"x")
    (fixture_brain / "knowledge" / "stale.pyc").write_bytes(b"x")
    out = tmp_path / "out"
    stage_pack(fixture_brain, out, brain_name="fixture-brain")
    assert not (out / "brain" / "kluris.yml").exists()
    assert not (out / "brain" / "brain-mri.html").exists()
    assert not list((out / "brain").rglob("__pycache__"))
    assert not list((out / "brain").rglob("*.pyc"))


def test_user_excludes_honored(fixture_brain, tmp_path):
    (fixture_brain / "knowledge" / "private.md").write_text(
        "---\nupdated: 2026-04-01\n---\n# Private\n", encoding="utf-8"
    )
    out = tmp_path / "out"
    stage_pack(
        fixture_brain, out, brain_name="fixture-brain",
        excludes=("knowledge/private.md",),
    )
    assert not (out / "brain" / "knowledge" / "private.md").exists()
    # Other knowledge neurons still present
    assert (out / "brain" / "knowledge" / "jwt.md").exists()


def test_no_pycache_or_pyc_in_app_or_runtime(fixture_brain, tmp_path):
    """Spec: ``shutil.ignore_patterns`` must keep dev-machine bytecode
    out of the staged app/ and kluris_runtime/ directories.
    """
    pack_src = (
        Path(__file__).resolve().parent.parent.parent
        / "src" / "kluris" / "pack"
    )
    runtime_src = (
        Path(__file__).resolve().parent.parent.parent
        / "src" / "kluris_runtime"
    )
    poison_pack = pack_src / "__pycache__"
    poison_pack.mkdir(exist_ok=True)
    poison_file = poison_pack / "poison.pyc"
    poison_file.write_bytes(b"\x00")
    poison_runtime = runtime_src / "__pycache__"
    poison_runtime.mkdir(exist_ok=True)
    poison_runtime_file = poison_runtime / "poison.pyc"
    poison_runtime_file.write_bytes(b"\x00")
    try:
        out = tmp_path / "out"
        stage_pack(fixture_brain, out, brain_name="fixture-brain")
        for sub in ("app", "kluris_runtime"):
            assert not list((out / sub).rglob("__pycache__")), (
                f"{sub}/ must not contain __pycache__/"
            )
            assert not list((out / sub).rglob("*.pyc")), (
                f"{sub}/ must not contain *.pyc files"
            )
    finally:
        try:
            poison_file.unlink()
        except OSError:
            pass
        try:
            poison_runtime_file.unlink()
        except OSError:
            pass


def test_dockerignore_only_allows_expected_paths(staged_brain):
    out, _ = staged_brain
    di = (out / ".dockerignore").read_text(encoding="utf-8")
    # Allow-list shape: deny-all on a line of its own, then explicit
    # allows for the directories that need to enter the build context.
    non_comment_lines = [
        l for l in di.splitlines()
        if l.strip() and not l.strip().startswith("#")
    ]
    assert non_comment_lines and non_comment_lines[0] == "*", (
        f".dockerignore should begin (after comments) with a single '*' "
        f"line; got {non_comment_lines[:3]}"
    )
    assert "!app/" in di
    assert "!kluris_runtime/" in di
    assert "!brain/" in di
    assert "!Dockerfile" in di


def test_dockerfile_contents(staged_brain):
    out, _ = staged_brain
    df = (out / "Dockerfile").read_text(encoding="utf-8")
    assert "COPY brain/ /app/brain/" in df
    assert "USER kluris" in df
    assert "PYTHONDONTWRITEBYTECODE=1" in df
    # chmod must come AFTER the COPYs (test by source-position).
    chmod_idx = df.index("chmod -R a-w /app")
    last_copy_idx = df.rindex("COPY ")
    assert chmod_idx > last_copy_idx
    # Must NOT chown /app to the runtime user.
    assert "chown -R kluris:kluris /app" not in df


def test_compose_has_build_and_image_lines(staged_brain):
    out, _ = staged_brain
    compose = (out / "docker-compose.yml").read_text(encoding="utf-8")
    assert "build: ." in compose
    assert "image: ${KLURIS_IMAGE:-kluris-pack/fixture-brain:latest}" in compose
    # No bind mount for brain/.
    assert "./brain" not in compose
    # Only kluris-data volume.
    assert "kluris-data:/data" in compose


def test_app_is_verbatim_copy_of_pack_source(staged_brain):
    out, _ = staged_brain
    pack_src = (
        Path(__file__).resolve().parent.parent.parent
        / "src" / "kluris" / "pack"
    )
    src_files = {
        p.relative_to(pack_src).as_posix()
        for p in pack_src.rglob("*")
        if p.is_file() and "__pycache__" not in p.parts
        and not p.name.endswith((".pyc", ".pyo"))
    }
    out_files = {
        p.relative_to(out / "app").as_posix()
        for p in (out / "app").rglob("*")
        if p.is_file()
    }
    assert src_files == out_files


def test_runtime_contains_only_allow_listed_files(staged_brain):
    out, _ = staged_brain
    expected = {
        "__init__.py",
        "deprecation.py",
        "frontmatter.py",
        "neuron_index.py",
        "search.py",
        "wake_up.py",
        "neuron_excerpt.py",
    }
    actual = {p.name for p in (out / "kluris_runtime").iterdir() if p.is_file()}
    assert expected == actual
    assert not (out / "kluris_runtime" / "cli.py").exists()


def test_subprocess_can_import_app_and_runtime(staged_brain):
    """Final contract: with ``PYTHONPATH=<output>`` ahead of everything
    else, ``import app.main`` and every runtime submodule must succeed.

    The staged ``kluris_runtime`` must be picked up from ``<output>``,
    not from the source tree — checked via ``__file__``. ``app.X`` is
    only reachable through ``<output>`` since ``app`` doesn't exist in
    ``src/``.
    """
    out, _ = staged_brain
    code = textwrap.dedent("""
        import sys, os
        # Verify the runtime resolves to the staged copy, not the
        # source-tree version (PYTHONPATH puts <output> first).
        import kluris_runtime
        expected = os.environ['EXPECTED_OUT']
        assert expected in kluris_runtime.__file__, (
            f"runtime loaded from unexpected path: {kluris_runtime.__file__}"
        )
        # ``app.*`` only exists in the staged output — no src/ shadow.
        import app.main
        assert expected in app.main.__file__, app.main.__file__
        import kluris_runtime.frontmatter
        import kluris_runtime.neuron_index
        import kluris_runtime.deprecation
        import kluris_runtime.search
        import kluris_runtime.wake_up
        import kluris_runtime.neuron_excerpt
        print("ok")
    """).strip()
    env = dict(os.environ)
    env["PYTHONPATH"] = str(out)
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    env["EXPECTED_OUT"] = str(out)
    result = subprocess.run(
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        env=env,
        cwd=str(out.parent),
        check=False,
    )
    assert result.returncode == 0, (
        f"stdout={result.stdout!r}\nstderr={result.stderr!r}"
    )
    assert "ok" in result.stdout


def test_manifest_lists_every_file(staged_brain):
    out, manifest = staged_brain
    actual = sorted(
        p.relative_to(out).as_posix()
        for p in out.rglob("*")
        if p.is_file()
    )
    assert manifest["files"] == actual
    assert manifest["brain"] == "fixture-brain"
    assert manifest["neuron_count"] >= 1


def test_existing_output_dir_raises(fixture_brain, tmp_path):
    out = tmp_path / "exists"
    out.mkdir()
    with pytest.raises(FileExistsError):
        stage_pack(fixture_brain, out, brain_name="fixture-brain")


def test_existing_output_error_hints_at_force_flag(fixture_brain, tmp_path):
    out = tmp_path / "exists"
    out.mkdir()
    with pytest.raises(FileExistsError) as exc:
        stage_pack(fixture_brain, out, brain_name="fixture-brain")
    assert "--force" in str(exc.value)


def test_force_rebuild_wipes_and_recreates(fixture_brain, tmp_path):
    """With ``force=True``, an existing dir is wiped and rebuilt — any
    file outside the preserved ``.env`` family is gone.
    """
    out = tmp_path / "out"
    stage_pack(fixture_brain, out, brain_name="fixture-brain")
    # Drop a stray file the rebuild should NOT preserve.
    (out / "stray.txt").write_text("stale", encoding="utf-8")
    # Drop a file matching a non-preserved name.
    (out / "deployer-notes.md").write_text("notes", encoding="utf-8")

    manifest = stage_pack(
        fixture_brain, out, brain_name="fixture-brain", force=True,
    )
    assert manifest["ok"] is True
    # Stray files are gone.
    assert not (out / "stray.txt").exists()
    assert not (out / "deployer-notes.md").exists()
    # Fresh templates are present again.
    assert (out / "Dockerfile").is_file()
    # No `.env` was generated by the first stage, and the deployer never
    # created one — nothing to preserve.
    assert manifest["preserved"] == []


def test_force_rebuild_preserves_env_credentials(fixture_brain, tmp_path):
    """The deployer's filled-in ``.env`` survives a ``--force`` rebuild
    so brain edits don't force re-typing credentials.
    """
    out = tmp_path / "out"
    stage_pack(fixture_brain, out, brain_name="fixture-brain")

    real_env = (
        "KLURIS_PROVIDER_SHAPE=anthropic\n"
        "KLURIS_BASE_URL=https://api.example.com\n"
        "KLURIS_API_KEY=sk-real-deployer-key-do-not-lose\n"
        "KLURIS_MODEL=claude-opus-4-7\n"
    )
    (out / ".env").write_text(real_env, encoding="utf-8")

    manifest = stage_pack(
        fixture_brain, out, brain_name="fixture-brain", force=True,
    )
    assert ".env" in manifest["preserved"]
    assert (out / ".env").read_text(encoding="utf-8") == real_env, (
        "deployer's .env credentials must survive --force rebuild"
    )
    # .env.example is regenerated (template), not preserved.
    assert (out / ".env.example").is_file()


@pytest.mark.parametrize(
    "preserved_name",
    [".env", ".env.local", ".env.production", ".env.staging"],
)
def test_force_rebuild_preserves_env_family(
    fixture_brain, tmp_path, preserved_name,
):
    out = tmp_path / "out"
    stage_pack(fixture_brain, out, brain_name="fixture-brain")
    secret = f"KLURIS_API_KEY=sk-secret-for-{preserved_name}\n"
    (out / preserved_name).write_text(secret, encoding="utf-8")

    manifest = stage_pack(
        fixture_brain, out, brain_name="fixture-brain", force=True,
    )
    assert preserved_name in manifest["preserved"]
    assert (out / preserved_name).read_text(encoding="utf-8") == secret


def test_force_with_no_existing_output_just_creates(fixture_brain, tmp_path):
    out = tmp_path / "fresh"
    manifest = stage_pack(
        fixture_brain, out, brain_name="fixture-brain", force=True,
    )
    assert manifest["ok"] is True
    assert manifest["preserved"] == []
    assert out.is_dir()
