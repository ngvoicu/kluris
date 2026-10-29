---
created: '2025-10-01'
parent: ./map.md
tags:
- auth
updated: '2026-01-15'
status: deprecated
deprecated_at: '2026-01-15'
replaced_by: ../architecture/auth-tokens.md
---
# Server-side sessions

Before January 2026 the web app kept a session id in a cookie and stored the
session in Redis. Sessions expired after eight hours of inactivity.

This was replaced by JWT access and refresh tokens so that the mobile apps and
the web app share one login flow.
