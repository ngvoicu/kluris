"""Env-driven configuration for the kluris pack chat server.

The packed app reads its config from environment variables at process
start. There is no in-app config UI and no persisted ``config.yml`` —
credential rotation is ``edit .env + docker compose down && up``.

Two auth shapes are supported:

- API key (Anthropic-style or OpenAI-style)
- OAuth 2.0 client_credentials (token URL + client ID + secret)

Exactly one must be configured. Both → fail-fast at boot.
"""

from __future__ import annotations

import os
from pathlib import Path

from pydantic import BaseModel, Field, SecretStr


class ConfigError(ValueError):
    """Raised when ``Config.load_from_env`` cannot build a valid config.

    The exception message lists every missing/conflicting variable so a
    deployer reading ``docker compose logs`` can fix the env in one
    pass. Secret values NEVER appear in the message.
    """


# Optional vars with sane defaults — deployer can override.
_DEFAULT_ANTHROPIC_VERSION = "2023-06-01"
_DEFAULT_MAX_AGENT_ROUNDS = 20
_DEFAULT_LOBE_OVERVIEW_BUDGET = 4096
_LOBE_OVERVIEW_BUDGET_MIN = 1024
_LOBE_OVERVIEW_BUDGET_MAX = 16384
_DEFAULT_MAX_MULTI_READ_PATHS = 5
_MAX_MULTI_READ_PATHS_MIN = 1
_MAX_MULTI_READ_PATHS_MAX = 20

# API-key shape values
_VALID_PROVIDER_SHAPES = {"anthropic", "openai"}

# Env-var name groups — kept as constants so error messages and tests
# don't drift.
_API_KEY_REQUIRED = (
    "KLURIS_PROVIDER_SHAPE",
    "KLURIS_BASE_URL",
    "KLURIS_API_KEY",
    "KLURIS_MODEL",
)
_OAUTH_REQUIRED = (
    "KLURIS_OAUTH_TOKEN_URL",
    "KLURIS_OAUTH_API_BASE_URL",
    "KLURIS_OAUTH_CLIENT_ID",
    "KLURIS_OAUTH_CLIENT_SECRET",
    "KLURIS_MODEL",
)


def _clamp(value: int, lo: int, hi: int) -> int:
    return max(lo, min(hi, value))


def _read_int(env: dict, name: str, default: int) -> int:
    raw = env.get(name)
    if raw is None or raw == "":
        return default
    try:
        return int(raw)
    except ValueError as exc:
        raise ConfigError(f"{name} must be an integer, got {raw!r}") from exc


_TRUE_LITERALS = {"1", "true", "yes", "on"}
_FALSE_LITERALS = {"0", "false", "no", "off", ""}

# Path suffixes that almost always mean the deployer pasted a full
# endpoint URL (Anthropic / OpenAI / OpenRouter docs all show the
# endpoint URL, not the host root). The provider classes append
# these themselves, so leaving them on ``base_url`` produces a
# doubled path like ``/v1/chat/completions/v1/chat/completions``
# which 404s. Strip them with a boot warning so a copy-paste from
# docs Just Works.
_ENDPOINT_SUFFIXES = (
    "/v1/chat/completions",
    "/v1/messages",
    "/chat/completions",  # some Azure deployments
    "/messages",
)


def _normalize_base_url(raw: str, var_name: str) -> tuple[str, str | None]:
    """Strip a known endpoint suffix from a base URL and return
    ``(cleaned, warning_or_None)``.

    The warning is plain text — main.py prints it to stderr at boot
    so the deployer sees what was changed in ``docker compose logs``.
    """
    cleaned = raw.rstrip("/")
    for suffix in _ENDPOINT_SUFFIXES:
        if cleaned.endswith(suffix):
            stripped = cleaned[: -len(suffix)]
            warning = (
                f"trimmed {suffix!r} from {var_name} "
                f"({cleaned!r} -> {stripped!r}); the provider appends "
                "the endpoint path itself, so set just the host root "
                "next time."
            )
            return stripped.rstrip("/"), warning
    return cleaned, None


def _read_bool(env: dict, name: str, default: bool) -> bool:
    raw = env.get(name)
    if raw is None:
        return default
    lowered = raw.strip().lower()
    if lowered in _TRUE_LITERALS:
        return True
    if lowered in _FALSE_LITERALS:
        return False
    raise ConfigError(
        f"{name} must be one of {sorted(_TRUE_LITERALS | {'0', 'false', 'no', 'off'})}, "
        f"got {raw!r}"
    )


