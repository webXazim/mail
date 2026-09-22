-- Upgrade 07: durable staged attachment storage.
--
-- Attachment bytes live on the API's persistent attachment volume. PostgreSQL
-- stores ownership, integrity/lifecycle metadata, and references from drafts /
-- scheduled sends. This removes base64 payloads from compose JSON and makes
-- cleanup deterministic.

CREATE TABLE IF NOT EXISTS staged_attachments (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id        UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  filename       TEXT NOT NULL,
  content_type   TEXT NOT NULL,
  byte_size      BIGINT NOT NULL DEFAULT 0 CHECK (byte_size >= 0),
  reserved_bytes BIGINT NOT NULL DEFAULT 0 CHECK (reserved_bytes >= 0),
  sha256_hex     TEXT NOT NULL DEFAULT '',
  storage_key    TEXT NOT NULL UNIQUE,
  status         TEXT NOT NULL DEFAULT 'uploading'
                 CHECK (status IN ('uploading', 'ready', 'consumed')),
  expires_at     TIMESTAMPTZ NOT NULL DEFAULT (now() + interval '24 hours'),
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS staged_attachments_user_status_idx
  ON staged_attachments(user_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS staged_attachments_expiry_idx
  ON staged_attachments(expires_at);

CREATE TABLE IF NOT EXISTS attachment_refs (
  attachment_id UUID NOT NULL REFERENCES staged_attachments(id) ON DELETE CASCADE,
  user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  owner_type    TEXT NOT NULL CHECK (owner_type IN ('draft', 'scheduled')),
  owner_id      UUID NOT NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (attachment_id, owner_type, owner_id)
);

CREATE INDEX IF NOT EXISTS attachment_refs_owner_idx
  ON attachment_refs(user_id, owner_type, owner_id);
CREATE INDEX IF NOT EXISTS attachment_refs_attachment_idx
  ON attachment_refs(attachment_id);
