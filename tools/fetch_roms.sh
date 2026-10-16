#!/usr/bin/env bash
# Fills the git-ignored machines/<family>/roms folders that tools/smoke_test.sh reads, with the
# firmware your RomM server holds. Each file is picked by SHA-1 (the same hashes as the
# machines/<family>/firmware.toml files), downloaded, and verified before it is kept.
#
#   tools/fetch_roms.sh [--debug] [--force]
#
#   --debug   use target/debug/retroclawd to list the server's firmware (default: release)
#   --force   download again even when the file is already there with the right hash
#
# Needs ROMM_URL and ROMM_TOKEN in the environment (a read-only token is enough) and a built app
# (tools/build_all.sh). HTTP_PROXY / HTTPS_PROXY are honoured by curl. A file the server does not
# have is reported and skipped; the exit status is 1 then, so a missing dump is not silent.
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=release; FORCE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --debug) PROFILE=debug; shift ;;
    --force) FORCE=1; shift ;;
    *) echo "usage: $0 [--debug] [--force]" >&2; exit 2 ;;
  esac
done
: "${ROMM_URL:?set ROMM_URL (the address of the RomM server)}"
: "${ROMM_TOKEN:?set ROMM_TOKEN (a read-only RomM API token)}"

APP="target/$PROFILE/retroclawd"
[[ -x $APP || -x $APP.exe ]] || { echo "no app at $APP (run tools/build_all.sh${PROFILE:+ --$PROFILE})" >&2; exit 1; }
export RETROCLAWD_MACHINE_PLUGIN_DIRS="$PWD/target/$PROFILE/plugins"

sha() { if command -v sha1sum >/dev/null 2>&1; then sha1sum "$1" | cut -d' ' -f1; else shasum -a 1 "$1" | cut -d' ' -f1; fi; }

# family | file under machines/<family>/roms | SHA-1
FILES=(
  "sinclair|zx81/zx81-550-original.rom|a0ade36540561cc1691bb6f0c42ceae12484a102"
  "sinclair|zx-spectrum/spec48.rom|5ea7c2b824672e914525d1d5c419d71b84a426a2"
  "sinclair|zx-spectrum/128.rom|16375d42ea109b47edded7a16028de7fdb3013a1"
  "amstrad|cpc/cpc464.rom|56d39c463da60968d93e58b4ba0e675829412a20"
  "amstrad|cpc/cpc6128.rom|29dd0487dc5a96cd00966d074de8700ddd4ee18b"
  "commodore|c64/kernal-901227-03.bin|1d503e56df85a62fee696e7618dc5b4e781df1bb"
  "commodore|c64/basic-901226-01.bin|79015323128650c742a3694c9429aa91f355905e"
  "commodore|c64/chargen-901225-01.bin|adc7c31e18c7c7413d54802ef2f4193da14711aa"
  "commodore|c128/basiclo-318018-04.bin|d53a7884404f7d18ebd60dd3080c8f8d71067441"
  "commodore|c128/basichi-318019-04.bin|c4fb4a714e48a7bf6c28659de0302183a0e0d6c0"
  "commodore|c128/kernal-318020-05.bin|ceb6e1a1bf7e08eb9cbc651afa29e26adccf38ab"
  "commodore|c128/chargen-390059-01.bin|29ed066d513f2d5c09ff26d9166ba23c2afb2b3f"
  "commodore|c128/basic64-901226-01.bin|79015323128650c742a3694c9429aa91f355905e"
  "commodore|c128/kernal64-901227-03.bin|1d503e56df85a62fee696e7618dc5b4e781df1bb"
)

# The server's firmware: "id  name  size  sha1" rows after a two-line header (names may hold spaces).
LIST="$("$APP" romm firmware | awk 'NR > 2 { print $1, $NF }')"
[[ -n $LIST ]] || { echo "the server lists no firmware" >&2; exit 1; }

missing=0
for entry in "${FILES[@]}"; do
  IFS='|' read -r family rel want <<<"$entry"
  dest="machines/$family/roms/$rel"
  if [[ $FORCE -eq 0 && -f $dest && $(sha "$dest") == "$want" ]]; then
    echo "have     $rel"; continue
  fi
  ids="$(awk -v h="$want" '$2 == h { print $1 }' <<<"$LIST")"
  if [[ -z $ids ]]; then
    echo "MISSING  $rel (SHA-1 $want is not on the server)"; missing=1; continue
  fi
  mkdir -p "$(dirname "$dest")"
  # The same dump can be listed more than once, some entries with their file gone from the
  # server's library (404): try each until one downloads. The file name in the URL is only a
  # label to RomM; the id decides. The token goes in on stdin, never on the command line where
  # `ps` would show it.
  got=""
  for id in $ids; do
    if printf 'header = "Authorization: Bearer %s"
' "$ROMM_TOKEN"         | curl -sSf -K - -o "$dest.part" "${ROMM_URL%/}/api/firmware/$id/content/firmware.bin" 2>/dev/null         && [[ $(sha "$dest.part") == "$want" ]]; then
      mv "$dest.part" "$dest"; got=$id; break
    fi
    rm -f "$dest.part"
  done
  if [[ -n $got ]]; then
    echo "fetched  $rel  (firmware $got)"
  else
    echo "FAILED   $rel (no listed entry for $want could be downloaded)"; missing=1
  fi
done
exit $missing
