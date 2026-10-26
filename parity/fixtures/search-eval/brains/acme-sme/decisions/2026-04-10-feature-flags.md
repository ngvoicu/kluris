---
created: '2026-04-10'
parent: ./map.md
tags:
- decision
updated: '2026-04-10'
---
# Feature flags in a config file

## Decision

Feature flags live in `flags.yml` in the api repository and ship with each
deploy. We do not run a flag service.

## Why

We have fewer than twenty flags and one deploy a day. A hosted flag service
would add a network call to every request and another vendor to pay.

## Consequences

Turning a flag on needs a deploy. A flag is deleted from the file, with its
code path, once it has been on for two weeks.
