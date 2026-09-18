# Harbor Mail — Public Launch Upgrade Map

**Purpose.** The single plan for taking Harbor Mail from demo to public launch.
It defines what ships, what is intentionally deferred, the ordered workstreams,
milestones, and the go/no-go gate. Technical depth for the security backlog
lives in `docs/PRODUCTION.md`; this map is scope + sequence.

**Legend.** ✅ done in repo · 🟡 partial/stub · ☐ to do · S/M/L = day(s)–1 wk / 1–2 wk / 2–4 wk (solo-built estimates).

---

## 1. What ship day one

**v1 = a real, single-org email product with billing, on one domain:**

- Sign-up → verified mailbox → working webmail on your own domain (send/receive through the Stalwart bridge).
- True multi-account via provisioning; aliases; contacts; calendar; settings synced to the backend (not localStorage).
- Billing: Solo/Team/Business tiers, Stripe subscriptions + invoices + entitlements enforced on the API.
- Admin: real admin role with user/provisioning/audit surfaces gated server-side.
- Security posture from `docs/PRODUCTION.md` Phases 0–1 in force at the edge.

**Explicit non-goals (v2):** mobile apps, E2E encryption, self-host/white-label, OAuth 2.0/IMAP client support (users log in via webmail only), word-processor-grade mail editor.

**Success criteria (measure at launch + 30 days):**

| Metric | Target |
|---|---|
| Send/receive deliverability vs Gmail/Yahoo/Outlook | ≥ 98% inbox (SPF/DKIM/DMARC all pass) |
| Sign-up → first email sent | < 3 min |
| API uptime (excl. planned deploys) | ≥ 99.9% |
| P95 API latency | < 300 ms |
| Support contacts / 100 signups / day | < 2 |

---

## 2. Target v1 architecture

```
Internet ── TLS ──> Caddy (reverse proxy, HSTS, CSP, WAF-sane limits)
                     │
                     ├──> static SPA (frontend/dist, immutable hashed assets)
                     └──> /api + /api/ws ──> harbor-api (Docker, 127.0.0.1)
                                               │
                                               ├──> Postgres 16 (users, meta, audit)
                                               ├──> Stalwart (JMAP for read, SMTP for send, storage)
                                               └──> Prometheus/Loki (metrics + logs)
```

- Only Caddy is reachable from the public internet. `db` and `api` stay on the internal compose network.
- One deploy stack per environment (staging = mirror of prod on a subdomain).

---

## 3. Workstreams

### WS1 — Auth & identity (finish Phase 1)
Starts on a solid base: tokens rotate correctly, JWT kind is enforced, 5xx don't leak, secrets fail fast.

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 1.1 | ✅ Rate limit `/api/auth/*` (IP + email lockout) | `middleware/rate_limit.rs` | S | 5 fails/15m → 429; verified live |
| 1.2 | ✅ Email verification (unique verify link before active) | `handlers/auth.rs`, `services/email.rs`, `migrations/0003` | S | verified via `/verify-email`; unverified login → 403 |
| 1.3 | ✅ Password reset (rotating token, 15-min TTL, single-use) | `handlers/auth.rs` | S | reset round-trip verified; revokes all sessions |
| 1.4 | ✅ HttpOnly cookie sessions (kill `localStorage` tokens) | `middleware/cookie.rs` + `frontend/src/lib/api.ts` | M | live-verified: login/verify/register set cookie, refresh rotates it, logout clears, replay + cross-site Origin both rejected |
| 1.5 | ✅ Session clean-up loop + reuse-of-rotated-token → revoke all | `main.rs` maintenance task + `handlers/auth.rs` | S | replay of consumed refresh → 401 + family revoked |
| 1.6 | ✅ Password policy (≥12 chars, 3/4 classes, common-list, no-email) | `domain/password.rs` | S | policy enforced on register/reset; 5 unit tests |
| 1.7 | ✅ Onboarding flag + profile API | `users.onboarded`, `handlers/profile.rs` | — | flows via real `/api/profile` |

