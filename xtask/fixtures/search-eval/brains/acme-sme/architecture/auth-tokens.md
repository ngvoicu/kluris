---
created: '2026-01-15'
parent: ./map.md
related:
- ../knowledge/deprecated-sessions.md
- ./overview.md
tags:
- auth
- security
updated: '2026-05-28'
---
# Auth tokens

The api issues short-lived JWT access tokens (15 minutes) and refresh tokens
(30 days). Mobile apps and the web app use the same flow.

## Signing keys

Tokens are signed with an RS256 key pair. The private key lives in the secrets
store, never in the repository. A new key pair is generated on the first of
every quarter; the previous public key stays published in the JWKS endpoint for
one more month so tokens signed with it keep verifying until they expire.

## Rotation steps

1. Generate the new pair with `ops keys new`.
2. Publish the new public key next to the old one.
3. Switch signing to the new private key.
4. After a month, remove the old public key.

## Third-party login

Google sign-in goes through OAuth2; we map the Google subject id to our user
id on first login.
