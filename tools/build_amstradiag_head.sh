#!/usr/bin/env bash
# Builds the AmstradDiag disk from the *repository head* (the release v1.3 has no sound test; it was added in 2024), into the
# git-ignored machines/amstrad/software/diagnostics/AmstradDiagHead.dsk that the sound case of retroclawd-tools' CPC suite runs.
#
#   tools/build_amstradiag_head.sh
#
#   amstrad-diagnostics (Noel Llopis and others; MIT) at commit 7ac70fc ("Sound test from Cacharreo"):
#       https://github.com/llopis/amstrad-diagnostics
#
# It downloads that commit's source, assembles the RAM build with sjasmplus (the one on PATH, else on Windows the official
# sjasmplus 1.24.0 release, SHA-256 checked, unpacked into the temporary folder), checks the assembled program against the hash
# it had when this was written, and puts it on a blank AMSDOS disk as DIAG (load and entry &0400) with `rcdisk` from
# retroclawd-tools (RCDISK=<path>, else the sibling checkout's target/release or target/debug). Nothing downloaded is run except
# sjasmplus itself, and nothing is committed: the diagnostics are third-party software, fetched for local testing.
#
# Needs curl and python3 (or unzip). Exits 1 if a download, the assembly or a check fails.
set -euo pipefail
cd "$(dirname "$0")/.."

COMMIT=7ac70fc5efaa3c02755b1238d532292551d3bd5e
SRC_URL="https://github.com/llopis/amstrad-diagnostics/archive/$COMMIT.zip"
DIAG_SHA=55ef33585e204a93902505d43574511871ca06fc3bda3af8d1e6925eb0bb6f18   # the assembled RAM build (diag.bin)
SJASM_URL="https://github.com/z00m128/sjasmplus/releases/download/v1.24.0/sjasmplus-1.24.0.win.zip"
SJASM_SHA=7e1f8840842039bdb97e51a59abda2e5c25a9da160097e8f79a62583ab72d0e0
DEST=machines/amstrad/software/diagnostics/AmstradDiagHead.dsk

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
unzip_to() { # zip dir
  if command -v python3 >/dev/null 2>&1; then python3 -I -c "import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$1" "$2"
  else unzip -q "$1" -d "$2"; fi
}

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

SJASM="$(command -v sjasmplus || true)"
if [[ -z $SJASM ]]; then
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) ;;
    *) echo "sjasmplus is not on PATH; install it (https://github.com/z00m128/sjasmplus) and run this again" >&2; exit 1 ;;
  esac
  curl -sSfL -o "$tmp/sjasmplus.zip" "$SJASM_URL"
  [[ $(sha256 "$tmp/sjasmplus.zip") == "$SJASM_SHA" ]] || { echo "MISMATCH sjasmplus download" >&2; exit 1; }
  mkdir "$tmp/sjasmplus"; unzip_to "$tmp/sjasmplus.zip" "$tmp/sjasmplus"
  SJASM="$(find "$tmp/sjasmplus" -name sjasmplus.exe | head -1)"
fi

curl -sSfL -o "$tmp/src.zip" "$SRC_URL"
mkdir "$tmp/src"; unzip_to "$tmp/src.zip" "$tmp/src"
srcdir="$(find "$tmp/src" -maxdepth 1 -mindepth 1 -type d | head -1)"
mkdir "$srcdir/build"
(cd "$srcdir" && "$SJASM" --nologo --msg=war -DRAMBuild=1 -DOutFile=build/diag.bin --lst=build/list.txt src/Main.asm)
[[ $(sha256 "$srcdir/build/diag.bin") == "$DIAG_SHA" ]] || { echo "MISMATCH the assembled diag.bin does not hash to $DIAG_SHA (a different sjasmplus?)" >&2; exit 1; }

RCDISK="${RCDISK:-}"
if [[ -z $RCDISK ]]; then
  for c in ../retroclawd-tools/target/release/rcdisk ../retroclawd-tools/target/debug/rcdisk; do
    for e in "" .exe; do [[ -x $c$e ]] && RCDISK="$c$e" && break 2; done
  done
fi
[[ -n $RCDISK ]] || { echo "rcdisk not found: build retroclawd-tools or set RCDISK=<path>" >&2; exit 1; }

mkdir -p "$(dirname "$DEST")"
"$RCDISK" create "$DEST" -f dsk --force >/dev/null
"$RCDISK" add "$DEST" "$srcdir/build/diag.bin" --name DIAG --type bin --header amsdos --load 0x400 --entry 0x400 >/dev/null
echo "built    amstrad-diagnostics $COMMIT -> $DEST"
