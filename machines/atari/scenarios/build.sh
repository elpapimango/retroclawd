#!/usr/bin/env bash
# Assemble the Atari 800 raw Program. Generated Media stays out of git.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tools="${1:-$here/../../../../retroclawd-tools/target/debug}"
out="$here/../software/scenarios"
mkdir -p "$out"
"$tools/rcasm" "$here/atari800/program.asm" -f bin -o "$out/atari800-program.bin" --force
