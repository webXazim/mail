#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }

ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
ENV_FILE=${CS_MAIL_ENV_FILE:-/opt/cs-mail/.env.production}
REMOTE=${CS_MAIL_GIT_REMOTE:-origin}
BRANCH=${CS_MAIL_GIT_BRANCH:-main}
TARGET=${1:-$BRANCH}

[[ -d "$ROOT/.git" ]] || { echo "$ROOT is not a Git checkout" >&2; exit 1; }
cd "$ROOT"

if [[ -n $(git status --porcelain --untracked-files=all) ]]; then
  echo "Refusing to pull over a dirty production worktree:" >&2
  git status --short >&2
  exit 1
fi

git fetch --prune "$REMOTE"
if git show-ref --verify --quiet "refs/remotes/$REMOTE/$TARGET"; then
  git checkout -q "$TARGET" 2>/dev/null || git checkout -q -b "$TARGET" "$REMOTE/$TARGET"
  git merge --ff-only "$REMOTE/$TARGET"
else
  # Allows an explicit immutable commit/tag: deploy-from-git.sh <sha-or-tag>.
  git checkout --detach "$TARGET"
fi

"$ROOT/deploy/production/clean-worktree.sh"
"$ROOT/deploy/production/verify-release.sh"
exec "$ROOT/deploy/production/deploy.sh" "$ENV_FILE"
