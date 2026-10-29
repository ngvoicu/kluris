---
created: '2026-01-08'
parent: ./map.md
related:
- ./on-call.md
tags:
- deploy
updated: '2026-05-25'
---
# Deploy runbook

## Steps

1. Merge to `main`; CI builds the images and runs the migrations check.
2. Start the deploy from the pipeline page; it rolls one container at a time.
3. Each old container gets SIGTERM, finishes its open requests within 30
   seconds and exits; the worker finishes its current job first.
4. Watch the error rate panel for ten minutes.

## Rolling back

Pick the previous image in the pipeline and deploy it. Migrations are always
backward compatible for one release, so the old image runs against the new
schema.

## Freeze

No deploys on Friday afternoon, and only fixes during the freeze before a
release.
