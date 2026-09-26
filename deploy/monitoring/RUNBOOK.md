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

## Billing lifecycle

Use **Platform Admin → Diagnostics → Public launch readiness** first. A durable billing/provider alert is not cleared by restarting the API because the gauges are rebuilt from PostgreSQL on every Prometheus scrape.

For `provisioning_dead`, `lifecycle_email_failed`, `invoice_email_failed`, or `purge_failed`, open **Platform Admin → Billing → Operations**, inspect the affected organization and error, repair the underlying provider/mail configuration, then use the scoped retry action. Never mark a failed purge complete directly in PostgreSQL.

For `provider_stale_mailboxes`, use **Reconcile provider** on the affected subscription and confirm Stalwart access/quota state returns to sync. If the count remains non-zero, inspect API/provisioning logs before changing subscription status manually.

For `payment_review_aging`, review the submitted invoice/payment reference. Approve only after payment is verified; otherwise reject it with an operator note. Do not enable `CS_MAIL_BILLING_INSTANT_ACTIVATION` to bypass a payment backlog.

## Recoverability

`cs_mail_operational_evidence_age_seconds` is rebuilt from the append-only
PostgreSQL evidence ledger on every scrape. A process restart cannot make stale
backup evidence look healthy.

For `CSMailLocalBackupStale`, run `sh manage backup`, inspect the generated
manifest/checksums, and confirm `cs-mail-backup.timer` is active. For
`CSMailRestoreDrillStale`, run `sh manage restore-drill`; do not acknowledge the
alert based only on the existence of a backup archive.

`CSMailOffsiteBackupStale` covers two independent data sets: CS Mail
(database/attachments) and the shared Stalwart mail store. Run the appropriate
external encrypted offsite backup job first. Only after that job succeeds and
produces a root-owned proof manifest, record it:

```bash
sh manage record-backup-proof cs-mail /absolute/path/to/cs-mail-offsite.manifest
sh manage record-backup-proof stalwart /absolute/path/to/stalwart-offsite.manifest
```

The proof command is deliberately not an offsite-copy command. Never use a
locally fabricated manifest as a substitute for confirming the remote backup
object/snapshot. During an actual restore incident, preserve the evidence and
restore into an isolated target first unless the incident runbook explicitly
requires an in-place recovery.

## Database capacity

`CSMailDatabaseCapacityWarning` fires at 70% of the operator-declared `CS_MAIL_DB_CAPACITY_BYTES`; `CSMailDatabaseCapacityCritical` fires at 85%. Run `sh manage capacity` and inspect the largest relations. Do not raise the declared capacity merely to silence the alert unless the underlying PostgreSQL volume actually has that durable space. Keep normal utilization below roughly 70% so VACUUM, indexes, migrations, WAL/temp work and recovery have headroom. If growth is legitimate, expand/migrate PostgreSQL before increasing tenant/mailbox limits.
