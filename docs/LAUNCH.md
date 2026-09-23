# Production launch gate

Before public launch, all of the following must pass:

- `mail.crescentsphere.com` DNS-only A record resolves to the VPS.
- Let's Encrypt HTTPS for `mail.crescentsphere.com` verifies and renews.
- `smtp.crescentsphere.com` remains DNS-only, resolves to the VPS and matches PTR/rDNS.
- Stalwart TLS verifies on IMAPS 993 and SMTP submission 465.
- Shared Stalwart owns 25/465/993; CS Mail starts no second mail service.
- CS Mail API remains loopback-only on 18080.
- Platform Admin remains loopback-only on 18081 and public admin routes return 404.
- PostgreSQL/Prometheus/Alertmanager are not Internet exposed.
- SPF/DKIM/DMARC/MX and open-relay checks pass.
- Cross-tenant authorization, app-password SMTP/IMAP, backup and restore certification pass.
- Frontend and Rust lint findings are reported; frontend typecheck/tests/build
  and Rust tests/release build remain blocking deployment gates.
- Alert delivery is tested end-to-end.
- Real external inbox placement is tested with unrelated providers.
- `CS_MAIL_BILLING_INSTANT_ACTIVATION` is changed to `false` before accepting real paid orders.
