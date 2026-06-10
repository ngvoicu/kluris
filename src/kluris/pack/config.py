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
# Per-response output token budget (the model's answer-length cap). Default
# matches the value the providers used to hardcode. Clamped so a typo can't
# break chat with a zero/absurd budget.
_DEFAULT_MAX_OUTPUT_TOKENS = 4096
_MAX_OUTPUT_TOKENS_MIN = 16
_MAX_OUTPUT_TOKENS_MAX = 200000
# Sampling temperature. ``None`` (the default) means "omit it from the request"
# so the model uses its own default — required for reasoning models that reject
# an explicit temperature. Clamped to the standard [0, 2] range when set.
_TEMPERATURE_MIN = 0.0
_TEMPERATURE_MAX = 2.0
# Reasoning effort — the OpenAI Chat Completions ``reasoning_effort`` field that
# tells a reasoning model how hard to think. ``None`` (default) omits it, so
# non-reasoning models and the Anthropic shape are untouched. Accepted values
# are model-dependent; this is the OpenAI union as of 2026 (gpt-5.1 takes a
# subset). The provider is the final authority — a model that rejects a value
# returns a 400 on the first chat. Anthropic uses a different mechanism
# (adaptive thinking) the pack does not drive, so the knob is OpenAI-shape only.
_VALID_REASONING_EFFORTS = {"none", "minimal", "low", "medium", "high", "xhigh"}
# Sliding-window budget (estimated tokens) for the CONVERSATION HISTORY replayed
# to the model each turn. Older turns are dropped once the transcript exceeds
# this, so a long chat never hits the model's context window. ``0`` (or any
# value <= 0) disables trimming. Generous-but-bounded default; deployers on a
# small-context model should lower it.
_DEFAULT_MAX_CONTEXT_TOKENS = 24000
# Per-TURN request-size budget (estimated tokens). WITHIN a single turn the agent
# loop re-sends every prior tool result each round; this caps the accumulated
# request by eliding the OLDEST tool-result payloads. Distinct from
# max_context_tokens (which trims cross-turn HISTORY before the turn starts).
# <= 0 disables (legacy unbounded behaviour).
_DEFAULT_MAX_TURN_TOKENS = 96000
# Per-neuron body cap (UTF-8 bytes) on the AGENT read paths (read_neuron /
# multi_read) so a batch read of fat files can't dominate the turn budget. The
# brain-explorer UI is never clamped. <= 0 disables.
_DEFAULT_MAX_NEURON_BYTES = 16384
# Eager in-turn compaction: tool results older than this many rounds are
# elided from the request (the model has already read them; an identical
# re-issue is re-served instantly from an in-memory store, and the synthesis
# fallback restores everything it needs). Bounds the per-round re-send cost
# of a long research turn well below max_turn_tokens. <= 0 disables eager
# eliding (only the max_turn_tokens ceiling applies).
_DEFAULT_KEEP_RESULT_ROUNDS = 3

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


def _read_float(env: dict, name: str, default: float | None) -> float | None:
    raw = env.get(name)
    if raw is None or raw == "":
        return default
    try:
        return float(raw)
    except ValueError as exc:
        raise ConfigError(f"{name} must be a number, got {raw!r}") from exc


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


# Host that means "real OpenAI" — the only case where the OpenAI shape is
# routed through LiteLLM's Responses API (/v1/responses) by default. Every
# other host (Azure, OpenRouter, vLLM, on-prem gateways) speaks Chat
# Completions and must NOT be sent to /responses unless the deployer opts in
# via KLURIS_USE_RESPONSES_API.
_OPENAI_PROPER_HOST = "api.openai.com"


def _is_openai_proper_host(base_url: str | None) -> bool:
    """True iff ``base_url`` is unset or points at ``api.openai.com``.

    Scheme-less values (``api.openai.com``) are tolerated — they parse into
    the URL path rather than the netloc, so we retry with a ``//`` prefix.
    """
    if not base_url:
        return True
    from urllib.parse import urlparse

    parsed = urlparse(base_url)
    netloc = parsed.netloc or urlparse("//" + base_url).netloc
    host = netloc.lower().split("@")[-1].split(":")[0]
    return host == _OPENAI_PROPER_HOST


