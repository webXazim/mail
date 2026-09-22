#!/usr/bin/env bash
set -euo pipefail
ENV_FILE=${1:-/opt/cs-mail/.env.production}
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
BACKUP_DIR=${CS_MAIL_BACKUP_DIR:-/opt/backups/cs-mail}
DUMP=${2:-$(ls -1t "$BACKUP_DIR"/cs-mail-*.dump 2>/dev/null | head -1)}
[[ -n ${DUMP:-} && -f "$DUMP" ]] || { echo "no dump found" >&2; exit 1; }
BASE=${DUMP%.dump}
ATTACH="$BASE.attachments.tgz"
MANIFEST="$BASE.manifest.txt"
[[ -s "$ATTACH" && -s "$MANIFEST" ]]
tar -tzf "$ATTACH" >/dev/null
expected=$(grep '^db_sha256=' "$MANIFEST" | cut -d= -f2)
[[ $(sha256sum "$DUMP" | awk '{print $1}') == "$expected" ]]
expected=$(grep '^attachments_sha256=' "$MANIFEST" | cut -d= -f2)
[[ $(sha256sum "$ATTACH" | awk '{print $1}') == "$expected" ]]

cd "$ROOT"
DB=csmail_restore_drill
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db dropdb -U csmail --if-exists "$DB" >/dev/null
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db createdb -U csmail "$DB"
cleanup() {
  rm -f "${race_log:-}" "${race_second_log:-}" 2>/dev/null || true
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db dropdb -U csmail --if-exists "$DB" >/dev/null 2>&1 || true
}
trap cleanup EXIT
cat "$DUMP" | docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db pg_restore -U csmail -d "$DB" --no-owner --no-privileges
for t in users organizations organization_domains mailboxes mail_send_requests scheduled_sends staged_attachments mailbox_imports mailbox_app_passwords launch_certification_runs; do
  live=$(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db psql -U csmail -d csmail -tAc "select count(*) from $t")
  drill=$(docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db psql -U csmail -d "$DB" -tAc "select count(*) from $t")
  [[ ${live//[[:space:]]/} == ${drill//[[:space:]]/} ]] || { echo "row-count mismatch: $t" >&2; exit 1; }
done

# Prove the global customer-domain uniqueness guard under a real
# concurrent race on the isolated restore database. The first transaction holds
# the unique key while sleeping; the second insert must block then fail rather
# than creating the same domain for another business.
db_psql() {
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db \
    psql -U csmail -d "$DB" -v ON_ERROR_STOP=1 "$@"
}
race_tag=$(date -u +%s)-$$
org1=$(db_psql -qAt -c "INSERT INTO organizations(name,slug) VALUES ('Certification race A','cert-race-a-$race_tag') RETURNING id" | tr -d '[:space:]')
org2=$(db_psql -qAt -c "INSERT INTO organizations(name,slug) VALUES ('Certification race B','cert-race-b-$race_tag') RETURNING id" | tr -d '[:space:]')
race_domain="cert-race-$race_tag.invalid"
race_log=$(mktemp)
race_second_log=$(mktemp)
(
  db_psql >"$race_log" 2>&1 <<SQL
BEGIN;
INSERT INTO organization_domains(organization_id,domain) VALUES ('$org1','$race_domain');
SELECT pg_sleep(3);
COMMIT;
SQL
) &
race_pid=$!
sleep 0.5
set +e
db_psql -c "INSERT INTO organization_domains(organization_id,domain) VALUES ('$org2','$race_domain')" >"$race_second_log" 2>&1
race_second_rc=$?
set -e
wait "$race_pid" || { cat "$race_log" >&2; exit 1; }
if [[ $race_second_rc -eq 0 ]]; then
  echo "domain race guard FAIL: duplicate customer domain insert succeeded" >&2
  exit 1
fi
if ! grep -Eqi 'duplicate key|unique constraint|organization_domains_global_unique_idx' "$race_second_log"; then
  echo "domain race guard FAIL: second insert failed for an unexpected reason" >&2
  cat "$race_second_log" >&2
  exit 1
fi
db_psql -c "DELETE FROM organizations WHERE id IN ('$org1','$org2')" >/dev/null
rm -f "$race_log" "$race_second_log"
race_log=
race_second_log=
echo "domain race drill PASS"

trap - EXIT
docker compose --env-file "$ENV_FILE" -f "$COMPOSE" exec -T db dropdb -U csmail "$DB"
echo "restore drill PASS"
