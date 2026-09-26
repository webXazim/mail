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
- Frontend lint/typecheck/tests/build and Rust fmt/Clippy/tests/release build are blocking deployment gates.
- Alert delivery is tested end-to-end.
- Real external inbox placement is tested with unrelated providers.
- `CS_MAIL_ENVIRONMENT=production`; payment-approved activation is the default. Controlled acceptance testing may temporarily enable instant activation: ordering a plan immediately reactivates suspended/past-due/expired service, while changes to an already active/trial plan still await payment review. Final launch certification requires `CS_MAIL_BILLING_INSTANT_ACTIVATION=false`.
- Platform Admin → Diagnostics → Public launch readiness has no unresolved blockers.
- Billing/provider durable issue gauges show no unexplained dead jobs, failed lifecycle/invoice notices, failed purges, or stale mailbox reconciliation.
- The latest live launch certification is passed and was run immediately before opening public controls.

- Local backup evidence is fresh for the exact deployed release, the isolated restore drill is fresh, and encrypted offsite proof is fresh for both CS Mail and shared Stalwart data.
- `sh manage launch-freeze` passes while every public platform switch is still closed.
- After freeze passes, controls are opened deliberately in order: signup → business creation → plan ordering → domain onboarding → mailbox provisioning → outbound sending, with one real flow and alert review between stages.
