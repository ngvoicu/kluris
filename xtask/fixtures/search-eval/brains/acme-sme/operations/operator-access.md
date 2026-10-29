---
created: '2026-02-15'
parent: ./map.md
related:
- ./backups.md
tags:
- postgres
- security
updated: '2026-05-12'
---
# Operator access to production

Developers never connect to production directly from the internet.

## Reaching the database

Open an SSH tunnel through the bastion host from your workstation:
`ssh -L 5433:db.internal:5432 bastion`, then point your SQL client at
`localhost:5433`. Use your personal read-only role; the admin role is only for
migrations and incidents, with a second person watching.

## Audit

Every session through the bastion is recorded, and the read-only roles are
reviewed each quarter.
