#!/bin/sh
# Fail when Kluris 3's tree could publish to PyPI: v2's
# .github/workflows/publish.yml is back, or any workflow uses PyPI's publish
# action. Also fail when ci.yml no longer runs the installed life cycle
# (build-versions and lifecycle-smoke, Unix and Windows) beside the
# installer smoke. release.yml keeps dist's own tag trigger, which matches v2 tags too;
# v3 stays off PyPI because no workflow in this tree can publish there (GitHub
# runs the workflows of the tagged commit). See CLAUDE.md, Release.
#
#   scripts/release/check_workflows.sh [tree]   default: this checkout
set -eu

root="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
workflows="$root/.github/workflows"
failed=0

if [ -e "$workflows/publish.yml" ] || [ -e "$workflows/publish.yaml" ]; then
    echo "check_workflows: .github/workflows/publish.yml is v2's PyPI release: it has no place in Kluris 3" >&2
    failed=1
fi
for file in "$workflows"/*.yml "$workflows"/*.yaml; do
    [ -f "$file" ] || continue
    if grep -n -i -E 'pypa/gh-action-pypi-publish|twine upload|hatch publish|upload\.pypi\.org' "$file" >&2; then
        echo "check_workflows: ${file#"$root"/} publishes to PyPI" >&2
        failed=1
    fi
done
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
[ "$failed" -eq 0 ] && echo "check_workflows: no workflow publishes to PyPI; ci.yml runs the life cycle"
exit "$failed"
