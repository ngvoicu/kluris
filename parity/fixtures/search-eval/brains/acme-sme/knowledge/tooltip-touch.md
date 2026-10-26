---
created: '2026-04-01'
parent: ./map.md
tags:
- frontend
- accessibility
updated: '2026-04-01'
---
# Tooltips on touch screens

The component library's tooltip opens on hover and keyboard focus only. On a
phone or tablet there is no hover, so a tooltip never appears when tapped.

## What we do

Never put information only in a tooltip. Use an info button that opens a
popover on tap, and keep the tooltip as an extra for mouse users.
