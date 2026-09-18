-- WS5.3 send-rate accounting. One row per user/domain per UTC day, incremented
-- atomically at submit time so a mass-send spike is rejected before SMTP. The
-- day is bound from the API (UTC) to keep the window independent of DB TZ.
CREATE TABLE IF NOT EXISTS send_counters (
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  day     DATE NOT NULL,
  count   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (user_id, day)
);

CREATE TABLE IF NOT EXISTS send_domain_counters (
  domain TEXT NOT NULL,
  day    DATE NOT NULL,
  count  INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (domain, day)
);
