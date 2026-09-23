#!/usr/bin/env bash
set -euo pipefail

[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
[[ $# -eq 1 ]] || { echo "usage: bootstrap-first-admin.sh verified-email@example.com" >&2; exit 2; }
email=${1,,}
[[ "$email" =~ ^[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}$ ]] || {
  echo "enter the exact verified account email address" >&2
  exit 2
}

# Only the first verified account can be promoted here. After an admin exists,
# role changes must use the localhost-only Platform Admin interface.
docker exec -i cs-mail-prod-db-1 sh -c \
  'exec psql -X -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"' <<SQL
BEGIN;
LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE;
DO \$bootstrap\$
BEGIN
  IF EXISTS (SELECT 1 FROM users WHERE platform_role='platform_admin' AND status='active') THEN
    RAISE EXCEPTION 'an active platform admin already exists';
  END IF;
  IF NOT EXISTS (SELECT 1 FROM users WHERE email='$email' AND status='active' AND email_verified_at IS NOT NULL) THEN
    RAISE EXCEPTION 'the account does not exist, is suspended, or is not email verified';
  END IF;
END
\$bootstrap\$;
UPDATE users
SET platform_role='platform_admin', updated_at=now()
WHERE email='$email' AND status='active' AND email_verified_at IS NOT NULL;
COMMIT;
SQL

echo "First platform admin promoted. Sign in with that account's existing password through the SSH-only admin URL."
