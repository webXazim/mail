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
