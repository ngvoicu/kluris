---
created: '2026-02-01'
parent: ./map.md
related:
- ./overview.md
tags:
- redis
- performance
updated: '2026-04-11'
---
# Caching

Read-heavy endpoints (practitioner calendars, price lists) are cached in Redis
for five minutes.

## Invalidation

Writes never update the cache in place. A write deletes the affected keys
after its transaction commits, and the next read fills them again. Deleting
before the commit caused stale calendars in March: a reader refilled the key
with the old row while the write was still open.

## Keys

Keys are `cal:<practitioner id>:<week>` and `prices:<clinic id>`. Never cache
anything that depends on the signed-in user.
