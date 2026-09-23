# CS Mail on the existing Messenger web proxy

The current VPS publishes web ports 80/443 from `cs-messenger-nginx-1` on the
`cs-messenger_messenger` Docker network. Host Nginx must stay inactive. CS Mail
uses a private `web` container on that network and a separate admin container
published only on `127.0.0.1:18081`. Stalwart remains the shared mail server.

`mail.crescentsphere.com` stays DNS-only. The Cloudflare Origin CA certificate
already mounted by Messenger is not publicly trusted for direct connections,
so this vhost uses a separate Let's Encrypt certificate. Do not replace the
Messenger certificate or its existing virtual hosts.

## 1. Configure the CS Mail environment

In `/opt/cs-mail/.env.production`, set each key exactly once:

```dotenv
CS_MAIL_WEB_PROXY_MODE=messenger
CS_MAIL_SHARED_WEB_NETWORK=cs-messenger_messenger
CS_MAIL_SHARED_EDGE_CONTAINER=cs-messenger-nginx-1
CS_MAIL_TRUSTED_PROXY_IPS=172.29.40.10,172.29.40.11
CS_MAIL_BACKEND_SUBNET=172.29.40.0/24
```

Keep `CS_MAIL_SHARED_PROVIDER_NETWORK=crescentsphere-mail-transport` for the
existing Stalwart. The control API key requires `sysAccountGet` and
`sysDomainGet`: the app resolves each target mailbox before using Stalwart's
`target%cs-mail-jmap@svc.crescentsphere.com` impersonation login.

## 2. Add an HTTP challenge vhost to Messenger Nginx

The active Messenger release from Docker's Compose labels is currently
`/srv/crescentsphere/releases/20260807T182934Z/messenger`. Repeat these
changes in Messenger's canonical deployment source; editing only a generated
release will be lost on its next redeploy.

Back up its two files before editing:

```bash
MESSENGER_DIR=/srv/crescentsphere/releases/20260807T182934Z/messenger
sudo cp -a "$MESSENGER_DIR/docker-compose.production.yml" "$MESSENGER_DIR/docker-compose.production.yml.before-cs-mail"
sudo cp -a "$MESSENGER_DIR/nginx/snm.production.conf" "$MESSENGER_DIR/nginx/snm.production.conf.before-cs-mail"
sudo install -d -m 0755 /var/www/letsencrypt/.well-known/acme-challenge
```

Under `services.nginx.volumes` in Messenger's
`docker-compose.production.yml`, **add** these two bind mounts alongside the
existing TLS and snippets mounts:

```yaml
      - /var/www/letsencrypt:/var/www/letsencrypt:ro
      - /etc/letsencrypt:/etc/letsencrypt:ro
```

Copy the HTTP-only vhost into Messenger's already-mounted snippets directory:

```bash
sudo install -m 0644 /opt/sites/cs-mail/deploy/production/nginx-shared-edge.bootstrap.conf \
  "$MESSENGER_DIR/nginx/snippets/cs-mail.conf"
```

Add this single top-level line to the **end** of
`$MESSENGER_DIR/nginx/snm.production.conf`:

```nginx
include /etc/nginx/snippets/app/cs-mail.conf;
```

Only one copy of this include may exist. Validate Compose and recreate just
Messenger's Nginx container to activate the new read-only mounts:

```bash
cd "$MESSENGER_DIR"
sudo docker compose -p cs-messenger -f docker-compose.yml -f docker-compose.production.yml config --quiet
sudo docker compose -p cs-messenger -f docker-compose.yml -f docker-compose.production.yml up -d --no-deps nginx
sudo docker exec cs-messenger-nginx-1 nginx -t
```

Check the challenge before asking Let's Encrypt for a certificate:

```bash
printf 'ok\n' | sudo tee /var/www/letsencrypt/.well-known/acme-challenge/cs-mail-probe >/dev/null
curl --noproxy '*' --resolve mail.crescentsphere.com:80:127.0.0.1 -fsS \
  http://mail.crescentsphere.com/.well-known/acme-challenge/cs-mail-probe
sudo rm /var/www/letsencrypt/.well-known/acme-challenge/cs-mail-probe
```

The response must be `ok`. `mail.crescentsphere.com` must also have a DNS-only
A record pointing at this VPS and public port 80 must be reachable.

## 3. Issue the certificate and activate the HTTPS vhost

Use the contact address configured in `CS_MAIL_LETSENCRYPT_EMAIL`:

```bash
sudo certbot certonly --webroot -w /var/www/letsencrypt \
  -d mail.crescentsphere.com --email YOUR_CONTACT_EMAIL \
  --agree-tos --no-eff-email --non-interactive
sudo install -m 0644 /opt/sites/cs-mail/deploy/production/nginx-shared-edge.conf \
  "$MESSENGER_DIR/nginx/snippets/cs-mail.conf"
sudo docker exec cs-messenger-nginx-1 nginx -t
sudo docker exec cs-messenger-nginx-1 nginx -s reload
```

Install the certificate renewal reload hook (Certbot writes renewed certs to
the host path already mounted by Messenger's Nginx):

```bash
sudo install -d -m 0755 /etc/letsencrypt/renewal-hooks/deploy
sudo tee /etc/letsencrypt/renewal-hooks/deploy/cs-mail-reload-edge.sh >/dev/null <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
docker exec cs-messenger-nginx-1 nginx -t
docker exec cs-messenger-nginx-1 nginx -s reload
HOOK
sudo chmod 0755 /etc/letsencrypt/renewal-hooks/deploy/cs-mail-reload-edge.sh
```

The HTTPS vhost can return 502 until the CS Mail web container starts. Its
certificate must still verify directly:

```bash
curl --noproxy '*' --resolve mail.crescentsphere.com:443:127.0.0.1 \
  -sS -o /dev/null -w '%{http_code}\n' https://mail.crescentsphere.com/
```

This may print `502`, but must not print a TLS trust error. Do not change
`mail.crescentsphere.com` to a Cloudflare-proxied record: large mailbox imports
need direct HTTPS.

## 4. Deploy CS Mail

Set a real HTTPS alert receiver in
`/opt/cs-mail/secrets/alert-webhook-url` (one line, root-owned, mode 0600).
Then use the normal production pipeline, which now detects Messenger mode and
starts the two private CS Mail web containers instead of host Nginx:

```bash
cd /opt/sites/cs-mail
sudo ./deploy/production/preflight.sh /opt/cs-mail/.env.production
sudo ./deploy/production/deploy-from-git.sh codex/shared-messenger-proxy
sudo ./deploy/production/status.sh /opt/cs-mail/.env.production
```

Keep public signup disabled while
`CS_MAIL_BILLING_INSTANT_ACTIVATION=true`. Restore `false` and deploy again
before accepting real paid orders. Keep the Cloudflare Email Routing MX
records until the mailbox acceptance test has passed, then cut over root MX
to `smtp.crescentsphere.com` and verify inbound/outbound delivery.

## Rollback of the edge route

If Messenger's Nginx rejects the new vhost, restore the two backed-up files
and recreate just `nginx` with the same `docker compose -p cs-messenger ...`
command. Leave all Messenger application containers running.
