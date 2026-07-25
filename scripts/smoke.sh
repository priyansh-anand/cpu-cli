#!/bin/sh
# Live smoke test of a built `cpu` binary on this machine: every output mode runs, the JSON
# identifies a CPU, and a dump replays exactly like the live machine. It checks nothing about
# clocks or caches, which CI virtual machines often don't report.
# A CPU counts as identified the way `cpu` itself decides it: a name or a logical CPU count (a
# Linux guest on Apple Silicon has no name, because Apple's hypervisor hides the core type).
# Set SMOKE_REQUIRE_NAME=1 to also require a name, as CI does on its known runners.
# Usage: scripts/smoke.sh path/to/cpu [directory to keep the dump in]
set -eu
cpu=$1
out=${2:-$(mktemp -d)}
mkdir -p "$out"
fail() {
    echo "smoke: $*" >&2
    exit 1
}

"$cpu" --color always > /dev/null || fail "boxed output failed"
"$cpu" --plain > /dev/null || fail "--plain failed"
"$cpu" --explain > /dev/null || fail "--explain failed"
"$cpu" --json > "$out/live.json" || fail "--json failed"
SMOKE_REQUIRE_NAME=${SMOKE_REQUIRE_NAME:-0} python3 - "$out/live.json" <<'PY' || fail "--json does not identify a CPU"
import json, os, sys
cpu = json.load(open(sys.argv[1]))
assert cpu["schema_version"] == 1, cpu.get("schema_version")
name = cpu["identity"].get("name", {}).get("value", "").strip()
logical = cpu.get("topology", {}).get("logical_cpus", {}).get("value")
assert name or logical, "neither a name nor a CPU count"
assert name or os.environ["SMOKE_REQUIRE_NAME"] != "1", "no CPU name, and SMOKE_REQUIRE_NAME=1"
print(f"smoke: {name or 'unnamed CPU'}, {logical} logical CPUs")
PY
rm -f "$out/cpu-dump.tar.gz"
"$cpu" --dump "$out/cpu-dump.tar.gz" > /dev/null || fail "--dump failed"
"$cpu" --json --from "$out/cpu-dump.tar.gz" > "$out/replay.json" || fail "--from failed"
cmp -s "$out/live.json" "$out/replay.json" || fail "the dump does not replay like the live machine"
echo "smoke: ok"
