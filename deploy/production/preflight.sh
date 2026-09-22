#!/usr/bin/env bash
set -euo pipefail

ENV_FILE=${1:-/opt/cs-mail/.env.production}
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
fail(){ echo "PRECHECK FAIL: $*" >&2; exit 1; }
ok(){ echo "ok: $*"; }

for cmd in docker curl openssl ss stat awk grep sed git tar sha256sum flock nginx systemctl timeout df find; do
  command -v "$cmd" >/dev/null 2>&1 || fail "required command is missing: $cmd"
done
docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 plugin is required"
[[ -f "$ENV_FILE" ]] || fail "missing $ENV_FILE"
perm=$(stat -c '%a' "$ENV_FILE")
(( 10#$perm <= 600 )) || fail "$ENV_FILE must be mode 600 or stricter (got $perm)"
set -a; source "$ENV_FILE"; set +a

[[ ${CS_MAIL_PROVIDER_NAMESPACE:-} == cs-mail ]] || fail "CS_MAIL_PROVIDER_NAMESPACE must remain cs-mail"
[[ ${CS_MAIL_API_HOST_PORT:-18080} == 18080 ]] || fail "production Nginx expects CS_MAIL_API_HOST_PORT=18080"
[[ ${CS_MAIL_PUBLIC_ORIGIN:-} == https://* ]] || fail "CS_MAIL_PUBLIC_ORIGIN must be HTTPS"
[[ -n ${CS_MAIL_SHARED_PROVIDER_NETWORK:-} ]] || fail "shared provider Docker network is not configured"
docker network inspect "$CS_MAIL_SHARED_PROVIDER_NETWORK" >/dev/null 2>&1 || fail "Docker network $CS_MAIL_SHARED_PROVIDER_NETWORK does not exist"

# Build/deploy needs enough room for a Rust image, Node build image and a backup.
avail_kb=$(df -Pk "$ROOT" | awk 'NR==2 {print $4}')
(( avail_kb >= 4*1024*1024 )) || fail "at least 4 GiB free disk is required before deploy"
ok "host tooling, environment permissions and free disk are valid"

alert_file=${CS_MAIL_ALERT_WEBHOOK_FILE:-/opt/cs-mail/secrets/alert-webhook-url}
[[ -f "$alert_file" ]] || fail "missing Alertmanager webhook secret: $alert_file"
alert_perm=$(stat -c '%a' "$alert_file")
(( 10#$alert_perm <= 600 )) || fail "$alert_file must be mode 600 or stricter (got $alert_perm)"
mapfile -t alert_lines < <(sed '/^[[:space:]]*$/d' "$alert_file")
(( ${#alert_lines[@]} == 1 )) || fail "$alert_file must contain exactly one non-empty line"
alert_url=${alert_lines[0]%$'\r'}
[[ "$alert_url" =~ ^https://[^[:space:]]+$ ]] || fail "$alert_file must contain exactly one HTTPS webhook URL"
ok "Alertmanager receiver secret is present and private"

docker compose --env-file "$ENV_FILE" -f "$COMPOSE" config >/dev/null
if docker compose --env-file "$ENV_FILE" -f "$COMPOSE" config --services | grep -qx mail; then
  fail "production compose must not start its own mail service"
fi
ok "production compose has no Stalwart service"

for port in 25 587 993; do
  ss -lnt | awk '{print $4}' | grep -Eq "[:.]${port}$" || fail "shared Stalwart is not listening on host TCP $port"
done
ok "shared Stalwart owns 25/587/993"

host=${CS_MAIL_CLIENT_HOST:-mail.crescentsphere.com}
timeout 15 openssl s_client -connect "$host:993" -servername "$host" -verify_return_error </dev/null 2>/dev/null | grep -q 'Verify return code: 0' \
  || fail "IMAPS certificate verification failed for $host:993"
timeout 15 openssl s_client -starttls smtp -connect "$host:587" -servername "$host" -verify_return_error </dev/null 2>/dev/null | grep -q 'Verify return code: 0' \
  || fail "SMTP STARTTLS certificate verification failed for $host:587"
ok "public IMAPS/SMTP TLS verifies for $host"

if command -v dig >/dev/null 2>&1; then
  dig +short A "$host" | grep -q . || fail "$host has no A record"
  ok "public DNS resolves for $host"
fi

echo "PRECHECK PASS"
