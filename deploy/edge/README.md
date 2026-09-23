# Independent platform edge on the shared VPS

This is the direct first-deployment path for `mail.crescentsphere.com`.
Source lives in CS Mail's `deploy/edge/`; its separate VPS Compose project
lives at `/srv/crescentsphere/platform-edge`. HAProxy takes public 80/443
after cutover. It passes root, `www`, and `dm` to Messenger's existing Nginx,
but sends `mail` to a dedicated Nginx TLS gateway and the private CS Mail web
container. CS Mail does not use a Messenger vhost. Mailer stays on its own
Cloudflare Tunnel; Stalwart stays on `smtp.crescentsphere.com`.

Keep Messenger on public 80/443 until section 3. A staged edge uses only
loopback 18082/18443. CS Mail's Docker backend network does not exist until
the first deployment; that is expected and is not a staging prerequisite.

## 1. Issue the web certificate with DNS-01

Create a Cloudflare API token scoped to the `crescentsphere.com` zone with
Zone DNS Edit permission. Install the DNS Cloudflare plugin from the same
package source as Certbot. For the apt-installed Certbot from
`deploy/production/bootstrap-vps.sh`:

```bash
sudo apt-get update
sudo apt-get install --no-install-recommends python3-certbot-dns-cloudflare
sudo certbot plugins | grep dns-cloudflare
sudo install -d -m 0700 /etc/letsencrypt/cloudflare
sudoedit /etc/letsencrypt/cloudflare/dns.ini
sudo chmod 0600 /etc/letsencrypt/cloudflare/dns.ini
```

The credentials file must contain one line with the real token and no shell
quotes. Never put the token in Git, chat, or shell history:

```ini
dns_cloudflare_api_token = REPLACE_WITH_CLOUDFLARE_TOKEN
```

Replace the contact address with `CS_MAIL_LETSENCRYPT_EMAIL` from the private
production environment:

```bash
sudo certbot certonly --dns-cloudflare \
  --dns-cloudflare-credentials /etc/letsencrypt/cloudflare/dns.ini \
  --dns-cloudflare-propagation-seconds 60 \
  -d mail.crescentsphere.com --email YOUR_CONTACT_EMAIL \
  --agree-tos --no-eff-email --non-interactive
sudo test -s /etc/letsencrypt/live/mail.crescentsphere.com/fullchain.pem
sudo test -s /etc/letsencrypt/live/mail.crescentsphere.com/privkey.pem
```

Certbot records the credentials-file path for renewal. Keep that file
root-owned and mode 0600. If Certbot came from Snap or another package source,
install its matching DNS plugin instead of mixing packages.

## 2. Stage on loopback

Check that `172.29.41.0/24` is not used by another Docker network. If it is,
change the subnet and the fixed IP references in Compose, HAProxy, and Nginx
together. The edge creates `cs-platform-web` for CS Mail; it only needs the
existing Messenger Docker network before startup.

```bash
sudo docker network inspect cs-messenger_messenger >/dev/null
sudo install -d -m 0755 /srv/crescentsphere/platform-edge
sudo cp /opt/sites/cs-mail/deploy/edge/docker-compose.yml \
  /opt/sites/cs-mail/deploy/edge/haproxy.cfg \
  /opt/sites/cs-mail/deploy/edge/nginx-mail.conf \
  /opt/sites/cs-mail/deploy/edge/reload-on-renew.sh \
  /srv/crescentsphere/platform-edge/
sudo test -e /srv/crescentsphere/platform-edge/.env || \
  sudo cp /opt/sites/cs-mail/deploy/edge/.env.example \
  /srv/crescentsphere/platform-edge/.env
cd /srv/crescentsphere/platform-edge
sudo docker compose config --quiet
sudo docker compose up -d
sudo docker compose ps
sudo docker exec cs-platform-edge-edge-1 haproxy -c -f /usr/local/etc/haproxy/haproxy.cfg
sudo docker exec cs-platform-edge-mail_tls-1 nginx -t
sudo docker network inspect cs-platform-web >/dev/null
```

