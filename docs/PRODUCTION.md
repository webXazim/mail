# Production architecture

CS Mail shares a VPS with other projects. Host Nginx owns public web ports
80/443 and routes by hostname; the CS Mail vhost owns only
`mail.crescentsphere.com`. The Rust API binds `127.0.0.1:18080`, Platform Admin
binds `127.0.0.1:18081`, and PostgreSQL/monitoring stay private.

The existing shared Stalwart remains the mail transport. Its public identity is
`smtp.crescentsphere.com`, which is DNS-only and also matches the VPS PTR/rDNS.
It owns host ports 25/465/993. CS Mail production Compose never starts a second
mail server or binds those ports.

Production deploys are Git-driven and deterministic. Builds/tests happen in
Docker using committed lockfiles. Frontend releases are published atomically
under `/opt/cs-mail/www/releases`; runtime state and secrets live outside Git
under `/opt/cs-mail`.

## Final public-launch invariants

The production runtime is explicit (`CS_MAIL_ENVIRONMENT=production`) and is
bound to the exact deployed source digest through `CS_MAIL_RELEASE_SHA256`.
Unsafe production settings such as instant unpaid plan activation cause API
startup to fail instead of silently falling back.

Migration `0048_launch_freeze_operational_evidence.sql` adds append-only local
backup, restore-drill and offsite-backup evidence. The final launch gate is
`sh manage launch-freeze`; it must pass with public controls closed before those
controls are opened deliberately from localhost Platform Admin.


### Upgrade 05F compatibility note

The production Compose stack hard-pins the API runtime to `CS_MAIL_ENVIRONMENT=production`. Older root-owned production env files may omit that key; an explicit non-production value is still rejected. Migration `0049_provisioning_operation_constraint_repair.sql` repairs long-lived databases whose provisioning operation CHECK constraint predates the `set_access` lifecycle job.
