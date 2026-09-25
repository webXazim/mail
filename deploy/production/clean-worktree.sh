#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}

# Clean only generated/ignored artifacts. In a Git checkout, NEVER rm -rf a
# path that may still be tracked by an older release: doing so would create a
# dirty worktree and block the next safe pull. `git clean -X` removes ignored
# untracked content while preserving every tracked file.
relative_generated_dirs=(
  frontend/node_modules
  frontend/dist
  frontend/coverage
  backend/target
  backend/.cs-mail-target
  .cache
)

if [[ -d "$ROOT/.git" ]]; then
  cd "$ROOT"
  for path in "${relative_generated_dirs[@]}"; do
    git clean -fdX -- "$path" >/dev/null 2>&1 || true
  done

  # Python/tool caches can exist below many source directories. Keep cleanup
  # path-scoped and tracked-file-safe instead of using an unrestricted clean.
  while IFS= read -r -d '' cache_dir; do
    rel=${cache_dir#"$ROOT"/}
    git clean -fdX -- "$rel" >/dev/null 2>&1 || true
  done < <(find "$ROOT" -type d \( -name __pycache__ -o -name .pytest_cache -o -name .mypy_cache \) -print0 2>/dev/null)
else
  for path in "${relative_generated_dirs[@]}"; do
    rm -rf -- "$ROOT/$path"
  done
  find "$ROOT" -type d \( -name __pycache__ -o -name .pytest_cache -o -name .mypy_cache \) -prune -exec rm -rf {} + 2>/dev/null || true
  find "$ROOT" -type f \( -name '*.pyc' -o -name '*.pyo' -o -name '*.log' -o -name '.DS_Store' -o -name '*.tsbuildinfo' \) -delete 2>/dev/null || true
fi

echo "CS Mail worktree cleanup PASS"
