-- WS3.3: scheduled sends and read-receipt bookkeeping.
--
-- scheduled_sends holds a full compose payload (the same shape POST /api/send
-- accepts) plus a delivery time. A background worker in main.rs picks up rows
-- whose send_at has passed and delivers them via the shared send core. Status
-- moves pending -> sent | failed; the row is kept for history until the client
-- cancels it (DELETE) which removes the row outright.
--
-- read_receipts / receipt_requests are per-user logs. The mail_id is the
-- client-side message id (JMAP email id or local id), so it is TEXT not UUID.

CREATE TABLE IF NOT EXISTS scheduled_sends (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  send_at     TIMESTAMPTZ NOT NULL,
  compose     JSONB NOT NULL DEFAULT '{}'::jsonb,
  status      TEXT NOT NULL DEFAULT 'pending',
  sent_id     TEXT NOT NULL DEFAULT '',
  error       TEXT NOT NULL DEFAULT '',
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS scheduled_sends_due_idx
  ON scheduled_sends(send_at) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS scheduled_sends_user_idx
  ON scheduled_sends(user_id, send_at);

CREATE TABLE IF NOT EXISTS read_receipts (
  id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  mail_id    TEXT NOT NULL,
  sender     TEXT NOT NULL DEFAULT '',
  email      TEXT NOT NULL DEFAULT '',
  subject    TEXT NOT NULL DEFAULT '',
  at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS read_receipts_user_mail_idx
  ON read_receipts(user_id, mail_id);

CREATE TABLE IF NOT EXISTS receipt_requests (
  id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  mail_id    TEXT NOT NULL,
  recipient  TEXT NOT NULL DEFAULT '',
  at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS receipt_requests_user_mail_idx
  ON receipt_requests(user_id, mail_id);
