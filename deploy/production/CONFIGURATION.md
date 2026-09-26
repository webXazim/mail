# CS Mail production configuration

## Authoritative locations

| Location | Purpose | Security |
|---|---|---|
| `/opt/cs-mail/.env.production` | runtime settings + application/provider credentials | `root:root 0600`, outside Git |
| `/opt/cs-mail/secrets/alertmanager.yml` | Rendered Alertmanager email config | `root:root 0600` |
| `/opt/cs-mail/secrets/alert-smtp-password` | Alertmanager SMTP password | `root:root 0600` |
| `/opt/cs-mail/runtime/` | generated deployment metadata | `root:root 0700` |
| `/opt/cs-mail/www/releases/` | immutable frontend releases | no secrets |
| `/opt/cs-mail/www/current` | atomic active frontend symlink | no secrets |
| `/etc/letsencrypt/` | web TLS private key/certificate | managed by Certbot |

Never put the live env, TLS private keys, provider tokens or alert SMTP password in
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
7. `CS_MAIL_ALERT_EMAIL_TO` and, if the SMTP username is not a permitted sender address, `CS_MAIL_ALERT_EMAIL_FROM`

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
sudo bash ./deploy/production/show-config.sh /opt/cs-mail/.env.production
```

Known credential values are redacted. Preflight also checks root ownership,
mode 0600, duplicate keys, unsafe shell interpolation and required secret
lengths before Compose is touched.

## Billing activation safety

Production defaults to payment-gated activation:

```dotenv
CS_MAIL_ENVIRONMENT=production
CS_MAIL_BILLING_INSTANT_ACTIVATION=false
```

For controlled acceptance testing, the public VPS may temporarily set
`CS_MAIL_BILLING_INSTANT_ACTIVATION=true`. Preflight and the API warn clearly,
and unpaid test orders can activate immediately. This does **not** make the
configuration launch-ready: `sh manage launch-freeze` and launch certification
require the flag to be `false` before public signup can be opened.

A public order therefore follows invoice → payment submission → operator/payment
verification → activation. Changing an environment value never settles an
existing invoice.

## Recoverability evidence

`backup.sh` and `restore-drill.sh` append evidence to PostgreSQL migration 0048.
The independent offsite jobs for CS Mail data and the shared Stalwart data must
produce a root-owned proof manifest after a successful encrypted offsite copy.
Record those manifests with:

```bash
sh manage record-backup-proof cs-mail /absolute/path/to/cs-mail-offsite.manifest
sh manage record-backup-proof stalwart /absolute/path/to/stalwart-offsite.manifest
```

Recording a manifest is **not** a backup operation; it only records evidence
from the external backup system. The final `sh manage launch-freeze` gate fails
when local backup, restore drill, or either offsite proof is stale/missing.

## Capacity and object storage

For a 20 GiB PostgreSQL planning budget keep:

```dotenv
CS_MAIL_DB_CAPACITY_BYTES=21474836480
```

This is an operator-declared **database budget**, not customer mailbox storage. Prometheus warns at 70% and becomes critical at 85%. Run `sh manage capacity` to inspect current DB size, headroom, active tenants/mailboxes, largest relations, local attachment/import spool and underlying filesystem capacity.

Production application attachments should use a private R2 bucket:

```dotenv
CS_MAIL_OBJECT_STORAGE_BACKEND=r2
CS_MAIL_R2_ACCOUNT_ID=<account-id>
CS_MAIL_R2_BUCKET=cs-mail-production
CS_MAIL_R2_ACCESS_KEY_ID=<access-key-id>
CS_MAIL_R2_SECRET_ACCESS_KEY=<secret-access-key>
CS_MAIL_R2_ENDPOINT=
CS_MAIL_R2_REGION=auto
CS_MAIL_R2_PREFIX=cs-mail/attachments
CS_MAIL_R2_REQUEST_TIMEOUT_SECS=30
CS_MAIL_R2_MAX_CONCURRENT_TRANSFERS=4
```

The transfer limit bounds memory pressure from large attachment uploads/downloads. CS Mail's R2 bucket does **not** move the shared Stalwart message store; configure Stalwart's S3-compatible Blob Store to a separate private R2 bucket before relying on the advertised multi-gigabyte mailbox quotas from a small VPS. See `../../docs/CAPACITY.md` and `../../docs/R2_STORAGE.md`.

When R2 is active, the local `backup.sh` archive contains PostgreSQL plus local/legacy attachment spool data, not the remote R2 objects. Its manifest records this explicitly; independent offsite recovery proof must cover the R2 object set as well as the database.
