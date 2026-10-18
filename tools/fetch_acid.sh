#!/usr/bin/env bash
# Copies the third-party acid tests the retroclawd-tools suites run (crates/rc-suite) from a folder you already have into the
# git-ignored machines/<family>/software/acid/ folders, verified by SHA-256. They have their own licences: they are copied
# here for local testing, never committed.
#
#   tools/fetch_acid.sh --from <folder> [--force]
#
#   <folder> = the cpcwiki "Acid Tests Suite" (Kevin Thacker) unpacked, e.g. C:\Users\you\Workspace\Emulators\Acid_Tests_Suite.
#   Patrik Rak's z80test 1.0 (MIT) is inside it: z80tests/patrik_z80test/spec/z80test-1.0/*.tap, run on the Spectrum 48K.
#   The CPC programs that print PASS/FAIL (ppi, psg, cpc, cpcrom, crtc2 = type1ctrl, plus/asic1) are the disks Kevin Thacker built: standard
#   CPCEMU .dsk files whose header reads "MV - CPCEMU / <date>" instead of the long form (our reader accepts both).
#   The CPC programs that print PASS/FAIL (ppi, psg, cpc, cpcrom, crtc2 (type1ctrl), plus/asic1) are the disks Kevin Thacker built; the disks are
#   standard CPCEMU .dsk files whose header reads "MV - CPCEMU / <date>" instead of the long form (our reader accepts both).
#
# Exits 1 if a file is missing or does not hash to the pinned value.
set -euo pipefail
cd "$(dirname "$0")/.."

FROM=""; FORCE=0
while [[ $# -gt 0 ]]; do
  case $1 in
    --from) FROM=${2:?--from needs a folder}; shift 2 ;;
    --force) FORCE=1; shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
[[ -n $FROM && -d $FROM ]] || { echo "usage: tools/fetch_acid.sh --from <Acid_Tests_Suite folder> [--force]" >&2; exit 2; }
command -v cygpath >/dev/null 2>&1 && FROM="$(cygpath -u "$FROM")"

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

Z=z80tests/patrik_z80test/spec/z80test-1.0
# path inside the folder | SHA-256 | destination
ITEMS=(
  "$Z/z80full.tap|625bba357fe565775112369eaf1355ae03d220bfdce5a7999cf74654b92939f3|machines/sinclair/software/acid/z80test/z80full.tap"
  "$Z/z80doc.tap|165def91c279943d3e3673a624f3692763621aa982f4a0ca3826df4427ccad6c|machines/sinclair/software/acid/z80test/z80doc.tap"
  "$Z/z80flags.tap|3b36e2d3186d6dcae2403c3e216c618307d382a2f98905beb60e1525afd0b9fa|machines/sinclair/software/acid/z80test/z80flags.tap"
  "$Z/z80docflags.tap|ccbd19a7d1e72e4f029717fbb530e5d03e47505d5259f54c765ba9e69d33f3b0|machines/sinclair/software/acid/z80test/z80docflags.tap"
  "$Z/z80ccf.tap|ca7d3e1857f01cef82d8caae80233753e9e287868ae170b40f83d37c90ef5ee2|machines/sinclair/software/acid/z80test/z80ccf.tap"
  "$Z/z80memptr.tap|5a31da7ee7e9917bde36f8340b2ea8f06f8c145053a3537f25853ef14ea6b445|machines/sinclair/software/acid/z80test/z80memptr.tap"
  "$Z/license.txt|8b55946b2cbf5ed0d7c14d07e589a1eae53203268e83b9d87c9f7ebf230cd521|machines/sinclair/software/acid/z80test/license.txt"
  "ppi/ppi.dsk|0a2f6d428a1eb83419983d80c0aaf1a8bd0a0a392440867bfc39ada9c8a185e2|machines/amstrad/software/acid/ppi/ppi.dsk"
  "psg/psg.dsk|1115b28b5cb5e87549650de3bd22ef0662ef690dd42ef92d38b885808976f8bf|machines/amstrad/software/acid/psg/psg.dsk"
  "cpc/cpc.dsk|589be8fb54c351398aad930c82eedb41cefd9920abe6ce870281acc11822cb9a|machines/amstrad/software/acid/cpc/cpc.dsk"
  "cpcrom/romtest.dsk|5c0ea73d83b99953d0b8c69cef8f4cccaebc1a6c56d7f17dfe250d8870350173|machines/amstrad/software/acid/cpcrom/romtest.dsk"
  "crtc/crtc2.dsk|6ac07ad343b687d74ec60d728c25d7f8753f40159d34805a9f9995de8176ab28|machines/amstrad/software/acid/crtc/crtc2.dsk"
  "plus/asic1.dsk|b60b1ab30d82581812387ab79db7540b8205b224e299895a58ee465395a946c1|machines/amstrad/software/acid/plus/asic1.dsk"
)

status=0
for item in "${ITEMS[@]}"; do
  IFS='|' read -r rel want dest <<<"$item"
  src="$FROM/$rel"
  if [[ $FORCE -eq 0 && -f $dest && $(sha256 "$dest") == "$want" ]]; then echo "have     $dest"; continue; fi
  if [[ ! -f $src ]]; then echo "MISSING  $src"; status=1; continue; fi
  got=$(sha256 "$src")
  if [[ $got != "$want" ]]; then echo "MISMATCH $src ($got, pinned $want)"; status=1; continue; fi
  mkdir -p "$(dirname "$dest")"; cp "$src" "$dest"; echo "copied   $dest"
done
exit $status
