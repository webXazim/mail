# Production architecture

Current API contract: **v32**. Current migration head: **0041_full_saas_control_plane.sql**.

CS Mail production consists of a React static frontend, a Rust/Axum API, PostgreSQL and a shared Stalwart provider. Tenant/business authority, billing entitlements, mailbox quotas, provisioning and Platform Admin controls are server-authoritative.

## Trust boundaries

- Public users reach only Nginx 80/443 and authenticated Stalwart client ports.
- Rust API is bound to `127.0.0.1:18080` on the host.
- Platform Admin is bound to `127.0.0.1:18081` and additionally requires the Nginx-injected local-admin marker.
- PostgreSQL, Stalwart management/JMAP, Prometheus and Alertmanager are not Internet-exposed.
- Production compose joins the existing Stalwart network and never creates a mail server.
- Provider mutations verify the immutable `cs-mail` ownership namespace before destructive actions.

## Deployment authority

Production deploys are Git-driven and deterministic. Frontend and backend are built in Docker using committed lock files. The repository checkout is source only; static build output is extracted to `/opt/cs-mail/www/releases`, persistent state is in Docker volumes, and secrets/runtime metadata live under `/opt/cs-mail`.

SQLx migrations run automatically when the new API starts. For that reason `deploy.sh` creates a pre-deploy backup before replacing a running API. Rollback never attempts to downgrade schema automatically.

See `deploy/production/README.md` for exact commands and `docs/LAUNCH.md` for acceptance gates.
