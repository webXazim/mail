#!/usr/bin/env bash
set -euo pipefail
ROOT=${CS_MAIL_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
fail(){ echo "RELEASE CHECK FAIL: $*" >&2; exit 1; }
ok(){ echo "ok: $*"; }

[[ -f "$ROOT/backend/Cargo.lock" ]] || fail "backend/Cargo.lock is required"
[[ -f "$ROOT/frontend/package-lock.json" ]] || fail "frontend/package-lock.json is required"
[[ -f "$ROOT/frontend/Dockerfile.production" ]] || fail "frontend production Dockerfile is missing"
[[ -f "$ROOT/deploy/production/docker-compose.yml" ]] || fail "production compose is missing"
[[ -f "$ROOT/deploy/production/nginx-mail.crescentsphere.com.conf" ]] || fail "production Nginx config is missing"
[[ -f "$ROOT/deploy/production/nginx-mail.crescentsphere.com.bootstrap.conf" ]] || fail "TLS bootstrap Nginx config is missing"
for shared_file in nginx-inner.conf nginx-admin-inner.conf nginx-shared-edge.bootstrap.conf nginx-shared-edge.conf SHARED_PROXY.md; do
  [[ -f "$ROOT/deploy/production/$shared_file" ]] || fail "shared Messenger proxy file is missing: $shared_file"
done
[[ -f "$ROOT/deploy/production/CREDENTIALS.md" ]] || fail "production credential checklist is missing"
for script in deploy.sh deploy-from-git.sh bootstrap-vps.sh init-env.sh setup-web-tls.sh preflight.sh backup.sh restore-drill.sh rollback.sh status.sh certify-launch.sh clean-worktree.sh smoke-test.sh show-config.sh validate-env.py render-alertmanager.py; do
  [[ -x "$ROOT/deploy/production/$script" ]] || fail "production script is missing/not executable: $script"
done
for edge_file in .env.example docker-compose.yml haproxy.cfg nginx-mail.conf reload-on-renew.sh README.md; do
  [[ -f "$ROOT/deploy/edge/$edge_file" ]] || fail "independent platform edge file is missing: $edge_file"
done
ok "production deployment scripts are present and executable"

[[ -f "$ROOT/deploy/production/CONFIGURATION.md" ]] || fail "production configuration guide is missing"
nginx_file="$ROOT/deploy/production/nginx-mail.crescentsphere.com.conf"
grep -q 'server_name mail.crescentsphere.com;' "$nginx_file" || fail "web vhost must target mail.crescentsphere.com"
grep -q 'listen 443 ssl;' "$nginx_file" || fail "web vhost must listen on shared HTTPS :443"
grep -q 'listen 80;' "$nginx_file" || fail "web vhost must listen on shared HTTP :80 for ACME/redirect"
grep -q 'listen 127.0.0.1:18081;' "$nginx_file" || fail "Platform Admin must remain localhost-only"
if grep -q 'listen 127.0.0.1:18082' "$nginx_file"; then fail "legacy Cloudflare Tunnel origin must not remain"; fi
if grep -q 'real_ip_header CF-Connecting-IP' "$nginx_file"; then fail "Cloudflare-only real-IP trust must not remain in direct-DNS mode"; fi
grep -q 'CS_MAIL_WEB_PROXY_MODE=edge' "$ROOT/deploy/production/.env.production.example" || fail "edge proxy mode is missing from the production env contract"
grep -q 'CS_MAIL_SHARED_WEB_NETWORK=cs-platform-web' "$ROOT/deploy/production/.env.production.example" || fail "platform web network is missing from the production env contract"
grep -q 'aliases: \[cs-mail-web\]' "$ROOT/deploy/production/docker-compose.yml" || fail "CS Mail private web alias is missing"
grep -q '127.0.0.1:.*:8081' "$ROOT/deploy/production/docker-compose.yml" || fail "admin web must bind only to loopback"
grep -q 'server_name mail.crescentsphere.com;' "$ROOT/deploy/production/nginx-shared-edge.conf" || fail "Messenger edge CS Mail vhost is missing"
grep -q 'ssl_certificate /etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem;' "$ROOT/deploy/production/nginx-shared-edge.conf" || fail "Messenger edge must use a public Let's Encrypt certificate"
for route in '/api/admin' '/mail/admin' '/api/metrics'; do
  grep -q "$route" "$ROOT/deploy/production/nginx-inner.conf" || fail "public inner web is missing protected route $route"
done
grep -q 'name: cs-platform-web' "$ROOT/deploy/edge/docker-compose.yml" || fail "independent platform web network is missing"
ok "independent platform edge topology is configured"

for generated in frontend/node_modules frontend/dist frontend/coverage backend/target backend/.cs-mail-target .cache; do
  [[ ! -e "$ROOT/$generated" ]] || fail "generated path must not be committed: $generated"
done
ok "generated dependency/build directories are absent"

if find "$ROOT" -type f \( -name '.env.production' -o -name '.env.certification' -o -name '*.pem' -o -name '*.key' -o -name 'id_rsa' -o -name 'id_ed25519' \) -print -quit | grep -q .; then
  fail "private environment/key material is present in the repository"
fi
if find "$ROOT" -type f -name '*cloudflared*.json' -print -quit | grep -q .; then
  fail "Cloudflare credential JSON must never be committed"
fi
ok "no obvious production secret/key files are present"

if grep -Eq '^[[:space:]]{2}(mail|stalwart):[[:space:]]*$' "$ROOT/deploy/production/docker-compose.yml"; then
  fail "production compose must not define a mail/Stalwart service"
fi
for port in 25 465 587 993; do
  if grep -Eq "^[[:space:]]*-[[:space:]]*['\"]?[^#]*:${port}([:/\"']|$)" "$ROOT/deploy/production/docker-compose.yml"; then
    fail "production compose must not bind host mail port $port"
  fi
done
ok "production compose preserves shared-Stalwart ownership"

grep -q 'cargo build --release --locked' "$ROOT/backend/Dockerfile" || fail "backend image must build Cargo.lock with --locked"
grep -q 'cargo clippy --locked --all-targets' "$ROOT/backend/Dockerfile" || fail "backend deploy image must run Clippy"
grep -q 'cargo test --locked --all-targets' "$ROOT/backend/Dockerfile" || fail "backend deploy image must run Rust tests"
grep -q 'npm ci' "$ROOT/frontend/Dockerfile.production" || fail "frontend image must use npm ci"
ok "locked backend/frontend builds are configured"

python3 "$ROOT/deploy/production/certify_launch.py" --static-only --root "$ROOT" --report /tmp/cs-mail-release-static.json >/dev/null
ok "static production launch certification passes"

echo "CS Mail release verification PASS"
