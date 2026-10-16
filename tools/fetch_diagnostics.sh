#!/usr/bin/env bash
# Downloads the open-source diagnostic programs the retroclawd-tools suites run (crates/rc-suite) into the git-ignored
# machines/<family>/software/diagnostics/ folders, pinned to a release and verified by SHA-256. They are third-party
# software with their own licences: they are fetched here, never committed.
#
#   tools/fetch_diagnostics.sh [--force]
#
#   zx-diagnostics v0.37 (Brendan Alford, Dylan Smith; LGPL-3.0) -- the Spectrum diagnostic ROM (testrom.bin), run as the
#       machine's 16K ROM: https://github.com/brendanalford/zx-diagnostics
#
# Needs curl and python3 (or unzip). HTTP_PROXY / HTTPS_PROXY are honoured by curl. Exits 1 if a download or its check fails.
set -euo pipefail
cd "$(dirname "$0")/.."

FORCE=0
[[ ${1:-} == --force ]] && FORCE=1

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

# name | zip URL | SHA-256 of the zip | entry in the zip | SHA-256 of the entry | destination
ITEMS=(
  "zx-diagnostics v0.37|https://github.com/brendanalford/zx-diagnostics/releases/download/v0.37/Release-v0.37.zip|bdb97f1108623c145fd106cd46d69101c836a7cdffa07d64cf073eda5e726306|testrom.bin|e38354b6b8f393a65a2512cb622c6f255a4d015e9022244a873f0c6c2b0cf464|machines/sinclair/software/diagnostics/zx-diagnostics-testrom.bin"
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
  if ! extract "$tmp/pkg.zip" "$entry" "$dest.part" || [[ $(sha256 "$dest.part") != "$entry_sha" ]]; then
    rm -f "$dest.part"; echo "FAILED   $name ($entry is missing or does not hash to $entry_sha)"; status=1; continue
  fi
  mv "$dest.part" "$dest"; echo "fetched  $name -> $dest"
done
exit $status
