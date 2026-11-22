#!/usr/bin/env bash
# Stages and archives a release for the current platform.
#
#   package_release.sh [--dev]
#
# Builds `rcd` (the player: every machine, no debugging code) and `rcdd` (the same with the `debug` feature) and
# lays them out as
#
#   retroclawd-<version>-<target>/
#     rcd[.exe]
#     rcdd[.exe]
#     VERSION.txt     what `rcd --list-plugins` reports (the machines inside)
#     SHA256SUMS
#
# then archives it as dist/retroclawd-<version>-<target>.{tar.gz,zip}. No machine libraries: the machines are
# part of the executables. Verified on Linux only; the Windows and macOS branches follow the same shape but have
# not been run.
set -euo pipefail
cd "$(dirname "$0")/../.."

PROFILE=release
FLAGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dev) PROFILE=debug; FLAGS=(--dev); shift ;;
    *) echo "usage: $0 [--dev]" >&2; exit 2 ;;
  esac
done

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' apps/retroclawd/Cargo.toml | head -1)"
case "$TARGET" in
  *windows*) EXE=.exe; ARCHIVE=zip ;;
  *)         EXE=;     ARCHIVE=tar ;;
esac

tools/build_all.sh ${FLAGS[@]+"${FLAGS[@]}"}

NAME="retroclawd-${VERSION}-${TARGET}"
STAGE="dist/${NAME}"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "target/${PROFILE}/rcd${EXE}" "target/${PROFILE}/rcdd${EXE}" "$STAGE/"

# The staged executables must work on their own.
( cd "$STAGE" && "./rcd${EXE}" --list-plugins ) > "$STAGE/VERSION.txt"
grep -q '^Host' "$STAGE/VERSION.txt"
( cd "$STAGE" && "./rcdd${EXE}" --list-plugins >/dev/null )

( cd "$STAGE" && find . -type f ! -name SHA256SUMS | sort | xargs sha256sum > SHA256SUMS )

mkdir -p dist
if [[ $ARCHIVE == zip ]]; then
  OUT="dist/${NAME}.zip"
  rm -f "$OUT"
  if command -v zip >/dev/null 2>&1; then
    ( cd dist && zip -qr "${NAME}.zip" "${NAME}" )
  else
    powershell -NoProfile -Command "Compress-Archive -Path 'dist/${NAME}' -DestinationPath '${OUT}'"
  fi
else
  OUT="dist/${NAME}.tar.gz"
  tar -C dist -czf "$OUT" "${NAME}"
fi
echo "wrote $OUT"
