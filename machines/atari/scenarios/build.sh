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
echo "built $out"
