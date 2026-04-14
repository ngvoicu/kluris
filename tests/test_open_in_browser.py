"""Tests for the WSL-aware browser-open helper used by `kluris mri --open`."""

import subprocess
from unittest.mock import MagicMock, patch

from kluris.cli import _is_wsl, _open_in_browser, _windows_path_if_wsl


def test_is_wsl_detects_wsl_distro_name_env(monkeypatch):
    monkeypatch.setenv("WSL_DISTRO_NAME", "Ubuntu-24.04")
    assert _is_wsl() is True


def test_is_wsl_returns_false_without_env_and_without_microsoft_in_proc(monkeypatch):
    monkeypatch.delenv("WSL_DISTRO_NAME", raising=False)
    with patch("builtins.open", side_effect=OSError("no /proc/version")):
        assert _is_wsl() is False


def test_windows_path_if_wsl_returns_translated_path_in_wsl(tmp_path, monkeypatch):
    monkeypatch.setenv("WSL_DISTRO_NAME", "Ubuntu-24.04")
    fake = MagicMock(stdout=r"\\wsl.localhost\Ubuntu-24.04\tmp\x.html" + "\n")
    with patch("subprocess.run", return_value=fake):
        out = _windows_path_if_wsl(tmp_path / "x.html")
    assert out == r"\\wsl.localhost\Ubuntu-24.04\tmp\x.html"


def test_windows_path_if_wsl_returns_none_outside_wsl(tmp_path, monkeypatch):
    monkeypatch.delenv("WSL_DISTRO_NAME", raising=False)
    with patch("builtins.open", side_effect=OSError):
        assert _windows_path_if_wsl(tmp_path / "x.html") is None


def test_windows_path_if_wsl_returns_none_when_wslpath_missing(tmp_path, monkeypatch):
    monkeypatch.setenv("WSL_DISTRO_NAME", "Ubuntu-24.04")
    with patch("subprocess.run", side_effect=FileNotFoundError):
        assert _windows_path_if_wsl(tmp_path / "x.html") is None


def test_windows_path_if_wsl_returns_none_when_wslpath_errors(tmp_path, monkeypatch):
    monkeypatch.setenv("WSL_DISTRO_NAME", "Ubuntu-24.04")
    err = subprocess.CalledProcessError(1, ["wslpath"])
    with patch("subprocess.run", side_effect=err):
        assert _windows_path_if_wsl(tmp_path / "x.html") is None


def test_open_in_browser_translates_path_and_uses_cmd_start_in_wsl(tmp_path, monkeypatch):
    """The reliable WSL open path: wslpath -w → cmd.exe /c start "" <win-path>.

    cmd.exe MUST be detached from the bash tty (DEVNULL + new session) or
    the parent terminal ends up with "read failed 5: I/O error" and garbled
    prompts after mri returns.
    """
    monkeypatch.setenv("WSL_DISTRO_NAME", "Ubuntu-24.04")
    html = tmp_path / "brain-mri.html"
    html.write_text("<html></html>", encoding="utf-8")

    fake_wslpath_result = MagicMock(stdout=r"\\wsl.localhost\Ubuntu-24.04\tmp\brain-mri.html" + "\n")

    def run_side_effect(args, **kwargs):
        if args[:2] == ["wslpath", "-w"]:
            return fake_wslpath_result
        return MagicMock(returncode=0)

    with patch("subprocess.run", side_effect=run_side_effect) as run, \
         patch("webbrowser.open") as wb_open:
        _open_in_browser(html)

    # Two calls: wslpath (inside _windows_path_if_wsl), cmd.exe /c start
    assert run.call_count == 2
    cmd_call = run.call_args_list[-1]
    cmd_args = cmd_call.args[0]
    assert cmd_args[:4] == ["cmd.exe", "/c", "start", ""]
    assert cmd_args[4] == r"\\wsl.localhost\Ubuntu-24.04\tmp\brain-mri.html"

    # The tty-detach kwargs — mandatory to avoid trashing the bash prompt.
    assert cmd_call.kwargs["stdin"] == subprocess.DEVNULL
    assert cmd_call.kwargs["stdout"] == subprocess.DEVNULL
    assert cmd_call.kwargs["stderr"] == subprocess.DEVNULL
    assert cmd_call.kwargs["start_new_session"] is True

    wb_open.assert_not_called()


def test_open_in_browser_falls_back_to_webbrowser_if_wslpath_missing(tmp_path, monkeypatch):
    """If wslpath is not on PATH, fall back to webbrowser.open rather than crashing."""
    monkeypatch.setenv("WSL_DISTRO_NAME", "Ubuntu-24.04")
    html = tmp_path / "brain-mri.html"
    html.write_text("<html></html>", encoding="utf-8")

    with patch("subprocess.run", side_effect=FileNotFoundError), \
         patch("webbrowser.open") as wb_open:
        _open_in_browser(html)

    wb_open.assert_called_once()
    assert wb_open.call_args.args[0].startswith("file://")


def test_open_in_browser_uses_webbrowser_outside_wsl(tmp_path, monkeypatch):
    monkeypatch.delenv("WSL_DISTRO_NAME", raising=False)
    html = tmp_path / "brain-mri.html"
    html.write_text("<html></html>", encoding="utf-8")

    with patch("builtins.open", side_effect=OSError), \
         patch("subprocess.run") as run, \
         patch("webbrowser.open") as wb_open:
        _open_in_browser(html)

    wb_open.assert_called_once()
    assert wb_open.call_args.args[0].startswith("file://")
    run.assert_not_called()
