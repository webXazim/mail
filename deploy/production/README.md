# CS Mail production deployment

This directory is the only authoritative production deployment path.
For the current shared VPS, start with `QUICKSTART.md`.

## Topology

- Web: `https://mail.crescentsphere.com` -> shared host Nginx 443 -> frontend/API.
- API: `127.0.0.1:18080` only.
- Platform Admin: `127.0.0.1:18081` only; SSH tunnel required.
- Mail: existing shared Stalwart at DNS-only `smtp.crescentsphere.com` on 25/465/993.
- PostgreSQL: Docker private network only.
- Prometheus/Alertmanager: loopback only.
- Runtime config: `/opt/cs-mail/.env.production`, outside Git, `root:root 0600`.

The Nginx site is name-based and safely coexists with the VPS's other projects
on ports 80/443.

## First VPS setup

If the checkout is already at `/srv/apps/mail` (as on this VPS), make the
path expected by the deployment and backup service point to that checkout once:

```bash
sudo install -d -m 0755 /opt/sites
test ! -e /opt/sites/cs-mail && sudo ln -s /srv/apps/mail /opt/sites/cs-mail
cd /opt/sites/cs-mail
```

Before deploying CS Mail, update the **existing** Mailer-owned Stalwart stack to
publish IMAPS 993 and confirm its TLS certificate covers
`smtp.crescentsphere.com`. Its Docker network is
`crescentsphere-mail-transport`. Create separate CS Mail management, JMAP and
SMTP submission credentials there; do not reuse Mailer's developer-mail
submission identity. The CS Mail application uses authenticated implicit TLS
to `smtp.crescentsphere.com:465` through that network.

If this is a new checkout, use the clone sequence below instead.

```bash
sudo install -d -m 0755 /opt/sites
sudo git clone <YOUR_PRIVATE_GITHUB_REPO> /opt/sites/cs-mail
cd /opt/sites/cs-mail
sudo ./deploy/production/bootstrap-vps.sh
```

Then follow `CREDENTIALS.md`. In short:

```bash
sudoedit /opt/cs-mail/.env.production
sudoedit /opt/cs-mail/secrets/alert-webhook-url
sudo ./deploy/production/show-config.sh /opt/cs-mail/.env.production
sudo ./deploy/production/setup-web-tls.sh /opt/cs-mail/.env.production
sudo ./deploy/production/preflight.sh /opt/cs-mail/.env.production
sudo ./deploy/production/deploy-from-git.sh main
```

DNS expected:

```text
mail.crescentsphere.com  A  <VPS IP>  DNS only
smtp.crescentsphere.com  A  <VPS IP>  DNS only   # existing
<VPS IP> PTR -> smtp.crescentsphere.com          # existing
```

Do not point MX/IMAP/SMTP at the web hostname merely because the web UI is named
`mail`. Stalwart's identity remains `smtp.crescentsphere.com`.

## Normal GitHub -> VPS update

```bash
cd /opt/sites/cs-mail
sudo ./deploy/production/deploy-from-git.sh main
```

The deploy pipeline locks deployment, validates configuration and shared
Stalwart, builds/tests frontend and backend in Docker, takes a pre-migration
backup, starts the release-tagged API, atomically publishes frontend assets,
validates/reloads only the CS Mail Nginx vhost, and runs local/public security
health gates.

## Platform Admin

Never open 18081 in UFW/provider firewall:

```bash
ssh -L 18081:127.0.0.1:18081 <user>@<vps>
```

Browse to `http://localhost:18081/mail/admin`.

## Backups and rollback

`bootstrap-vps.sh` enables `cs-mail-backup.timer`. Shared Stalwart data requires
its own host/provider backup because it is a platform-wide service.

```bash
systemctl status cs-mail-backup.timer
sudo ./deploy/production/restore-drill.sh /opt/cs-mail/.env.production
```

Database migrations are forward-only. Review compatibility before:

```bash
sudo ./deploy/production/rollback.sh /opt/cs-mail/.env.production --acknowledge-forward-migrations
```

## Launch certification

Use disposable certification accounts in `/opt/cs-mail/.env.certification`
(`root:root 0600`) and run:

```bash
sudo ./deploy/production/certify-launch.sh \
  /opt/cs-mail/.env.production \
  /opt/cs-mail/.env.certification
```

Keep `CS_MAIL_BILLING_INSTANT_ACTIVATION=false` for production. Enable it only
temporarily for isolated acceptance testing.
