#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
PREVIOUS="$STATE/runtime/previous.env"
FAILED_CANDIDATE="$STATE/runtime/rollback-candidate.env"
CURRENT="$STATE/runtime/current.env"

[[ ${2:-} == --acknowledge-forward-migrations ]] || {
  cat >&2 <<'MSG'
Rollback can restore the previous API image and frontend assets, but SQLx
migrations are forward-only and are NOT downgraded automatically. Re-run with:
  rollback.sh <env-file> --acknowledge-forward-migrations
only after confirming the previous API is compatible with the migrated schema.
MSG
  exit 2
}
TARGET_META="$PREVIOUS"
[[ -f "$FAILED_CANDIDATE" ]] && TARGET_META="$FAILED_CANDIDATE"
[[ -f "$TARGET_META" ]] || { echo "no rollback deployment metadata found" >&2; exit 1; }
# shellcheck disable=SC1090
source "$TARGET_META"
[[ -n ${CS_MAIL_API_IMAGE:-} && -n ${CS_MAIL_WEB_RELEASE_DIR:-} ]] || { echo "previous metadata incomplete" >&2; exit 1; }
[[ -d "$CS_MAIL_WEB_RELEASE_DIR" ]] || { echo "previous frontend release is missing: $CS_MAIL_WEB_RELEASE_DIR" >&2; exit 1; }

set -a; source "$ENV_FILE"; set +a
export CS_MAIL_API_IMAGE
export CS_MAIL_RELEASE_SHA256
if [[ -e "$STATE/www/current" && ! -L "$STATE/www/current" ]]; then
  echo "$STATE/www/current exists but is not a symlink; refusing unsafe rollback" >&2
  exit 1
fi
ln -sfn "$CS_MAIL_WEB_RELEASE_DIR" "$STATE/www/current"
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" up -d --no-build db api
for _ in $(seq 1 45); do
  curl -fsS "http://127.0.0.1:${CS_MAIL_API_HOST_PORT:-18080}/api/health/ready" >/dev/null 2>&1 && break
  sleep 2
done
curl -fsS "http://127.0.0.1:${CS_MAIL_API_HOST_PORT:-18080}/api/health/ready" >/dev/null
nginx -t
systemctl reload nginx
cp -f "$TARGET_META" "$CURRENT"
rm -f "$FAILED_CANDIDATE"
echo "CS Mail code/static rollback PASS (database migrations were not rolled back)"
