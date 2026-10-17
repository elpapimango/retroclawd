#!/usr/bin/env bash
# Downloads the open-source diagnostic programs the retroclawd-tools suites run (crates/rc-suite) into the git-ignored
# machines/<family>/software/diagnostics/ folders, pinned to a release and verified by SHA-256. They are third-party
# software with their own licences: they are fetched here, never committed.
#
#   tools/fetch_diagnostics.sh [--force]
#
#   zx-diagnostics v0.37 (Brendan Alford, Dylan Smith; LGPL-3.0) -- the Spectrum diagnostic ROM (testrom.bin), run as the
#       machine's 16K ROM: https://github.com/brendanalford/zx-diagnostics
#   amstrad-diagnostics v1.3 (Lyle Llopis; MIT) -- the CPC/GX4000 diagnostics, as a disk (AmstradDiag.dsk) and a Plus
#       cartridge (AmstradDiag.cpr, which has a malformed header the CPC plugin repairs):
#       https://github.com/llopis/amstrad-diagnostics
#
#   C64 Unified Diagnostic Cartridge (crystalct; CERN-OHL-W-2.0, which licenses the hardware design -- the dead-test code inside is
#       Commodore's own, so this is only ever downloaded for local testing, never committed or redistributed) -- the modified
#       "dead test" and the standard diagnostic rev 586220++, as the cartridge's 32K EPROM image (the standard diagnostic in the
#       first 8K, the dead test, a raw Ultimax ROM for $E000, in the second): https://github.com/crystalct/C64UnifDiagCart
#
# Needs curl and python3 (or unzip). HTTP_PROXY / HTTPS_PROXY are honoured by curl. Exits 1 if a download or its check fails.
set -euo pipefail
cd "$(dirname "$0")/.."

FORCE=0
[[ ${1:-} == --force ]] && FORCE=1

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# name | URL | SHA-256 of the download | entry in the zip (empty: the download is the file itself) | SHA-256 of the entry | destination
ITEMS=(
  "zx-diagnostics v0.37|https://github.com/brendanalford/zx-diagnostics/releases/download/v0.37/Release-v0.37.zip|bdb97f1108623c145fd106cd46d69101c836a7cdffa07d64cf073eda5e726306|testrom.bin|e38354b6b8f393a65a2512cb622c6f255a4d015e9022244a873f0c6c2b0cf464|machines/sinclair/software/diagnostics/zx-diagnostics-testrom.bin"
  "amstrad-diagnostics v1.3 AmstradDiag.cpr|https://github.com/llopis/amstrad-diagnostics/releases/download/v1.3/AmstradDiag.zip|65dbf840e590bbe9ae4ccea934b54180eabf7ada6d54283e30725d3f868e8f92|AmstradDiag.cpr|bf82fff3646eb707a0fbc153bfe78bd5ba55a82ff604be8bf06d3e6211352310|machines/amstrad/software/diagnostics/AmstradDiag.cpr"
  "amstrad-diagnostics v1.3 AmstradDiag.dsk|https://github.com/llopis/amstrad-diagnostics/releases/download/v1.3/AmstradDiag.zip|65dbf840e590bbe9ae4ccea934b54180eabf7ada6d54283e30725d3f868e8f92|AmstradDiag.dsk|a6b5dc9abc9f0dcd3dd74b2ec23862f2f55ccc3a2b6f62505e9a93cdf1a6fe97|machines/amstrad/software/diagnostics/AmstradDiag.dsk"
  "c64-unified-diag Unified_32k|https://raw.githubusercontent.com/crystalct/C64UnifDiagCart/c6c4f7a210d0b35df4ea680d7a0d0ad30faf8b7b/files/Unified_32k.bin|d3b759c3eb777292b25e6278dc8e01f0a8549bfb31c455d24d8fd070124bbf26||d3b759c3eb777292b25e6278dc8e01f0a8549bfb31c455d24d8fd070124bbf26|machines/commodore/software/diagnostics/unified_32k.bin"
)

extract() { # zip entry dest
  if command -v python3 >/dev/null 2>&1; then
    python3 -I -c "import sys,zipfile; open(sys.argv[3],'wb').write(zipfile.ZipFile(sys.argv[1]).read(sys.argv[2]))" "$1" "$2" "$3"
  else
    unzip -p "$1" "$2" > "$3"
  fi
}

status=0
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
for item in "${ITEMS[@]}"; do
  IFS='|' read -r name url zip_sha entry entry_sha dest <<<"$item"
  if [[ $FORCE -eq 0 && -f $dest && $(sha256 "$dest") == "$entry_sha" ]]; then
    echo "have     $name -> $dest"; continue
  fi
  mkdir -p "$(dirname "$dest")"
  if ! curl -sSfL -o "$tmp/pkg.zip" "$url"; then echo "FAILED   $name (download of $url)"; status=1; continue; fi
  if [[ $(sha256 "$tmp/pkg.zip") != "$zip_sha" ]]; then echo "MISMATCH $name (the download does not hash to $zip_sha)"; status=1; continue; fi
  if [[ -z $entry ]]; then # a plain file, not a zip
    mv "$tmp/pkg.zip" "$dest"; echo "fetched  $name -> $dest"; continue
  fi
  if ! extract "$tmp/pkg.zip" "$entry" "$dest.part" || [[ $(sha256 "$dest.part") != "$entry_sha" ]]; then
    rm -f "$dest.part"; echo "FAILED   $name ($entry is missing or does not hash to $entry_sha)"; status=1; continue
  fi
  mv "$dest.part" "$dest"; echo "fetched  $name -> $dest"
done
exit $status
