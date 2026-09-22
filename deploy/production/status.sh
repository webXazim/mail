#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"

[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a
if [[ -f "$STATE/runtime/current.env" ]]; then
  # shellcheck disable=SC1090
  source "$STATE/runtime/current.env"
fi
export CS_MAIL_API_IMAGE=${CS_MAIL_API_IMAGE:-cs-mail-api:production}

printf 'Release: %s\n' "${CS_MAIL_RELEASE_LABEL:-unknown}"
printf 'Git SHA: %s\n' "${CS_MAIL_DEPLOYED_GIT_SHA:-unknown}"
printf 'Source SHA-256: %s\n' "${CS_MAIL_RELEASE_SHA256:-unknown}"
printf 'API image: %s\n\n' "$CS_MAIL_API_IMAGE"

docker compose --env-file "$ENV_FILE" -f "$COMPOSE" ps
printf '\nLoopback API readiness: '
if curl -fsS "http://127.0.0.1:${CS_MAIL_API_HOST_PORT:-18080}/api/health/ready" >/dev/null; then echo PASS; else echo FAIL; fi
printf 'Public API readiness: '
if curl -fsS "${CS_MAIL_PUBLIC_ORIGIN:-https://mail.crescentsphere.com}/api/health/ready" >/dev/null; then echo PASS; else echo FAIL; fi
printf 'Local admin listener: '
if curl -fsS -H 'Host: localhost' http://127.0.0.1:18081/login >/dev/null; then echo PASS; else echo FAIL; fi
