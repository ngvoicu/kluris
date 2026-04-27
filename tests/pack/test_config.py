"""TEST-PACK-06 — env-driven Config.

Asserts that :class:`kluris.pack.config.Config` builds cleanly from the
environment, fails fast when misconfigured, and never leaks secrets.
"""

from __future__ import annotations

import logging
from pathlib import Path

import pytest
from pydantic import SecretStr

from kluris.pack.config import Config, ConfigError

# Convenience builders for the two valid configurations.

_API_KEY_ENV = {
    "KLURIS_PROVIDER_SHAPE": "anthropic",
    "KLURIS_BASE_URL": "https://api.example.com",
    "KLURIS_API_KEY": "sk-secret-xyz",
    "KLURIS_MODEL": "claude-opus-4-7",
}

_OAUTH_ENV = {
    "KLURIS_OAUTH_TOKEN_URL": "https://idp.example.com/token",
    "KLURIS_OAUTH_API_BASE_URL": "https://api.example.com",
    "KLURIS_OAUTH_CLIENT_ID": "kluris-app",
    "KLURIS_OAUTH_CLIENT_SECRET": "oauth-secret-xyz",
    "KLURIS_MODEL": "internal-model-v2",
}


def test_loads_api_key_anthropic_shape():
    cfg = Config.load_from_env(_API_KEY_ENV)
    assert cfg.auth_mode == "api_key"
    assert cfg.provider_shape == "anthropic"
    assert cfg.base_url == "https://api.example.com"
    assert isinstance(cfg.api_key, SecretStr)
    assert cfg.api_key.get_secret_value() == "sk-secret-xyz"
    assert cfg.model == "claude-opus-4-7"
    assert cfg.api_url == "https://api.example.com"


def test_loads_api_key_openai_shape():
    env = dict(_API_KEY_ENV, KLURIS_PROVIDER_SHAPE="openai")
    cfg = Config.load_from_env(env)
    assert cfg.provider_shape == "openai"


def test_loads_oauth_path():
    cfg = Config.load_from_env(_OAUTH_ENV)
    assert cfg.auth_mode == "oauth"
    assert cfg.oauth_token_url == "https://idp.example.com/token"
    assert cfg.oauth_api_base_url == "https://api.example.com"
    assert cfg.oauth_client_id == "kluris-app"
    assert isinstance(cfg.oauth_client_secret, SecretStr)
    assert cfg.api_url == "https://api.example.com"


def test_oauth_scope_optional():
    env = dict(_OAUTH_ENV, KLURIS_OAUTH_SCOPE="read:brain")
    cfg = Config.load_from_env(env)
    assert cfg.oauth_scope == "read:brain"


def test_strips_trailing_slash_from_base_url():
    env = dict(_API_KEY_ENV, KLURIS_BASE_URL="https://api.example.com/")
    cfg = Config.load_from_env(env)
    assert cfg.base_url == "https://api.example.com"


@pytest.mark.parametrize(
    "raw, expected",
    [
        # Common copy-paste-from-docs mistakes:
        (
            "https://openrouter.ai/api/v1/chat/completions",
            "https://openrouter.ai/api",
        ),
        (
            "https://api.openai.com/v1/chat/completions",
            "https://api.openai.com",
        ),
        (
            "https://api.anthropic.com/v1/messages",
            "https://api.anthropic.com",
        ),
        # Trailing slash on the endpoint also handled:
        (
            "https://openrouter.ai/api/v1/chat/completions/",
            "https://openrouter.ai/api",
        ),
    ],
)
def test_normalizes_known_endpoint_suffixes_in_base_url(raw, expected):
    """Deployers commonly paste the full endpoint URL from provider
    docs into KLURIS_BASE_URL. The provider class appends the path
    itself, so a doubled path ``/v1/.../v1/...`` 404s. Strip the
    suffix automatically and surface a boot warning.
    """
    env = dict(_API_KEY_ENV, KLURIS_BASE_URL=raw)
    cfg = Config.load_from_env(env)
    assert cfg.base_url == expected
    assert cfg.boot_warnings, "trim should surface a boot warning"
    assert "KLURIS_BASE_URL" in cfg.boot_warnings[0]


def test_unmodified_base_url_emits_no_warning():
    env = dict(_API_KEY_ENV, KLURIS_BASE_URL="https://api.example.com")
    cfg = Config.load_from_env(env)
    assert cfg.boot_warnings == []


def test_oauth_api_base_url_also_normalized():
    """The same trim applies to KLURIS_OAUTH_API_BASE_URL."""
    env = {
        "KLURIS_OAUTH_TOKEN_URL": "https://idp.example.com/token",
        "KLURIS_OAUTH_API_BASE_URL": "https://gw.example.com/v1/chat/completions",
        "KLURIS_OAUTH_CLIENT_ID": "x",
        "KLURIS_OAUTH_CLIENT_SECRET": "y",
        "KLURIS_MODEL": "m",
    }
    cfg = Config.load_from_env(env)
    assert cfg.oauth_api_base_url == "https://gw.example.com"
    assert cfg.boot_warnings
    assert "KLURIS_OAUTH_API_BASE_URL" in cfg.boot_warnings[0]


