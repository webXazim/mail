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

Production source is expected at `/opt/sites/cs-mail`; secrets/runtime state live outside Git under `/opt/cs-mail`. The public web app runs behind Messenger's shared Docker Nginx at `https://mail.crescentsphere.com`, while IMAP/SMTP and VPS PTR/rDNS remain on the existing DNS-only `smtp.crescentsphere.com` Stalwart identity. See `deploy/production/SHARED_PROXY.md`.

One-time host preparation after cloning the repository:

```bash
sudo /opt/sites/cs-mail/deploy/production/bootstrap-vps.sh
sudoedit /opt/cs-mail/.env.production
sudoedit /opt/cs-mail/secrets/alert-webhook-url
# Follow /opt/sites/cs-mail/deploy/production/SHARED_PROXY.md for edge TLS.
```

See `deploy/production/CREDENTIALS.md` for the small set of operator-supplied Stalwart/TLS/alert values.

Normal deployments from GitHub:

```bash
cd /opt/sites/cs-mail
sudo ./deploy/production/deploy-from-git.sh main
```

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
sudo ./deploy/production/status.sh /opt/cs-mail/.env.production
sudo ./deploy/production/backup.sh /opt/cs-mail/.env.production
sudo ./deploy/production/restore-drill.sh /opt/cs-mail/.env.production
sudo ./deploy/production/certify-launch.sh /opt/cs-mail/.env.production /opt/cs-mail/.env.certification
```

Rollback is intentionally explicit because database migrations are forward-only:

```bash
sudo ./deploy/production/rollback.sh /opt/cs-mail/.env.production --acknowledge-forward-migrations
```

See `deploy/production/README.md`, `deploy/production/CONFIGURATION.md`, `docs/PRODUCTION.md` and `docs/LAUNCH.md`.

## Platform state

API contract: **v32**  
Migration head: **0041_full_saas_control_plane.sql**

The localhost-only Platform Admin controls users, businesses, memberships, hosted domains/mailboxes, subscription/payment lifecycle, storage allocations, provider/recovery operations, audit/security functions and emergency SaaS switches. The public Nginx vhost returns `404` for `/mail/admin*` and `/api/admin/*`; operators access the admin UI only through an SSH tunnel to `127.0.0.1:18081`.

Acceptance testing currently keeps:

```env
CS_MAIL_BILLING_INSTANT_ACTIVATION=true
```

Switch it to `false` before real payment-gated public activation.
