# CS Mail monitoring runbook

Production monitoring is started by `deploy/production/deploy.sh` using `deploy/production/docker-compose.yml` with the `monitoring` profile.

Prometheus: `127.0.0.1:19090`  
Alertmanager: `127.0.0.1:19093`

Neither listener should be exposed publicly. `/api/metrics` is blocked at public Nginx and is scraped over the private Docker network.

## First response

1. Run `sudo /opt/sites/cs-mail/deploy/production/status.sh /opt/cs-mail/.env.production`.
2. Inspect `docker compose --env-file /opt/cs-mail/.env.production -f /opt/sites/cs-mail/deploy/production/docker-compose.yml ps`.
3. Inspect API logs with `docker compose ... logs --tail=200 api`.
4. Check PostgreSQL/host disk and memory before restarting services.
5. For mail-delivery/provider failures, verify the shared Stalwart service separately; do not create a second mail service from CS Mail.
6. Use the localhost Platform Admin recovery center for failed provisioning/import/scheduled-send/billing-email jobs.
7. Use emergency service switches if an incident requires pausing signup, ordering, domain onboarding, mailbox provisioning or customer outbound sending.

Before destructive recovery, create a fresh CS Mail backup. Shared Stalwart recovery must follow its own platform-level backup/runbook because multiple products use it.
