# Production launch checklist

A successful `deploy.sh` means the release is running; it is not by itself approval for public customer onboarding.

## Required deployment gates

- Git production checkout is clean and release verification passes.
- Frontend lint, typecheck, tests and build pass in Docker.
- Rust API builds from committed `Cargo.lock` with `--locked`.
- Existing production state receives a fresh pre-deploy backup.
- SQLx migrations complete and API `/api/health/ready` passes.
- Nginx configuration validates and public HTTPS health passes.
- Public `/api/admin/*`, `/mail/admin*` and `/api/metrics` return `404`.
- Platform Admin works only through the SSH-tunneled localhost listener.
- Prometheus and Alertmanager start successfully.

## Mail/DNS gates

- A/AAAA records are intentional.
- PTR/rDNS matches the public Stalwart hostname.
- MX, SPF, DKIM and DMARC are correct and aligned.
- IMAPS 993 and SMTP submission 587 present valid certificates.
- Port 25 relay policy is not an Internet open relay.
- External Gmail/Outlook (or other unrelated providers) can send to and receive from CS Mail.
- SPF/DKIM/DMARC results are verified from received message headers.
- Inbox-vs-spam placement is manually observed.

## SaaS/billing gates

- Registration and verification/reset transactional mail work.
- Customer domain verification and provisioning work end to end.
- Plan order records the exact plan + mailbox quantity.
- During acceptance testing `CS_MAIL_BILLING_INSTANT_ACTIVATION=true` may remain enabled.
- Before paid public launch, set instant activation to `false` and confirm payment approval assigns the invoiced plan.
- Plan expiration/grace/suspension works.
- Business storage pool and per-mailbox Stalwart quotas agree.

## Operational gates

- Daily CS Mail backup timer is active.
- A fresh `restore-drill.sh` passes.
- Shared Stalwart has a separate off-VPS backup and tested recovery plan.
- Alertmanager delivers a real test alert to the operator receiver.
- Disk/CPU/memory headroom is measured on the production VPS.
- Platform emergency switches and failed-job recovery are tested.

## Final automated certification

After deployment, use disposable cross-tenant accounts/app passwords and run:

```bash
sudo /opt/sites/cs-mail/deploy/production/certify-launch.sh \
  /opt/cs-mail/.env.production \
  /opt/cs-mail/.env.certification
```

The command must exit 0 and report `LAUNCH CERTIFICATION: PASS`. Revoke disposable app passwords afterward.
