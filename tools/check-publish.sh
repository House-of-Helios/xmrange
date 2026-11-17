#!/bin/bash
# Before anything goes to GitHub: every file and every commit on the branch that leaves (default
# `public`, built by tools/publish.sh; the local full history never leaves) must carry no private
# data. Usage: tools/check-publish.sh [ref]. Looks for Monero addresses other than the disclosed default (GhostDrip's,
# read from config.rs), home-directory paths, e-mail addresses and public IPv4 addresses (allowed:
# private and documentation ranges, 1.2.3.4, and MoneroOcean's pool server in test fixtures). Prints
# counts and where, never the matched text. Exit 1 on any finding.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
DEF="$(grep -m1 -oE '"4[1-9A-HJ-NP-Za-km-z]{94}"' crates/xmrange-core/src/config.rs | tr -d '"')"
[ ${#DEF} -eq 95 ] || { echo "CHECKER BROKEN: default wallet not found"; exit 2; }
REF="${1:-public}"
git rev-parse -q --verify "$REF^{commit}" >/dev/null || { echo "no branch $REF"; exit 2; }
ADDR='[48][0-9AB][1-9A-HJ-NP-Za-km-z]{93}'
# Public addresses that upstream text carries: XMRig's donation address (in the patch's context
# lines) and MoneroOcean's example address (in its docs). Matched by their ends, never printed.
PUBLIC_ENDS=("48edfH:icoD" "89Txfr:nWHL")
public() { local a="$1" e; for e in "${PUBLIC_ENDS[@]}"; do [ "${a:0:6}" = "${e%%:*}" ] && [ "${a: -4}" = "${e##*:}" ] && return 0; done; return 1; }
ME="$(id -un)"
fail=0
report() { # <label> <text>
  local label="$1" text="$2" n a
  n=0
  while read -r a; do [ -z "$a" ] && continue; public "$a" || n=$((n + 1)); done < <(grep -oE -- "$ADDR" <<<"$text" | grep -vxF -- "$DEF" | sort -u)
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n monero address(es) other than the default and the public ones"; fail=1; }
  n=$(grep -cE "/Users/$ME|$HOME" <<<"$text" || true)
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n line(s) with this Mac's home path or user name"; fail=1; }
  n=$(grep -oiE '[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}' <<<"$text" | grep -viE '^(support@moneroocean\.stream|git@.*)$|@example\.com$' | sort -u | wc -l | tr -d ' ')
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n e-mail address(es)"; fail=1; }
  # The pattern is split ("co-author"+"ed-by") so this file never matches itself.
  n=$(grep -ciE "co-author""ed-by|generated with \[?claude" <<<"$text" || true)
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n attribution line(s)"; fail=1; }
  # The old shared-folder update road is never named in public (owner ruling 2026-11-17).
  # Split ("shipping"+"[_ -]?port") so this file never matches itself.
  n=$(grep -ciE "shipping""[_ -]?port" <<<"$text" || true)
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n shared-folder line(s)"; fail=1; }
  n=$(grep -cE '(github_pat|gh[pousr])_[A-Za-z0-9_]{20,}' <<<"$text" || true)
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n GitHub token(s)"; fail=1; }
  n=$(grep -oE '\b([0-9]{1,3}\.){3}[0-9]{1,3}\b' <<<"$text" | grep -vE '^(127\.|0\.|10\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|255\.)' \
    | grep -vE '^(192\.0\.2\.|198\.51\.100\.|203\.0\.113\.)|^1\.2\.3\.4$|^205\.172\.58\.170$' | sort -u | wc -l | tr -d ' ')
  # SVG path data is runs of dotted numbers, never addresses.
  [[ "$label" == *.svg ]] && n=0
  [ "$n" -gt 0 ] && { echo "FOUND: $label: $n public IPv4 address(es)"; fail=1; }
  return 0
}
# Names NUL-delimited (any byte is legal in a file name); a file that can't be read is a finding.
while IFS= read -r -d '' f; do
  if ! text="$(git show "$REF:$f")"; then echo "FOUND: $f: could not be read"; fail=1; continue; fi
  grep -qI . <<<"$text" || continue   # binary or empty
  report "$f" "$text"
done < <(git ls-tree -r -z --name-only "$REF")
report "git history of $REF" "$(git log "$REF" -p --no-color -- . ':(exclude)*.svg')"
report "git history of $REF *.svg" "$(git log "$REF" -p --no-color -- '*.svg')"
report "commit authors" "$(git log "$REF" --format='%an <%ae> %cn <%ce>')"
[ $fail -eq 0 ] && echo "publish check: PASS ($REF: every file + its whole history)"
exit $fail