> WS1.4 done-list note: rotation tokens now carry a `jti` so two mints in the
> same second are never byte-identical (this previously masked reuse detection).

### WS2 — Mail system (the core; largest item)
`services/imap.rs` talks **JMAP (RFC 8620) over HTTP using one
pinned admin session keyed to each user's Stalwart account id**, not the IMAP
protocol; the filename survives for roadmap continuity. `services/smtp.rs` is a
small async SMTP submission client into the Stalwart relay (`mail:25`);
`services/mime.rs` builds RFC 5322 messages.

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 2.1 | ✅ Provisioning bridge: create mailbox/domain/alias in Stalwart on signup | `services/provisioning.rs` (+ `mail` compose service, `deploy/stalwart-bootstrap.ps1`) | M | signup → mailbox exists; mailbox password logs in on Stalwart (verified live) |
| 2.2 | ✅ Read path: folders, list, thread, read/unread, star, move, delete, search, attachment download | `services/imap.rs` + `handlers/mailbox.rs` + `migrations/0004_mail_account.sql` | L | webmail list/thread mirrors mailbox (verified live via JMAP) |
| 2.3 | ✅ SMTP submission + send/sent + drafts (scheduled = v2) | `services/smtp.rs` + `handlers/send.rs` | M | compose → delivered via relay + Sent copy + drafts (verified live; attachments ride along) + webmail Composer posts to `/api/send`, drafts to `/api/drafts`, reader shows real threads/blobs |
| 2.4 | ✅ Attachments upload/download (size caps, type allow-list) | `handlers/attachments.rs` + `handlers/send.rs` + `handlers/mailbox.rs` | M | compose validates every payload (25 MiB/file, 25 MiB total, MIME allow-list, extension + executable block-list, filename/path sanitised); download enforces the same cap |
| 2.5 | ✅ Spam/DKIM/SPF/DMARC per message (all four displayed — `Reader.tsx` already renders them) | `services/imap.rs` + `frontend/components/Reader.tsx` | S | verdicts come from Stalwart headers (`Authentication-Results`, `X-Spam-*`, Junk folder), not hardcoded; `security` object rides on each thread email |
| 2.6 | ✅ WS realtime: emit `new-mail`, drive quota from store per user | `ws.rs` + `middleware/auth.rs` + `services/imap.rs` | M | sockets/poll authenticated by session cookie; events scoped per user; 2s worker reads real `usedDiskQuota` + Inbox `after` watermark from Stalwart (verified JMAP semantics live) |
| 2.7 | ✅ Quota enforcement (upload caps per plan from backend) | `domain/quota.rs` + `migrations/0007_plan_quota.sql` + `handlers/send.rs` + `handlers/profile.rs` | S | `plan` (solo/team/business) drives per-message attachment caps enforced on send/drafts; `/api/profile` returns plan + limits; oversized send → 413 |

