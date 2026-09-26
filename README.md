# CS Mail

CS Mail is a public multi-tenant business-email SaaS built with React/Vite, Rust/Axum, PostgreSQL and a shared self-hosted Stalwart provider. The production web/API stack is isolated from Stalwart: CS Mail never starts a second mail server and never takes ownership of host mail ports 25/587/993.

## Repository layout

```text
backend/                    Rust/Axum API + SQLx migrations
frontend/                   React/Vite application
deploy/production/          authoritative GitHub -> VPS production deployment
deploy/development/         local/CI-only compose files
deploy/monitoring/          Prometheus/Alertmanager configuration
docs/                       current production and launch documentation
.github/workflows/ci.yml    blocking release validation
```

There is intentionally **no Docker Compose file at the repository root**. This prevents an operator from accidentally starting the local Stalwart topology on the production VPS.

## Production deployment

Production source is expected at `/opt/sites/cs-mail`; secrets/runtime state live outside Git under `/opt/cs-mail`. The independent platform edge serves `https://mail.crescentsphere.com` through CS Mail's dedicated TLS gateway, while IMAP/SMTP and VPS PTR/rDNS remain on the existing DNS-only `smtp.crescentsphere.com` Stalwart identity. See `deploy/edge/README.md`.

One-time host preparation after cloning the repository:

```bash
sudo /opt/sites/cs-mail/deploy/production/bootstrap-vps.sh
sudoedit /opt/cs-mail/.env.production
sudoedit /opt/cs-mail/secrets/alert-webhook-url
# Follow /opt/sites/cs-mail/deploy/edge/README.md before the first deploy.
```

See `deploy/production/CREDENTIALS.md` for the small set of operator-supplied Stalwart/TLS/alert values.

Normal deployments from GitHub:

```bash
cd /srv/apps/mail
sh manage deploy
```

`sh manage deploy` runs the existing guarded Git pull, build, migration, backup,
and health pipeline. It requests sudo when needed. Use `sh manage status` and
`sh manage preflight` for operational checks; `sh manage help` lists the other
short commands. The production environment remains at
`/opt/cs-mail/.env.production`.

The deploy pipeline:

1. refuses a dirty Git checkout and fast-forwards from GitHub;
2. verifies the release layout and static launch contract;
3. builds/tests the frontend inside Node 22 Docker and extracts only `dist`;
4. runs Rust fmt/Clippy/tests and builds the API from committed `Cargo.lock` with `--locked`;
5. creates a pre-deploy PostgreSQL + attachment backup when a live stack exists;
6. starts PostgreSQL and the release-tagged API (SQLx migrations run at API startup);
7. publishes frontend assets atomically under `/opt/cs-mail/www/current`;
8. validates/reloads the shared Nginx vhost and verifies loopback API, local/public HTTPS readiness, private admin access and admin/metrics blocking;
9. records the deployed Git/source digest under `/opt/cs-mail/runtime/current.env`;
10. cleans old build cache/release artifacts without touching secrets or persistent data.

Node.js and Rust do **not** need to be installed on the VPS host; Docker performs both builds.

Useful commands:

```bash
sudo bash ./deploy/production/status.sh /opt/cs-mail/.env.production
sudo bash ./deploy/production/backup.sh /opt/cs-mail/.env.production
sudo bash ./deploy/production/restore-drill.sh /opt/cs-mail/.env.production
sudo bash ./deploy/production/certify-launch.sh /opt/cs-mail/.env.production /opt/cs-mail/.env.certification
sh manage record-backup-proof cs-mail /absolute/path/to/cs-mail-offsite.manifest
sh manage record-backup-proof stalwart /absolute/path/to/stalwart-offsite.manifest
sh manage launch-freeze
```

Rollback is intentionally explicit because database migrations are forward-only:

```bash
sudo bash ./deploy/production/rollback.sh /opt/cs-mail/.env.production --acknowledge-forward-migrations
```

See `deploy/production/README.md`, `deploy/production/CONFIGURATION.md`, `docs/PRODUCTION.md` and `docs/LAUNCH.md`.

## Platform state

API contract: **v32**  
Migration head: **0051_mailbox_delete_cleanup_queue.sql**

The localhost-only Platform Admin controls users, businesses, memberships, hosted domains/mailboxes, subscription/payment lifecycle, storage allocations, provider/recovery operations, audit/security functions and emergency SaaS switches. The public Nginx vhost returns `404` for `/mail/admin*` and `/api/admin/*`; operators access the admin UI only through an SSH tunnel to `127.0.0.1:18081`.

Production defaults to payment-approved activation:

```env
CS_MAIL_ENVIRONMENT=production
CS_MAIL_BILLING_INSTANT_ACTIVATION=false
```

During controlled acceptance testing, the production deployment can explicitly
set `CS_MAIL_BILLING_INSTANT_ACTIVATION=true`; preflight and the API emit a
warning and test orders may activate immediately. The final public-launch
certification/freeze remains fail-closed and will not pass until the flag is
returned to `false`.

### Billing recovery operations

Upgrade 03 adds an operator-visible billing lifecycle health surface, durable retained-data purge runs, explicit exact-name purge confirmation, retry/reconciliation controls, and launch diagnostics for provider/billing drift. Retention expiry is deliberately non-destructive: mailbox/provider data is only purged after the configured retention deadline and a platform administrator explicitly confirms the operation. See `UPGRADE_03_RECOVERY_OPERATIONS.md`.


## Latest production hardening

See `UPGRADE_05_PUBLIC_LAUNCH_FREEZE.md` for the final public-launch freeze: exact-release runtime identity, strict production configuration, append-only backup/restore/offsite evidence, blocking quality gates and the closed-controls opening sequence. `UPGRADE_04_PRODUCTION_LAUNCH_VERIFICATION.md` documents the preceding readiness/monitoring layer.

## Capacity and storage

Production separates PostgreSQL metadata from object/mail blobs. Use `sh manage capacity` to report PostgreSQL utilization, largest relations, tenant/mailbox counts, local spool use and host disk headroom. A 20 GiB PostgreSQL planning capacity should normally stay below ~14 GiB (70%); 85% is critical. Cloudflare R2 for CS Mail attachments does not move Stalwart message bodies—configure Stalwart's S3-compatible Blob Store separately for R2 before selling multi-gigabyte mailbox quotas from a small VPS. See `docs/CAPACITY.md` and `docs/R2_STORAGE.md`.
