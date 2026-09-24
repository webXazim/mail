# Production credentials and first-deploy checklist

The live production file is **`/opt/cs-mail/.env.production`**. It is created by
`bootstrap-vps.sh`, owned by `root:root`, mode `0600`, and must never be copied
into the Git repository.

## Generated automatically — do not invent or rotate on every deploy

`bootstrap-vps.sh` generates these once and preserves existing values:

- `POSTGRES_PASSWORD`
- `CS_MAIL_JWT_SECRET`
- `CS_MAIL_DELIVERY_EVENT_SECRET`
- `CS_MAIL_PROVISIONING_KEY`
- `CS_MAIL_TOTP_KEY`

All other non-secret production defaults are already populated in the env
example and copied into the live file on first bootstrap.

## Values you must enter

### 1. Existing Stalwart Docker network

Set:

```env
CS_MAIL_SHARED_PROVIDER_NETWORK=<existing-network-name>
```

Discover it without changing anything:

```bash
docker ps --format 'table {{.Names}}\t{{.Image}}'
docker network ls
# Replace <stalwart-container> with the real container name:
docker inspect <stalwart-container> --format '{{json .NetworkSettings.Networks}}' | jq
```

CS Mail joins this existing external network. Production Compose deliberately
does not start another Stalwart instance.
For the Mailer Compose stack in this workspace, the network name is
`crescentsphere-mail-transport`; verify it exists on the VPS before entering it.

### 2. Stalwart management token

Set:

```env
CS_MAIL_MAIL_ADMIN_TOKEN=<dedicated-management-token>
```

Create/use a dedicated service token in the existing Stalwart administration
rather than reusing a human/master password. `CS_MAIL_MAIL_ADMIN_SECRET` should
remain blank when token authentication is available.

The defaults assume the provider is resolvable as `stalwart` on the shared
Docker network:

```env
CS_MAIL_MAIL_ADMIN_URL=http://stalwart:8080/
CS_MAIL_MAIL_ADMIN_USERNAME=admin
CS_MAIL_SMTP_HOST=smtp.crescentsphere.com
```

If the management network alias differs, change only the hostname in
`CS_MAIL_MAIL_ADMIN_URL`. Keep `CS_MAIL_SMTP_HOST` at
`smtp.crescentsphere.com` so its TLS certificate verifies; the Mailer Stalwart
Compose file supplies this private network alias.

### 3. Stalwart JMAP service account

Set a dedicated service principal:

```env
CS_MAIL_MAIL_JMAP_USERNAME=<service-user>
CS_MAIL_MAIL_JMAP_SECRET=<strong-service-secret>
```

Do not reuse a normal customer mailbox credential.

### 4. Dedicated Stalwart SMTP submission credential

The API submits customer messages to Stalwart through the private Docker
network using verified implicit TLS on port 465. Create a dedicated submission
principal with a sender policy restricted to hosted business domains, then set:

```env
CS_MAIL_SMTP_HOST=smtp.crescentsphere.com
CS_MAIL_SMTP_PORT=465
CS_MAIL_SMTP_USERNAME=<business-mail-submission-user>
CS_MAIL_SMTP_PASSWORD=<strong-service-password>
```

The Docker network aliases `smtp.crescentsphere.com` directly to Stalwart, so
certificate verification does not depend on public hairpin routing. Keep this
credential separate from Mailer's developer-mail submission principal and from
customer app passwords.

### 5. Let's Encrypt contact email

Set:

```env
CS_MAIL_LETSENCRYPT_EMAIL=you@example.com
```

This is not an application login. It is used by Certbot for the certificate on
`mail.crescentsphere.com`.

### 6. Alert email

Alertmanager uses the dedicated CS Mail SMTP credential above over verified
implicit TLS on port 465. Set the recipient in `.env.production`:

```env
CS_MAIL_ALERT_EMAIL_TO=you@example.com
CS_MAIL_ALERT_EMAIL_FROM=alerts@crescentsphere.com
```

The recipient defaults to `CS_MAIL_LETSENCRYPT_EMAIL` if left blank. The sender
defaults to `CS_MAIL_SMTP_USERNAME` when that username is a full email address.
The sender must be allowed by the Stalwart submission principal's sender policy.
Deployment writes the SMTP password into a private, root-owned file for
Alertmanager. The old `alert-webhook-url` file is unused.

## Values already fixed for your VPS design

```env
CS_MAIL_PUBLIC_ORIGIN=https://mail.crescentsphere.com
CS_MAIL_WEB_HOST=mail.crescentsphere.com
CS_MAIL_API_HOST_PORT=18080
CS_MAIL_ADMIN_HOST_PORT=18081
CS_MAIL_CLIENT_HOST=smtp.crescentsphere.com
CS_MAIL_CLIENT_IMAP_PORT=993
CS_MAIL_CLIENT_SMTP_PORT=465
CS_MAIL_EXPECTED_PTR=smtp.crescentsphere.com
CS_MAIL_PROVIDER_NAMESPACE=cs-mail
CS_MAIL_MAIL_DEFAULT_DOMAIN=crescentsphere.com
CS_MAIL_BILLING_INSTANT_ACTIVATION=false
```

`smtp.crescentsphere.com` remains the existing DNS-only Stalwart/PTR identity.
Do not change VPS reverse DNS to `mail.crescentsphere.com`.

## DNS required before first TLS setup

Keep the existing direct-mail record:

```text
smtp.crescentsphere.com  A  <VPS_PUBLIC_IP>  DNS only
PTR/rDNS: <VPS_PUBLIC_IP> -> smtp.crescentsphere.com
```

Add the web record:

```text
mail.crescentsphere.com  A  <VPS_PUBLIC_IP>  DNS only
```

DNS-only is recommended for the web hostname because CS Mail supports large
MBOX uploads and because direct traffic preserves the real client IP without
Cloudflare proxy trust configuration.

## First-deploy sequence

```bash
cd /opt/sites/cs-mail
sudo ./deploy/production/bootstrap-vps.sh
sudoedit /opt/cs-mail/.env.production
sudoedit /opt/cs-mail/.env.production  # set CS_MAIL_ALERT_EMAIL_TO/FROM
sudo ./deploy/production/show-config.sh /opt/cs-mail/.env.production
# Follow deploy/edge/README.md for the DNS-01 certificate and edge cutover.
sh manage preflight
sh manage deploy
sh manage status
```

Platform Admin stays private. From your workstation:

```bash
ssh -L 18081:127.0.0.1:18081 <ssh-user>@<vps>
```

Then open `http://localhost:18081/mail/admin/control-plane`. Never expose TCP
18081 publicly. Customer business administration (domains, users, and hosted
addresses) is at `https://mail.crescentsphere.com/mail/business` for active
business owners and admins; it is separate from platform operations.