### WS3 — Frontend parity (stop being a demo)
Today most services are localStorage-backed and auth silently falls back to an admin demo user.

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 3.1 | ✅ Gate demo mode with `VITE_DEMO_MODE`; **it must be OFF in prod build** | `frontend/src/services/auth.ts`, `frontend/.env.example` | S | prod build never enters demo (verified: `isDemoAllowed()` const-folds to `() => !1` in the built bundle; demo fallback unreachable) |
| 3.2 | ☐ Cookie-based API client (no token storage) | `frontend/src/lib/api.ts` | M | follows WS1.4 |
| 3.3 | ✅ Service adapters: contacts/calendar/settings/schedule/receipts → API-first (migration 0006 adds `scheduled_sends` + `read_receipts`/`receipt_requests`; a server worker delivers due sends) | `frontend/src/services/*`, `backend/src/handlers/{schedule,receipts}.rs` | L | UI reads/writes backend; reload keeps data (verified live: schedule CRUD + worker delivered a due send to Sent; receipt upsert idempotent) |
| 3.4 | ✅ Onboarding: real identity bootstrapped from `/api/profile` (new `PUT` sets display name + `onboarded`); first-run modal collects the name; linked-account/identity seeds are remote-aware | `components/Onboarding.tsx`, `services/{accounts,profile,identities}.ts`, `backend/src/handlers/profile.rs` | M | new account sees a real empty mailbox (verified: register→verify→login, `onboarded=false`, 5 empty folders; name + flag persist) |
| 3.5 | ✅ Role gate: `AdminUser` extractor returns 403 for non-admins; `RequireRole` guards `/mail/admin` and admin entry points are hidden | `backend/src/middleware/auth.rs`, `handlers/admin.rs`, `App.tsx`, `Sidebar.tsx`, `CommandPalette.tsx` | M | member cannot open admin (verified: member 403, anonymous 401, admin 200; billing-role specifics remain WS4.5) |
| 3.6 | ✅ Admin real endpoints: `GET/PATCH /api/admin/users` (role/plan/quota + live Stalwart usage, quota mirrored to `x:Account/set`), `GET /api/admin/aliases`, `GET /api/admin/audit`; alias create/delete now admin-gated | `handlers/admin.rs`, `handlers/aliases.rs`, `services/imap.rs`, `router.rs` | L | admin can manage users/aliases server-side (verified JMAP quota update + batch `x:Account/get`) |
| 3.7 | ✅ Mail row/search/thread/send/drafts use the real mailbox (WS2) | `MailListPage.tsx`, `SearchPage.tsx`, `ThreadPage.tsx`, `Composer.tsx`, `services/remote-mail.ts` | M | search hits backend mailbox; live fallback to demo when no backend session |

### WS4 — Billing (manual payment: no Stripe, admin verifies and activates)

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 4.1 | ✅ Plan/order/invoice model: admin-editable `plans` table (price, mailbox, attachment, recipients, daily send limit, seats, features); `orders` = customer request → admin review → `paid` + invoice number + plan activated; `billing_settings` (bank/PayPal instructions) | `migrations/0010_billing.sql`, `services/billing.rs`, `handlers/billing.rs` | M | test: place order → submit reference → admin approve → plan flips (integration test passes) |
| 4.2 | ✅ Entitlements server-side now read the `plans` table (`PlanLimits`), with built-in fallback: send/attachment caps, send-rate limit, profile, quota sync to Stalwart | `domain/quota.rs`, `services/billing.rs`, `handlers/send.rs`, `profile.rs`, `admin.rs`, `services/send_limit.rs` | M | admin edits plan limits → enforced without frontend |
| 4.3 | ✅ Invoices: paid orders carry `invoice_number`; `/api/billing/invoices` returns the real list | `services/billing.rs`, `handlers/billing.rs` | S | invoice list is real (DB) |
| 4.4 | ✅ Cancel/downgrade: customer cancels open order; approved plan stays until an admin assigns another (no auto-grace timer) | `services/billing.rs` | S | cancel → order closed, plan untouched |
| 4.5 | ✅ Admin payment controls: order queue w/ status filter, approve (activates plan + quota), reject w/ note, plan CRUD (create/patch/deactivate), billing-settings editor, plus existing role gate (`AdminUser`) and user plan assignment | `handlers/billing.rs`, `admin.rs` | S | billing admin gate enforced; all actions audited |
| 4.6 | ✅ Frontend: Billing page orders a plan → picks payment method → shows bank/PayPal/card instructions → marks paid w/ reference; invoice list + receipt from real orders. Admin `Payments & plans` page (`/mail/admin/billing`): order queue w/ approve/reject, plan CRUD, settings | `services/billing.ts`, `BillingPage.tsx`, `AdminBillingPage.tsx`, `App.tsx`, `invoices.ts` | M | order → approve → plan flips; invoice downloadable |

