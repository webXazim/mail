#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"

[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a
if [[ -f "$STATE/runtime/current.env" ]]; then source "$STATE/runtime/current.env"; fi
export CS_MAIL_API_IMAGE=${CS_MAIL_API_IMAGE:-cs-mail-api:production}
DB_CAPACITY=${CS_MAIL_DB_CAPACITY_BYTES:-21474836480}

human_bytes() {
  python3 - "$1" <<'PY'
import sys
n=float(sys.argv[1])
for unit in ('B','KiB','MiB','GiB','TiB'):
    if n < 1024 or unit == 'TiB':
        print(f"{n:.2f} {unit}")
        break
    n /= 1024
PY
}

sql_scalar() {
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
    psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -Atqc "$1"
}

printf '=== CS Mail capacity report ===\n'
printf 'Configured PostgreSQL capacity: %s\n' "$(human_bytes "$DB_CAPACITY")"
DB_BYTES=$(sql_scalar "SELECT pg_database_size(current_database())")
printf 'Current PostgreSQL size:        %s\n' "$(human_bytes "$DB_BYTES")"
python3 - "$DB_BYTES" "$DB_CAPACITY" <<'PY'
import sys
used=int(sys.argv[1]); cap=max(int(sys.argv[2]),1)
ratio=used/cap
head=max(cap-used,0)
print(f"Database utilization:          {ratio:.1%}")
print(f"Database declared headroom:    {head/1024**3:.2f} GiB")
if ratio >= .85:
    print("Database capacity state:       CRITICAL (>=85%)")
elif ratio >= .70:
    print("Database capacity state:       WARNING (>=70%)")
else:
    print("Database capacity state:       OK")
PY

printf '\nPostgreSQL volume filesystem (1K blocks):\n'
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  sh -c "df -Pk /var/lib/postgresql/data | tail -n 1" 2>/dev/null || true

printf '\nTenant counts:\n'
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -P pager=off -c \
  "SELECT
     (SELECT count(*) FROM organizations WHERE is_system=FALSE) AS businesses,
     (SELECT count(*) FROM users) AS users,
     (SELECT count(*) FROM mailboxes WHERE deleted_at IS NULL AND status<>'deleted') AS mailboxes;"

printf '\nLargest PostgreSQL tables/indexes:\n'
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
  psql -U "${POSTGRES_USER:-csmail}" -d "${POSTGRES_DB:-csmail}" -P pager=off -c \
  "SELECT relname AS relation,
          pg_size_pretty(pg_total_relation_size(relid)) AS total,
          pg_size_pretty(pg_relation_size(relid)) AS table_only,
          pg_size_pretty(pg_indexes_size(relid)) AS indexes
     FROM pg_catalog.pg_statio_user_tables
    ORDER BY pg_total_relation_size(relid) DESC
    LIMIT 15;"

printf '\nObject/local spool:\n'
printf 'CS Mail object backend:        %s\n' "${CS_MAIL_OBJECT_STORAGE_BACKEND:-local}"
printf 'R2 bucket:                     %s\n' "${CS_MAIL_R2_BUCKET:-(not configured)}"
if docker compose --env-file "$ENV_FILE" -f "$COMPOSE" ps --status running -q api >/dev/null 2>&1; then
  LOCAL_BYTES=$(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T api sh -c "du -sk /srv/attachments 2>/dev/null | awk '{print \\$1 * 1024}'" 2>/dev/null || echo 0)
  LOCAL_BYTES=${LOCAL_BYTES:-0}
  printf 'Local attachment/import spool: %s\n' "$(human_bytes "$LOCAL_BYTES")"
fi

printf '\nHost filesystem:\n'
df -h "$STATE" "$ROOT" 2>/dev/null | awk 'NR==1 || !seen[$1]++'

cat <<'NOTE'

Capacity interpretation:
- PostgreSQL stores CS Mail metadata/queues/drafts/audit state, not the permanent
  mailbox message bodies.
- CS_MAIL_OBJECT_STORAGE_BACKEND=r2 moves CS Mail attachment objects off-VPS,
  but does not move Stalwart's own message blob store.
- To honor multi-GB mailbox quotas on a small VPS, configure the shared Stalwart
  Blob Store to an S3-compatible/R2 backend as a separate provider operation.
- Keep PostgreSQL below ~70% for normal operation and treat 85% as critical so
  VACUUM, indexes, migrations, and temporary files still have working room.
NOTE
