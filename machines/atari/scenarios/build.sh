#!/usr/bin/env bash
# Build raw/XEX/CAR Programs and ATR/XFD SIO boot disks. Media stays out of git.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tools="${1:-$here/../../../../retroclawd-tools/target/debug}"
out="$here/../software/scenarios"
mkdir -p "$out"
"$tools/rcasm" "$here/atari800/program.asm" -f bin -o "$out/atari800-program.bin" --force
"$tools/rcasm" "$here/atari800/boot.asm" -f bin -o "$out/atari800-boot.bin" --force
cp "$here/atari800/program.toml" "$out/atari800-program.toml"
"$tools/rcld" build "$out/atari800-program.toml" --force
"$tools/rcdisk" create "$out/atari800-boot.atr" --sectors 720 --sector-size 128 --force
"$tools/rcdisk" add "$out/atari800-boot.atr" "$out/atari800-boot.bin" --sector 1
"$tools/rcdisk" extract "$out/atari800-boot.atr" --sector 1 --count 720 -o "$out/atari800-boot.xfd" --force
"$tools/rcasm" "$here/atari800/cartridge.asm" -f bin -o "$out/atari800-cartridge.bin" --force
cp "$here/atari800/cartridge.toml" "$out/atari800-cartridge.toml"
"$tools/rcld" build "$out/atari800-cartridge.toml" --force