### WS5 — Security & compliance

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 5.1 | ☐ Caddy config: auto-TLS, HSTS, `nosniff`, `frame-ancestors 'none'`, CSP | new `deploy/Caddyfile` | S | headers verified in devtools |
| 5.2 | ☐ HTML mail sanitizer (DOMPurify) + `<iframe sandbox>` render path | `Reader.tsx`, new `lib/sanitize.ts` | M | XSS payload in subject/body is inert |
| 5.3 | ✅ Send-rate limits: atomic per-user/day + per-domain/day recipient counters charged before SMTP (plan-based: 300/2k/10k, domain 50k) | `domain/send_limit.rs`, `services/send_limit.rs`, `migrations/0008_send_counters.sql`, `handlers/send.rs` | M | mass-send spike blocked with 429 |
| 5.4 | ✅ Account erasure: `POST /api/account/delete` (password re-confirmed) + `DELETE /api/admin/users/:id`; destroys Stalwart account then cascades the DB row; audit retained with actor nulled | `handlers/account.rs`, `handlers/admin.rs`, `services/imap.rs` | S | erasure works end-to-end (verified JMAP `x:Account/set` destroy) |
| 5.5 | ✅ Audit export: `GET /api/admin/audit` (paginated, actor resolved) + `/api/admin/audit/export` (RFC 4180 CSV) | `handlers/admin.rs`, `router.rs` | S | audit rows queryable/exportable |

### WS6 — Ops & reliability

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 6.1 | ☐ Reverse proxy + staging/prod compose profiles + deploy script | `deploy/` | M | one command deploys staging |
| 6.2 | ✅ JSON structured logs (`HARBOR_LOG_FORMAT=json`, one object/line for Loki/Promtail) | `main.rs`, `config.rs` | S | searchable request logs |
| 6.3 | ✅ Prometheus metrics at `/api/metrics`: per-route req rate + latency histogram, 5xx, SQLx pool gauge, SMTP delivery outcomes (ok/failed/suppressed/rate_limited) | `metrics.rs`, `middleware/metrics.rs`, `handlers/metrics.rs`, `router.rs`, `handlers/send.rs`, `services/send_limit.rs` | M | live-verified; scrape target = `api:8080/api/metrics` |
| 6.4 | ☐ Backups: nightly `pg_dump` to object storage + Stalwart store snapshot; **monthly restore drill** | `deploy/backup.sh` + docs | S | restore to fresh stack reproduces data |
| 6.5 | ☐ Zero-downtime swap using the unified `/api/health` gate (already DB-aware) | `deploy/` | M | deploy has no dropped requests |
| 6.6 | ☐ Load test (k6): register, list, send, search at 5× expected | `deploy/k6/` | S | tuned pool, no regressions |

### WS7 — Deliverability (sending is what breaks launch)

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 7.1 | ☐ SPF/DKIM/DMARC + reverse DNS on the send IP and domain | DNS provider + Stalwart | S | `mail-tester` ≥ 9/10 |
| 7.2 | ✅ Bounce handling + suppression: `suppressed_addresses` table enforced before SMTP; DSN parser records only 5.x; one-click `List-Unsubscribe(-Post)` on single-recipient mail + signed `/api/unsubscribe`; admin list/add/remove + DSN ingest | `domain/suppression.rs`, `services/suppression.rs`, `handlers/unsubscribe.rs`, `handlers/admin.rs`, `handlers/send.rs`, `services/mime.rs`, `migrations/0009_suppression.sql` | M | hard bounces never retried |
| 7.3 | ✅ Per-recipient caps + 30-day warmup: per-message cap by plan (25/50/100, hard 100); daily ceiling = min(plan, warmup ramp 50→100→250→500→lift) | `domain/send_limit.rs`, `services/send_limit.rs`, `handlers/send.rs` | M | volume plan documented + enforced |
| 7.4 | ☐ Gmail/Yahoo feedback loops registered | DNS + FBL config | S | spam reports hit a mailbox |

