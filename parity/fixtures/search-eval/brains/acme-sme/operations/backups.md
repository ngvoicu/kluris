---
created: '2026-01-05'
parent: ./map.md
related:
- ./operator-access.md
tags:
- postgres
- backups
updated: '2026-05-15'
---
# Database backups and restore drills

## Nightly dump

At 02:00 UTC a job runs `pg_dump` of the application database and uploads the
compressed file to object storage, kept for 35 days.

## Roles are not in the dump

`pg_dump` copies one database: tables, data and grants, but not the cluster's
roles. Roles and their passwords are dumped separately with
`pg_dumpall --roles-only` into the same bucket, once a week and after every
role change.

## Restore drill

Once a month the on-call person restores last night's dump into a scratch
server, runs the smoke tests against it, and writes the time it took in the
drill log. The last drill took 22 minutes.
