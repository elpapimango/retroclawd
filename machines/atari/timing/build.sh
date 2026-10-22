#!/usr/bin/env bash
# Builds the timing probes (p*.s) into software/scenarios/timing/pNN.st (not committed), each a boot floppy.
#   timing/build.sh [path to the retroclawd-tools target dir]   default ../../../retroclawd-tools/target/debug
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tools="${1:-$here/../../../../retroclawd-tools/target/debug}"
out="$here/../software/scenarios/timing"
mkdir -p "$out"
for src in "$here"/p[0-9][0-9]_*.s; do
  name="$(basename "$src" .s)"
  python "$here/mkprobe.py" "$tools" "$src" "$out/${name%%_*}.st"
done
echo "built $out"