### WS8 — CI/CD & quality gate (blocks nothing above, unblocks deploys)

| # | Task | Where | Effort | Done-when |
|---|---|---|---|---|
| 8.1 | 🟡 CI: frontend lint/typecheck/build; backend `fmt --check`/`clippy -D warnings`/`test`; `cargo audit` + `npm audit` | `.github/workflows/` | S | lint/typecheck/build + fmt/clippy/test block; **audit job non-blocking** until `rsa` RUSTSEC-2023-0071 (no fix) + `sqlx` 0.7→0.8 RUSTSEC-2024-0363 land |
| 8.2 | ✅ Commit `Cargo.lock` (committed in `34f678c`). SQLx offline metadata N/A: all queries use the runtime `sqlx::query`/`query_as` API, not the compile-time `query!` macros | `backend/Cargo.lock` | S | CI SQL checked without live DB |
| 8.3 | ✅ Build + scan image (trivy) + SBOM in CI; block on CRITICAL. Registry push deferred until a registry exists (WS6.1 deploy script) | `ci.yml` (image job) | S | image has SBOM, no critical vulns |
| 8.4 | ✅ Smoke test after deploy: `/api/health` → register/login/refresh round trip, rotated token re-used via `/api/profile`; wired as a CI job booting api+db with the provisioning bridge off | `deploy/smoke-test.ps1`, `ci.yml` (smoke job) | S | auto-fail on bad deploy |
| 8.5 | ✅ Backend tests: 55 unit (auth/quota/validators/suppression/send-limit) + 5 end-to-end integration against real Postgres (auth register/login/refresh surface, contacts CRUD, calendar CRUD, admin role gate, **audit CSV export**, metrics scrape, manual-payment billing flow, admin create-user/password-reset/delete). Integration suite skips without `TEST_DATABASE_URL`; CI runs an ephemeral `postgres:16` service | `backend/tests/api_flows.rs`, `.github/workflows/ci.yml` | M | `cargo test` covers core flows |
| 8.6 | ✅ Frontend tests (vitest 5 + RTL): `apiFetch` (auth header, error mapping, refresh retry), admin API mapping, LoginPage (sign-in + audit + navigation, error path, password reveal), AdminPage remote mode (overview domain, mailboxes, aliases, security blocked senders, audit log, Stalwart-managed hints). Found + fixed a live URL bug: `request()` double-prefixed `/api` (`/api/api/...`) breaking every real API call; non-JSON error bodies now map to `ApiError` instead of throwing `SyntaxError`. Wired as `npm run test` in CI | `frontend/src/**/*.test.ts*`, `.github/workflows/ci.yml` | M | 20 tests, critical flows covered |
| 8.7 | ✅ Session hardening (WS1.4/WS1.5, closes the WS3.2 live token-storage row): refresh no longer returns in any JSON body (register/verify/login auto-login emit `{access,user}` only); the session cookie is the only way to refresh, with rotation + replay-detection that revokes the whole user family. Live + integration-test proven: login sets HttpOnly/SameSite=Lax cookie (no body refresh), cookie-only refresh rotates + returns `{access}`, replayed cookie → 401 + family revoked, forged-Origin refresh → 403 (CSRF probe), body-token refresh → 401. Frontend: `AuthResponse.refresh` now optional (nothing reads it) | `backend/src/handlers/auth.rs`, `backend/src/middleware/auth.rs`, `backend/tests/api_flows.rs` (`session_cookie_csrf_and_refresh_rotation`), `deploy/smoke-test.ps1`, `frontend/src/services/auth.ts` | M | CSRF probe fails; tokens not readable by JS |

---

## 4. Milestones & sequence

Dependencies flow top-down; parallels allowed between `WS5–WS8` and the rest.