def test_boot_warnings_excluded_from_repr():
    """``boot_warnings`` is logged separately by main.py — it should
    NOT clutter the redacted repr/str of Config.
    """
    env = dict(
        _API_KEY_ENV,
        KLURIS_BASE_URL="https://openrouter.ai/api/v1/chat/completions",
    )
    cfg = Config.load_from_env(env)
    assert cfg.boot_warnings  # something was warned about
    assert "boot_warnings" not in repr(cfg)
    assert "boot_warnings" not in str(cfg)


def test_missing_required_api_key_var_raises():
    env = dict(_API_KEY_ENV)
    del env["KLURIS_API_KEY"]
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "KLURIS_API_KEY" in str(exc.value)


def test_missing_required_oauth_var_raises():
    env = dict(_OAUTH_ENV)
    del env["KLURIS_OAUTH_CLIENT_SECRET"]
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "KLURIS_OAUTH_CLIENT_SECRET" in str(exc.value)


def test_no_auth_configured_lists_both_paths():
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env({})
    msg = str(exc.value)
    assert "KLURIS_API_KEY" in msg
    assert "KLURIS_OAUTH_TOKEN_URL" in msg


def test_both_auth_paths_set_raises():
    """API-key + OAuth in the same env → fail-fast with both-paths error."""
    env = {**_API_KEY_ENV, **_OAUTH_ENV}
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "only one" in str(exc.value).lower()


def test_invalid_provider_shape_raises():
    env = dict(_API_KEY_ENV, KLURIS_PROVIDER_SHAPE="bedrock")
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "KLURIS_PROVIDER_SHAPE" in str(exc.value)


def test_anthropic_version_default_and_override():
    cfg_default = Config.load_from_env(_API_KEY_ENV)
    assert cfg_default.anthropic_version == "2023-06-01"

    env_override = dict(_API_KEY_ENV, KLURIS_ANTHROPIC_VERSION="2099-01-01")
    cfg_override = Config.load_from_env(env_override)
    assert cfg_override.anthropic_version == "2099-01-01"


def test_lobe_overview_budget_clamped():
    env_low = dict(_API_KEY_ENV, KLURIS_LOBE_OVERVIEW_BUDGET="100")
    assert Config.load_from_env(env_low).lobe_overview_budget == 1024

    env_high = dict(_API_KEY_ENV, KLURIS_LOBE_OVERVIEW_BUDGET="999999")
    assert Config.load_from_env(env_high).lobe_overview_budget == 16384

    env_ok = dict(_API_KEY_ENV, KLURIS_LOBE_OVERVIEW_BUDGET="8192")
    assert Config.load_from_env(env_ok).lobe_overview_budget == 8192


def test_max_multi_read_paths_clamped():
    env_low = dict(_API_KEY_ENV, KLURIS_MAX_MULTI_READ_PATHS="0")
    assert Config.load_from_env(env_low).max_multi_read_paths == 1

    env_high = dict(_API_KEY_ENV, KLURIS_MAX_MULTI_READ_PATHS="50")
    assert Config.load_from_env(env_high).max_multi_read_paths == 20

    env_ok = dict(_API_KEY_ENV, KLURIS_MAX_MULTI_READ_PATHS="7")
    assert Config.load_from_env(env_ok).max_multi_read_paths == 7


def test_max_agent_rounds_default_and_override():
    assert Config.load_from_env(_API_KEY_ENV).max_agent_rounds == 20
    env = dict(_API_KEY_ENV, MAX_AGENT_ROUNDS="3")
    assert Config.load_from_env(env).max_agent_rounds == 3


def test_max_agent_rounds_zero_is_unlimited_sentinel():
    """``MAX_AGENT_ROUNDS=0`` means "unlimited" at the agent layer.
    The Config just stores the integer; the loop checks ``<= 0``.
    """
    env = dict(_API_KEY_ENV, MAX_AGENT_ROUNDS="0")
    assert Config.load_from_env(env).max_agent_rounds == 0


def test_brain_and_data_dirs_overridable():
    env = dict(
        _API_KEY_ENV,
        KLURIS_BRAIN_DIR="/custom/brain",
        KLURIS_DATA_DIR="/custom/data",
    )
    cfg = Config.load_from_env(env)
    assert cfg.brain_dir == Path("/custom/brain")
    assert cfg.data_dir == Path("/custom/data")


def test_repr_redacts_api_key():
    cfg = Config.load_from_env(_API_KEY_ENV)
    rendered = repr(cfg)
    assert "sk-secret-xyz" not in rendered
    assert "***" in rendered


def test_str_redacts_api_key():
    cfg = Config.load_from_env(_API_KEY_ENV)
    rendered = str(cfg)
    assert "sk-secret-xyz" not in rendered