class Config(BaseModel):
    """Validated chat-server configuration.

    Built once at boot via :meth:`load_from_env`. Both ``__repr__`` and
    ``__str__`` redact every secret to ``***`` so logging the config
    object never leaks credentials.
    """

    # Auth shape: exactly one set
    provider_shape: str | None = None  # "anthropic" | "openai" | None (oauth path)
    base_url: str | None = None
    api_key: SecretStr | None = None

    oauth_token_url: str | None = None
    oauth_api_base_url: str | None = None
    oauth_client_id: str | None = None
    oauth_client_secret: SecretStr | None = None
    oauth_scope: str | None = None

    # Common
    model: str = ""
    anthropic_version: str = _DEFAULT_ANTHROPIC_VERSION

    # Tunables. ``max_agent_rounds=0`` is the "unlimited" sentinel —
    # the loop runs until the model emits an end without any pending
    # tool_uses. Useful for deep-research questions on a brain you
    # trust to converge; risky against a sparse brain (cost runaway).
    max_agent_rounds: int = _DEFAULT_MAX_AGENT_ROUNDS
    lobe_overview_budget: int = _DEFAULT_LOBE_OVERVIEW_BUDGET
    max_multi_read_paths: int = _DEFAULT_MAX_MULTI_READ_PATHS

    # Filesystem
    brain_dir: Path = Field(default=Path("/app/brain"))
    data_dir: Path = Field(default=Path("/data"))

    # TLS — for corporate gateways that present a self-signed or
    # private-CA-signed cert. ``tls_ca_bundle`` is the secure option
    # (point at the corporate root CA file). ``tls_insecure`` disables
    # verification entirely; opt-in only, with a loud boot warning.
    tls_ca_bundle: Path | None = None
    tls_insecure: bool = False

    # Escape hatch: skip the boot tool-capability smoke-test entirely.
    # Some endpoints don't even implement the chat-completions probe
    # in a way the structural check survives (custom envelopes, batch-
    # only proxies, etc.). Deployers who know their endpoint works
    # can opt out at boot. Loud warning printed.
    skip_boot_smoke: bool = False

    # Warnings collected during ``load_from_env`` (e.g. base-URL
    # endpoint-suffix trim). main.py prints these to stderr at boot
    # so they show up in ``docker compose logs``. Excluded from the
    # redacted ``__repr__`` to keep config logging compact.
    boot_warnings: list[str] = Field(default_factory=list)

    @property
    def auth_mode(self) -> str:
        """``"api_key"`` or ``"oauth"`` — whichever path is configured."""
        return "oauth" if self.oauth_token_url else "api_key"

    @property
    def httpx_verify(self) -> "bool | object":
        """Return the ``verify`` argument every ``httpx.AsyncClient`` should use.

        - Custom CA bundle path → an :class:`ssl.SSLContext` built
          from the bundle (httpx 0.28+ deprecates ``verify=<str>``;
          the SSLContext form is the long-term-supported API).
        - ``KLURIS_TLS_INSECURE=1`` → ``False`` (escape hatch — TLS
          verification disabled entirely).
        - Neither set → ``True`` (system CA bundle, the default).
        """
        if self.tls_ca_bundle is not None:
            import ssl

            return ssl.create_default_context(cafile=str(self.tls_ca_bundle))
        if self.tls_insecure:
            return False
        return True

    @property
    def api_url(self) -> str:
        """The base URL the provider class will POST to.

        For the API-key path this is the deployer-supplied
        ``KLURIS_BASE_URL``. For the OAuth path it is
        ``KLURIS_OAUTH_API_BASE_URL`` (the proxied API endpoint, which
        is not the same host as the token URL).
        """
        if self.auth_mode == "oauth":
            return self.oauth_api_base_url or ""
        return self.base_url or ""

    def __repr__(self) -> str:
        return self._redacted_str()

    def __str__(self) -> str:
        return self._redacted_str()

    def _redacted_str(self) -> str:
        fields: list[str] = []
        for name, value in self.model_dump().items():
            if name == "boot_warnings":
                # Noisy and orthogonal — main.py logs them separately.
                continue
            if isinstance(value, SecretStr):
                rendered = "***"
            elif name in {"api_key", "oauth_client_secret"} and value is not None:
                rendered = "***"
            elif isinstance(value, Path):
                rendered = str(value)
            else:
                rendered = repr(value)
            fields.append(f"{name}={rendered}")
        return f"Config({', '.join(fields)})"

    @classmethod
    def load_from_env(cls, env: dict | None = None) -> "Config":
        """Build a :class:`Config` from environment variables.

        Pass ``env`` (a dict) for tests; defaults to ``os.environ``.
        Raises :class:`ConfigError` on missing/conflicting variables.
        """
        env = dict(env if env is not None else os.environ)
        warnings: list[str] = []

        api_key_set = {var for var in _API_KEY_REQUIRED if env.get(var)}
        oauth_set = {var for var in _OAUTH_REQUIRED if env.get(var)}

        # "Set" here means "populated to a non-empty value". A var that
        # is present but empty is treated as unset — that's how a
        # commented-out .env line behaves after envsubst.
        api_key_active = bool(api_key_set - {"KLURIS_MODEL"})
        oauth_active = bool(oauth_set - {"KLURIS_MODEL"})

        if api_key_active and oauth_active:
            raise ConfigError(
                "only one of API key or OAuth may be configured; both "
                "API-key vars and OAuth vars are set in the environment"
            )

        if not api_key_active and not oauth_active:
            raise ConfigError(
                "no auth configured; set either the API-key vars "
                f"({', '.join(_API_KEY_REQUIRED)}) or the OAuth vars "
                f"({', '.join(_OAUTH_REQUIRED)})"
            )

        if api_key_active:
            missing = [v for v in _API_KEY_REQUIRED if not env.get(v)]
            if missing:
                raise ConfigError(
                    f"missing required API-key vars: {', '.join(missing)}"
                )
            shape = env["KLURIS_PROVIDER_SHAPE"].strip().lower()
            if shape not in _VALID_PROVIDER_SHAPES:
                raise ConfigError(
                    f"KLURIS_PROVIDER_SHAPE must be one of "
                    f"{sorted(_VALID_PROVIDER_SHAPES)}, got {shape!r}"
                )
            base_url, warning = _normalize_base_url(
                env["KLURIS_BASE_URL"], "KLURIS_BASE_URL",
            )
            if warning:
                warnings.append(warning)
            return cls._build(
                provider_shape=shape,
                base_url=base_url,
                api_key=SecretStr(env["KLURIS_API_KEY"]),
                model=env["KLURIS_MODEL"],
                anthropic_version=env.get(
                    "KLURIS_ANTHROPIC_VERSION", _DEFAULT_ANTHROPIC_VERSION
                ),
                env=env,
                warnings=warnings,
            )

        # OAuth path
        missing = [v for v in _OAUTH_REQUIRED if not env.get(v)]
        if missing:
            raise ConfigError(
                f"missing required OAuth vars: {', '.join(missing)}"
            )
        oauth_api_base_url, oauth_warning = _normalize_base_url(
            env["KLURIS_OAUTH_API_BASE_URL"], "KLURIS_OAUTH_API_BASE_URL",
        )
        if oauth_warning:
            warnings.append(oauth_warning)
        return cls._build(
            oauth_token_url=env["KLURIS_OAUTH_TOKEN_URL"],
            oauth_api_base_url=oauth_api_base_url,
            oauth_client_id=env["KLURIS_OAUTH_CLIENT_ID"],
            oauth_client_secret=SecretStr(env["KLURIS_OAUTH_CLIENT_SECRET"]),
            oauth_scope=env.get("KLURIS_OAUTH_SCOPE") or None,
            model=env["KLURIS_MODEL"],
            anthropic_version=env.get(
                "KLURIS_ANTHROPIC_VERSION", _DEFAULT_ANTHROPIC_VERSION
            ),
            env=env,
            warnings=warnings,
        )

    @classmethod
    def _build(cls, *, env: dict, warnings: list[str] | None = None, **kwargs) -> "Config":
        max_rounds = _read_int(env, "MAX_AGENT_ROUNDS", _DEFAULT_MAX_AGENT_ROUNDS)
        budget = _clamp(
            _read_int(env, "KLURIS_LOBE_OVERVIEW_BUDGET", _DEFAULT_LOBE_OVERVIEW_BUDGET),
            _LOBE_OVERVIEW_BUDGET_MIN,
            _LOBE_OVERVIEW_BUDGET_MAX,
        )
        max_multi = _clamp(
            _read_int(env, "KLURIS_MAX_MULTI_READ_PATHS", _DEFAULT_MAX_MULTI_READ_PATHS),
            _MAX_MULTI_READ_PATHS_MIN,
            _MAX_MULTI_READ_PATHS_MAX,
        )
        brain_dir = Path(env.get("KLURIS_BRAIN_DIR", "/app/brain"))
        data_dir = Path(env.get("KLURIS_DATA_DIR", "/data"))

        ca_bundle_raw = env.get("KLURIS_CA_BUNDLE")
        ca_bundle = Path(ca_bundle_raw) if ca_bundle_raw else None
        if ca_bundle is not None and not ca_bundle.exists():
            raise ConfigError(
                f"KLURIS_CA_BUNDLE points at a missing file: {ca_bundle}"
            )
        tls_insecure = _read_bool(env, "KLURIS_TLS_INSECURE", False)
        if ca_bundle is not None and tls_insecure:
            raise ConfigError(
                "KLURIS_CA_BUNDLE and KLURIS_TLS_INSECURE are mutually "
                "exclusive — pick one (the bundle is the secure choice)"
            )

        skip_boot_smoke = _read_bool(env, "KLURIS_SKIP_BOOT_SMOKE", False)

        return cls(
            **kwargs,
            max_agent_rounds=max_rounds,
            lobe_overview_budget=budget,
            max_multi_read_paths=max_multi,
            brain_dir=brain_dir,
            data_dir=data_dir,
            tls_ca_bundle=ca_bundle,
            tls_insecure=tls_insecure,
            skip_boot_smoke=skip_boot_smoke,
            boot_warnings=list(warnings or []),
        )
