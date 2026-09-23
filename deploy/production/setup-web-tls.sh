#!/usr/bin/env bash
set -euo pipefail
[[ ${EUID:-$(id -u)} -eq 0 ]] || { echo "run as root" >&2; exit 1; }
ROOT=${CS_MAIL_ROOT:-/opt/sites/cs-mail}
STATE=${CS_MAIL_STATE_ROOT:-/opt/cs-mail}
ENV_FILE=${1:-$STATE/.env.production}
BOOTSTRAP="$ROOT/deploy/production/nginx-mail.crescentsphere.com.bootstrap.conf"
FINAL="$ROOT/deploy/production/nginx-mail.crescentsphere.com.conf"
[[ -f "$ENV_FILE" ]] || { echo "missing $ENV_FILE" >&2; exit 1; }
set -a; source "$ENV_FILE"; set +a
if [[ ${CS_MAIL_WEB_PROXY_MODE:-host} == messenger || ${CS_MAIL_WEB_PROXY_MODE:-host} == edge ]]; then
  echo "A Docker edge owns 80/443; follow deploy/production/SHARED_PROXY.md or deploy/edge/README.md" >&2
  exit 1
fi

WEB_HOST=${CS_MAIL_WEB_HOST:-mail.crescentsphere.com}
EMAIL=${CS_MAIL_LETSENCRYPT_EMAIL:-}
WEBROOT=${CS_MAIL_ACME_WEBROOT:-/var/www/letsencrypt}
SITE=${CS_MAIL_NGINX_SITE:-/etc/nginx/sites-available/cs-mail.conf}
LINK=${CS_MAIL_NGINX_LINK:-/etc/nginx/sites-enabled/cs-mail.conf}
CERT=${CS_MAIL_WEB_TLS_CERT:-/etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem}
KEY=${CS_MAIL_WEB_TLS_KEY:-/etc/letsencrypt/live/mail.crescentsphere.com/privkey.pem}

[[ "$WEB_HOST" == mail.crescentsphere.com ]] || { echo "CS_MAIL_WEB_HOST must be mail.crescentsphere.com" >&2; exit 1; }
[[ -n "$EMAIL" && "$EMAIL" == *@*.* ]] || { echo "set CS_MAIL_LETSENCRYPT_EMAIL in $ENV_FILE" >&2; exit 1; }
command -v certbot >/dev/null 2>&1 || { echo "certbot is required; run bootstrap-vps.sh first" >&2; exit 1; }
command -v dig >/dev/null 2>&1 || { echo "dig is required; run bootstrap-vps.sh first" >&2; exit 1; }
dig +short A "$WEB_HOST" | grep -q . || { echo "$WEB_HOST has no A record yet" >&2; exit 1; }

# Refuse a duplicate enabled server_name belonging to another project. Replacing
# our own cs-mail.conf during an Upgrade 37 -> 38 cutover is allowed.
our_real=$(readlink -f "$SITE" 2>/dev/null || true)
conflicts=()
for dir in /etc/nginx/sites-enabled /etc/nginx/conf.d; do
  [[ -d "$dir" ]] || continue
  while IFS= read -r -d '' f; do
    real=$(readlink -f "$f" 2>/dev/null || printf '%s' "$f")
    [[ -n "$our_real" && "$real" == "$our_real" ]] && continue
    if grep -Eq 'server_name[[:space:]]+[^;]*mail\.crescentsphere\.com([^[:alnum:].-]|;)' "$f"; then conflicts+=("$f"); fi
  done < <(find -L "$dir" -maxdepth 1 -type f -print0)
done
(( ${#conflicts[@]} == 0 )) || { echo "mail.crescentsphere.com already belongs to another enabled Nginx config: ${conflicts[*]}" >&2; exit 1; }

install -d -m 0755 "$WEBROOT"
install -m 0644 "$BOOTSTRAP" "$SITE"
ln -sfn "$SITE" "$LINK"
nginx -t
systemctl reload nginx

if [[ ! -s "$CERT" || ! -s "$KEY" ]]; then
  echo "Requesting Let's Encrypt certificate for $WEB_HOST..."
  certbot certonly --webroot -w "$WEBROOT" -d "$WEB_HOST" \
    --email "$EMAIL" --agree-tos --no-eff-email --non-interactive
fi
[[ -s "$CERT" && -s "$KEY" ]] || { echo "certificate/key not created" >&2; exit 1; }

install -d -m 0755 /etc/letsencrypt/renewal-hooks/deploy
cat > /etc/letsencrypt/renewal-hooks/deploy/cs-mail-reload-nginx.sh <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
nginx -t && systemctl reload nginx
HOOK
chmod 0755 /etc/letsencrypt/renewal-hooks/deploy/cs-mail-reload-nginx.sh

install -m 0644 "$FINAL" "$SITE"
ln -sfn "$SITE" "$LINK"
nginx -t
systemctl reload nginx

echo "Web TLS ready: https://$WEB_HOST"
echo "Certificate: $CERT"
