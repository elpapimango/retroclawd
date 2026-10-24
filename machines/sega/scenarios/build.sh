#!/usr/bin/env bash
# Builds every probe ROM in scenarios/probes/ into software/scenarios/ (git-ignored).
# Needs rcasm from retroclawd-tools (RCASM, default ../../../retroclawd-tools/target/release/rcasm).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/../software/scenarios"
rcasm="${RCASM:-$here/../../../../retroclawd-tools/target/release/rcasm}"
mkdir -p "$out"
for src in "$here"/probes/*.s; do
    name="$(basename "$src" .s)"
    "$rcasm" -c 68000 -f bin -o "$out/$name.bin" "$src"
    python -I "$here/mkrom.py" "$out/$name.bin" "$out/$name.md" --name "$name"
done
ls -l "$out"/*.md