def _ensure_v1_suffix(base_url: str | None) -> str | None:
    """Append ``/v1`` to an OpenAI-compatible gateway base when it is absent.

    LiteLLM (via the OpenAI SDK) appends only the leaf ``/chat/completions``
    (or ``/responses``) to ``api_base``, so the deployer's host-root
    ``KLURIS_BASE_URL`` (e.g. ``https://openrouter.ai/api``) must carry the
    ``/v1`` itself or the request 404s — matching the retired providers, which
    POSTed to ``<base>/v1/chat/completions``. Idempotent: a base already ending
    in ``/v1`` is left alone.
    """
    if not base_url:
        return None
    trimmed = base_url.rstrip("/")
    if trimmed.endswith("/v1"):
        return trimmed
    return trimmed + "/v1"


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

    # Per-response output token budget sent to the LLM as the unified
    # ``max_tokens`` (LiteLLM translates it per provider — including to
    # ``max_completion_tokens`` for OpenAI reasoning models). ``temperature``
    # is None by default → omitted so the model uses its own default.
    max_output_tokens: int = _DEFAULT_MAX_OUTPUT_TOKENS
    temperature: float | None = None

    # Reasoning effort for the OpenAI Chat Completions shape (api-key ``openai``
    # + OAuth). ``None`` → omitted from the request. The Anthropic shape ignores
    # it (driving Anthropic reasoning needs adaptive-thinking block round-trips
    # the pack doesn't do); ``load_from_env`` surfaces a boot warning if the
    # knob is set on an Anthropic endpoint.
    reasoning_effort: str | None = None

    # Opt-in: force LiteLLM's OpenAI Responses API (/v1/responses) for a
    # NON-OpenAI host that genuinely implements it. By default only
    # api.openai.com is routed through Responses; OpenAI-compatible gateways
    # stay on Chat Completions. Ignored on the Anthropic and OAuth paths.
    use_responses_api: bool = False

    # Sliding-window budget (estimated tokens) for replayed conversation
    # history. <= 0 disables trimming. The agent loop drops the oldest turns
    # to keep the transcript under this so long chats never overflow the
    # model's context window.
    max_context_tokens: int = _DEFAULT_MAX_CONTEXT_TOKENS

    # Per-turn request-size budget (estimated tokens). The agent loop elides the
    # oldest tool-result payloads when one turn's transcript would exceed this,
    # bounding the quadratic re-send cost of a broad, many-round query. <= 0
    # disables. (max_context_tokens trims cross-turn history; this bounds the
    # single turn in flight.)
    max_turn_tokens: int = _DEFAULT_MAX_TURN_TOKENS

    # Per-neuron body cap (UTF-8 bytes) on the agent read paths (read_neuron /
    # multi_read). <= 0 disables. The brain-explorer UI is never clamped.
    max_neuron_bytes: int = _DEFAULT_MAX_NEURON_BYTES

    # Tool results older than this many rounds are elided from the in-turn
    # request (re-servable instantly on an identical re-issue). <= 0 disables.
    keep_result_rounds: int = _DEFAULT_KEEP_RESULT_ROUNDS

    # Optional shared-secret gate for the whole HTTP surface (except
    # /healthz). Unset (the default) keeps the server open and prints a loud
    # boot warning instead — backward compatible, but every deployer sees the
    # nudge. Accepted as ``Authorization: Bearer <token>``, the
    # ``kluris_access`` cookie, or a one-time ``?token=`` query that sets
    # the cookie for browser use.
    access_token: SecretStr | None = None

    # Per-IP fixed-window rate limit on POST /chat (requests per minute).
    # 0 disables (default) — one anonymous client can otherwise drive
    # unbounded provider spend on a public deployment.
    rate_limit_per_min: int = 0

    # Opt-in retention: sessions older than this many days are pruned at
    # boot. 0 (default) keeps everything — deleting a deployer's history
    # must never be a surprise.
    session_retention_days: int = 0

    # Pin the system prompt at boot instead of re-reading
    # /data/config/system_prompt.md on every request. Live-editing stays the
    # default; pinning closes the "a /data write instantly rewires the
    # agent's behavior" channel on hardened deployments.
    lock_system_prompt: bool = False

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

    # Diagnostic: when set, the provider writes ONE summary line per LLM
    # stream to stderr — round chunk count, whether any text/tool-call
    # surfaced, and the final ``finish_reason``. This is the supported way
    # to diagnose an empty "model returned no content" turn (see
    # ``agent.py``): the failing round is the last summary line before the
    # error. No request/response payloads are logged, so it is safe to
    # leave on; ``LITELLM_LOG=DEBUG`` is suppressed by ``configure_litellm``
    # and far noisier, so this is the preferred knob.
    debug_stream: bool = False

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
    def is_anthropic_shape(self) -> bool:
        """True only for the Anthropic api-key path.

        Drives which OpenAI-only params (``store``, ``reasoning_effort``,
        ``stream_options``) the provider may send: LiteLLM rejects them for
        Anthropic, and ``reasoning_effort`` in particular would make LiteLLM
        enable adaptive thinking — which the pack's tool loop cannot
        round-trip — so the provider gates them off here.
        """
        return self.auth_mode == "api_key" and self.provider_shape == "anthropic"

    @property
    def litellm_model(self) -> str:
        """The LiteLLM model string the provider passes to ``acompletion``.

        Backward-compat translation of today's ``.env`` (provider shape +
        base URL) onto LiteLLM routing:

        - Anthropic api-key → ``anthropic/<model>`` (/v1/messages).
        - OpenAI api-key, OpenAI-proper host (or ``use_responses_api``) →
          ``openai/responses/<model>`` (/v1/responses — the reasoning+tools win).
        - OpenAI api-key, non-OpenAI host → ``openai/<model>`` (/v1/chat/completions).
        - OAuth gateway → ``openai/<model>`` (/v1/chat/completions, bearer auth).
        """
        if self.auth_mode == "oauth":
            return f"openai/{self.model}"
        if self.provider_shape == "anthropic":
            return f"anthropic/{self.model}"
        # OpenAI api-key shape.
        if _is_openai_proper_host(self.base_url) or self.use_responses_api:
            return f"openai/responses/{self.model}"
        return f"openai/{self.model}"

    @property
    def litellm_api_base(self) -> str | None:
        """The ``api_base`` the provider passes to ``acompletion`` (or ``None``).

        OpenAI-proper resolves to ``None`` so LiteLLM uses its own default
        OpenAI base (the ``openai/responses/`` prefix already selects the
        endpoint). Anthropic passes the host unchanged (LiteLLM's handler
        appends ``/v1/messages`` itself). Every OpenAI-compatible gateway
        (api-key or OAuth) gets ``/v1`` appended so LiteLLM's leaf-only
        ``/chat/completions`` (or ``/responses``) append lands correctly.
        """
        if self.auth_mode == "oauth":
            return _ensure_v1_suffix(self.oauth_api_base_url)
        if self.provider_shape == "anthropic":
            return self.base_url or None
        # OpenAI api-key shape.
        if _is_openai_proper_host(self.base_url):
            return None
        return _ensure_v1_suffix(self.base_url)

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
        max_output = _clamp(
            _read_int(env, "KLURIS_MAX_OUTPUT_TOKENS", _DEFAULT_MAX_OUTPUT_TOKENS),
            _MAX_OUTPUT_TOKENS_MIN,
            _MAX_OUTPUT_TOKENS_MAX,
        )
        temperature = _read_float(env, "KLURIS_TEMPERATURE", None)
        if temperature is not None:
            temperature = max(_TEMPERATURE_MIN, min(_TEMPERATURE_MAX, temperature))
        reasoning_effort = env.get("KLURIS_REASONING_EFFORT")
        if reasoning_effort is not None:
            reasoning_effort = reasoning_effort.strip().lower() or None
        if (
            reasoning_effort is not None
            and reasoning_effort not in _VALID_REASONING_EFFORTS
        ):
            raise ConfigError(
                f"KLURIS_REASONING_EFFORT must be one of "
                f"{sorted(_VALID_REASONING_EFFORTS)}, got {reasoning_effort!r}"
            )
        max_context = _read_int(
            env, "KLURIS_MAX_CONTEXT_TOKENS", _DEFAULT_MAX_CONTEXT_TOKENS
        )
        max_turn_tokens = _read_int(
            env, "KLURIS_MAX_TURN_TOKENS", _DEFAULT_MAX_TURN_TOKENS
        )
        max_neuron_bytes = _read_int(
            env, "KLURIS_MAX_NEURON_BYTES", _DEFAULT_MAX_NEURON_BYTES
        )
        keep_result_rounds = _read_int(
            env, "KLURIS_KEEP_RESULT_ROUNDS", _DEFAULT_KEEP_RESULT_ROUNDS
        )
        access_token_raw = env.get("KLURIS_ACCESS_TOKEN", "").strip()
        access_token = SecretStr(access_token_raw) if access_token_raw else None
        rate_limit_per_min = _read_int(env, "KLURIS_RATE_LIMIT_PER_MIN", 0)
        session_retention_days = _read_int(env, "KLURIS_SESSION_RETENTION_DAYS", 0)
        lock_system_prompt = _read_bool(env, "KLURIS_LOCK_SYSTEM_PROMPT", False)
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
        use_responses_api = _read_bool(env, "KLURIS_USE_RESPONSES_API", False)
        debug_stream = _read_bool(env, "KLURIS_DEBUG_STREAM", False)

        # ``reasoning_effort`` is an OpenAI Chat Completions field. The OAuth
        # path also targets that shape (no ``provider_shape`` kwarg), so it is
        # honored there. Only the Anthropic api-key shape ignores it — warn so
        # the deployer isn't left expecting reasoning the pack doesn't wire.
        warn_list = list(warnings or [])
        if reasoning_effort is not None and kwargs.get("provider_shape") == "anthropic":
            warn_list.append(
                "KLURIS_REASONING_EFFORT is set but only applies to the OpenAI "
                "provider shape; the pack does not drive Anthropic adaptive "
                "thinking, so the value is ignored for this Anthropic endpoint."
            )

        return cls(
            **kwargs,
            max_agent_rounds=max_rounds,
            lobe_overview_budget=budget,
            max_multi_read_paths=max_multi,
            max_output_tokens=max_output,
            temperature=temperature,
            reasoning_effort=reasoning_effort,
            use_responses_api=use_responses_api,
            max_context_tokens=max_context,
            max_turn_tokens=max_turn_tokens,
            max_neuron_bytes=max_neuron_bytes,
            keep_result_rounds=keep_result_rounds,
            access_token=access_token,
            rate_limit_per_min=rate_limit_per_min,
            session_retention_days=session_retention_days,
            lock_system_prompt=lock_system_prompt,
            brain_dir=brain_dir,
            data_dir=data_dir,
            tls_ca_bundle=ca_bundle,
            tls_insecure=tls_insecure,
            skip_boot_smoke=skip_boot_smoke,
            debug_stream=debug_stream,
            boot_warnings=warn_list,
        )
