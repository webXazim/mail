-- WS7.2 deliverability: addresses that must never receive mail again. Records
-- hard bounces (5.x), spam complaints and one-click unsubscribes. Enforced at
-- send time, so a listed address is dropped before SMTP.
CREATE TABLE IF NOT EXISTS suppressed_addresses (
  email      CITEXT PRIMARY KEY,
  reason     TEXT NOT NULL,
  source     TEXT NOT NULL DEFAULT 'bounce',
  detail     TEXT NOT NULL DEFAULT '',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS suppressed_source_idx ON suppressed_addresses(source);
