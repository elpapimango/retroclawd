#!/usr/bin/env bash
# Builds every probe ROM in scenarios/probes/ into software/scenarios/ (git-ignored).
# Needs rcasm from retroclawd-tools (RCASM, default ../../../retroclawd-tools/target/release/rcasm).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/../software/scenarios"
rcasm="${RCASM:-$here/../../../../retroclawd-tools/target/release/rcasm}"
mkdir -p "$out"
for src in "$here"/probes/*.s; do
    case "$src" in *.z80.s) continue ;; esac
    name="$(basename "$src" .s)"
    if [ -f "$here/probes/$name.z80.s" ]; then
        "$rcasm" -c z80 -f bin --force -o "$out/$name.z80.bin" "$here/probes/$name.z80.s"
        python -I "$here/bin2dc.py" "$out/$name.z80.bin" "$out/$name.z80.inc"
    fi
    "$rcasm" -c 68000 -f bin --force -I "$out" -o "$out/$name.bin" "$src"
    extra=$(sed -n 's/^; mkrom: *//p' "$src" | tr '
' ' ')
    # shellcheck disable=SC2086
    python -I "$here/mkrom.py" "$out/$name.bin" "$out/$name.md" --name "$name" $extra
done
ls -l "$out"/*.md
