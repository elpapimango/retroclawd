#!/usr/bin/env bash
# Move the machine repositories (and this application) to a newer commit of retroclawd-crates.
#
# The shared crates live in their own repository, checked out here as the submodule `shared/`. The
# application uses them by path; the machine repositories depend on them by git revision
# (`rev = "<sha>"` in their Cargo.toml files). After you change a shared crate:
#
#   cd shared && git commit ... && git push origin HEAD:main     # the new commit must be on GitHub
#   cd .. && tools/bump_pin.sh                                   # pin everything to shared/'s HEAD
#
#   tools/bump_pin.sh [--rev <sha>] [--families a,b] [--no-test] [--push]
#
#   --rev        retroclawd-crates commit to pin (default: the HEAD of shared/; it must be pushed)
#   --families   only these families (default: all three)
#   --no-test    skip `cargo test --workspace` in each family
#   --push       push each family's main branch (otherwise the commits stay local, in machines/<f>)
#
# It checks out that commit in shared/; then for each family it checks the submodule out on an
# up-to-date `main` (refusing a dirty tree or unpushed commits), rewrites every `rev = "<old>"`, refreshes
# Cargo.lock, runs the family's tests, and commits "Pin retroclawd-crates <sha>". Finally it records the
# new submodule commits here in one commit. Pushing this repository stays your call. It also prints what
# changed in the plugin ABI between the two revisions, because a family library is only loadable by an app
# speaking the same ABI major / protocol version (shared/docs/plugin-abi.md).
#
# Optional: BUMP_COMMIT_TRAILER="Co-Authored-By: ..." is appended to the commit messages.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
command -v cygpath >/dev/null 2>&1 && ROOT="$(cygpath -m "$PWD")"

REV=""; FAMILIES="sinclair,amstrad,commodore"; TEST=1; PUSH=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --rev) REV="$2"; shift 2 ;;
    --families) FAMILIES="$2"; shift 2 ;;
    --no-test) TEST=0; shift ;;
    --push) PUSH=1; shift ;;
    *) echo "usage: $0 [--rev <sha>] [--families a,b] [--no-test] [--push]" >&2; exit 2 ;;
  esac
done

export CARGO_NET_GIT_FETCH_WITH_CLI=true
export CARGO_TARGET_DIR="$ROOT/target/families"

git diff --cached --quiet || { echo "the index has staged changes; commit or unstage them first (the bump makes its own commit here)" >&2; exit 1; }
# Use shared/ as it is checked out (the commit you just pushed there); only fetch it when missing.
[[ -e shared/.git ]] || git submodule update --init shared >/dev/null
NEW="$(git -C shared rev-parse "${REV:-HEAD}^{commit}")"
git -C shared fetch -q origin
if [[ -z "$(git -C shared branch -r --contains "$NEW" 2>/dev/null)" ]]; then
  echo "retroclawd-crates commit $NEW is not on GitHub yet: push it first (the families fetch it from there)" >&2
  exit 1
fi

IFS=, read -ra FAM <<< "$FAMILIES"
for f in "${FAM[@]}"; do
  [[ -d machines/$f ]] || { echo "unknown family: $f" >&2; exit 2; }
  [[ -e machines/$f/.git ]] || git submodule update --init "machines/$f" >/dev/null
done

# The pin currently in use: every family must agree on one.
pins() { git -C "machines/$1" ls-files '*Cargo.toml' | while read -r m; do
           grep -o 'retroclawd-crates", rev = "[0-9a-f]\{40\}"' "machines/$1/$m" || true; done \
         | grep -o '[0-9a-f]\{40\}' | sort -u; }
OLD=""
for f in "${FAM[@]}"; do
  p="$(pins "$f")"
  [[ $(wc -w <<< "$p") -le 1 ]] || { echo "machines/$f pins several revisions: $p" >&2; exit 1; }
  [[ -z $p ]] && { echo "machines/$f has no retroclawd-crates pin" >&2; exit 1; }
  [[ -z $OLD || $OLD == "$p" ]] || { echo "families pin different revisions ($OLD vs $p); bump them together" >&2; exit 1; }
  OLD="$p"
