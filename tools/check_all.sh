#!/usr/bin/env bash
# Every check the project has, in one run: so a change in one place cannot quietly break a case that lives in
# another repository (a disk-timing change once broke two rc-suite CPC cases that nobody ran).
#
#   tools/check_all.sh [--no-build] [--acid] [--skip-unit]
#
#   --no-build    use what is in target/release (default: tools/build_all.sh first)
#   --acid        also run the CPC acid suite (rc-suite cpc_acid; slow, about 15 minutes)
#   --skip-unit   skip the cargo unit tests of this repository, shared/ and the families
#
# In order: the release build of the app and every family; `cargo test` in this repository, shared/ and each
# family; tools/smoke_test.sh; every `machines/<family>/*.manifest.toml` through rc-scenarios; and the rc-suite
# integration cases. The last two need the retroclawd-tools checkout (RC_TOOLS, default ../retroclawd-tools) and
# the goldens folder (RC_GOLDENS, default ../goldens); without them those steps are skipped and say so. Every
# step runs even after a failure; the summary lists what failed and the exit status is 1 if anything did.
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

export CARGO_NET_GIT_FETCH_WITH_CLI=true
TOOLS="${RC_TOOLS:-$ROOT/../retroclawd-tools}"
GOLDENS="${RC_GOLDENS:-$ROOT/../goldens}"
FAILED=()
LOGS="$ROOT/target/check_all"; rm -rf "$LOGS"; mkdir -p "$LOGS"

# step <name> <command...>: runs it, keeps its output in target/check_all/<name>.log, records a failure.
step() {
  local name="$1"; shift
  printf '%-34s ' "$name"
  if "$@" >"$LOGS/$name.log" 2>&1; then echo ok
  else echo "FAILED (target/check_all/$name.log)"; FAILED+=("$name"); fi
}

if [[ $BUILD -eq 1 ]]; then
  step build tools/build_all.sh
fi

if [[ $UNIT -eq 1 ]]; then
  step unit-app cargo test --workspace --release
  step unit-shared bash -c "cd shared && cargo test --workspace --release"
  for f in sinclair amstrad commodore atari sega; do
    [[ -d machines/$f ]] || continue
    step "unit-$f" bash -c "cd machines/$f && CARGO_TARGET_DIR='$ROOT/target/families' cargo test --workspace --release"
  done
fi

step smoke tools/smoke_test.sh --no-build

if [[ ! -d $TOOLS ]]; then
  echo "skip: no retroclawd-tools checkout at $TOOLS (RC_TOOLS); scenarios and rc-suite not run"
else
  export RETROCLAWD_BIN="$ROOT/target/release/retroclawd" RETROCLAWD_MACHINE_PLUGIN_DIRS="$ROOT/target/release/plugins" \
    RC_ROMS="$ROOT/machines"
  [[ -x $RETROCLAWD_BIN || -x $RETROCLAWD_BIN.exe ]] || RETROCLAWD_BIN="$RETROCLAWD_BIN.exe"
  step tools-build bash -c "cd '$TOOLS' && cargo build --release -p rc-suite"
  SCEN="$TOOLS/target/release/rc-scenarios"
  if [[ ! -d $GOLDENS ]]; then
    echo "skip: no goldens at $GOLDENS (RC_GOLDENS); scenario manifests not run"
  else
    export RC_GOLDENS="$GOLDENS"
    for m in machines/*/*.manifest.toml; do
      step "scenarios-$(basename "$m" .manifest.toml)" "$SCEN" run "$m"
    done
  fi
  for t in "$TOOLS"/crates/rc-suite/tests/*.rs; do
    name="$(basename "$t" .rs)"
    [[ $name == cpc_acid && $ACID -eq 0 ]] && continue
    step "rc-suite-$name" bash -c "cd '$TOOLS' && cargo test --release -p rc-suite --test $name"
  done
fi

echo
if [[ ${#FAILED[@]} -eq 0 ]]; then echo "all checks passed"; exit 0; fi
echo "FAILED: ${FAILED[*]}" >&2
exit 1
