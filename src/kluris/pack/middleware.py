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


def redact_secrets(text: str) -> str:
    """Scrub bearer tokens, ``x-api-key`` values, bare ``sk-`` keys, and JWTs.

    Used both by the log filter and by the provider's error-mapping layer
    before any provider message reaches the chat UI.
    """
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
    """Attach :class:`RedactingLogFilter` to the root logger.

    Idempotent — safe to call from the app factory at every boot.
    """
    root = logging.getLogger()
    if not any(isinstance(f, RedactingLogFilter) for f in root.filters):
        root.addFilter(RedactingLogFilter())
