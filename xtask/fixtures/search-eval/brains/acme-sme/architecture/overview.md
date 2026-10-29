---
created: '2026-01-10'
parent: ./map.md
related:
- ./job-queue.md
- ./caching.md
- ./auth-tokens.md
tags:
- architecture
updated: '2026-05-20'
---
# System overview

The platform is one Spring Boot API, one React web app and a worker process
that runs background jobs. All three share a single Postgres database.

## Components

- **api**: the public REST API used by the web app and the mobile apps.
- **web**: the React single-page app; it talks only to the api.
- **worker**: pulls jobs from the queue table and runs them (emails, reminders,
  invoice generation).

## Data flow

A client books a session in the web app. The api writes the booking, then
enqueues a reminder job. The worker sends the reminder a day before the
session. Payments are handled by the billing provider and reported back by
webhook.

## Where to look next

The queue is described in the job queue note; caching rules live in the
caching note; tokens and keys in the auth tokens note.
