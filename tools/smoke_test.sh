#!/usr/bin/env bash
# Cross-repository smoke test: boot every machine headless with the real ROMs from the
# git-ignored machines/<family>/roms folders (your own dumps; a case whose ROM is missing is
# skipped) and compare the screenshot hashes with tools/smoke/baseline.sha256.
# tools/fetch_roms.sh fills those folders from your RomM server.
#
#   tools/smoke_test.sh [--debug] [--no-build] [--update] [--families a,b]
#
#   --debug       use the debug build (default: release)
#   --no-build    do not run tools/build_all.sh first (use what is in target/<profile>)
#   --update      write the current hashes to the baseline instead of comparing
#                 (do this on purpose after a change that is meant to alter a picture)
#   --families    only build/check these families (smoke cases of other families are skipped)
#
# It proves: the app finds and loads the libraries every family builds, each machine's ROM
# loads, and the emulated machine still draws the same boot screen. Screenshots go to
# target/smoke/<case>.png. Exit status 1 on a mismatch or a failed boot.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
# Git Bash on Windows: hand native programs (cargo, the app) C:/... style paths.
command -v cygpath >/dev/null 2>&1 && ROOT="$(cygpath -m "$PWD")"

PROFILE=release; BUILD=1; UPDATE=0; FAMILIES="sinclair,amstrad,commodore"; BUILD_FLAGS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) PROFILE=debug; BUILD_FLAGS+=(--debug); shift ;;
    --no-build) BUILD=0; shift ;;
    --update) UPDATE=1; shift ;;
    --families) FAMILIES="$2"; shift 2 ;;
    *) echo "usage: $0 [--debug] [--no-build] [--update] [--families a,b]" >&2; exit 2 ;;
  esac
done

if [[ $BUILD -eq 1 ]]; then
  tools/build_all.sh ${BUILD_FLAGS[@]+"${BUILD_FLAGS[@]}"} --families "$FAMILIES" >/dev/null
fi

# The app uses shared/ (retroclawd-crates) by path; each family pins a commit of it. Different
# commits are normal between bumps, but then the libraries were built against other shared code than
# the app: say so (tools/bump_pin.sh brings them together).
SHARED_HEAD="$(git -C shared rev-parse HEAD 2>/dev/null || true)"
for f in sinclair amstrad commodore; do
  [[ -d machines/$f ]] || continue
  pin="$(git -C machines/$f grep -ho 'retroclawd-crates", rev = "[0-9a-f]*"' -- '*Cargo.toml' 2>/dev/null | grep -o '[0-9a-f]\{40\}' | sort -u | head -1 || true)"
  if [[ -n $pin && -n $SHARED_HEAD && $pin != "$SHARED_HEAD" ]]; then
    echo "note: machines/$f pins retroclawd-crates ${pin:0:10}, the app uses shared/ at ${SHARED_HEAD:0:10} (tools/bump_pin.sh aligns them)"
  fi
done

APP="$ROOT/target/$PROFILE/retroclawd"
[[ -x "$APP" || -x "$APP.exe" ]] || { echo "no app at $APP (run without --no-build)" >&2; exit 1; }
export RETROCLAWD_MACHINE_PLUGIN_DIRS="$ROOT/target/$PROFILE/plugins"
OUT="$ROOT/target/smoke"; rm -rf "$OUT"; mkdir -p "$OUT"
BASELINE="$ROOT/tools/smoke/baseline.sha256"

sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

S=machines/sinclair/roms; A=machines/amstrad/roms; C=machines/commodore/roms
# The C128 wants BASIC low and high as one 32K file.
if [[ -f $C/c128/basiclo-318018-04.bin ]]; then
  cat $C/c128/basiclo-318018-04.bin $C/c128/basichi-318019-04.bin > "$OUT/c128-basic.bin"
fi

# case-name | family | max cycles (or Nf = N frames) | machine + arguments
CASES=(
  "zx81|sinclair|100f|zx81 --rom $S/zx81/zx81-550-original.rom"
  "zx-spectrum-48|sinclair|2000000|zx-spectrum --rom $S/zx-spectrum/spec48.rom"
  "zx-spectrum-128|sinclair|3000000|zx-spectrum-128 --rom $S/zx-spectrum/128.rom"
  "cpc464|amstrad|4000000|cpc464 --rom $A/cpc/cpc464.rom"
  "cpc6128|amstrad|4000000|cpc6128 --rom $A/cpc/cpc6128.rom"
  "c64|commodore|2000000|c64 --rom-kernal $C/c64/kernal-901227-03.bin --rom-basic $C/c64/basic-901226-01.bin --rom-char $C/c64/chargen-901225-01.bin"
  "c128|commodore|6000000|c128 --rom-basic $OUT/c128-basic.bin --rom-kernal $C/c128/kernal-318020-05.bin --rom-char $C/c128/chargen-390059-01.bin --rom-c64-basic $C/c128/basic64-901226-01.bin --rom-c64-kernal $C/c128/kernal64-901227-03.bin"
)

declare -A OLD
if [[ -f $BASELINE && $UPDATE -eq 0 ]]; then
  while read -r name hash; do hash="${hash%$'\r'}"; [[ -n ${name:-} ]] && OLD[$name]=$hash; done < "$BASELINE"
fi

fail=0; NEW=()
for entry in "${CASES[@]}"; do
  IFS='|' read -r name fam cycles args <<< "$entry"
  [[ ",$FAMILIES," == *",$fam,"* ]] || continue
  png="$OUT/$name.png"
  first_rom="$(grep -oE "machines/[^ ]+" <<< "$args" | head -1)"
  if [[ -n $first_rom && ! -f $first_rom ]]; then echo "skip  $name: $first_rom not present"; continue; fi
  # shellcheck disable=SC2086
  # "100f" = 100 whole frames (a ROM that HALTs for the next frame, like the ZX81's, needs them); a plain number
  # is --max-cycles.
  if [[ $cycles == *f ]]; then run_limit=(--frames "${cycles%f}"); else run_limit=(--max-cycles "$cycles"); fi
  if ! "$APP" $args --headless "${run_limit[@]}" --dump-screen "$png" >"$OUT/$name.log" 2>&1; then
    echo "FAIL  $name: the app did not boot the machine (see target/smoke/$name.log)"; fail=1; continue
  fi
  hash="$(sha "$png")"; NEW+=("$name $hash")
  if [[ $UPDATE -eq 1 ]]; then echo "set   $name $hash"
  elif [[ -z ${OLD[$name]:-} ]]; then echo "NEW   $name $hash (no baseline; run with --update)"; fail=1
  elif [[ ${OLD[$name]} != "$hash" ]]; then echo "DIFF  $name: picture changed (target/smoke/$name.png)"; fail=1
  else echo "ok    $name"; fi
done

if [[ $UPDATE -eq 1 ]]; then
  # keep baseline lines for cases that were not run
  { [[ -f $BASELINE ]] && while read -r n h; do
      keep=1; for l in "${NEW[@]}"; do [[ ${l%% *} == "$n" ]] && keep=0; done
      [[ $keep -eq 1 && -n ${n:-} ]] && echo "$n $h"
    done < "$BASELINE"
    printf '%s\n' "${NEW[@]}"; } | sort > "$BASELINE.tmp" && mv "$BASELINE.tmp" "$BASELINE"
  echo "baseline written: tools/smoke/baseline.sha256"
  exit $fail
fi
[[ $fail -eq 0 ]] && echo "smoke test passed" || { echo "smoke test FAILED" >&2; exit 1; }