Probe `dm` and root through staging. Their direct local HTTPS probes use `-k`
because Messenger uses a Cloudflare Origin CA certificate. The mail probe must
validate the Let's Encrypt certificate without `-k`; HTTP 502 is expected
until CS Mail starts.

```bash
curl --noproxy '*' --resolve dm.crescentsphere.com:18443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://dm.crescentsphere.com:18443/
curl --noproxy '*' --resolve crescentsphere.com:18443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://crescentsphere.com:18443/
curl --noproxy '*' --resolve mail.crescentsphere.com:18443:127.0.0.1 \
  -sS -o /dev/null -w '%{http_code}\n' https://mail.crescentsphere.com:18443/
```

## 3. Move public ports to the edge

Use a maintenance window. The active Messenger release must include the
reviewed Compose change that removes only Nginx's public 80/443 bindings.
Its private Nginx, TLS files, and root/`dm` vhosts remain. Do not recreate
Messenger Nginx until all staging probes pass.

```bash
MESSENGER_DIR=$(sudo docker inspect cs-messenger-nginx-1 \
  --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}')
sudo cp -a "$MESSENGER_DIR/docker-compose.production.yml" \
  "$MESSENGER_DIR/docker-compose.production.yml.before-platform-edge"
# Ensure the active release file and canonical Messenger source omit nginx ports.
cd "$MESSENGER_DIR"
sudo docker compose -p cs-messenger -f docker-compose.yml \
  -f docker-compose.production.yml config --quiet
sudo docker compose -p cs-messenger -f docker-compose.yml \
  -f docker-compose.production.yml up -d --no-deps nginx
```

Set `/srv/crescentsphere/platform-edge/.env` to these public bindings:

```dotenv
CS_EDGE_HTTP_BIND=0.0.0.0:80
CS_EDGE_HTTPS_BIND=0.0.0.0:443
```

Then publish only the edge service and verify Messenger remains reachable:

```bash
cd /srv/crescentsphere/platform-edge
sudo docker compose up -d --force-recreate edge
sudo ss -lntp '( sport = :80 or sport = :443 )'
curl --noproxy '*' --resolve dm.crescentsphere.com:443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://dm.crescentsphere.com/
curl --noproxy '*' --resolve crescentsphere.com:443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://crescentsphere.com/
```

## 4. Deploy CS Mail directly to the edge

Set these values in `/opt/cs-mail/.env.production`:

```dotenv
CS_MAIL_WEB_PROXY_MODE=edge
CS_MAIL_SHARED_WEB_NETWORK=cs-platform-web
CS_MAIL_SHARED_EDGE_CONTAINER=cs-platform-edge-edge-1
CS_MAIL_TRUSTED_PROXY_IPS=172.29.40.10,172.29.40.11
CS_MAIL_BACKEND_SUBNET=172.29.40.0/24
```

Keep `CS_MAIL_SHARED_PROVIDER_NETWORK` pointed at the existing Stalwart
network. Complete all other credentials and the alert receiver in
`deploy/production/CREDENTIALS.md`. The first CS Mail deploy creates
`cs-mail-prod_backend` and its web container. The mail route may return 502
until this deployment completes.

```bash
cd /opt/sites/cs-mail
sh manage preflight
sh manage deploy
sh manage status
curl --noproxy '*' --resolve mail.crescentsphere.com:443:127.0.0.1 \
  -fsS https://mail.crescentsphere.com/api/health/ready
```

Complete `deploy/production/GO_LIVE.md` before changing MX or admitting
customers. Keep existing Cloudflare Email Routing MX until a real mailbox has
passed external send and receive checks.

## 5. Renewal and rollback

```bash
sudo install -m 0755 /srv/crescentsphere/platform-edge/reload-on-renew.sh \
  /etc/letsencrypt/renewal-hooks/deploy/cs-mail-reload-edge.sh
sudo certbot renew --dry-run
```

If the edge fails, stop its published `edge` service to release 80/443, then
restore the saved Messenger production Compose and recreate only Messenger
Nginx. Preserve CS Mail's database, Stalwart's data, and mailbox DNS. The
company homepage can later replace Messenger's root/`www` route without
changing `dm`, `mail`, or `mailer`.
