#!/usr/bin/env bash
# Builds the media the scenario manifests load, with rctools, into software/scenarios/ (not committed).
#   scenarios/build.sh [path to the rctools target dir]   default ../../../target/release
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tools="${1:-$here/../../../target/release}"
out="$here/../software/scenarios"
mkdir -p "$out"
"$tools/rcld" build "$here/cpc/hello.toml" -o "$out/cpc-hello.dsk" --force
