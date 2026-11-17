#!/bin/bash
# Put the code on GitHub (House-of-Helios/xmrange), and only the code. Bookkeeper's protocol.
#
# The full history stays on this Mac. GitHub gets its own line of history on the local branch
# `public`: each run adds one commit holding this commit's files minus the private paths below,
# and pushes `public` to GitHub's `main`. Nothing from the private history, and nothing under a
# private path, ever enters `public`: it is built from a filtered tree, never from a merge.
#
# Usage:  tools/publish.sh ["what changed"]
set -euo pipefail
cd "$(dirname "$0")/.."

REPO="House-of-Helios/xmrange"
# Working notes, research, workflows, retired tools, measurements from the owner's Macs, the agent's
# instructions, and the dev-app builder (it writes to a folder on the owner's Macs).
PRIVATE=(_Plans _Research _Workflows _Archive _Past data THESIS.md CLAUDE.md tools/make-dev-app.sh)

[ -z "$(git status --porcelain)" ] || { echo "❌ uncommitted changes — commit first"; exit 1; }
git remote get-url origin >/dev/null 2>&1 || git remote add origin "https://github.com/$REPO.git"

INDEX=$(mktemp); trap 'rm -f "$INDEX"' EXIT
export GIT_INDEX_FILE="$INDEX"
git read-tree HEAD
for p in "${PRIVATE[@]}"; do git rm -r -q --cached --ignore-unmatch -- "$p"; done
TREE=$(git write-tree)
unset GIT_INDEX_FILE

PARENT=$(git rev-parse -q --verify refs/heads/public || true)
if [ -n "$PARENT" ] && [ "$(git rev-parse "$PARENT^{tree}")" = "$TREE" ]; then
  echo "🐙 nothing new for GitHub"
  COMMIT="$PARENT"
else
  MSG="${1:-$(git log -1 --format=%s)}"
  # A fixed identity: never this Mac's git name or e-mail.
  COMMIT=$(GIT_AUTHOR_NAME="House of Helios" GIT_AUTHOR_EMAIL="" GIT_COMMITTER_NAME="House of Helios" GIT_COMMITTER_EMAIL="" \
    git commit-tree "$TREE" ${PARENT:+-p "$PARENT"} -m "$MSG")
fi

# Both gates run on the new commit BEFORE `public` moves: a failing tree never becomes `public`.
# The guard that matters most: no private path in what is about to leave.
for p in "${PRIVATE[@]}"; do
  if git ls-tree -r --name-only "$COMMIT" | grep -q -e "^$p\$" -e "^$p/"; then echo "❌ $p is in the public tree"; exit 1; fi
done
tools/check-publish.sh "$COMMIT" || { echo "❌ privacy check failed — public not moved, nothing pushed"; exit 1; }
if [ "$COMMIT" != "$PARENT" ]; then
  git update-ref refs/heads/public "$COMMIT" ${PARENT:+"$PARENT"}
  echo "🐙 public ← $(git rev-parse --short "$COMMIT") ($MSG)"
fi

git push -q origin public:main
echo "✅ https://github.com/$REPO"
