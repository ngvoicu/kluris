"""Lightweight request plumbing for the chat server.

The Kluris app intentionally has NO built-in UI auth, NO CSRF, NO
session-bearer auth. Public exposure is the deployer's responsibility
(reverse proxy, VPN, cloud IAM). This module is the catch-all for
small request-time concerns that don't fit elsewhere — currently just
a redaction filter for log records.
"""

from __future__ import annotations

import logging
import re

# Strings that must never reach a log handler — or the chat UI.
_REDACTION_TOKEN = "***"
_BEARER_PATTERN = re.compile(r"(Bearer\s+)\S+", re.IGNORECASE)
_X_API_KEY_PATTERN = re.compile(r"(x-api-key:\s*)\S+", re.IGNORECASE)
# Bare provider API keys (OpenAI ``sk-...`` / ``sk-proj-...`` / Anthropic
# ``sk-ant-...``). LiteLLM/OpenAI error bodies echo a partial key
# ("Incorrect API key provided: sk-proj-..."), and those error strings are
# surfaced to the chat UI by the agent loop — so they must be scrubbed too.
_API_KEY_PATTERN = re.compile(r"sk-[A-Za-z0-9._-]{6,}")
# Bare JWT bearer tokens (header.payload.signature, base64url). The OAuth path
# fetches such a bearer; a gateway that echoes the credential in an error body
# (without a "Bearer " prefix) would otherwise leak it to the chat UI / session
# store. ``eyJ`` is the base64 of ``{"`` that every JWT header starts with.
_JWT_PATTERN = re.compile(r"\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*")

# Literal secret VALUES registered at boot (the configured API key, OAuth
# client secret, access token). The regexes above only recognize well-known
# shapes — an Azure key, an opaque gateway secret, or a client_secret echoed
# in an IdP error body matches none of them. Value-based redaction is exact
# and shape-independent: whatever the deployer actually configured can never
# reach a log line or the chat UI. Registered once at boot, read-only after.
_LITERAL_SECRETS: list[str] = []


def register_secret(value: str | None) -> None:
    """Register a configured secret's literal value for redaction.

    Idempotent; ``None`` / empty / very short values are ignored (redacting
    1-2 char strings would shred ordinary text).
    """
    if not value or len(value) < 6:
        return
    if value not in _LITERAL_SECRETS:
        _LITERAL_SECRETS.append(value)


def _clear_registered_secrets() -> None:
    """Reset registered literals (tests only)."""
    _LITERAL_SECRETS.clear()


def redact_secrets(text: str) -> str:
    """Scrub registered secret values, bearer tokens, ``x-api-key`` values,
    bare ``sk-`` keys, and JWTs.

    Used both by the log filter and by the provider's error-mapping layer
    before any provider message reaches the chat UI.
    """
    for literal in _LITERAL_SECRETS:
        if literal in text:
            text = text.replace(literal, _REDACTION_TOKEN)
    text = _BEARER_PATTERN.sub(r"\1" + _REDACTION_TOKEN, text)
    text = _X_API_KEY_PATTERN.sub(r"\1" + _REDACTION_TOKEN, text)
    text = _API_KEY_PATTERN.sub(_REDACTION_TOKEN, text)
    text = _JWT_PATTERN.sub(_REDACTION_TOKEN, text)
    return text


class RedactingLogFilter(logging.Filter):
    """Strip bearer tokens, ``x-api-key`` values, and bare keys from logs.

    A defense-in-depth layer alongside :class:`Config`'s ``__repr__``
    redaction. If anyone accidentally logs a header dict, this filter
    catches it before the formatter writes the line.
    """

    def filter(self, record: logging.LogRecord) -> bool:
        if record.args:
            try:
                record.msg = record.getMessage()
                record.args = ()
            except Exception:  # pragma: no cover (defensive)
                return True
        if isinstance(record.msg, str):
            record.msg = redact_secrets(record.msg)
        return True


def install_redacting_filter() -> None:
    """Attach :class:`RedactingLogFilter` to the root and uvicorn loggers.

    Idempotent — safe to call from the app factory at every boot.

    uvicorn's ``uvicorn.access`` logger has ``propagate=False`` and its own
    handler, so a filter on the root logger never sees its records. The access
    line carries the full request target including any ``?token=`` query, so
    the filter MUST live on that logger directly (a logger runs its own
    filters for records it emits) to scrub the gating secret from
    ``docker logs``.
    """
    for name in ("", "uvicorn.access", "uvicorn"):
        logger = logging.getLogger(name)
        if not any(isinstance(f, RedactingLogFilter) for f in logger.filters):
            logger.addFilter(RedactingLogFilter())
