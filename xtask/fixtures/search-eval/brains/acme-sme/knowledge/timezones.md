---
created: '2026-01-20'
parent: ./map.md
tags:
- dates
updated: '2026-04-02'
---
# Dates, times and time zones

## Storage

Timestamps are stored in UTC (`timestamptz`), always. A booking also stores
the clinic's IANA zone name, for example `Europe/Bucharest`, so a session at
9:00 stays at 9:00 across a daylight-saving change.

## Display

The web app renders every time in the clinic's zone, never in the browser's
zone, because receptionists often work remotely from another country.

## Pitfall

Never do date arithmetic on local times in SQL: convert to the clinic zone
with `AT TIME ZONE` first, then add days.
