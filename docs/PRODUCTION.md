# Harbor Mail — Production Upgrade Map

Target: a secure, observable, recoverable, releaseable mail product. Current:

- `frontend/` — React/Vite SPA, largely localStorage-backed ("demo-first") with a silent demo fallback.
- `backend/` — Rust (axum + sqlx/Postgres) API: auth (register/login/refresh/logout), profile, contacts, aliases, settings, calendar CRUD, unauthenticated `/api/ws`, DB-aware `/api/health`.
- `docker-compose.yml` — Postgres 16 + api service.

How to read this map: each phase lists **What / Why / Where / Done-when**. Work is
ordered by severity, then by dependency. Items marked `[done]` are already
landed in this repo. Items marked `[advisory]` are configuration/process
recommendations for the VPS (reverse proxy, backups), not code.

---

## Phase 0 — Security hardening `[in progress]`

What                                      | Status
------------------------------------------|---------------------------------------
Run containers unprivileged (no root)      | [done] Dockerfile `USER 10001`
Never default secrets; fail fast if absent | [done] compose `HARBOR_JWT_SECRET` required, 32-byte boot check in `config.rs`
No public Postgres/DB port                 | [done] DB bound to 127.0.0.1
Drop container caps, no-new-privileges     | [done] compose `cap_drop` + `security_opt`
Read-only root fs + `tmpfs /tmp` for api   | [done] compose `read_only`
Don't cache `/api/*` in the service worker | [done] `frontend/public/sw.js`
JWT `kind` enforced (refresh ≠ access)     | [done] `backend/src/middleware/auth.rs`
5xx never leak internals to clients        | [done] `backend/src/error.rs`
Body-size limit + request tracing          | [done] `backend/src/router.rs`
Graceful shutdown, DB-aware health         | [done] `main.rs`, `handlers/health.rs`

Remaining (see phases below): rate limiting, HTTPS/HSTS + security headers at
the edge, cookie-based sessions, real mail HTML sanitization, dependency
scanning, secret rotation with zero insecure defaults.

---

## Phase 1 — Auth & identity (backend)

1. **Rate limit auth endpoints.** `tower-governor` (or IP+email lockout).
   - Why: login/register are the only brute-force surface; no limit today.
   - Where: `backend/src/router.rs` (wrap the auth paths), new `middleware/rate_limit.rs`.
   - Done-when: >N failed logins per IP/minute → 429; bulk-credential tests fail-fast.
2. **Password policy.** Enforce length ≥ 12 + reject the top-1k breached terms (offline list or `Have-I-Been-Pwned` k-anonymity via range lookup).
   - Where: `backend/src/handlers/auth.rs` `register`.
3. **Email verification + real password reset.**
   - The frontend stubs `requestPasswordReset`/`verifyEmail` with `{ok:true}` (`frontend/src/services/auth.ts`). Implement verified-email gates and rotating reset tokens (HMAC or DB-backed, 15-min TTL, single-use).
   - Where: new `backend/src/handlers/auth.rs` flows + `services/smtp.rs` (now a stub).
4. **Move tokens out of `localStorage` into `HttpOnly; SameSite=Strict; Secure` cookies.**
   - Why: any XSS steals `localStorage` tokens today. Cookies are not readable by JS.
   - Where: backend sets/rotates cookies on `/api/auth/*`; `frontend/src/lib/api.ts` sends `credentials:'same-origin'`, drops its own storage; CSRF protection via `SameSite` + posted CSRF token for state-changing calls.
   - Done-when: `document.cookie` shows no session material; CSRF attempt fails.
5. **Session lifecycle.** Periodic `DELETE FROM sessions WHERE expires_at < now()` (tokio interval); detect reuse of a rotated refresh token → revoke all user sessions.
   - Where: `backend/src/services/...`; hook into `spawn_ticker`.

---

## Phase 2 — API & data integrity

1. **Strict input validation** (regex/email crate, length caps, JSON schema rejection) for every handler; currently several rely on `contains('@')` or no cap.
   - Where: `handlers/*.rs`; consider `validator`/`garde`.
2. **Ownership everywhere.** Contacts/calendar/settings already scope by `user_id`; assert the same for aliases (list is global today) and audit reads (no admin API exists; admin is frontend-only demo data).
3. **Mail pipeline.** `services/imap.rs`/`smtp.rs` are stubs. Behind the mail store (Stalwart) the API should expose a REAL mailbox surface and push `new-mail` WS events; the ticker quota must come from the store, not hardcoded 0.
4. **Pagination + list caps** for contacts/calendar/aliases/audit so one user can't force huge payloads.
5. **DB hardening:** connection pool already capped; add statement timeouts (`tcp_keepalives`, `statement_timeout`), and use `query!`/`query_as!` compile-checked SQL with `SQLX_OFFLINE=true` in CI to catch SQL drift.

---

## Phase 3 — Frontend real-mode parity

The frontend is demo-first: mailbox, calendar, contacts, settings, admin are
localStorage. For production the backend must be the source of truth, with the
local store only as an offline/dev fallback.

