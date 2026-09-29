# Route the shared edge to the new Connect production stack

The new Messenger deployment is `/srv/apps/connect` with Compose project
`cs-connect`. Its private network is `cs-connect_messenger` and its Nginx
container is `cs-connect-nginx-1`. The old `cs-messenger` project is a separate
test stack and is not a data source for production. The edge keeps Mail's
`mail.crescentsphere.com` route while changing only the Connect and widget
backends.

First complete the fresh-stack preparation and private deployment in
Messenger's `docs/MOVE_MESSENGER_TO_APPS.md`. Confirm the new Nginx is healthy
and that the edge currently owns public ports 80/443. If the old Messenger
container still publishes either port, stop and inspect the live topology
before using these steps.

```bash
sudo docker network inspect cs-connect_messenger >/dev/null
sudo docker inspect cs-connect-nginx-1 \
  --format '{{.State.Health.Status}} {{index .Config.Labels "com.docker.compose.project"}}'
sudo docker inspect cs-platform-edge-edge-1 \
  --format '{{.State.Status}} {{index .Config.Labels "com.docker.compose.project"}}'
sudo ss -lntp '( sport = :80 or sport = :443 )'
```

The two `docker inspect` results should identify a healthy `cs-connect` Nginx
and a running `cs-platform-edge`. Preserve Mail's existing
`/srv/crescentsphere/platform-edge/.env`, `nginx-mail.conf`, certificates, and
private `edge_private` network. The updated CS Mail repository must contain
both `deploy/edge/docker-compose.yml` and `deploy/edge/haproxy.cfg` from the
same commit.

Validate the candidate HAProxy file before installing it:

```bash
sudo docker run --rm --entrypoint haproxy \
  -v /opt/sites/cs-mail/deploy/edge/haproxy.cfg:/usr/local/etc/haproxy/haproxy.cfg:ro \
  haproxy:3.2.23-alpine -c -f /usr/local/etc/haproxy/haproxy.cfg
```

During a maintenance window, preserve the current live edge files, install
both new files, validate Compose, then recreate only the edge service:

```bash
cd /srv/crescentsphere/platform-edge
stamp=$(date -u +%Y%m%dT%H%M%SZ)
sudo cp -p docker-compose.yml "docker-compose.yml.pre-connect-$stamp"
sudo cp -p haproxy.cfg "haproxy.cfg.pre-connect-$stamp"
sudo install -m 0644 /opt/sites/cs-mail/deploy/edge/docker-compose.yml docker-compose.yml
sudo install -m 0644 /opt/sites/cs-mail/deploy/edge/haproxy.cfg haproxy.cfg
sudo docker compose config --quiet
sudo docker compose up -d --no-deps --force-recreate edge
sudo docker exec cs-platform-edge-edge-1 \
  haproxy -c -f /usr/local/etc/haproxy/haproxy.cfg
```

Probe the local edge before adding DNS records. Mail must continue to answer
with its existing certificate. Connect and widget use the new production
origin certificate, so the local probes may use `-k` if it is a Cloudflare
Origin CA certificate.

```bash
curl --noproxy '*' --resolve connect.crescentsphere.com:443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' https://connect.crescentsphere.com/
curl --noproxy '*' --resolve widget.crescentsphere.com:443:127.0.0.1 \
  -ksS -o /dev/null -w '%{http_code}\n' \
  https://widget.crescentsphere.com/support-widget/loader.js
curl --noproxy '*' --resolve mail.crescentsphere.com:443:127.0.0.1 \
  -fsS -o /dev/null -w '%{http_code}\n' https://mail.crescentsphere.com/
cd /srv/apps/connect
bash scripts/production-readiness.sh --probe --skip-public
```

If edge recreation or local probes fail, restore both saved edge files with
the same `stamp` and recreate only `edge`. The old test network must still
exist for that rollback. This restores routing; it does not change either
application's database.

After local readiness passes, add proxied Cloudflare A records for `connect`
and `widget` to the actual VPS public IP and run the full Connect public
readiness probe. No DNS port value is used; HAProxy receives 80/443. Do not
route the apex or `www` to Messenger.

## Restore the real visitor IP for Support abuse limits

The first Connect cutover keeps `SUPPORT_TRUST_PROXY_HEADERS=False`. Before
enabling it, upgrade both the Connect Nginx configuration and the HAProxy edge
to PROXY v2. HAProxy passes the original TCP peer to Nginx. Nginx accepts
`CF-Connecting-IP` only when that peer belongs to Cloudflare's published
proxy ranges, then sends one verified IP to Django and Axum. All HTTPS
Connect/widget traffic must pass through the shared edge. Mail's backend
already uses PROXY v2 and is not changed by this upgrade.

Perform this short coordinated update while Connect may be interrupted. Keep
the old configuration files until every probe passes. Do not copy `.env` into
Git or print its secrets.

1. Before pulling, back up `/srv/apps/connect/nginx/snm.production.conf` and
   `/srv/crescentsphere/platform-edge/haproxy.cfg` with a common timestamp.
   Record the current Connect and Mail commit IDs.
2. Pull the reviewed Connect `axum` and Mail `main` commits. Run
   `docker compose --env-file .env -f docker-compose.yml -f docker-compose.production.yml config --quiet`
   in `/srv/apps/connect`. Validate the candidate Nginx file with
   `docker compose --env-file .env -f docker-compose.yml -f docker-compose.production.yml run --rm --no-deps nginx nginx -t`.
   Validate the Mail source HAProxy file with the one-off `haproxy -c`
   command above. Do not recreate either live service if a validation fails.
3. Recreate only `cs-connect` Nginx with
   `docker compose --env-file .env -f docker-compose.yml -f docker-compose.production.yml up -d --no-deps --force-recreate nginx`.
   Then install the reviewed Mail `deploy/edge/haproxy.cfg` into
   `/srv/crescentsphere/platform-edge/haproxy.cfg` and recreate only the
   `cs-platform-edge` edge service. This order can briefly interrupt Connect
   and widget; Mail continues through its unchanged backend.
4. Repeat the three local HTTPS probes above and run
   `bash scripts/production-readiness.sh --probe --skip-public` in Connect.
   A direct HTTPS request to Connect Nginx without HAProxy will now fail by
   design because that listener requires PROXY v2.
5. Set `SUPPORT_TRUST_PROXY_HEADERS=True` in `/srv/apps/connect/.env` and set
   `SUPPORT_TRUSTED_PROXY_CIDRS` to the subnet from
   `docker network inspect cs-connect_messenger`. For the current deployment
   it is `172.27.0.0/16`. Recreate only Connect `web` to load those values,
   then run `docker compose ... exec -T web python manage.py check --deploy`
   and the readiness probe again. Confirm `support.W002` is gone.

If Connect/widget probes fail, first restore the saved edge HAProxy file and
recreate `edge`; then restore the saved Connect Nginx file and recreate
`nginx`. Leave `SUPPORT_TRUST_PROXY_HEADERS=False` until this coordinated
update passes. Keep the Mail 200 probe in every check.
