# CS Mail production deployment

This directory is the **only authoritative production deployment path** for `mail.crescentsphere.com`.

## Production topology

- Host Nginx: public 80/443 plus localhost-only Platform Admin on `127.0.0.1:18081`.
- CS Mail API: Docker, host loopback `127.0.0.1:18080` only.
- CS Mail PostgreSQL: Docker private network only.
- Prometheus/Alertmanager: loopback only.
- Stalwart: **existing shared provider**, external Docker network, owns host 25/587/993. Production compose never creates or binds a mail service.
- Static frontend: immutable release directories under `/opt/cs-mail/www/releases`; Nginx serves the atomic `/opt/cs-mail/www/current` symlink.
- Secrets/runtime metadata: `/opt/cs-mail`, outside Git.

## One-time VPS preparation

Clone your private GitHub repository:

```bash
sudo install -d -m 0755 /opt/sites
sudo git clone <YOUR_PRIVATE_GITHUB_REPO> /opt/sites/cs-mail
cd /opt/sites/cs-mail
sudo ./deploy/production/bootstrap-vps.sh
```

`bootstrap-vps.sh` installs common Ubuntu/Debian host tools, verifies Docker Compose v2, creates production directories and enables the daily CS Mail backup timer. It does **not** install Docker because this VPS already hosts the shared mail stack and Docker ownership should remain deliberate.

Configure `/opt/cs-mail/.env.production` (root:root, `0600`). Identify the existing Stalwart Docker network with `docker network ls` / `docker inspect` and configure dedicated CS Mail management/JMAP credentials. Create the root-only Alertmanager webhook file configured by `CS_MAIL_ALERT_WEBHOOK_FILE`.

TLS for both Nginx HTTPS and Stalwart IMAPS/SMTP must already validate for `mail.crescentsphere.com` before `preflight.sh` will allow a production deploy.

## Deploy from GitHub

Normal production update:

```bash
sudo /opt/sites/cs-mail/deploy/production/deploy-from-git.sh main
```

To deploy an explicit immutable Git tag/commit:

```bash
sudo /opt/sites/cs-mail/deploy/production/deploy-from-git.sh <tag-or-commit>
```

`deploy-from-git.sh` refuses local modifications/untracked files, fetches GitHub using `--ff-only` semantics for branches, cleans known generated directories, runs release verification, then delegates to `deploy.sh`.

### What deploy.sh guarantees

- single-deployer lock using `flock`;
- production env permissions are checked;
- static release certification and shared-Stalwart preflight must pass;
- source identity is a deterministic SHA-256 of `git archive HEAD`;
- frontend lint/typecheck/tests/build happen in a pinned Node 22 Docker image;
- API Docker build runs rustfmt, blocking Clippy, Rust tests, then compiles from committed `Cargo.lock` using `cargo build --release --locked`;
- images are tagged by Git/source identity;
- existing live DB + attachments are backed up **before** a new API can run SQLx migrations;
- API readiness must pass before frontend/Nginx cutover;
- frontend publication is an atomic symlink switch outside the Git tree;
- Nginx must pass `nginx -t` before reload;
- public health, localhost admin, public-admin blocking and metrics blocking are checked after cutover;
- successful release metadata is stored in `/opt/cs-mail/runtime/current.env`;
- previous successful metadata is retained for an explicit rollback;
- old frontend trees/dangling build cache are pruned while persistent volumes/backups are untouched.

Deployment logs are written to `/var/log/cs-mail/deploy-*.log`.

## Platform Admin

Never expose TCP 18081 publicly. From an operator workstation:

```bash
ssh -L 18081:127.0.0.1:18081 <user>@<vps>
```

Then browse to `http://localhost:18081/mail/admin`.

## Backups

The bootstrap installs `cs-mail-backup.timer`, which runs daily. Backups contain:

- PostgreSQL custom-format dump;
- CS Mail persistent attachment/MBOX staging volume;
- manifest with checksums and deployed release identity.

The shared Stalwart provider is intentionally not included. Its backup is a platform-level responsibility because restoring it affects every service using that mail server.

Check the timer:

```bash
systemctl status cs-mail-backup.timer
systemctl list-timers cs-mail-backup.timer
```

Run a restore drill without touching the live database:

```bash
sudo /opt/sites/cs-mail/deploy/production/restore-drill.sh /opt/cs-mail/.env.production
```

## Rollback

Code/static rollback cannot reverse SQL migrations. Review migration compatibility first, then:

```bash
sudo /opt/sites/cs-mail/deploy/production/rollback.sh \
  /opt/cs-mail/.env.production \
  --acknowledge-forward-migrations
```

If a deployment failed before being marked successful, rollback uses the preserved last-known-good metadata. After a successful deployment it uses `previous.env`.

## Final public-launch certification

Create `/opt/cs-mail/.env.certification` with disposable cross-tenant/app-password test credentials, root-owned mode `0600`, then run:

```bash
sudo /opt/sites/cs-mail/deploy/production/certify-launch.sh \
  /opt/cs-mail/.env.production \
  /opt/cs-mail/.env.certification
```

The wrapper automatically merges the exact deployed source SHA from `/opt/cs-mail/runtime/current.env`; you no longer hand-maintain release hashes in `.env.production`.

A public launch still requires external inbox-placement checks at unrelated providers and the deliberate switch from instant testing activation to payment-gated activation.
