#!/usr/bin/env bash
set -euo pipefail

ENV_FILE=${1:-/opt/cs-mail/.env.production}
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
COMPOSE="$ROOT/deploy/production/docker-compose.yml"
fail(){ echo "PRECHECK FAIL: $*" >&2; exit 1; }
ok(){ echo "ok: $*"; }

for cmd in docker curl openssl ss stat awk grep sed git tar sha256sum flock nginx systemctl timeout df find dig; do
  command -v "$cmd" >/dev/null 2>&1 || fail "required command is missing: $cmd"
done
docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 plugin is required"
[[ -f "$ENV_FILE" ]] || fail "missing $ENV_FILE"
[[ ! -L "$ENV_FILE" ]] || fail "$ENV_FILE must not be a symlink"
perm=$(stat -c '%a' "$ENV_FILE")
(( 10#$perm <= 600 )) || fail "$ENV_FILE must be mode 600 or stricter (got $perm)"
[[ $(stat -c '%u:%g' "$ENV_FILE") == 0:0 ]] || fail "$ENV_FILE must be owned by root:root"
"$ROOT/deploy/production/validate-env.py" "$ENV_FILE" || fail "production environment validation failed"
set -a; source "$ENV_FILE"; set +a

[[ ${CS_MAIL_PROVIDER_NAMESPACE:-} == cs-mail ]] || fail "CS_MAIL_PROVIDER_NAMESPACE must remain cs-mail"
[[ ${CS_MAIL_API_HOST_PORT:-18080} == 18080 ]] || fail "production Nginx expects CS_MAIL_API_HOST_PORT=18080"
[[ ${CS_MAIL_ADMIN_HOST_PORT:-18081} == 18081 ]] || fail "production Nginx expects CS_MAIL_ADMIN_HOST_PORT=18081"
[[ ${CS_MAIL_WEB_HOST:-mail.crescentsphere.com} == mail.crescentsphere.com ]] || fail "CS_MAIL_WEB_HOST must be mail.crescentsphere.com"
[[ ${CS_MAIL_PUBLIC_ORIGIN%/} == "https://${CS_MAIL_WEB_HOST:-mail.crescentsphere.com}" ]] || fail "CS_MAIL_PUBLIC_ORIGIN must exactly match https://CS_MAIL_WEB_HOST"
[[ ${CS_MAIL_CORS_ORIGINS:-} == *"https://${CS_MAIL_WEB_HOST:-mail.crescentsphere.com}"* ]] || fail "CS_MAIL_CORS_ORIGINS must include the public web origin"
case ${CS_MAIL_REQUIRE_PUBLIC_HTTPS_HEALTH:-true} in true|false) ;; *) fail "CS_MAIL_REQUIRE_PUBLIC_HTTPS_HEALTH must be true or false" ;; esac
case ${CS_MAIL_BILLING_INSTANT_ACTIVATION:-true} in true|false) ;; *) fail "CS_MAIL_BILLING_INSTANT_ACTIVATION must be true or false" ;; esac
[[ ${CS_MAIL_CLIENT_HOST:-smtp.crescentsphere.com} == smtp.crescentsphere.com ]] || fail "CS_MAIL_CLIENT_HOST must remain smtp.crescentsphere.com"
[[ ${CS_MAIL_EXPECTED_PTR:-smtp.crescentsphere.com} == smtp.crescentsphere.com ]] || fail "CS_MAIL_EXPECTED_PTR must remain smtp.crescentsphere.com"
[[ ${CS_MAIL_CLIENT_HOST} != ${CS_MAIL_WEB_HOST} ]] || fail "web and mail protocol hostnames must be different"
[[ ${CS_MAIL_WEB_TLS_CERT:-/etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem} == /etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem ]] || fail "CS_MAIL_WEB_TLS_CERT must use the managed mail.crescentsphere.com Let's Encrypt path"
[[ ${CS_MAIL_WEB_TLS_KEY:-/etc/letsencrypt/live/mail.crescentsphere.com/privkey.pem} == /etc/letsencrypt/live/mail.crescentsphere.com/privkey.pem ]] || fail "CS_MAIL_WEB_TLS_KEY must use the managed mail.crescentsphere.com Let's Encrypt path"
[[ -n ${CS_MAIL_SHARED_PROVIDER_NETWORK:-} ]] || fail "shared provider Docker network is not configured"
docker network inspect "$CS_MAIL_SHARED_PROVIDER_NETWORK" >/dev/null 2>&1 || fail "Docker network $CS_MAIL_SHARED_PROVIDER_NETWORK does not exist"

