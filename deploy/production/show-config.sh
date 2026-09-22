#!/usr/bin/env bash
set -euo pipefail
ENV_FILE=${1:-/opt/cs-mail/.env.production}
[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }

# Never print credential values. This is safe to paste into an operations
# ticket while still making configuration drift visible.
secret_re='^(POSTGRES_PASSWORD|CS_MAIL_JWT_SECRET|CS_MAIL_DELIVERY_EVENT_SECRET|CS_MAIL_PROVISIONING_KEY|CS_MAIL_TOTP_KEY|CS_MAIL_MAIL_ADMIN_TOKEN|CS_MAIL_MAIL_ADMIN_SECRET|CS_MAIL_MAIL_JMAP_SECRET|CS_MAIL_SMTP_PASSWORD)='
while IFS= read -r line || [[ -n "$line" ]]; do
  if [[ "$line" =~ $secret_re ]]; then
    key=${line%%=*}
    value=${line#*=}
    if [[ -n "$value" ]]; then
      printf '%s=<redacted:set>\n' "$key"
    else
      printf '%s=<redacted:empty>\n' "$key"
    fi
  else
    printf '%s\n' "$line"
  fi
done < "$ENV_FILE"
