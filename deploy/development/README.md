# Development-only stack

Files in this directory are for local development and CI. They may start a local Stalwart and use loopback development ports.

**Never use this compose file for the public VPS.** Production uses only `deploy/production/docker-compose.yml` and the already-running shared Stalwart service.

Example local setup:

```bash
cd deploy/development
cp .env.example .env
# fill local-only values
docker compose up -d
```

`ci-compose.yml` is a minimal PostgreSQL + API topology used by GitHub Actions smoke tests and intentionally has no mail provider.
