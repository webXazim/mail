#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
CERT_ENV=${2:-$STATE/.env.certification}
RUNTIME_RELEASE=${CS_MAIL_RELEASE_METADATA_FILE:-$STATE/runtime/current.env}

[[ -f "$ENV_FILE" ]] || { echo "missing runtime environment file: $ENV_FILE" >&2; exit 1; }
[[ -f "$CERT_ENV" ]] || { echo "missing certification secrets file: $CERT_ENV" >&2; exit 1; }
[[ -f "$RUNTIME_RELEASE" ]] || { echo "missing deployed release metadata: $RUNTIME_RELEASE" >&2; exit 1; }
for f in "$ENV_FILE" "$CERT_ENV" "$RUNTIME_RELEASE"; do
  perm=$(stat -c '%a' "$f")
  (( 10#$perm <= 600 )) || { echo "$f must be mode 600 or stricter (got $perm)" >&2; exit 1; }
done

merged=$(mktemp)
trap 'rm -f "$merged"' EXIT
chmod 600 "$merged"
cat "$ENV_FILE" > "$merged"
printf '\n# generated deployment identity\n' >> "$merged"
cat "$RUNTIME_RELEASE" >> "$merged"

python3 "$ROOT/deploy/production/certify_launch.py" \
  --root "$ROOT" \
  --env-file "$merged" \
  --cert-env-file "$CERT_ENV"
