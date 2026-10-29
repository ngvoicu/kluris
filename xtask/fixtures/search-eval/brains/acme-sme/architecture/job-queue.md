---
created: '2026-01-12'
parent: ./map.md
related:
- ./overview.md
- ../operations/on-call.md
tags:
- queue
- worker
updated: '2026-05-02'
---
# Job queue

Background work runs through a queue table in Postgres, polled by the worker
every two seconds. We chose a table over a broker so a job and the row that
caused it commit in the same transaction.

## Retries

A job that throws is retried with exponential backoff: after 1, 4, 16 and 64
minutes. Each attempt is recorded with its stack trace in `job_attempts`.

## Dead letters

After the fifth failed attempt the job moves to the DLQ, the dead-letter table,
and stops retrying. Nothing in the DLQ runs again by itself: the on-call person
reviews it every morning and either requeues the job with `jobs requeue <id>`
or discards it with a note.

## Idempotency

Every job must be safe to run twice, because a worker can die after the work
but before marking the job done. Use the booking id as an idempotency key when
calling external services.
