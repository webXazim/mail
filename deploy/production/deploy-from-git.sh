#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }

ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}
ENV_FILE=${CS_MAIL_ENV_FILE:-/opt/cs-mail/.env.production}
REMOTE=${CS_MAIL_GIT_REMOTE:-origin}
BRANCH=${CS_MAIL_GIT_BRANCH:-main}
TARGET=${1:-$BRANCH}

[[ -d "$ROOT/.git" ]] || { echo "$ROOT is not a Git checkout" >&2; exit 1; }
cd "$ROOT"

is_generated_path() {
  case "$1" in
    frontend/dist|frontend/dist/*|\
    frontend/node_modules|frontend/node_modules/*|\
    frontend/coverage|frontend/coverage/*|\
    backend/target|backend/target/*|\
    backend/.cs-mail-target|backend/.cs-mail-target/*|\
    .cache|.cache/*|\
    */__pycache__|*/__pycache__/*|\
    *.pyc|*.pyo|*.tsbuildinfo|*.log|*.DS_Store)
      return 0
      ;;
  esac
  return 1
}

# Upgrade 05C used an unconditional filesystem cleanup. On an older checkout
# where generated artifacts had historically been committed, that cleanup
# legitimately removed those tracked artifacts and made the worktree appear
# dirty before Git had a chance to pull the commit that untracks them.
#
# Repair ONLY tracked generated artifacts back to the currently checked-out
# commit. Never repair source/config files here: any other tracked change still
# blocks deployment below.
repair_generated_tracked_drift() {
  local path
  local -a repair=()
  while IFS= read -r -d '' path; do
    if is_generated_path "$path"; then
      repair+=("$path")
    fi
  done < <(git diff --name-only -z HEAD --)

  if ((${#repair[@]})); then
    echo "Repairing generated tracked artifacts left dirty by an older cleanup..."
    git restore --source=HEAD --staged --worktree -- "${repair[@]}"
  fi
}

repair_generated_tracked_drift
bash "$ROOT/deploy/production/clean-worktree.sh" >/dev/null

if [[ -n $(git status --porcelain --untracked-files=all) ]]; then
  echo "Refusing to pull over a dirty production worktree:" >&2
  git status --short >&2
  echo >&2
  echo "Only generated build/cache drift is auto-repaired. Review the source changes above." >&2
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

# The pulled release is authoritative. Generated artifacts are never required
# from Git; remove only ignored/untracked copies before release verification.
bash "$ROOT/deploy/production/clean-worktree.sh"
bash "$ROOT/deploy/production/verify-release.sh"
exec bash "$ROOT/deploy/production/deploy.sh" "$ENV_FILE"
