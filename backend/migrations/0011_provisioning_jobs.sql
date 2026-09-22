-- Upgrade 03: durable PostgreSQL <-> Stalwart provisioning/reconciliation.
-- Provider mutations are represented as jobs so process restarts or temporary
-- Stalwart outages cannot permanently strand application state.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS mail_sync_status TEXT NOT NULL DEFAULT 'pending'
    CHECK (mail_sync_status IN ('pending','ready','retrying','error')),
  ADD COLUMN IF NOT EXISTS mail_sync_error TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS mail_synced_at TIMESTAMPTZ;

UPDATE users
SET mail_sync_status = CASE
  WHEN COALESCE(mail_account_id, '') <> '' THEN 'ready'
  ELSE 'pending'
END
WHERE mail_sync_status = 'pending';

CREATE TABLE IF NOT EXISTS provisioning_jobs (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id           UUID REFERENCES users(id) ON DELETE SET NULL,
  operation         TEXT NOT NULL CHECK (operation IN ('ensure_mailbox','set_quota','delete_mailbox')),
  target_email      CITEXT NOT NULL,
  account_id        TEXT,
  quota_bytes       BIGINT,
  secret_ciphertext BYTEA,
  status            TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','processing','retry','succeeded','dead')),
  attempts          INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  max_attempts      INTEGER NOT NULL DEFAULT 12 CHECK (max_attempts > 0),
  next_attempt_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
  locked_at         TIMESTAMPTZ,
  locked_by         TEXT,
  last_error        TEXT NOT NULL DEFAULT '',
  last_failure_transient BOOLEAN NOT NULL DEFAULT FALSE,
  dedupe_key        TEXT NOT NULL,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at      TIMESTAMPTZ
);

-- Only queued jobs deduplicate. A job in `processing` no longer occupies the
-- key, so a state change that races an in-flight worker creates a fresh job
-- instead of being overwritten and then incorrectly marked complete.
CREATE UNIQUE INDEX IF NOT EXISTS provisioning_jobs_queued_dedupe_idx
  ON provisioning_jobs(dedupe_key)
  WHERE status IN ('pending','retry');

CREATE INDEX IF NOT EXISTS provisioning_jobs_ready_idx
  ON provisioning_jobs(next_attempt_at, created_at)
  WHERE status IN ('pending','retry');

CREATE INDEX IF NOT EXISTS provisioning_jobs_user_idx
  ON provisioning_jobs(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS provisioning_jobs_processing_idx
  ON provisioning_jobs(locked_at)
  WHERE status = 'processing';
