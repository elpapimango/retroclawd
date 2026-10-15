#!/usr/bin/env bash
# Stages and archives a release for the current platform.
#
#   package_release.sh [--debug] [--machines <dir>]... [--firmware <file>]...
#
# Builds the whole workspace (the app and the debugger plugin libraries), then
# lays out docs/plugin-abi.md's "release artifact" convention. This repository
# builds no machine: every machine family lives in its own repository
# (retroclawd-sinclair / -amstrad / -commodore) and builds its own
# `*_machine_plugin` libraries. Pass each family's `target/<profile>` folder
# (or any folder holding its libraries) with --machines to include them:
#
#   retroclawd-<version>-<target>/
#     retroclawd[.exe]
#     plugins/<machine plugin libraries>
#     plugins/<family>.firmware.toml   with --firmware <family's firmware.toml>: what `--firmware auto` matches
#     PLUGINS.txt     what `retroclawd --list-plugins` reports for this layout
#     SHA256SUMS
#
# and archives it as dist/retroclawd-<version>-<target>.{tar.gz,zip}.
# Verified on Linux only; the Windows and macOS branches follow the same shape
# but have not been run.
set -euo pipefail

PROFILE_DIR=release
CARGO_FLAGS=(--release)
MACHINE_DIRS=()
FIRMWARE_FILES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) PROFILE_DIR=debug; CARGO_FLAGS=(); shift ;;
    --machines) [[ $# -ge 2 ]] || { echo "--machines needs a folder" >&2; exit 2; }
                MACHINE_DIRS+=("$2"); shift 2 ;;
    --firmware) [[ $# -ge 2 ]] || { echo "--firmware needs a firmware.toml" >&2; exit 2; }
                FIRMWARE_FILES+=("$2"); shift 2 ;;
    *) echo "usage: $0 [--debug] [--machines <dir>]... [--firmware <file>]..." >&2; exit 2 ;;
  esac
done

CARGO_BIN="cargo"
command -v "$CARGO_BIN" >/dev/null 2>&1 || CARGO_BIN="cargo.exe"

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' apps/retroclawd/Cargo.toml | head -1)"
case "$TARGET" in
  *windows*) EXE=retroclawd.exe; LIB_GLOB='*_machine_plugin.dll'; ARCHIVE=zip ;;
  *apple*)   EXE=retroclawd;     LIB_GLOB='lib*_machine_plugin.dylib'; ARCHIVE=tar ;;
  *)         EXE=retroclawd;     LIB_GLOB='lib*_machine_plugin.so'; ARCHIVE=tar ;;
esac

"$CARGO_BIN" build --workspace "${CARGO_FLAGS[@]}"

NAME="retroclawd-${VERSION}-${TARGET}"
STAGE="dist/${NAME}"
rm -rf "$STAGE"
mkdir -p "$STAGE/plugins"

cp "target/${PROFILE_DIR}/${EXE}" "$STAGE/"
shopt -s nullglob
libs=()
for dir in ${MACHINE_DIRS[@]+"${MACHINE_DIRS[@]}"}; do
  libs+=("$dir"/${LIB_GLOB})
done
if [[ ${#libs[@]} -eq 0 ]]; then
  echo "note: no machine plugin libraries given (--machines <dir>); the archive holds the app only" >&2
else
  cp "${libs[@]}" "$STAGE/plugins/"
fi

# Each family's firmware.toml is staged as <family>.firmware.toml; the family is the
# name of the directory holding it (machines/sinclair/firmware.toml -> sinclair.firmware.toml).
for f in ${FIRMWARE_FILES[@]+"${FIRMWARE_FILES[@]}"}; do
  family="$(basename "$(cd "$(dirname "$f")" && pwd)")"
  cp "$f" "$STAGE/plugins/${family}.firmware.toml"
done

# The staged layout must work on its own: list what the app finds there.
( cd "$STAGE" && "./${EXE}" --list-plugins ) | sed "s#${PWD}/${STAGE}/##g" > "$STAGE/PLUGINS.txt"
grep -q '^host compatibility: ' "$STAGE/PLUGINS.txt"
for lib in "${libs[@]}"; do
  base="$(basename "$lib")"
  grep -qF "$base" "$STAGE/PLUGINS.txt" || {
    echo "the staged app did not load $base:" >&2
    cat "$STAGE/PLUGINS.txt" >&2
    exit 1
  }
done

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