1. **Gate the demo fallback.** `authApi.login/register` today fall back to an
   admin demo user on any non-`ApiError` failure — a backend outage silently
   turns into phantom admin auth. Gate with `VITE_DEMO_MODE` (default on in dev,
   **off** in the production build).
   - Where: `frontend/src/services/auth.ts`, `vite` env, `frontend/.env.example`.
2. **Service layer adapter.** Give each service (`accounts`, `calendar`,
   `contacts`, `settings`, `admin`, `audit`, …) a thin abstraction: try
   `apiFetch` first, fall back to the existing localStorage implementation only
   when demo mode is on.
   - Where: new `frontend/src/services/*.api.ts` + shared `fallback` helper.
3. **Real-time.** `ws.ts` should send the auth token (query param or cookie) and
   subscribe per-user after login; reconnect with backoff already exists.
4. **Role-based gating.** Admin surfaces (`AdminPage`, audit) must require
   `role==='admin'` from the backend and 403 server-side, not trust localStorage.
5. **XSS posture for HTML mail.** Bodies are text today; when real HTML mail
   lands, render through an allow-list sanitizer (DOMPurify) inside an
   `<iframe sandbox>`, never `dangerouslySetInnerHTML` raw.

---

## Phase 4 — Edge, TLS, observability, ops `[advisory]`

1. **Reverse proxy + TLS.** Put Caddy (or Nginx) in front: automatic HTTPS,
   HTTP→HTTPS redirect, `Strict-Transport-Security`, `X-Content-Type-Options`,
   `frame-ancestors 'none'`, per-origin CSP. Serve `frontend/` static build;
   reverse-proxy `/api` + `/api/ws` to `127.0.0.1:8080`.
2. **Structured logging.** Switch tracing to JSON in prod; ship to Loki/Splunk/ELK.
   - Where: `backend/src/main.rs` (EnvFilter already set).
3. **Metrics.** Prometheus endpoint (`/api/metrics`) for request rate, latency,
   DB pool, 5xx rate; alerting on error budget.
4. **Backups.** Daily `pg_dump` to object storage + point-in-time recovery;
   test a restore monthly. Volume snapshots for `pgdata`.
5. **Secrets.** One `.env` on the server, chmod 600, rotated quarterly;
   `docker compose` must never run with a committed/known secret.
6. **Resource limits.** Landed in compose (`mem_limit`/`cpus`); verify with
   load tests that OOM doesn't cascade.

---

## Phase 5 — Delivery (CI/CD + quality)

1. **CI (GitHub Actions / Gitea Actions):**
   - frontend: `npm ci`, `npm run typecheck`, `npm run lint`, `npm run build`.
   - backend: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
     (`SQLX_OFFLINE=true` + generated `sqlx-data.json` only once compile-time
     `query!` macros are adopted; the API uses runtime `query`/`query_as` today.)
   - containers: `docker build`, push to registry with content-hash tag.
   - Implemented: `.github/workflows/ci.yml` (WS8.1) — fmt/clippy/test and frontend
     lint/typecheck/build are blocking; `cargo audit`/`npm audit` run non-blocking
     until the `sqlx` 0.8 upgrade lands.
2. **Dependency scanning:** `cargo audit` + `npm audit` in CI; `trivy` in the image.
3. **Semver tags + immutable releases**, rollback = `docker compose --profile <rev>`.
4. **Smoke test after deploy:** `/api/health` ready-gate then a synthetic
   register/login/refresh round trip in a throwaway tenant.
5. **Commit `Cargo.lock`** (present but untracked) for reproducible backend
   builds — `backend/Cargo.lock` already exists; `git add backend/Cargo.lock`.
   Until then CI generates it on the fly before `cargo audit`.

---

## Phase 6 — Scale & performance (post-launch)

- Postgres read replicas for the read path; connection pooling (PgBouncer) behind `HARBOR_DB_MAX_CONNECTIONS`.
- Caching tier (Redis) for settings/contacts/calendar reads + pub/sub for push.
- CDN + immutable asset hashing for the SPA (Vite already emits hashed chunks).
- Load tests (`k6`/`oha`) to set real limits; tune pool size in `config.rs`.

---

## Cross-cutting checklist

- [ ] No secrets or default credentials committed anywhere (scan each PR).
- [ ] Every env var the code reads exists in `.env.example`/compose, and vice-versa.
- [ ] Migrations are additive and reversible; never drop columns from prod.
- [ ] `cargo clippy -D warnings` and `cargo test` green before release.
- [ ] CI blocks on failed audits, lint, typecheck, or build.
- [ ] A documented restore drill works (backup → fresh stack → same data).

## Suggested sequencing

```
Phase 0 (done) → Phase 1 auth → Phase 2 data/API → Phase 4 edge/ops (parallel)
→ Phase 3 frontend parity → Phase 5 CI/release → Phase 6 scale
```

Phase 4 (edge TLS + logging + backups) can start in parallel with Phase 1 since
it is infrastructure, not code.