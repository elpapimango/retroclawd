#!/usr/bin/env bash
# One command to build the app AND the machines.
#
#   tools/build_all.sh [--debug] [--families sinclair,amstrad,commodore] [--update]
#
# The shared crates (retroclawd-crates) are the git submodule `shared/`; the machines live in three
# repositories that are git submodules under machines/.
# This script checks out the submodules it needs, builds the app (this workspace),
# builds each family workspace, and puts every machine library into
# target/<profile>/plugins/ -- where the app looks first -- so afterwards
#
#   target/release/retroclawd zx81 --rom <your ZX81 ROM>
#
# just works. Works in Git Bash on Windows, Linux and macOS.
#   --debug      debug build instead of release
#   --families   comma-separated subset (default: all three)
#   --update     move each submodule (shared/ and the families) to the newest commit of its `main` first
#                (default: the commit this repository records)
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
# Git Bash on Windows: hand native programs (cargo, the app) C:/... style paths.
command -v cygpath >/dev/null 2>&1 && ROOT="$(cygpath -m "$PWD")"

PROFILE=release; FLAGS=(--release); FAMILIES="sinclair,amstrad,commodore"; UPDATE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) PROFILE=debug; FLAGS=(); shift ;;
    --families) FAMILIES="$2"; shift 2 ;;
    --update) UPDATE=1; shift ;;
    *) echo "usage: $0 [--debug] [--families a,b] [--update]" >&2; exit 2 ;;
  esac
done

# The family repositories pull the shared crates from this (private) repository
# by git; use the git command line so your normal git credentials apply.
export CARGO_NET_GIT_FETCH_WITH_CLI=true
# One target folder for all families: the shared crates (z80, machine, ...) compile once.
export CARGO_TARGET_DIR="$ROOT/target/families"

case "$(rustc -vV | sed -n 's/^host: //p')" in
  *windows*) LIB_GLOB='*_machine_plugin.dll' ;;
  *apple*)   LIB_GLOB='lib*_machine_plugin.dylib' ;;
  *)         LIB_GLOB='lib*_machine_plugin.so' ;;
esac

IFS=, read -ra FAM <<< "$FAMILIES"
# The app builds against the shared crates by path, so shared/ is always needed.
if [[ $UPDATE -eq 1 ]]; then git submodule update --init --remote shared; else git submodule update --init shared; fi
for f in "${FAM[@]}"; do
  [[ -d machines/$f ]] || { echo "unknown family: $f" >&2; exit 2; }
  if [[ $UPDATE -eq 1 ]]; then
    git submodule update --init --remote "machines/$f"
  else
    git submodule update --init "machines/$f"
  fi
done

echo "== app + debugger plugins ($PROFILE)"
CARGO_TARGET_DIR="$ROOT/target" cargo build --workspace "${FLAGS[@]}"

PLUGINS="$ROOT/target/$PROFILE/plugins"
rm -rf "$PLUGINS"; mkdir -p "$PLUGINS"
for f in "${FAM[@]}"; do
  echo "== machines/$f ($PROFILE)"
  ( cd "machines/$f" && cargo build --workspace "${FLAGS[@]}" )
  shopt -s nullglob
  libs=("$CARGO_TARGET_DIR/$PROFILE"/$LIB_GLOB)
  shopt -u nullglob
  [[ ${#libs[@]} -gt 0 ]] && cp -f "${libs[@]}" "$PLUGINS/"
  # What --firmware auto matches against RomM (hashes only, no ROMs).
  [[ -f "machines/$f/firmware.toml" ]] && cp -f "machines/$f/firmware.toml" "$PLUGINS/$f.firmware.toml"
done

echo
echo "built: target/$PROFILE/retroclawd with machine libraries in target/$PROFILE/plugins/:"
ls "$PLUGINS"
echo
"$ROOT/target/$PROFILE/retroclawd" --list-plugins | sed -n '1,12p'
