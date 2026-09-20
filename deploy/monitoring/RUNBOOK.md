# Harbor monitoring RUNBOOK

Operational handbook for the alert rules in
`deploy/monitoring/prometheus.rules.yml`. Each `#anchor` below is the exact
fragment referenced by a rule's `runbook:` annotation (and pinned by
`prometheus.rules.test.yml`). When an alert pings you, jump straight to its
anchor.

Everything here assumes the compose stack from `docker-compose.yml` (api + db +
mail) **and** the monitoring services:

```powershell
docker compose --profile monitoring up -d prometheus alertmanager
```

Quick truth: is Prometheus itself alive and scraping the api? Both must be true
for ANY alert below to be meaningful:

- `http://127.0.0.1:9090/targets` — api target `UP`, no scrape errors.
- `http://127.0.0.1:9090/alerts` — no failed rule evaluations.
- `docker compose logs prometheus alertmanager --since 10m` — no `level=error`.

```powershell
# one-liner, proves the whole chain is scraping + firing (HTTP 200 = ok):
docker compose exec prometheus sh -c 'wget -qO- http://127.0.0.1:9090/-/healthy'
docker compose exec alertmanager sh -c 'wget -qO- http://127.0.0.1:9093/-/healthy'
```

## 5xx

Fired by `HarborHigh5xxRate` when a single `path` serves >5% 5xx for 10m
(feeds off `harbor_http_responses_5xx_total{path}` / `harbor_http_requests_total{path}`).
Cross-route version: `HarborErrorBudgetBurning` when the overall 5xx share
exceeds 2% over 1h for 30m.

Check the actual share per path first:

```powershell
docker compose exec prometheus sh -c 'wget -qO- "http://127.0.0.1:9090/api/v1/query?query=sum%20by(path)(rate(harbor_http_responses_5xx_total%7Bpath%3D~%22.%2B%22%7D%5B5m%5D))%20/%20on(path)%20sum%20by(path)(rate(harbor_http_requests_total%7Bpath%3D~%22.%2B%22%7D%5B5m%5D))"'
```

What the 5xx counter increments on (see `backend/src/metrics.rs`): any response
whose HTTP status >= 500 after the handler ran, plus errors surfaced through
`axum`'s rejection path labeled by route `path`.

Walk-down, cheapest first:

1. **Is it one path or all?** Look at the alert's `path` label. One specific
   route (e.g. `/api/mail/search`) points at that handler (SQLx query, Stalwart
   round trip — see #latency). All paths simultaneously -> shared dependency:
2. **api logs** — look for a panic backtrace or a steady stream of real 500s:
   ```powershell
   docker compose logs api --since 30m | Select-String -Pattern "5xx|ERROR|panic"
   ```
3. **DB pool** — a locked table / leaking query turns lookups into 500s.
   Check `harbor_db_pool_connections{state="idle"}` (see #db). A pool of zero
   idle connections with requests still flowing is the classic cause of both
   this and `HarborDbPoolExhausted`.
4. **Test email round trips** — if it's mail routes, check SMTP delivery
   outcomes (see #rate-limited) and Stalwart via `docker compose logs mail`.

Escalation: if the 5xx share is systemic (>50% of all paths) or the DB pool is
starved, this is a do-not-deploy condition — see #db and the `/api/health` gate.

## latency

Fired by `HarborSlowRoute` when the 95th percentile of
`harbor_http_request_duration_seconds_bucket{path}` exceeds 2s for 10m.

Get the per-path p95:

```powershell
docker compose exec prometheus sh -c 'wget -qO- "http://127.0.0.1:9090/api/v1/query?query=histogram_quantile(0.95,%20rate(harbor_http_request_duration_seconds_bucket%7Bpath%3D~%22.%2B%22%7D%5B5m%5D))"'
```

NOTE (the reason a test file exists for this): the rule must NOT aggregate away
`le` — any `sum by (...)` that drops the label breaks the histogram boundaries
and `histogram_quantile` silently returns nothing (a dead rule). If the query
above returns no/empty series for a path that clearly has traffic, that's the
bug, not the latency.

Common causes for mail routes (p95 > 2s):

1. **SQLx pool pressure** — all connections busy (see #db); queries queue
   behind the pool instead of running.
2. **Stalwart/IMAP-SMTP round trips** — slow Stalwart puts latency on send/
   search paths that call out. `docker compose logs mail`.
3. **DNS** — a slow MX/PTR lookup during message delivery.
4. **Backup / batch job colliding** with interactive requests (check cron +
   `deploy/harbor-backup.ps1` runtime).

## rate-limited

Fired by `HarborSmtpRateLimited` when `harbor_smtp_sends_total{result="rate_limited"}`
exceeds 0.02/s for 15m — users are hitting Stalwart send caps (warmup ramp or
plan caps).

Delivery outcome metrics (see `backend/src/metrics.rs`):
`harbor_smtp_sends_total{result=ok|failed|rate_limited|suppressed}`.

Check who/which mailbox is being throttled and the current send rate:

```powershell
docker compose exec prometheus sh -c 'wget -qO- "http://127.0.0.1:9090/api/v1/query?query=sum%20by(result)(rate(harbor_smtp_sends_total%5B15m%5D))"'
```

Walk-down:

1. **Warmup ramp** — a brand-new mailbox under rate limits. Check
   `docker compose logs mail` for Stalwart `too many` / backoff lines.
2. **Plan caps** — a user legitimately at their send ceiling. Confirm against
   the Stripe plan + per-mailbox quota in the Admin surface.
3. **Rate-limiting config** — Stalwart's `max-rate` per sender. Tune
   `deploy/stalwart-bootstrap.ps1` / Stalwart admin if the cap is too low for a
   paid plan. Intentional throttling (backoff) is healthy; sustained hit for
   >15m is a config or plan issue.

Escalation: if `result="failed"` is ALSO climbing at the same time, delivery is
being dropped, not just delayed — treat as #5xx-adjacent and check the bounce
path + suppression list.

## email-suppressed

Fired by `HarborSmtpSuppressed` when
`harbor_smtp_sends_total{result="suppressed"}` exceeds the threshold for 15m —
addresses are being suppressed (bounce / FBL / blocklist).

This ALWAYS means a recipient repeatedly rejected our mail — dig into *why* the
address is bouncing before re-enabling it:

```powershell
docker compose exec prometheus sh -c 'wget -qO- "http://127.0.0.1:9090/api/v1/query?query=sum%20by(result)(rate(harbor_smtp_sends_total%7Bresult%3D%22suppressed%22%7D%5B15m%5D))"'
```

1. **Is it one address or a domain?** One address that hard-bounced (permanent
   reject, 550) — remove from the suppression list after the user fixes the
   mailbox. An entire domain suppressed = AppRiver/Spamhaus-level problem with
   our sending IP or DKIM — stop sending, fix reputation first.
2. **DSN ingests** — check `docker compose logs mail` + Stalwart DSN handling;
   a silent DSN ingest bug makes real bounces look like suppressions.

## db

Fired by `HarborDbPoolExhausted` (zero idle `harbor_db_pool_connections` for
5m) and `HarborDbPoolGone` (absent metric for 5m).

Check the pool from api's own metrics endpoint:

```powershell
docker compose exec api wget -qO- http://127.0.0.1:8080/api/metrics | Select-String "harbor_db_pool_connections"
```

1. **Zero idle + requests flowing** — leak: an un-committed transaction, a
   query that never releases its connection, or a table locked by a long
   write. Look at `backend/src` for SQLx queries run inside a transaction but
   never committed, and `docker compose logs api` for `pool timed out`.
2. **Metric absent entirely** (`HarborDbPoolGone`) — the `/api/metrics`
   endpoint is unreachable or the api is down. Check `docker compose ps api`
   + `/api/health`. This is the api-dead signal.
3. **Pool too small** — `HARBOR_DB_MAX_CONNECTIONS` in compose config vs.
   concurrency. Right-size, then it self-clears.

DB pool is not "just a performance knob": a dead SQLx pool means lookups/inserts
queue forever and every route that touches the DB starts 500ing — see #5xx.
It is a code-level do-not-deploy condition (LAUNCH L190 + WS6.5 `/api/health`).
