-- Upgrade 18: launch hardening.
--
-- Rate-limit state is intentionally durable/shared so auth and public abuse
-- controls cannot be bypassed by switching between API replicas. Keys are
-- application-hashed before storage; raw emails/IP combinations are never
-- persisted in this table.

CREATE TABLE IF NOT EXISTS request_rate_limits (
  key_hash           BYTEA PRIMARY KEY,
  window_started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  count              INTEGER NOT NULL DEFAULT 0 CHECK (count >= 0),
  lock_until         TIMESTAMPTZ,
  updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS request_rate_limits_updated_idx
  ON request_rate_limits(updated_at);