# This VPS hosts multiple web projects. Refuse a duplicate enabled vhost for the
# exact CS Mail hostname rather than relying on Nginx's conflict warning/order.
our_link=${CS_MAIL_NGINX_LINK:-/etc/nginx/sites-enabled/cs-mail.conf}
our_real=$(readlink -f "$our_link" 2>/dev/null || true)
conflicts=()
for dir in /etc/nginx/sites-enabled /etc/nginx/conf.d; do
  [[ -d "$dir" ]] || continue
  while IFS= read -r -d '' f; do
    real=$(readlink -f "$f" 2>/dev/null || printf '%s' "$f")
    [[ -n "$our_real" && "$real" == "$our_real" ]] && continue
    if grep -Eq 'server_name[[:space:]]+[^;]*mail\.crescentsphere\.com([^[:alnum:].-]|;)' "$f"; then
      conflicts+=("$f")
    fi
  done < <(find -L "$dir" -maxdepth 1 -type f -print0)
done
(( ${#conflicts[@]} == 0 )) || fail "mail.crescentsphere.com is already declared by another enabled Nginx config: ${conflicts[*]}"

# Catch the common exact Docker subnet collision before Compose attempts to
# create its bridge. Existing cs-mail-prod_backend is expected on upgrades.
requested_subnet=${CS_MAIL_BACKEND_SUBNET:-172.29.40.0/24}
while IFS= read -r net_id; do
  [[ -n "$net_id" ]] || continue
  net_name=$(docker network inspect -f '{{.Name}}' "$net_id" 2>/dev/null || true)
  [[ "$net_name" == cs-mail-prod_backend ]] && continue
  while IFS= read -r used_subnet; do
    [[ -n "$used_subnet" ]] || continue
    [[ "$used_subnet" != "$requested_subnet" ]] || fail "CS_MAIL_BACKEND_SUBNET=$requested_subnet is already used by Docker network $net_name"
  done < <(docker network inspect -f '{{range .IPAM.Config}}{{println .Subnet}}{{end}}' "$net_id" 2>/dev/null || true)
done < <(docker network ls -q)

avail_kb=$(df -Pk "$ROOT" | awk 'NR==2 {print $4}')
(( avail_kb >= 4*1024*1024 )) || fail "at least 4 GiB free disk is required before deploy"
ok "host tooling, environment permissions and free disk are valid"

cert=${CS_MAIL_WEB_TLS_CERT:-/etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem}
key=${CS_MAIL_WEB_TLS_KEY:-/etc/letsencrypt/live/mail.crescentsphere.com/privkey.pem}
[[ -s "$cert" && -s "$key" ]] || fail "web TLS certificate is missing; run deploy/production/setup-web-tls.sh first"
openssl x509 -in "$cert" -noout -checkend 604800 >/dev/null || fail "web TLS certificate expires within 7 days"
openssl x509 -in "$cert" -noout -checkhost "${CS_MAIL_WEB_HOST}" >/dev/null || fail "web TLS certificate does not cover ${CS_MAIL_WEB_HOST}"
ok "web TLS certificate exists, matches hostname and is not near expiry"

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

mail_host=${CS_MAIL_CLIENT_HOST:-smtp.crescentsphere.com}
timeout 15 openssl s_client -connect "$mail_host:993" -servername "$mail_host" -verify_return_error </dev/null 2>/dev/null | grep -q 'Verify return code: 0' \
  || fail "IMAPS certificate verification failed for $mail_host:993"
timeout 15 openssl s_client -starttls smtp -connect "$mail_host:587" -servername "$mail_host" -verify_return_error </dev/null 2>/dev/null | grep -q 'Verify return code: 0' \
  || fail "SMTP STARTTLS certificate verification failed for $mail_host:587"
ok "public IMAPS/SMTP TLS verifies for $mail_host"

web_host=${CS_MAIL_WEB_HOST:-mail.crescentsphere.com}
dig +short A "$web_host" | grep -q . || fail "$web_host has no A record"
dig +short A "$mail_host" | grep -q . || fail "$mail_host has no A record"
ok "public DNS resolves for web and mail hostnames"

echo "PRECHECK PASS"
