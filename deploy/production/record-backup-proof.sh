#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
TYPE=${1:-}
PROOF=${2:-}
ENV_FILE=${3:-/opt/cs-mail/.env.production}
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"

case "$TYPE" in
  cs-mail) KIND=cs_mail_offsite_backup ;;
  stalwart) KIND=stalwart_offsite_backup ;;
  *) echo "usage: record-backup-proof.sh {cs-mail|stalwart} /absolute/path/to/backup-proof-manifest [env-file]" >&2; exit 2 ;;
esac

[[ "$PROOF" = /* ]] || { echo "proof manifest path must be absolute" >&2; exit 1; }
[[ -f "$PROOF" && ! -L "$PROOF" && -s "$PROOF" ]] || { echo "proof manifest must be a non-empty regular file, not a symlink" >&2; exit 1; }
[[ $(stat -c '%u' "$PROOF") -eq 0 ]] || { echo "proof manifest must be owned by root" >&2; exit 1; }
mode=$(stat -c '%a' "$PROOF")
(( (8#$mode & 0022) == 0 )) || { echo "proof manifest must not be group/world writable" >&2; exit 1; }
now=$(date +%s)
mtime=$(stat -c '%Y' "$PROOF")
(( now - mtime <= 48*3600 )) || { echo "proof manifest is older than 48 hours; record fresh offsite-backup evidence" >&2; exit 1; }
[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a
if [[ -f "$STATE/runtime/current.env" ]]; then
  # shellcheck disable=SC1090
  source "$STATE/runtime/current.env"
fi
release_sha=${CS_MAIL_RELEASE_SHA256:-}
[[ "$release_sha" =~ ^[0-9a-f]{64}$ ]] || { echo "current deployed release SHA-256 is unavailable" >&2; exit 1; }
proof_sha=$(sha256sum "$PROOF" | awk '{print $1}')

cd "$ROOT"
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -v ON_ERROR_STOP=1 \
    -v kind="$KIND" -v release_sha="$release_sha" -v artifact_ref="$PROOF" -v artifact_sha="$proof_sha" <<'SQL' >/dev/null
INSERT INTO operational_evidence(kind,status,release_sha256,artifact_ref,artifact_sha256,detail)
VALUES (:'kind','passed',:'release_sha',:'artifact_ref',:'artifact_sha',jsonb_build_object('recorded_by','record-backup-proof.sh'));
SQL
printf 'offsite backup proof recorded: %s (%s)\n' "$KIND" "$proof_sha"
