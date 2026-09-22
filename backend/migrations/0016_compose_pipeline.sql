-- Upgrade 09: production compose / draft / send pipeline.
--
-- Sender identities are application-authoritative. The primary mailbox identity
-- is seeded from users; additional verified addresses are added by later alias /
-- identity workflows. Drafts keep the selected identity. Send requests form an
-- idempotency ledger so browser/network retries never blindly submit twice.

CREATE TABLE IF NOT EXISTS sender_identities (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  email         CITEXT NOT NULL,
  display_name  TEXT NOT NULL DEFAULT '',
  reply_to      CITEXT,
  is_primary    BOOLEAN NOT NULL DEFAULT FALSE,
  status        TEXT NOT NULL DEFAULT 'verified'
                CHECK (status IN ('pending','verified','disabled')),
  source        TEXT NOT NULL DEFAULT 'primary'
                CHECK (source IN ('primary','alias','verified_external')),
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (user_id, email)
);

CREATE UNIQUE INDEX IF NOT EXISTS sender_identities_one_primary_idx
  ON sender_identities(user_id) WHERE is_primary;
CREATE INDEX IF NOT EXISTS sender_identities_user_status_idx
  ON sender_identities(user_id, status, is_primary DESC, created_at);

INSERT INTO sender_identities (user_id, email, display_name, is_primary, status, source)
SELECT id, email, display_name, TRUE, 'verified', 'primary'
FROM users
ON CONFLICT (user_id, email) DO UPDATE
SET is_primary = TRUE,
    status = 'verified',
    source = 'primary',
    display_name = CASE
      WHEN sender_identities.display_name = '' THEN EXCLUDED.display_name
      ELSE sender_identities.display_name
    END,
    updated_at = now();

ALTER TABLE mail_drafts
  ADD COLUMN IF NOT EXISTS identity_id UUID REFERENCES sender_identities(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS client_key UUID,
  ADD COLUMN IF NOT EXISTS send_key UUID;

CREATE UNIQUE INDEX IF NOT EXISTS mail_drafts_user_client_key_idx
  ON mail_drafts(user_id, client_key) WHERE client_key IS NOT NULL;

UPDATE mail_drafts SET send_key = gen_random_uuid() WHERE send_key IS NULL;
ALTER TABLE mail_drafts ALTER COLUMN send_key SET NOT NULL;

UPDATE mail_drafts d
SET identity_id = i.id
FROM sender_identities i
WHERE d.user_id = i.user_id
  AND i.is_primary = TRUE
  AND d.identity_id IS NULL;

CREATE TABLE IF NOT EXISTS mail_send_requests (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id           UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  idempotency_key   TEXT NOT NULL,
  request_hash      TEXT NOT NULL,
  identity_id       UUID REFERENCES sender_identities(id) ON DELETE SET NULL,
  draft_id          UUID REFERENCES mail_drafts(id) ON DELETE SET NULL,
  status            TEXT NOT NULL DEFAULT 'prepared'
                    CHECK (status IN ('prepared','submitting','sent','failed','uncertain')),
  message_id        TEXT NOT NULL,
  sent_id           TEXT NOT NULL DEFAULT '',
  recipients        JSONB NOT NULL DEFAULT '[]'::jsonb,
  budget_charged    BOOLEAN NOT NULL DEFAULT FALSE,
  attempt_count     INTEGER NOT NULL DEFAULT 0,
  last_error        TEXT NOT NULL DEFAULT '',
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  submitted_at      TIMESTAMPTZ,
  sent_at           TIMESTAMPTZ,
  UNIQUE (user_id, idempotency_key)
);

CREATE INDEX IF NOT EXISTS mail_send_requests_reconcile_idx
  ON mail_send_requests(status, updated_at)
  WHERE status IN ('submitting','uncertain','sent');
CREATE INDEX IF NOT EXISTS mail_send_requests_user_created_idx
  ON mail_send_requests(user_id, created_at DESC);
