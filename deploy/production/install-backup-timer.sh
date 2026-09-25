#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }

ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-${CS_MAIL_ENV_FILE:-$STATE/.env.production}}
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
SERVICE=/etc/systemd/system/cs-mail-backup.service
TIMER=/etc/systemd/system/cs-mail-backup.timer

for value in "$ROOT" "$STATE" "$ENV_FILE"; do
  [[ "$value" != *$'\n'* && "$value" != *$'\r'* ]] || { echo "backup service paths must be one line" >&2; exit 1; }
done
[[ -f "$ROOT/deploy/production/backup.sh" ]] || { echo "missing backup script under $ROOT" >&2; exit 1; }
[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }

# systemd supports quoted Environment values, but ExecStart argument parsing is
# intentionally kept simple for production paths. Reject whitespace rather than
# generating an ambiguous service command.
case "$ROOT" in
  *[[:space:]]*) echo "CS Mail checkout path must not contain whitespace: $ROOT" >&2; exit 1 ;;
esac
case "$ENV_FILE" in
  *[[:space:]]*) echo "CS Mail env-file path must not contain whitespace: $ENV_FILE" >&2; exit 1 ;;
esac

cat > "$SERVICE" <<EOF
[Unit]
Description=CS Mail production backup
After=docker.service
Requires=docker.service

[Service]
Type=oneshot
Environment="CS_MAIL_ROOT=$ROOT"
Environment="CS_MAIL_STATE_ROOT=$STATE"
Environment="CS_MAIL_ENV_FILE=$ENV_FILE"
ExecStart=/usr/bin/env bash $ROOT/deploy/production/backup.sh $ENV_FILE
Nice=10
IOSchedulingClass=best-effort
IOSchedulingPriority=7
EOF
install -m 0644 "$SCRIPT_DIR/systemd/cs-mail-backup.timer" "$TIMER"
systemctl daemon-reload
systemctl enable --now cs-mail-backup.timer >/dev/null

echo "CS Mail backup timer installed for checkout: $ROOT"
