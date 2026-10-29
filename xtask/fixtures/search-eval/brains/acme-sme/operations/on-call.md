---
created: '2026-01-03'
parent: ./map.md
related:
- ../architecture/job-queue.md
tags:
- on-call
updated: '2026-05-05'
---
# On-call rota and paging

One developer is on call per week, Monday to Monday. Alerts page through the
phone app; an alert not acknowledged in ten minutes escalates to the team lead.

## Morning checks

Look at the dead-letter table, the error rate panel and the overnight backup
result before 10:00.
