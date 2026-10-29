---
created: '2026-03-15'
parent: ./map.md
tags:
- ci
- testing
updated: '2026-05-11'
---
# Flaky tests on the macOS CI runner

Browser tests that pass locally sometimes fail on the hosted macOS runner.

## Cause

The runner is slower and shares its CPU, so animations take longer than the
test's two-second wait. A test that clicks a button while the dialog is still
fading in hits the overlay instead.

## Fix

Wait for the element to be stable (`toBeVisible` plus no running animation)
instead of a fixed sleep, and disable CSS transitions in the test build with
`prefers-reduced-motion`. Retrying a failed test once is allowed, but a test
that needs the retry twice in a week is quarantined and gets an issue.
