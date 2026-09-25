#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"

[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a
if [[ -f "$STATE/runtime/current.env" ]]; then source "$STATE/runtime/current.env"; fi
export CS_MAIL_API_IMAGE=${CS_MAIL_API_IMAGE:-cs-mail-api:production}

printf 'Release: %s\n' "${CS_MAIL_RELEASE_LABEL:-unknown}"
printf 'Git SHA: %s\n' "${CS_MAIL_DEPLOYED_GIT_SHA:-unknown}"
printf 'Source SHA-256: %s\n' "${CS_MAIL_RELEASE_SHA256:-unknown}"
printf 'API image: %s\n\n' "$CS_MAIL_API_IMAGE"

docker compose --env-file "$ENV_FILE" -f "$COMPOSE" ps
printf '\nAlertmanager readiness: '
if curl -fsS http://127.0.0.1:19093/-/ready >/dev/null; then echo PASS; else echo FAIL; fi
printf '\nLoopback API readiness: '
if curl -fsS "http://127.0.0.1:${CS_MAIL_API_HOST_PORT:-18080}/api/health/ready" >/dev/null; then echo PASS; else echo FAIL; fi
printf 'Local HTTPS vhost readiness: '
if curl -fsS --resolve "${CS_MAIL_WEB_HOST}:443:127.0.0.1" "${CS_MAIL_PUBLIC_ORIGIN}/api/health/ready" >/dev/null; then echo PASS; else echo FAIL; fi
printf 'Public API readiness: '
if curl -fsS "${CS_MAIL_PUBLIC_ORIGIN}/api/health/ready" >/dev/null; then echo PASS; else echo FAIL; fi
printf 'Local admin listener: '
if curl -fsS -H 'Host: localhost' "http://127.0.0.1:${CS_MAIL_ADMIN_HOST_PORT:-18081}/login" >/dev/null; then echo PASS; else echo FAIL; fi


printf 'Backup timer: '
if systemctl is-active --quiet cs-mail-backup.timer 2>/dev/null; then echo PASS; else echo FAIL; fi

printf '\nRecoverability evidence:\n'
if docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
    psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -tAc \
    "SELECT to_regclass('public.operational_evidence') IS NOT NULL" 2>/dev/null | grep -qx t; then
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
    psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -P pager=off -c \
    "SELECT DISTINCT ON (kind) kind,status,recorded_at,round(extract(epoch FROM (now()-recorded_at))/3600.0,1) AS age_hours,left(release_sha256,12) AS release FROM operational_evidence ORDER BY kind,recorded_at DESC" || true
else
  echo 'operational_evidence table unavailable (migration 0048 not applied or database unreachable)'
fi
