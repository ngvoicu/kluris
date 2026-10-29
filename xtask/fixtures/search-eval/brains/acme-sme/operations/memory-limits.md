---
created: '2026-02-10'
parent: ./map.md
tags:
- containers
- incidents
updated: '2026-05-18'
---
# Container memory limits

Each container runs with a hard memory limit: 1 GiB for the api, 512 MiB for
the worker.

## OOM kills

When the JVM heap plus native memory passes the limit, the kernel's OOM killer
ends the process and the orchestrator restarts it. The restart shows as exit
code 137 in the container events, with no Java stack trace.

## What fixed it last time

The worker was killed every few hours while generating large invoice PDFs. We
capped the heap at 60% of the limit with `-XX:MaxRAMPercentage=60`, streamed
the PDF to storage instead of building it in memory, and the kills stopped.
Raising the limit was not needed.
