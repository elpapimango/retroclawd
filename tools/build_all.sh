#!/usr/bin/env bash
# One command to build the emulator: `rcd` (play) and `rcdd` (develop).
#
#   tools/build_all.sh [--dev] [--only rcd|rcdd] [--features a,b] [--tools]
#
#   rcd    every machine, no debugging code at all (the player)
#   rcdd   the same program built with the `debug` feature (debuggers, ZRCP, tracing, watch points, reports)
#
# Both come out in target/<profile>/ (rcd from the normal build, rcdd copied from target/<profile>-debug/,
# a separate cargo target folder so switching between the two never rebuilds everything).
#
#   --dev        a debug-profile build instead of release
#   --only X     build just rcd or just rcdd
#   --features   machine selection, e.g. `home` or `console,arcade` (default: all-machines).
#                `home` = Sinclair, Amstrad, Commodore, Atari; `console` = Sega, SNK; `arcade` = SNK
#   --tools      also build the retroclawd tools (rcasm, rcdis, rcdisk, rctape, rcchd, rc-scenarios, ...)
#
# Works in Git Bash on Windows, Linux and macOS.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
command -v cygpath >/dev/null 2>&1 && ROOT="$(cygpath -m "$PWD")"

PROFILE=release; FLAGS=(--release); ONLY=""; FEATURES=""; TOOLS=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dev) PROFILE=debug; FLAGS=(); shift ;;
    --only) ONLY="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    --tools) TOOLS=1; shift ;;
    *) echo "usage: $0 [--dev] [--only rcd|rcdd] [--features a,b] [--tools]" >&2; exit 2 ;;
  esac
done

SELECT=()
if [[ -n $FEATURES ]]; then SELECT=(--no-default-features --features "$FEATURES"); fi

EXE=""; case "$(rustc -vV | sed -n 's/^host: //p')" in *windows*) EXE=".exe" ;; esac

if [[ -z $ONLY || $ONLY == rcd ]]; then
  echo "== rcd ($PROFILE)"
  cargo build ${FLAGS[@]+"${FLAGS[@]}"} -p retroclawd --bin rcd ${SELECT[@]+"${SELECT[@]}"}
fi
if [[ -z $ONLY || $ONLY == rcdd ]]; then
  echo "== rcdd ($PROFILE, debug feature)"
  if [[ -n $FEATURES ]]; then DBG_SELECT=(--no-default-features --features "debug,$FEATURES"); else DBG_SELECT=(--features debug); fi
  cargo build ${FLAGS[@]+"${FLAGS[@]}"} -p retroclawd --bin rcdd --target-dir "$ROOT/target/$PROFILE-debug" "${DBG_SELECT[@]}"
  mkdir -p "target/$PROFILE"
  cp -f "target/$PROFILE-debug/$PROFILE/rcdd$EXE" "target/$PROFILE/rcdd$EXE"
fi
if [[ $TOOLS -eq 1 ]]; then
  echo "== tools ($PROFILE)"
  cargo build ${FLAGS[@]+"${FLAGS[@]}"} -p rc-common -p rcasm -p rcdis -p rcdisk -p rcchd -p rctape -p rcsnap -p rcld -p rc-suite
fi

echo
echo "built:"
ls -la "target/$PROFILE"/rcd"$EXE" "target/$PROFILE"/rcdd"$EXE" 2>/dev/null || true
