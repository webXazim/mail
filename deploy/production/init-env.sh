#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
EXAMPLE="$ROOT/deploy/production/.env.production.example"

install -d -o root -g root -m 0711 "$STATE"
install -d -o root -g root -m 0700 "$STATE/secrets"
[[ ! -L "$ENV_FILE" ]] || { echo "$ENV_FILE must not be a symlink" >&2; exit 1; }
if [[ ! -f "$ENV_FILE" ]]; then
  [[ -f "$EXAMPLE" ]] || { echo "missing $EXAMPLE" >&2; exit 1; }
  install -m 0600 "$EXAMPLE" "$ENV_FILE"
fi
chown root:root "$ENV_FILE"
chmod 600 "$ENV_FILE"

# Migrate the previous Tunnel-era schema to the direct-Nginx topology. These
# keys/defaults were introduced by Upgrade 37 and are no longer authoritative.
python3 - "$ENV_FILE" <<'PYMIGRATE'
from pathlib import Path
import sys
p=Path(sys.argv[1])
lines=[]
for line in p.read_text().splitlines():
    if line.startswith("CS_MAIL_TUNNEL_ORIGIN_PORT=") or line.startswith("CS_MAIL_REQUIRE_PUBLIC_TUNNEL_HEALTH="):
        continue
    if line == "CS_MAIL_CLIENT_HOST=mx.crescentsphere.com":
        line = "CS_MAIL_CLIENT_HOST=smtp.crescentsphere.com"
    elif line == "CS_MAIL_EXPECTED_PTR=mx.crescentsphere.com":
        line = "CS_MAIL_EXPECTED_PTR=smtp.crescentsphere.com"
    lines.append(line)
p.write_text("\n".join(lines)+"\n")
PYMIGRATE

# Append newly introduced keys without overwriting operator values.
python3 - "$EXAMPLE" "$ENV_FILE" <<'PYENV'
from pathlib import Path
import re, sys
example=Path(sys.argv[1]).read_text().splitlines()
target=Path(sys.argv[2])
lines=target.read_text().splitlines()
keys={line.split("=",1)[0] for line in lines if re.match(r"^[A-Z][A-Z0-9_]*=", line)}
missing=[]
for line in example:
    if re.match(r"^[A-Z][A-Z0-9_]*=", line):
        key=line.split("=",1)[0]
        if key not in keys:
            missing.append(line)
if missing:
    lines += ["", "# Added by CS Mail configuration schema upgrade"] + missing
    target.write_text("\n".join(lines)+"\n")
PYENV

set_value_if_blank() {
  local key=$1 value=$2 current
  current=$(awk -F= -v key="$key" '$1==key {sub(/^[^=]*=/, ""); print; exit}' "$ENV_FILE")
  [[ -n "$current" ]] && return 0
  python3 - "$ENV_FILE" "$key" "$value" <<'PY'
from pathlib import Path
import sys
path=Path(sys.argv[1]); key=sys.argv[2]; value=sys.argv[3]
lines=path.read_text().splitlines(); found=False
for i,line in enumerate(lines):
    if line.startswith(key+'='):
        lines[i]=key+'='+value; found=True; break
if not found: lines.append(key+'='+value)
path.write_text('\n'.join(lines)+'\n')
PY
}

# Generate only secrets owned by CS Mail. Shared-Stalwart credentials/network
# remain operator-supplied and are never guessed or overwritten.
set_value_if_blank POSTGRES_PASSWORD "$(openssl rand -hex 32)"
set_value_if_blank CS_MAIL_JWT_SECRET "$(openssl rand -base64 48 | tr -d '\n')"
set_value_if_blank CS_MAIL_DELIVERY_EVENT_SECRET "$(openssl rand -base64 48 | tr -d '\n')"
set_value_if_blank CS_MAIL_PROVISIONING_KEY "$(openssl rand -base64 48 | tr -d '\n')"
set_value_if_blank CS_MAIL_TOTP_KEY "$(openssl rand -base64 48 | tr -d '\n')"
chown root:root "$ENV_FILE"
chmod 600 "$ENV_FILE"

echo "CS Mail environment initialized: $ENV_FILE"
echo "Generated CS Mail-owned secrets were preserved if already present."
echo "Operator input still required: Stalwart network/token/JMAP credentials, Let's Encrypt email, alert webhook."
echo "Web: https://mail.crescentsphere.com via shared host Nginx :443"
echo "Mail protocols/PTR: smtp.crescentsphere.com (DNS-only)"
echo "Next: sudoedit $ENV_FILE, then run setup-web-tls.sh before the first deploy."
