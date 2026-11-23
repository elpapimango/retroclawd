#!/usr/bin/env bash
# Every check the project has, in one run: so a change in one place cannot quietly break a case that lives
# somewhere else (a disk-timing change once broke two rc-suite CPC cases that nobody ran).
#
#   tools/check_all.sh [--no-build] [--acid] [--skip-unit]
#
#   --no-build    use what is in target/release (default: tools/build_all.sh --tools first)
#   --acid        also run the CPC acid suite (rc-suite cpc_acid; slow, about 15 minutes)
#   --skip-unit   skip `cargo test --workspace`
#
# In order: the release build of rcd and rcdd and the tools; `cargo test --workspace`; tools/smoke_test.sh; every
# `machines/<family>/*.manifest.toml` through rc-scenarios; and the rc-suite integration cases. The scenario and
# suite steps need the goldens folder (RC_GOLDENS, default ../goldens) and ROM folders; without them those steps
# are skipped and say so. The scenarios and the suite run against rcdd (they use trace and diagnostic options that
# only the debug build has). Every step runs even after a failure; the summary lists what failed and the exit
# status is 1 if anything did.
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
command -v cygpath >/dev/null 2>&1 && ROOT="$(cygpath -m "$PWD")"

BUILD=1; ACID=0; UNIT=1
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build) BUILD=0; shift ;;
    --acid) ACID=1; shift ;;
    --skip-unit) UNIT=0; shift ;;
    *) echo "usage: $0 [--no-build] [--acid] [--skip-unit]" >&2; exit 2 ;;
  esac
done

GOLDENS="${RC_GOLDENS:-$ROOT/../goldens}"
FAILED=()
LOGS="$ROOT/target/check_all"; rm -rf "$LOGS"; mkdir -p "$LOGS"
EXE=""; case "$(rustc -vV | sed -n 's/^host: //p')" in *windows*) EXE=".exe" ;; esac

# step <name> <command...>: runs it, keeps its output in target/check_all/<name>.log, records a failure.
step() {
  local name="$1"; shift
  printf '%-34s ' "$name"
  if "$@" >"$LOGS/$name.log" 2>&1; then echo ok
  else echo "FAILED (target/check_all/$name.log)"; FAILED+=("$name"); fi
}

if [[ $BUILD -eq 1 ]]; then
  step build tools/build_all.sh --tools
fi

if [[ $UNIT -eq 1 ]]; then
  step unit cargo test --workspace --features debug --release
fi

step smoke tools/smoke_test.sh --no-build

export RETROCLAWD_BIN="$ROOT/target/release/rcdd$EXE" RC_ROMS="$ROOT/machines"
SCEN="$ROOT/target/release/rc-scenarios$EXE"
if [[ ! -d $GOLDENS ]]; then
  echo "skip: no goldens at $GOLDENS (RC_GOLDENS); scenario manifests not run"
else
  export RC_GOLDENS="$GOLDENS"
  for m in machines/*/*.manifest.toml; do
    step "scenarios-$(basename "$m" .manifest.toml)" "$SCEN" run "$m"
  done
fi
for t in rctools/crates/rc-suite/tests/*.rs; do
  name="$(basename "$t" .rs)"
  [[ $name == cpc_acid && $ACID -eq 0 ]] && continue
  step "rc-suite-$name" cargo test --release -p rc-suite --test "$name"
done

echo
if [[ ${#FAILED[@]} -eq 0 ]]; then echo "all checks passed"; exit 0; fi
echo "FAILED: ${FAILED[*]}" >&2
exit 1
