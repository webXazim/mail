#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }

ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${CS_MAIL_ENV_FILE:-$STATE/.env.production}
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y --no-install-recommends \
    ca-certificates certbot curl git nginx openssl dnsutils jq rsync tar gzip util-linux python3
fi

command -v docker >/dev/null 2>&1 || {
  echo "Docker is required. Install Docker Engine + Compose plugin before continuing." >&2
  exit 1
}
docker compose version >/dev/null 2>&1 || {
  echo "Docker Compose v2 plugin is required." >&2
  exit 1
}

install -d -m 0755 "$ROOT"
install -d -o root -g root -m 0711 "$STATE"
install -d -o root -g root -m 0700 "$STATE/secrets" "$STATE/runtime" "$STATE/runtime/deployments"
install -d -m 0755 "$STATE/www" "$STATE/www/releases" /var/www/letsencrypt
install -d -m 0700 /opt/backups/cs-mail /var/log/cs-mail

"$SCRIPT_DIR/init-env.sh" "$ENV_FILE"

# Create the alert secret file if absent, but never guess a receiver URL.
if [[ ! -e "$STATE/secrets/alert-webhook-url" ]]; then
  install -o root -g root -m 0600 /dev/null "$STATE/secrets/alert-webhook-url"
fi

install -m 0644 "$SCRIPT_DIR/systemd/cs-mail-backup.service" /etc/systemd/system/cs-mail-backup.service
install -m 0644 "$SCRIPT_DIR/systemd/cs-mail-backup.timer" /etc/systemd/system/cs-mail-backup.timer
systemctl daemon-reload
systemctl enable --now cs-mail-backup.timer

echo "CS Mail VPS bootstrap PASS"
echo "1) Edit: sudoedit $ENV_FILE"
echo "2) Fill the REQUIRED OPERATOR INPUT values documented in CREDENTIALS.md"
echo "3) Put one HTTPS alert receiver URL in $STATE/secrets/alert-webhook-url"
echo "4) Create DNS-only A mail.crescentsphere.com -> this VPS"
echo "5) Run: $SCRIPT_DIR/setup-web-tls.sh $ENV_FILE"
echo "6) Deploy: $SCRIPT_DIR/deploy-from-git.sh main"
echo "Mail/PTR identity remains smtp.crescentsphere.com."
