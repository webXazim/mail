#!/usr/bin/env bash
set -euo pipefail
ENV_FILE=${1:-/opt/cs-mail/.env.production}
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a
BACKUP_DIR=${CS_MAIL_BACKUP_DIR:-/opt/backups/cs-mail}
RETENTION_DAYS=${CS_MAIL_BACKUP_RETENTION_DAYS:-30}
REASON=${CS_MAIL_BACKUP_REASON:-scheduled}
[[ "$RETENTION_DAYS" =~ ^[1-9][0-9]*$ ]] || { echo "CS_MAIL_BACKUP_RETENTION_DAYS must be a positive integer" >&2; exit 1; }
if [[ -f "$STATE/runtime/current.env" ]]; then
  # shellcheck disable=SC1090
  source "$STATE/runtime/current.env"
fi
export CS_MAIL_API_IMAGE=${CS_MAIL_API_IMAGE:-cs-mail-api:production}

install -d -m 0700 "$BACKUP_DIR"
stamp=$(date -u +%Y%m%d-%H%M%S)
dump="$BACKUP_DIR/cs-mail-$stamp.dump"
attachments="$BACKUP_DIR/cs-mail-$stamp.attachments.tgz"
manifest="$BACKUP_DIR/cs-mail-$stamp.manifest.txt"

cd "$ROOT"
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  pg_dump -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -Fc > "$dump"
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" run --rm --no-deps -T \
  --entrypoint tar api -C /srv/attachments -czf - . > "$attachments"
[[ -s "$dump" && -s "$attachments" ]]

{
  echo "created=$(date -u +%FT%TZ)"
  echo "reason=$REASON"
  echo "release_label=${CS_MAIL_RELEASE_LABEL:-unknown}"
  echo "release_sha256=${CS_MAIL_RELEASE_SHA256:-unknown}"
  echo "git_sha=${CS_MAIL_DEPLOYED_GIT_SHA:-unknown}"
  echo "db_sha256=$(sha256sum "$dump" | awk '{print $1}')"
  echo "attachments_sha256=$(sha256sum "$attachments" | awk '{print $1}')"
  echo "provider_namespace=cs-mail"
  echo "shared_stalwart_backup=external-responsibility"
  echo "note=Stalwart is shared with other CrescentSphere products and is intentionally not snapshotted by the CS Mail stack."
} > "$manifest"
chmod 600 "$dump" "$attachments" "$manifest"

# Migration 0048 makes backup health visible to the launch-readiness endpoint
# and Prometheus. Pre-deploy backups can run before that migration exists, so
# evidence recording deliberately degrades to a warning instead of making a
# valid pre-migration backup fail.
if docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -tAc \
  "SELECT to_regclass('public.operational_evidence') IS NOT NULL" 2>/dev/null | grep -qx t; then
  manifest_sha=$(sha256sum "$manifest" | awk '{print $1}')
  release_sha=${CS_MAIL_RELEASE_SHA256:-}
  [[ "$release_sha" =~ ^[0-9a-f]{64}$ ]] || release_sha=
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
    psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -v ON_ERROR_STOP=1 \
      -v release_sha="$release_sha" -v artifact_ref="$manifest" -v artifact_sha="$manifest_sha" -v reason="$REASON" <<'SQL' >/dev/null
INSERT INTO operational_evidence(kind,status,release_sha256,artifact_ref,artifact_sha256,detail)
VALUES ('local_backup','passed',:'release_sha',:'artifact_ref',:'artifact_sha',jsonb_build_object('reason',:'reason'));
SQL
else
  echo "backup evidence ledger not available yet; backup itself succeeded" >&2
fi

find "$BACKUP_DIR" -type f -name 'cs-mail-*' -mtime "+$RETENTION_DAYS" -delete
printf 'backup PASS: %s\n' "$dump"
