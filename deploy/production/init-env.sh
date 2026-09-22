#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
EXAMPLE="$ROOT/deploy/production/.env.production.example"

install -d -m 0700 "$STATE" "$STATE/secrets"
if [[ ! -f "$ENV_FILE" ]]; then
  [[ -f "$EXAMPLE" ]] || { echo "missing $EXAMPLE" >&2; exit 1; }
  install -m 0600 "$EXAMPLE" "$ENV_FILE"
fi
chmod 600 "$ENV_FILE"

set_value_if_blank() {
  local key=$1 value=$2 current
  current=$(awk -F= -v key="$key" '$1==key {sub(/^[^=]*=/, ""); print; exit}' "$ENV_FILE")
  [[ -n "$current" ]] && return 0
  python3 - "$ENV_FILE" "$key" "$value" <<'PY'
from pathlib import Path
import sys
path=Path(sys.argv[1]); key=sys.argv[2]; value=sys.argv[3]
lines=path.read_text().splitlines()
found=False
for i,line in enumerate(lines):
    if line.startswith(key+'='):
        lines[i]=key+'='+value
        found=True
        break
if not found:
    lines.append(key+'='+value)
path.write_text('\n'.join(lines)+'\n')
PY
}

# Generate only secrets owned by CS Mail. Shared-Stalwart credentials/network
# stay blank until the operator deliberately supplies least-privilege values.
set_value_if_blank POSTGRES_PASSWORD "$(openssl rand -hex 32)"
set_value_if_blank CS_MAIL_JWT_SECRET "$(openssl rand -base64 48 | tr -d '\n')"
set_value_if_blank CS_MAIL_DELIVERY_EVENT_SECRET "$(openssl rand -base64 48 | tr -d '\n')"
set_value_if_blank CS_MAIL_PROVISIONING_KEY "$(openssl rand -base64 48 | tr -d '\n')"
set_value_if_blank CS_MAIL_TOTP_KEY "$(openssl rand -base64 48 | tr -d '\n')"
chmod 600 "$ENV_FILE"

echo "CS Mail environment initialized: $ENV_FILE"
echo "Generated local secrets were preserved if already present. Configure shared Stalwart/network/JMAP credentials and the alert webhook before deploying."
