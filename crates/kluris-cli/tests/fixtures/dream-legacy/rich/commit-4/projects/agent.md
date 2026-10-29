---
base: &id001
- x
- y
copy: *id001
created: '2026-01-01'
emoji: "\U0001F680 launch"
long: a long line that goes past the eighty column limit so that the writer folds
  it onto two lines
nested:
- 2026-01-03
parent: ./map.md
related:
- ./alpha.md
tags:
- auth
- jwt
- true
- false
- 2026
- 1.1
title: Agent written
updated: '2026-03-04'
when: '2026-01-02T10:00:00+00:00'
---

# Agent written

Text with [alpha](./alpha.md).
