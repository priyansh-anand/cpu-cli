#!/bin/sh
# Fails if `cargo package` would ship any path outside the published allowlist, so a stray file
# in the checkout (a note, a local plan, a dump) can never reach crates.io.
# Usage: scripts/check-package.sh
set -eu
list=$(cargo package --list --allow-dirty --locked)
allowed='^(src/|tests/|data/|schema/|scripts/|docs/[^/]+\.md$|\.github/ISSUE_TEMPLATE/[^/]+\.md$|build\.rs$|Cargo\.(toml|toml\.orig|lock)$|\.cargo_vcs_info\.json$|LICENSE$|README\.md$|CHANGELOG\.md$|CONTRIBUTING\.md$)'
bad=$(printf '%s\n' "$list" | grep -vE "$allowed" || true)
if [ -n "$bad" ]; then
    echo "check-package: these files would be published:" >&2
    printf '%s\n' "$bad" >&2
    exit 1
fi
echo "check-package: $(printf '%s\n' "$list" | wc -l | tr -d ' ') files, all allowlisted"