done
if [[ "$OLD" == "$NEW" ]]; then echo "families already pin ${NEW:0:10}"; fi
echo "pin: ${OLD:0:10} -> ${NEW:0:10}"

echo; echo "== plugin ABI between the two revisions"
const() { git -C shared show "$1:$2" 2>/dev/null | sed -n "s/^pub const $3: u16 = \([0-9]*\);.*/\1/p" | head -1; }
for spec in "crates/plugin-api/src/abi.rs ABI_MAJOR_V1" "crates/plugin-api/src/abi.rs ABI_MINOR_V1" \
            "crates/plugin-api/src/protocol.rs PROTOCOL_VERSION" \
            "crates/plugin-api/src/protocol.rs MACHINE_PROFILE_CONTRACT_VERSION"; do
  set -- $spec; o="$(const "$OLD" "$1" "$2")"; n="$(const "$NEW" "$1" "$2")"
  if [[ "$o" == "$n" ]]; then echo "  $2: $n"; else echo "  $2: $o -> $n   <-- changed; families may need code changes"; fi
done
git -C shared diff --stat "$OLD" "$NEW" -- crates | tail -15 || true

# The application follows by path: put shared/ on the pinned commit.
git -C shared checkout -q "$NEW"

TRAILER="${BUMP_COMMIT_TRAILER:+$'\n\n'$BUMP_COMMIT_TRAILER}"
for f in "${FAM[@]}"; do
  echo; echo "== machines/$f"
  cd "$ROOT/machines/$f"
  [[ -z "$(git status --porcelain)" ]] || { echo "working tree is not clean" >&2; exit 1; }
  git fetch -q origin main
  git merge-base --is-ancestor HEAD origin/main || { echo "HEAD has commits that are not on origin/main; push or drop them first" >&2; exit 1; }
  git checkout -q -B main origin/main
  if [[ "$OLD" != "$NEW" ]]; then
    git ls-files '*Cargo.toml' '*.md' | while read -r file; do
      sed -i "s/$OLD/$NEW/g; s/${OLD:0:10}/${NEW:0:10}/g" "$file"
    done
  fi
  cargo metadata --format-version 1 >/dev/null     # re-resolves Cargo.lock for the new rev
  grep -q "$NEW" Cargo.lock || { echo "Cargo.lock does not mention the new rev" >&2; exit 1; }
  # cargo's own exit status decides: piping it into grep under `pipefail` made a failing run look like
  # "no failure lines found" and carried on to commit and push.
  if [[ $TEST -eq 1 ]]; then
    if ! test_log="$(cargo test --workspace 2>&1)"; then
      grep -E "^error|test result: F|FAILED|panicked" <<< "$test_log" || true
      echo "tests failed" >&2; exit 1
    fi
    echo "tests passed"
  fi
  git add -A
  if git diff --cached --quiet; then echo "nothing to change"; else
    git -c user.name="${GIT_AUTHOR_NAME:-$(git config user.name || echo bump-pin)}" \
        -c user.email="${GIT_AUTHOR_EMAIL:-$(git config user.email || echo noreply@example.invalid)}" \
        commit -q -m "Pin retroclawd-crates ${NEW:0:10}

Shared crates now come from retroclawd-crates commit $NEW (was ${OLD:0:10}).$TRAILER"
  fi
  [[ $PUSH -eq 1 ]] && git push origin main
  cd "$ROOT"
done

git add machines shared
if git diff --cached --quiet; then echo "nothing to record here"; else
  git commit -q -m "Bump shared/ and the machine repositories to retroclawd-crates ${NEW:0:10}$TRAILER"
  echo; echo "recorded the new submodule commits in this repository; push it when you are ready."
  [[ $PUSH -eq 1 ]] || echo "(the family commits are local: rerun the bump with --push, or push each machines/<f> yourself, before pushing this repository)"
fi
