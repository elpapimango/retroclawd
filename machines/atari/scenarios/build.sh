#!/usr/bin/env bash
# Builds the media the Atari ST scenario manifest loads into software/scenarios/ (not committed), with
# retroclawd-tools' rcdisk and make_scenarios.py (see README.md).
#   scenarios/build.sh [path to the retroclawd-tools target dir]   default ../../../retroclawd-tools/target/debug
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tools="${1:-$here/../../../../retroclawd-tools/target/debug}"
out="$here/../software/scenarios"
mkdir -p "$out"
python "$here/make_scenarios.py" "$out"

# keys.st: a blank disk whose boot program echoes keys and shows the joystick in palette 0.
"$tools/rcdisk" create "$out/keys.st" --force
"$tools/rcdisk" boot "$out/keys.st" "$out/keys-boot.bin"

# write.st: a blank disk whose boot program writes logical sector 30 through the BIOS, reads it back and says
# whether the two agree (S3W).
"$tools/rcdisk" create "$out/write.st" --force
"$tools/rcdisk" boot "$out/write.st" "$out/write-boot.bin"

# raster.st: a blank disk whose boot program changes palette entry 0 in an HBL handler, giving coloured bands (S10).
"$tools/rcdisk" create "$out/raster.st" --force
"$tools/rcdisk" boot "$out/raster.st" "$out/raster-boot.bin"

# files.st: two text files to list on the desktop.
"$tools/rcdisk" create "$out/files.st" --force
printf 'Hello from rcdisk.\r\n' > "$out/HELLO.TXT"
printf 'A floppy made by retroclawd-tools for the S3 scenario.\r\n' > "$out/README.TXT"
"$tools/rcdisk" add "$out/files.st" "$out/HELLO.TXT" "$out/README.TXT"
rm "$out/HELLO.TXT" "$out/README.TXT"
# hd.img: an 8M hard disk with an AHDI partition holding HELLO.TXT (S11: EmuTOS shows it as drive C).
"$tools/rcdisk" hd new "$out/hd.img" --size 8M --force
printf 'Hello from a hard disk made by rcdisk.
' > "$out/HELLO.TXT"
"$tools/rcdisk" hd add "$out/hd.img" "$out/HELLO.TXT"
rm "$out/HELLO.TXT"

# ste_tone.bin: the STE DMA sound plays a stereo square wave on the left channel (S18).
"$tools/rcasm" --cpu 68000 -f bin "$here/ste_tone.s" -o "$out/ste_tone.bin" --force

# hello.prg: a GEMDOS program made by rcasm's 68000 back end and rcld (S8P).
"$tools/rcld" build "$here/hello.layout.toml" -o "$out/hello.prg" --force

# The 8-bit Atari 800/XL/XE media: raw/XEX/CAR programs and ATR/XFD SIO boot disks.
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

echo "built $out"
