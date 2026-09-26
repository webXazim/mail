# CS Mail production acceptance

CS Mail implements business tenancy in its own database and API: organizations
own domains and mailboxes; users need an active membership and an assigned
mailbox; provider objects carry a namespace and organization marker. This
does not depend on Stalwart Enterprise tenants. Treat Stalwart as a shared
mail protocol/storage provider and keep its admin API private.

Before accepting another business, complete these checks in order:

1. Deploy the independent platform edge and then CS Mail using
   `../edge/README.md`. Verify the direct HTTPS certificate for
   `mail.crescentsphere.com`, the private admin route over an SSH tunnel, and
   that `/api/admin`, `/mail/admin`, and `/api/metrics` return 404 publicly.
2. Use two separate businesses and two separate domains in the launch
   certification. The account from business A must not read or change B's
   domains, members, mailboxes, messages, drafts, or settings. Run the full
   `certify-launch.sh` gate and retain its report. Do not treat a static-only
   pass as this live test.
3. Before opening any public switch, use localhost **Platform Admin → Diagnostics → Public launch readiness**. Resolve every blocker, then run the full live launch certification and re-check readiness. A static certification pass is only a release-layout check; it does not replace provider/DNS/TLS/tenant/protocol/backup acceptance.

4. Migration 0042 closes public signup, business creation, plan ordering,
   domain onboarding, mailbox provisioning, and outbound sending in the
   platform control plane. Open only what is needed for an operator-led test;
   keep public capabilities disabled until the two-business test, billing
   flow, and abuse review are complete.
   Keep `CS_MAIL_BILLING_INSTANT_ACTIVATION=false` for the real payment-flow
   acceptance test. If you intentionally exercise the instant-activation test
   path, use only disposable test orders and return the flag to `false` before
   launch certification. Use the platform admin interface to enable controls
   deliberately after the gates pass.
5. Verify a real mailbox over web, IMAP TLS 993, and authenticated SMTP TLS
   465. Send externally, receive externally, reply, attach a file, and confirm
   sent, inbox, and spam behavior. Apply the shared sender policy in Mailer's
   `STALWART_DOMAIN_PROVISIONING.md` to Stalwart's AUTH and MAIL FROM stages.
   Confirm `cs-mail-submit` accepts an existing mailbox sender but rejects a
   Mailer bounce sender; confirm `mailer-submit` accepts a verified Mailer
   bounce sender but rejects a CS Mail mailbox sender. Test these negative
   cases with authenticated SMTP, not just the applications' API checks.
6. For each customer domain, verify ownership before provisioning. Publish MX
   to `smtp.crescentsphere.com` only after the destination mailbox is ready;
   publish the provider's actual DKIM key, a single valid SPF policy and a
   DMARC policy, then validate alignment on received test mail. Keep
   `smtp.crescentsphere.com` DNS-only. Ensure the VPS provider's PTR matches
   that hostname. Replace Cloudflare Email Routing MX for
   `crescentsphere.com` only after its migration test passes.
7. Verify backup coverage for the CS Mail database and attachments and for
   Stalwart's mail data. Store encrypted copies off this VPS; run the restore
   drill and document the recovery time. Check alerts for API health, mail
   queues, failed provisioning jobs, disk space, TLS expiry, and backup age.
8. Keep Mailer developer sending credentials, rate limits, and provider
   namespace separate from CS Mail. A shared Stalwart and outbound IP remain
   a shared reputation and outage boundary. Before broad public developer
   sending, allocate a separate outbound IP or mail node and change Mailer's
   egress/DNS/PTR accordingly; CS Mail business mail should retain its own
   stable sending identity.
9. Reconcile every pre-existing Mailer domain and DKIM signature against its
   Mailer database provider IDs before opening either product to customers.
   Existing unmarked provider domains require an operator ownership review;
   never attach a CS Mail domain to Mailer merely because its name matches.

After launch, deploy pinned images and schema changes through the checked
release pipeline, preserve a predeploy backup, and run the public health and
security gates before marking a release successful. Rehearse both the web
edge rollback and the database/mail-data restoration. The company homepage
can replace Messenger's root/www route later without touching mail routing.

## Final launch freeze and opening sequence

The production API must run with `CS_MAIL_ENVIRONMENT=production`. Controlled
acceptance testing may temporarily use `CS_MAIL_BILLING_INSTANT_ACTIVATION=true`,
but the final launch freeze is fail-closed: public launch requires
`CS_MAIL_BILLING_INSTANT_ACTIVATION=false`.

Before the first public opening, complete the independent encrypted offsite
backup jobs and have each job emit a root-owned, non-world-writable proof
manifest. The proof should identify the external backup set/object and its
checksum or provider snapshot identifier. Then record the successful external
jobs:

```bash
cd /opt/sites/cs-mail
sh manage record-backup-proof cs-mail /absolute/path/to/cs-mail-offsite.manifest
sh manage record-backup-proof stalwart /absolute/path/to/stalwart-offsite.manifest
sh manage launch-freeze
```

`record-backup-proof` does not perform or verify a remote copy by itself. It
records the evidence produced by the independent backup system. `launch-freeze`
runs release verification, production preflight, the full live certification,
a fresh local backup and restore drill, and requires fresh offsite evidence for
both CS Mail and shared Stalwart. It also requires every public platform switch
to remain closed while the evidence is created.

Only after **PUBLIC LAUNCH FREEZE PASS**, open switches in localhost Platform
Admin in this order, checking alerts and one real customer path after each
stage: **Public signup → Business creation → Plan ordering → Domain onboarding
→ Mailbox provisioning → Customer outbound sending**. Do not edit the
`platform_controls` table directly. If a stage creates unexplained errors,
close that stage again and investigate before opening the next one.
