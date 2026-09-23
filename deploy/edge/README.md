# Independent platform edge for the shared VPS

This is the central VPS edge project. Copy this directory to
`/srv/crescentsphere/platform-edge` and operate it independently from the
Messenger and CS Mail Compose projects. It owns public TCP 80/443 after
cutover. HAProxy sends
`mail.crescentsphere.com` to a dedicated Nginx TLS gateway and passes all
other HTTPS connections through to Messenger's existing Nginx. Messenger
continues to terminate its own HTTPS and retain its existing root, www and dm
routes. The edge connects to the existing `cs-messenger_messenger` network;
CS Mail's private `cs-mail-web` alias is already on that network. No Stalwart
Enterprise or tenant feature is needed: CS Mail's organizations, memberships,
domain claims, mailbox ownership, quotas and provisioning queue provide its
application tenancy. Stalwart remains the protocol/storage provider.

**Prerequisites:** Deploy CS Mail using `deploy/production/SHARED_PROXY.md`
first. That creates and tests the publicly trusted certificate in
`/etc/letsencrypt/live/mail.crescentsphere.com/`. Keep the Cloudflare MX
records until a real mailbox acceptance test passes. Do not cut over while
Messenger or CS Mail is unhealthy.

## 1. Stage without occupying public ports

Run on the VPS after the new commit is pulled. Keep the canonical copy under
its own platform deployment source so later CS Mail or Messenger releases do
not silently replace the active edge configuration:

```bash
sudo install -d -m 0755 /srv/crescentsphere/platform-edge
sudo cp /opt/sites/cs-mail/deploy/edge/docker-compose.yml \
  /opt/sites/cs-mail/deploy/edge/haproxy.cfg \
  /opt/sites/cs-mail/deploy/edge/nginx-mail.conf \
  /opt/sites/cs-mail/deploy/edge/reload-on-renew.sh \
  /srv/crescentsphere/platform-edge/
sudo cp /opt/sites/cs-mail/deploy/edge/.env.example \
  /srv/crescentsphere/platform-edge/.env
cd /srv/crescentsphere/platform-edge
sudo docker network inspect cs-messenger_messenger >/dev/null
sudo docker network inspect cs-mail-prod_backend >/dev/null
sudo docker compose config --quiet
sudo docker compose up -d
sudo docker compose ps
sudo docker exec cs-platform-edge-edge-1 haproxy -c -f /usr/local/etc/haproxy/haproxy.cfg
sudo docker exec cs-platform-edge-mail_tls-1 nginx -t
curl --noproxy '*' --resolve mail.crescentsphere.com:18443:127.0.0.1 \
  -fsS https://mail.crescentsphere.com:18443/api/health/ready
curl --noproxy '*' --resolve dm.crescentsphere.com:18443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://dm.crescentsphere.com:18443/
curl --noproxy '*' --resolve crescentsphere.com:18443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://crescentsphere.com:18443/
```

The first curl must succeed with certificate verification enabled. The dm
probe uses `-k` only because Messenger's Cloudflare Origin CA certificate is
not trusted by direct local curl. Inspect its HTTP status and compare with the
same dm request to the currently published port 443. Stage ports 18082/18443
bind only to loopback. Check that `172.29.41.0/24` is free before starting;
if it is occupied, change the subnet and all three fixed IP references in the
Compose, HAProxy and Nginx files together.

## 2. Cut over public ports

Find Messenger's active release from its Compose label. Back up its production
Compose file. In the active release **and the canonical Messenger deployment
source**, remove only the `nginx` service's `ports: [80:80, 443:443]`
publication. Leave the service, networks, TLS mounts, and vhosts intact.
Validate with `docker compose config` before recreating `nginx`. Messenger's
websites will be briefly unavailable between that recreation and starting
the platform edge; schedule the cutover accordingly.

```bash
MESSENGER_DIR=$(sudo docker inspect cs-messenger-nginx-1 \
  --format '{{index .Config.Labels "com.docker.compose.project.working_dir"}}')
sudo cp -a "$MESSENGER_DIR/docker-compose.production.yml" \
  "$MESSENGER_DIR/docker-compose.production.yml.before-platform-edge"
# Edit $MESSENGER_DIR/docker-compose.production.yml: remove the nginx ports block.
cd "$MESSENGER_DIR"
sudo docker compose -p cs-messenger -f docker-compose.yml \
  -f docker-compose.production.yml config --quiet
sudo docker compose -p cs-messenger -f docker-compose.yml \
  -f docker-compose.production.yml up -d --no-deps nginx
sudo docker exec cs-messenger-nginx-1 nginx -t
```

In `/srv/crescentsphere/platform-edge/.env`, set exactly:

```dotenv
CS_EDGE_HTTP_BIND=0.0.0.0:80
CS_EDGE_HTTPS_BIND=0.0.0.0:443
```

Then publish the independent edge:

```bash
cd /srv/crescentsphere/platform-edge
sudo docker compose up -d --force-recreate edge
sudo ss -lntp '( sport = :80 or sport = :443 )'
curl --noproxy '*' --resolve mail.crescentsphere.com:443:127.0.0.1 \
  -fsS https://mail.crescentsphere.com/api/health/ready
curl --noproxy '*' --resolve dm.crescentsphere.com:443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://dm.crescentsphere.com/
curl --noproxy '*' --resolve crescentsphere.com:443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://crescentsphere.com/
```

In `/opt/cs-mail/.env.production`, change
`CS_MAIL_WEB_PROXY_MODE=edge` and
`CS_MAIL_SHARED_EDGE_CONTAINER=cs-platform-edge-edge-1`. Keep
`CS_MAIL_SHARED_WEB_NETWORK=cs-messenger_messenger` and the trusted proxy
IPs unchanged. Run `deploy/production/preflight.sh`, then the normal CS Mail
deployment. Remove the CS Mail include and certificate mounts from Messenger
only after the new edge is healthy; keep the other Messenger vhosts untouched.

## 3. Certificate renewal and release ownership

Replace the old Certbot hook that reloads `cs-messenger-nginx-1` for CS Mail:

```bash
sudo install -m 0755 /srv/crescentsphere/platform-edge/reload-on-renew.sh \
  /etc/letsencrypt/renewal-hooks/deploy/cs-mail-reload-edge.sh
sudo certbot renew --dry-run
```

Certbot's webroot remains `/var/www/letsencrypt`, now served by the
independent edge. Verify the mail certificate again after the dry run.
Maintain this edge Compose project separately from Messenger and CS Mail app
releases. Review and pin image updates as part of the platform release cycle.

When a company homepage is ready, change the root/www backend deliberately;
until then, keep Messenger's existing root/www service. Developer mail's
`mailer.crescentsphere.com` currently uses Cloudflare Tunnel and needs no
edge route here unless that topology changes.

## Rollback

If the new edge fails, first stop its published `edge` service to release
80/443. Restore the saved Messenger production Compose file and recreate only
Messenger `nginx` with the same `docker compose -p cs-messenger ... up -d
--no-deps nginx` command. Set CS Mail proxy mode back to `messenger`, restore
its edge-container value, and verify all three websites. Preserve the CS Mail
database, mail server, and mailbox DNS throughout this web rollback.