def test_fstring_redacts_oauth_secret():
    cfg = Config.load_from_env(_OAUTH_ENV)
    rendered = f"{cfg}"
    assert "oauth-secret-xyz" not in rendered


def test_logging_does_not_leak_secret(caplog):
    cfg = Config.load_from_env(_API_KEY_ENV)
    with caplog.at_level(logging.INFO):
        logging.getLogger("test").info("config: %s", cfg)
        logging.getLogger("test").info("config: %r", cfg)
    for record in caplog.records:
        assert "sk-secret-xyz" not in record.getMessage()


def test_tls_defaults_are_secure(api_env=None):
    cfg = Config.load_from_env(_API_KEY_ENV)
    assert cfg.tls_ca_bundle is None
    assert cfg.tls_insecure is False
    assert cfg.httpx_verify is True


def test_tls_ca_bundle_override(tmp_path):
    """Custom CA bundle → ``httpx_verify`` returns an
    :class:`ssl.SSLContext` built from that bundle. The
    ``str``-form ``verify`` argument is deprecated in httpx 0.28+.
    """
    import ssl

    # Use a real (system-trusted) CA bundle so ``create_default_context``
    # can actually load it. We're verifying the wiring, not the cert
    # validation logic.
    bundle = Path(ssl.get_default_verify_paths().cafile or "")
    if not bundle or not bundle.exists():
        pytest.skip("no system CA bundle available to use as a fixture")

    env = dict(_API_KEY_ENV, KLURIS_CA_BUNDLE=str(bundle))
    cfg = Config.load_from_env(env)
    assert cfg.tls_ca_bundle == bundle
    assert cfg.tls_insecure is False
    verify = cfg.httpx_verify
    assert isinstance(verify, ssl.SSLContext)


def test_tls_ca_bundle_missing_path_errors(tmp_path):
    env = dict(_API_KEY_ENV, KLURIS_CA_BUNDLE=str(tmp_path / "nope.pem"))
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "KLURIS_CA_BUNDLE" in str(exc.value)


def test_tls_insecure_truthy_disables_verification():
    env = dict(_API_KEY_ENV, KLURIS_TLS_INSECURE="1")
    cfg = Config.load_from_env(env)
    assert cfg.tls_insecure is True
    assert cfg.httpx_verify is False


@pytest.mark.parametrize("literal", ["true", "yes", "on", "TRUE"])
def test_tls_insecure_other_truthy_literals(literal):
    env = dict(_API_KEY_ENV, KLURIS_TLS_INSECURE=literal)
    assert Config.load_from_env(env).tls_insecure is True


@pytest.mark.parametrize("literal", ["0", "false", "no", "off", ""])
def test_tls_insecure_falsy_literals(literal):
    env = dict(_API_KEY_ENV, KLURIS_TLS_INSECURE=literal)
    assert Config.load_from_env(env).tls_insecure is False


def test_tls_insecure_invalid_literal_errors():
    env = dict(_API_KEY_ENV, KLURIS_TLS_INSECURE="maybe")
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "KLURIS_TLS_INSECURE" in str(exc.value)


def test_tls_ca_bundle_and_insecure_mutually_exclusive(tmp_path):
    bundle = tmp_path / "corp.pem"
    bundle.write_text("# x\n", encoding="utf-8")
    env = dict(
        _API_KEY_ENV,
        KLURIS_CA_BUNDLE=str(bundle),
        KLURIS_TLS_INSECURE="1",
    )
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    msg = str(exc.value).lower()
    assert "mutually exclusive" in msg


def test_skip_boot_smoke_default_false():
    cfg = Config.load_from_env(_API_KEY_ENV)
    assert cfg.skip_boot_smoke is False


def test_skip_boot_smoke_truthy():
    env = dict(_API_KEY_ENV, KLURIS_SKIP_BOOT_SMOKE="1")
    cfg = Config.load_from_env(env)
    assert cfg.skip_boot_smoke is True


def test_skip_boot_smoke_invalid_literal_errors():
    env = dict(_API_KEY_ENV, KLURIS_SKIP_BOOT_SMOKE="maybe")
    with pytest.raises(ConfigError) as exc:
        Config.load_from_env(env)
    assert "KLURIS_SKIP_BOOT_SMOKE" in str(exc.value)


def test_no_ui_auth_env_vars_honored():
    """The Kluris app has no built-in UI auth. Setting common UI-auth
    env vars must not change Config behavior — they're simply ignored.
    """
    env = dict(
        _API_KEY_ENV,
        KLURIS_UI_BEARER_TOKEN="should-be-ignored",
        KLURIS_AUTH_TOKEN="ditto",
        KLURIS_BASIC_AUTH="user:pass",
    )
    cfg = Config.load_from_env(env)
    # No new attributes appeared from those env vars.
    for name in ("ui_bearer_token", "auth_token", "basic_auth"):
        assert not hasattr(cfg, name)
