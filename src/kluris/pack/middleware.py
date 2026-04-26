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

# Strings that must never reach a log handler.
_REDACTION_TOKEN = "***"
_BEARER_PATTERN = re.compile(r"(Bearer\s+)\S+", re.IGNORECASE)
_X_API_KEY_PATTERN = re.compile(r"(x-api-key:\s*)\S+", re.IGNORECASE)


class RedactingLogFilter(logging.Filter):
    """Strip bearer tokens and ``x-api-key`` values from log messages.

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
            record.msg = _BEARER_PATTERN.sub(r"\1" + _REDACTION_TOKEN, record.msg)
            record.msg = _X_API_KEY_PATTERN.sub(r"\1" + _REDACTION_TOKEN, record.msg)
        return True


def install_redacting_filter() -> None:
    """Attach :class:`RedactingLogFilter` to the root logger.

    Idempotent — safe to call from the app factory at every boot.
    """
    root = logging.getLogger()
    if not any(isinstance(f, RedactingLogFilter) for f in root.filters):
        root.addFilter(RedactingLogFilter())
