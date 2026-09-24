# CS Mail production deployment

This directory is the only authoritative production deployment path.
For the current shared VPS, start with `QUICKSTART.md`.

## Topology

- Web: `https://mail.crescentsphere.com` -> independent platform edge -> CS Mail TLS gateway -> CS Mail web container -> frontend/API.
- API: `127.0.0.1:18080` only.
- Platform Admin: `127.0.0.1:18081` only; SSH tunnel required.
- Mail: existing shared Stalwart at DNS-only `smtp.crescentsphere.com` on 25/465/993.
- PostgreSQL: Docker private network only.
- Prometheus/Alertmanager: loopback only.
- Runtime config: `/opt/cs-mail/.env.production`, outside Git, `root:root 0600`.

The active VPS proxy is currently `cs-messenger-nginx-1`. Follow
`../edge/README.md` to stage and cut over the independent platform edge before
the first CS Mail deployment. CS Mail's application-owned tenancy uses the
organization and mailbox schema; it does not require Stalwart Enterprise
tenants. Complete `GO_LIVE.md` before accepting other businesses. The
host-Nginx TLS script is for a different topology.

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
sudoedit /opt/cs-mail/.env.production  # set CS_MAIL_ALERT_EMAIL_TO/FROM
sudo ./deploy/production/show-config.sh /opt/cs-mail/.env.production
# Complete ../edge/README.md first: DNS-01 certificate and edge cutover.
sh manage preflight
sh manage deploy
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
sh manage deploy
```

`sh manage deploy` invokes the same `deploy-from-git.sh` pipeline and requests
sudo when needed. `sh manage status`, `sh manage preflight`, and
`sh manage help` provide the other common commands. The full scripts remain
available for specialized operations.

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

Browse to `http://localhost:18081/mail/admin/control-plane`. Platform user
operations are under `/mail/admin/operations`; platform billing is under
`/mail/admin/billing`. These routes are available only through the SSH tunnel.
Business owners and business admins manage their own domains, team members,
hosted mailboxes, and addresses at `https://mail.crescentsphere.com/mail/business`
after their plan is active. A platform role does not grant business ownership.

For a domain hosted on Cloudflare, the business owner or admin can open the
pending domain's **Use Cloudflare to add and verify this record** section. They
can choose **Connect Cloudflare** to authorize DNS changes without creating an
API token. This button appears after the operator registers a Cloudflare public
OAuth client for `https://mail.crescentsphere.com/mail/business`, grants the
client **Zone Read** and **DNS Write** scopes, verifies the client URL domain in
Cloudflare, and sets `CS_MAIL_CLOUDFLARE_OAUTH_CLIENT_ID` in `.env.production`.
Register the client as a browser-based Authorization Code + PKCE client with
token authentication method `none`; CS Mail never requires its client secret.
The API container must be recreated after setting the variable. A user may
instead create an API token limited to that zone with **Zone Read** and
**DNS Write**, paste it, and choose **Set up with Cloudflare**. CS Mail publishes the
ownership TXT record, verifies it against public DNS, provisions the domain on
the mail server, and publishes the generated MX, SPF, DKIM and DMARC records.
Public mail DNS readiness is then checked until the domain is active. The token
is not persisted; if verification takes too long or setup is interrupted, the
owner can resume from **Publish mail DNS with Cloudflare** with a new token.
Existing conflicting mail records are never replaced automatically. The manual
DNS setup path remains available for other DNS providers.

Resend's provider sign-in flow uses Domain Connect. Cloudflare requires CS Mail
to publish a Domain Connect template and have Cloudflare onboard it before that
specific standard can be offered. The Cloudflare OAuth option above gives a
consent-based connection while that provider onboarding is pending.

Platform Admin has no separate password in `.env.production`. After the first
CS Mail account has verified its email, promote that exact account once:

```bash
sudo ./deploy/production/bootstrap-first-admin.sh you@example.com
```

The command refuses to run if an active platform admin already exists. Sign in
through the SSH tunnel using that account's normal CS Mail email and password.

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
