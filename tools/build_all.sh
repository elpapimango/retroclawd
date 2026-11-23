#!/usr/bin/env bash
# One command to build the emulator: `rcd` (play) and `rcdd` (develop).
#
#   tools/build_all.sh [--dev] [--only rcd|rcdd] [--features a,b] [--tools]
#   tools/build_all.sh [--dev] --variant a|c|h
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
#   --variant K  build only the single-kind program: rcda (arcade: SNK), rcdc (console: Sega, SNK) or rcdh (home
#                computers: Sinclair, Amstrad, Commodore, Atari); no debug code, like rcd
#   --tools      also get the tool suite (rcasm, rcdis, rcdisk, rctape, rcsnap, rcld, rcchd: the separate retroclawd-tools project,
#                installed from crates.io into target/rc-tools and copied next to rcd; RC_TOOLS_SRC=<checkout> builds a local copy)
#                and build rc-scenarios (the scenario runner, in this repository)
#
# Works in Git Bash on Windows, Linux and macOS.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
command -v cygpath >/dev/null 2>&1 && ROOT="$(cygpath -m "$PWD")"

PROFILE=release; FLAGS=(--release); ONLY=""; FEATURES=""; TOOLS=0; VARIANT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dev) PROFILE=debug; FLAGS=(); shift ;;
    --only) ONLY="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    --variant) VARIANT="$2"; shift 2 ;;
    --tools) TOOLS=1; shift ;;
    *) echo "usage: $0 [--dev] [--only rcd|rcdd] [--features a,b] [--tools] [--variant a|c|h]" >&2; exit 2 ;;
  esac
done

if [[ -n $VARIANT ]]; then
  case "$VARIANT" in
    a) BIN=rcda; FEAT=variant-arcade ;;
    c) BIN=rcdc; FEAT=variant-console ;;
    h) BIN=rcdh; FEAT=variant-home ;;
    *) echo "--variant takes a, c or h" >&2; exit 2 ;;
  esac
  EXE=""; case "$(rustc -vV | sed -n 's/^host: //p')" in *windows*) EXE=".exe" ;; esac
  echo "== $BIN ($PROFILE)"
  cargo build ${FLAGS[@]+"${FLAGS[@]}"} -p retroclawd --bin "$BIN" --no-default-features --features "$FEAT" --target-dir "$ROOT/target/$PROFILE-$VARIANT"
  mkdir -p "target/$PROFILE"
  cp -f "target/$PROFILE-$VARIANT/$PROFILE/$BIN$EXE" "target/$PROFILE/$BIN$EXE"
  ls -la "target/$PROFILE/$BIN$EXE"
  exit 0
fi

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
  RCTOOLS=(rcasm rcdis rcdisk rcchd rctape rcsnap rcld)
  if [[ -n ${RC_TOOLS_SRC:-} ]]; then
    for t in "${RCTOOLS[@]}"; do cargo install --path "$RC_TOOLS_SRC/crates/$t" --root "$ROOT/target/rc-tools" --force; done
  else
    RC_TOOLS_VERSION="${RC_TOOLS_VERSION:-0.2.0}"   # exact version, bump on purpose
    cargo install "${RCTOOLS[@]/%/@$RC_TOOLS_VERSION}" --root "$ROOT/target/rc-tools" --locked
  fi
  mkdir -p "target/$PROFILE"
  for t in "${RCTOOLS[@]}"; do cp -f "target/rc-tools/bin/$t$EXE" "target/$PROFILE/$t$EXE"; done
  cargo build ${FLAGS[@]+"${FLAGS[@]}"} -p rc-suite
fi

echo
echo "built:"
ls -la "target/$PROFILE"/rcd"$EXE" "target/$PROFILE"/rcdd"$EXE" 2>/dev/null || true
