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
