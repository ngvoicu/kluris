#!/bin/sh
# Fail when the release workflow is missing, or when ci.yml no longer runs
# the installed life cycle (build-versions and lifecycle-smoke, Unix and
# Windows) beside the installer smoke. See CLAUDE.md, Release.
#
#   scripts/release/check_workflows.sh [tree]   default: this checkout
set -eu

root="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
workflows="$root/.github/workflows"
failed=0

if [ ! -f "$workflows/release.yml" ]; then
    echo "check_workflows: no .github/workflows/release.yml" >&2
    failed=1
fi
for needed in scripts/release/build-versions.sh scripts/release/lifecycle-smoke.sh \
    'scripts\release\build-versions.ps1' 'scripts\release\lifecycle-smoke.ps1'; do
    if ! grep -qF "$needed" "$workflows/ci.yml"; then
        printf 'check_workflows: ci.yml does not run %s (the installed life cycle)\n' "$needed" >&2
        failed=1
    fi
done
[ "$failed" -eq 0 ] && echo "check_workflows: release.yml is there; ci.yml runs the life cycle"
exit "$failed"