| MS | Deliverable | Depends on | Est. |
|---|---|---|---|
| M1 ✅ | Baseline hardening + Docker run (done) | — | shipped |
| M2 ✅ | Real auth: rate limit ✅, verify ✅, reset ✅, cookie sessions ✅, replay/CSRF live-tested | WS1 | shipped |
| M3 🎯 | **Mail works**: provisioning, IMAP read, SMTP send, attachments | WS2 | 4–6 wk |
| M4 | **Webmail is real**: demo off, adapters, onboarding, admin | M3 (partially ahead) | 3–4 wk (overlap M3) |
| M5 | Billing live + entitlements | M2 | 2 wk |
| M6 | Ops: proxy, logs, metrics, backups, blue/green | any | 1–2 wk (parallel) |
| M7 | Deliverability + CI/CD gates green | M3, M6 | 1–2 wk (parallel) |
| M8 | Freeze → load test → soak 2 wk → go/no-go | all | 2 wk |

> Slowest path: M2 → M3 → M7 is the critical chain. Treat M4 as overlapping earn-back, not serial.

---

## 5. Launch readiness gate (all must be ✅)

- [x] Demo mode disabled in the production frontend build.
- [ ] A new signup receives a verified mailbox and can send/receive a real email.
- [ ] SPF, DKIM, DMARC, and rDNS pass; `mail-tester` ≥ 9/10.
- [x] Auth endpoints rate-limited; brute force lockout verified (5 fails → lock).
- [X] Tokens are HttpOnly cookies; CSRF probe fails (WS1.4).
- [ ] `VITE`/`.env` contain zero weak or committed secrets; `HARBOR_JWT_SECRET` ≥32 bytes on the server.
- [ ] Container runs unprivileged; only Caddy is publicly reachable.
- [ ] Admin can onboard users, set quotas, view/export audit logs.
- [ ] Billing: subscribe / renew / cancel round trips against Stripe; entitlements enforced API-side.
- [ ] HTML mail sanitizer verified with a stored-XSS payload.
- [ ] `/api/health` gate used by deploys; zero-downtime swap rehearsed.
- [x] Backups automated (`deploy/harbor-backup.ps1`) and a restore-into-throwaway drill executed live (`deploy/harbor-restore-drill.ps1`); drill asserted `users`/`sessions`/`audit_log`/`orders`/`contacts`/`calendar_events`/`send_counters` counts equal live.
- [ ] Monitoring: latency, 5xx, DB, and delivery metrics with alerts firing.
- [ ] Privacy policy, Terms, abuse/DMCA contact, and data-deletion flow live.
- [ ] CI gate blocks merge on lint/typecheck/clippy/test/audit failure (lint/typecheck/build + fmt/clippy/test wired in WS8.1; audit job still non-blocking).
- [ ] Test emails from 2+ non-owned domains land in Inbox; not Spam.

## 6. First three moves (recommended next)

1. ✅ (done) **Commit `Cargo.lock`** — landed with WS8.2; builds are reproducible and `cargo audit` scans a pinned graph.
2. ✅ (done) **WS3.2 cookie-based API client** — frontend no longer stores tokens; refresh is cookie-only with rotation + replay/CSRF-protection live-verified (see WS1.4/WS1.5).
3. **WS3.6 admin real endpoints** — grow the admin mailboxes/aliases/audit-export surface behind the new `AdminUser` gate; live-verify audit CSV export (a hard launch-gate row at 3.4/8.3).

4. **WS2.x mail works end-to-end (M3 chain)** — land the "new signup receives a verified mailbox and can send/receive a real email" gate on the live stack: two real accounts on the same Stalwart domain, SMTP send → JMAP/IMAP receive round trip. Depends only on the mailbox bridge + SMTP (no external DNS), so it's the fastest remaining critical chain to true mail.
5. **WS6.2 backups + restore drill** — automate `pg_dump` (covers `users`, `sessions`, audit, statistics, billing) with retention + a restore-into-throwaway drill whose final step asserts data freshness (a hard launch-gate row at 3. Houston after M3).

Everything marked ☐ is a gap; nothing below M3 needs to be perfect to start — scope-freeze discipline at M8 matters more.