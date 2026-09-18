-- WS2.3: saved compose sessions. Bodies live in Postgres because the Stalwart
-- JMAP store for this deployment has object creation disabled; drafts are
-- materialized to MIME at send time and destroyed on success.
-- Recipient lists and attachments are JSONB; address entries look like
-- {"name": "Alice", "email": "alice@example.com"}.

CREATE TABLE IF NOT EXISTS mail_drafts (
  id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  to_list      JSONB NOT NULL DEFAULT '[]'::jsonb,
  cc_list      JSONB NOT NULL DEFAULT '[]'::jsonb,
  bcc_list     JSONB NOT NULL DEFAULT '[]'::jsonb,
  subject      TEXT NOT NULL DEFAULT '',
  body_text    TEXT NOT NULL DEFAULT '',
  body_html    TEXT NOT NULL DEFAULT '',
  attachments  JSONB NOT NULL DEFAULT '[]'::jsonb,
  in_reply_to  TEXT NOT NULL DEFAULT '',
  references_list JSONB NOT NULL DEFAULT '[]'::jsonb,
  created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mail_drafts_user_updated_idx
  ON mail_drafts(user_id, updated_at DESC);