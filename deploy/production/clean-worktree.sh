#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}

# Explicit cleanup only. Never use an unrestricted `git clean` in production;
# operators may keep local diagnostic files outside these generated paths.
paths=(
  "$ROOT/frontend/node_modules"
  "$ROOT/frontend/dist"
  "$ROOT/frontend/coverage"
  "$ROOT/backend/target"
  "$ROOT/backend/.cs-mail-target"
  "$ROOT/.cache"
)
for path in "${paths[@]}"; do
  rm -rf -- "$path"
done

find "$ROOT" -type d \( -name __pycache__ -o -name .pytest_cache -o -name .mypy_cache \) -prune -exec rm -rf {} + 2>/dev/null || true
find "$ROOT" -type f \( -name '*.pyc' -o -name '*.pyo' -o -name '*.log' -o -name '.DS_Store' -o -name '*.tsbuildinfo' \) -delete 2>/dev/null || true

echo "CS Mail worktree cleanup PASS"
