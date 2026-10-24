"""TEST-PACK-59 — Docker e2e (skipped unless Docker is reachable).

The full e2e: build the mock-LLM image, run ``kluris pack`` against
the fixture brain, render a test-only ``docker-compose.test.yml``
overlay, ``docker compose up --build``, wait for ``/healthz``, POST
``/chat`` and assert SSE tokens.

In default CI / no-Docker environments this whole module skips. The
mock LLM container source lives at
``tests/pack/fixtures/mock_llm/``; the overlay is rendered into
``tmp_path`` per test from
``tests/pack/fixtures/docker-compose.test.yml.template``.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import time
import urllib.request
import uuid
from pathlib import Path

import pytest

from kluris.core.pack import stage_pack


def _docker_available() -> bool:
    if shutil.which("docker") is None:
        return False
    if os.environ.get("KLURIS_RUN_DOCKER_E2E") != "1":
        return False
    result = subprocess.run(
        ["docker", "info"],
        text=True,
        capture_output=True,
        check=False,
    )
    return result.returncode == 0


_DOCKER_AVAILABLE = _docker_available()

pytestmark = [
    pytest.mark.docker_network,
    pytest.mark.skipif(
        not _DOCKER_AVAILABLE,
        reason=(
            "Docker e2e requires a running daemon AND "
            "KLURIS_RUN_DOCKER_E2E=1; default CI skips this module."
        ),
    ),
]


def _run(args: list[str], *, cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        args,
        cwd=str(cwd) if cwd else None,
        text=True,
        capture_output=True,
        check=True,
    )


def _looks_like_unavailable_daemon(exc: subprocess.CalledProcessError) -> bool:
    text = f"{exc.stdout}\n{exc.stderr}".lower()
    return any(
        marker in text
        for marker in (
            "docker desktop is unable to start",
            "cannot connect to the docker daemon",
            "is the docker daemon running",
        )
    )


def _wait_for_healthz(url: str, *, timeout_s: float = 90.0) -> None:
    deadline = time.time() + timeout_s
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as resp:
                if resp.status == 200:
                    return
        except Exception as exc:  # pragma: no cover - opt-in Docker path
            last_error = exc
            time.sleep(1)
    raise AssertionError(f"healthz never became ready: {last_error}")


def test_e2e_full_round_trip(tmp_path, fixture_brain):  # pragma: no cover (opt-in)
    """Full Docker round-trip — implementation kept behind the env
    gate so default ``pytest tests/`` runs hermetically.

    Run with:
        KLURIS_RUN_DOCKER_E2E=1 pytest -m docker_network \\
            tests/pack/test_e2e_docker.py
    """
    mock_dir = Path(__file__).resolve().parent / "fixtures" / "mock_llm"
    overlay_template = (
        Path(__file__).resolve().parent
        / "fixtures"
        / "docker-compose.test.yml.template"
    )
    brain_name = f"fixture-brain-{uuid.uuid4().hex[:8]}"
    out = tmp_path / f"{brain_name}-pack"
    project = f"klurispack{uuid.uuid4().hex[:8]}"

    try:
        _run(["docker", "build", "-t", "kluris-pack-mock-llm:test", str(mock_dir)])
    except subprocess.CalledProcessError as exc:
        if _looks_like_unavailable_daemon(exc):
            pytest.skip(f"Docker daemon unavailable: {exc.stderr.strip()}")
        raise
    stage_pack(fixture_brain, out, brain_name=brain_name)
    (out / ".env").write_text(
        "\n".join([
            "KLURIS_PROVIDER_SHAPE=openai",
            "KLURIS_BASE_URL=http://mock-llm:8080",
            "KLURIS_API_KEY=test-key-not-real",
            "KLURIS_MODEL=mock-model",
            "",
        ]),
        encoding="utf-8",
    )
    (out / "docker-compose.test.yml").write_text(
        overlay_template.read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    compose = [
        "docker",
        "compose",
        "-p",
        project,
        "-f",
        "docker-compose.yml",
        "-f",
        "docker-compose.test.yml",
    ]
    try:
        _run(compose + ["up", "-d", "--build"], cwd=out)
        _wait_for_healthz("http://127.0.0.1:8765/healthz")
        _run(
            compose + [
                "exec",
                "-T",
                "kluris-pack",
                "sh",
                "-lc",
                (
                    'test "$(id -u)" != "0" && '
                    "test ! -w /app && "
                    "test -f /app/brain/brain.md && "
                    "test -w /data"
                ),
            ],
            cwd=out,
        )

        req = urllib.request.Request(
            "http://127.0.0.1:8765/chat",
            data=json.dumps({"message": "hello"}).encode("utf-8"),
            headers={"content-type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=20) as resp:
            body = resp.read().decode("utf-8")
        assert "data: " in body
        assert "Hello " in body
        assert "[DONE]" in body
    except Exception:
        try:
            logs = _run(compose + ["logs", "--no-color"], cwd=out)
            print(logs.stdout)
            print(logs.stderr)
        except Exception:
            pass
        raise
    finally:
        subprocess.run(
            compose + ["down", "-v", "--remove-orphans"],
            cwd=str(out),
            text=True,
            capture_output=True,
            check=False,
        )
