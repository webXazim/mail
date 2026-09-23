# Shortest production path on the shared VPS

This project is the business mailbox SaaS. The separate Mailer project is the
developer sending SaaS. Both use one Stalwart and one public IP, but keep their
own application databases, credentials and web hostnames.

## 1. Finish the existing Stalwart first

In the **Mailer** checkout on the VPS, pull the revision that publishes port 993
in `docker-compose.stalwart.yml`. Confirm Stalwart's data/config volumes have a
recoverable offsite backup, then run `sudo sh manage stalwart-up`. This recreates
only the independent Stalwart container; the existing mail queue and data
volumes remain. Confirm `smtp.crescentsphere.com` has a trusted certificate on
465 and 993 and that unauthenticated relay to an unrelated domain fails.

Create three separate CS Mail credentials in Stalwart: a restricted management
API token, a cross-account JMAP service principal, and a business-mail SMTP
submission principal. Configure that SMTP identity to send only for hosted
business domains. Keep Mailer's developer-mail submission principal separate.

The shared Docker network is `crescentsphere-mail-transport`; the network alias
`smtp.crescentsphere.com` routes privately to Stalwart while preserving TLS
hostname verification. Keep `smtp.crescentsphere.com` as the A/PTR mail identity.

## 2. Deploy the mailbox app

The current VPS checkout is `/srv/apps/mail`. After pulling the new CS Mail
revision, make the path used by the deployment scripts and backup timer point
to it:

```bash
sudo install -d -m 0755 /opt/sites
if [ ! -e /opt/sites/cs-mail ]; then sudo ln -s /srv/apps/mail /opt/sites/cs-mail; fi
test "$(readlink -f /opt/sites/cs-mail)" = /srv/apps/mail
cd /opt/sites/cs-mail
sudo ./deploy/production/bootstrap-vps.sh
sudoedit /opt/cs-mail/.env.production
sudoedit /opt/cs-mail/secrets/alert-webhook-url
sudo ./deploy/production/show-config.sh /opt/cs-mail/.env.production
# Follow ../edge/README.md to issue the web certificate with DNS-01,
# stage the independent edge, and move 80/443 from Messenger to the edge.
sudo ./deploy/production/preflight.sh /opt/cs-mail/.env.production
sudo ./deploy/production/deploy-from-git.sh main
sudo ./deploy/production/status.sh /opt/cs-mail/.env.production
```

Fill the operator values listed in `CREDENTIALS.md`. In particular:

```dotenv
CS_MAIL_SHARED_PROVIDER_NETWORK=crescentsphere-mail-transport
CS_MAIL_WEB_PROXY_MODE=edge
CS_MAIL_SHARED_WEB_NETWORK=cs-platform-web
CS_MAIL_SHARED_EDGE_CONTAINER=cs-platform-edge-edge-1
CS_MAIL_SMTP_HOST=smtp.crescentsphere.com
CS_MAIL_SMTP_PORT=465
CS_MAIL_BILLING_INSTANT_ACTIVATION=false
```

The web hostname `mail.crescentsphere.com` already resolves to the VPS IP. The
mailbox web TLS certificate is separate from Stalwart's mail certificate. Keep
API 18080 and Platform Admin 18081 on loopback only; do not open either in the
host or provider firewall.

## 3. Prove launch readiness

Run the restore drill and `certify-launch.sh` with disposable certification
accounts as described in `CREDENTIALS.md` and `README.md`. Verify customer MX,
SPF, DKIM and DMARC, inbound/outbound delivery, IMAP login, alert delivery and
external inbox placement. Keep automated backups for the CS Mail database and
attachments, and separate encrypted offsite backups for the shared Stalwart
volumes. Public signups should begin only after these checks pass.
