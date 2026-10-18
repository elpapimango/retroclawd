#!/usr/bin/env bash
# Copies the third-party acid tests the retroclawd-tools suites run (crates/rc-suite) from a folder you already have into the
# git-ignored machines/<family>/software/acid/ folders, verified by SHA-256. They have their own licences: they are copied
# here for local testing, never committed.
#
#   tools/fetch_acid.sh --from <folder> [--force]
#
#   <folder> = the cpcwiki "Acid Tests Suite" (Kevin Thacker) unpacked, e.g. C:\Users\you\Workspace\Emulators\Acid_Tests_Suite.
#   Patrik Rak's z80test 1.0 (MIT) is inside it: z80tests/patrik_z80test/spec/z80test-1.0/*.tap, run on the Spectrum 48K.
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
