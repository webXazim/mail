# CS Mail production configuration

## Authoritative locations

| Location | Purpose | Security |
|---|---|---|
| `/opt/cs-mail/.env.production` | runtime settings + application/provider credentials | `root:root 0600`, outside Git |
| `/opt/cs-mail/secrets/alert-webhook-url` | Alertmanager receiver | `root:root 0600` |
| `/opt/cs-mail/runtime/` | generated deployment metadata | `root:root 0700` |
| `/opt/cs-mail/www/releases/` | immutable frontend releases | no secrets |
| `/opt/cs-mail/www/current` | atomic active frontend symlink | no secrets |
| `/etc/letsencrypt/` | web TLS private key/certificate | managed by Certbot |

Never put the live env, TLS private keys, provider tokens or alert webhook in
GitHub.

## Fixed production topology

- `mail.crescentsphere.com` — DNS-only A record to the VPS; the independent platform edge owns 80/443. Follow `../edge/README.md`.
- `127.0.0.1:18080` — CS Mail Rust API, never public.
- `127.0.0.1:18081` — Platform Admin, SSH tunnel only.
- `smtp.crescentsphere.com` — existing DNS-only Stalwart mail/PTR identity.
- Stalwart remains the existing shared service on 25/465/993.
- PostgreSQL and monitoring remain private/loopback.

The platform edge routes `mail` to CS Mail's private TLS gateway and routes
root/`www` and `dm` to Messenger's existing private Nginx. CS Mail does not
install a host Nginx vhost in this topology.

## Environment organization

The production example intentionally lists almost every operational value with
a safe default. `bootstrap-vps.sh` copies it to `/opt/cs-mail/.env.production`
and generates CS Mail-owned cryptographic secrets once.

Only the following normal first-deploy values require operator input:

1. `CS_MAIL_SHARED_PROVIDER_NETWORK`
2. `CS_MAIL_MAIL_ADMIN_TOKEN`
3. `CS_MAIL_MAIL_JMAP_USERNAME`
4. `CS_MAIL_MAIL_JMAP_SECRET`
5. `CS_MAIL_LETSENCRYPT_EMAIL`
6. `CS_MAIL_SMTP_USERNAME` / `CS_MAIL_SMTP_PASSWORD` for dedicated TLS submission
7. `/opt/cs-mail/secrets/alert-webhook-url`

See `CREDENTIALS.md` for discovery/setup commands.

## TLS

Issue the first `mail.crescentsphere.com` certificate through the Cloudflare
DNS-01 procedure in `../edge/README.md`. This does not require taking port 80
from Messenger. Then stage and publish the independent platform edge and install
the documented Certbot renewal hook.

Stalwart's separate certificate on `smtp.crescentsphere.com:465/993` is not
managed by CS Mail; the preflight verifies that it is publicly trusted.

## Safe inspection

```bash
sudo ./deploy/production/show-config.sh /opt/cs-mail/.env.production
```

Known credential values are redacted. Preflight also checks root ownership,
mode 0600, duplicate keys, unsafe shell interpolation and required secret
lengths before Compose is touched.

## Billing testing flag

`CS_MAIL_BILLING_INSTANT_ACTIVATION=false` is the production default. Temporarily
enable instant activation only for isolated acceptance tests; restore `false`
before accepting real paid orders.
