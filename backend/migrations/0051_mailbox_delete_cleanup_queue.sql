-- Durable cleanup queue for mailbox-owned blobs/directories.
-- Provider deletion and user-facing mailbox removal must not be rolled back by
-- a later object-store cleanup failure. Cleanup jobs are idempotent and retried
-- independently after the mailbox row has been hard-deleted.
CREATE TABLE IF NOT EXISTS mailbox_cleanup_jobs (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  mailbox_id      UUID NOT NULL,
  storage_backend TEXT NOT NULL CHECK (storage_backend IN ('local','r2','mailbox_dirs')),
  storage_key     TEXT NOT NULL,
  status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','processing','retry','dead')),
  attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  max_attempts    INTEGER NOT NULL DEFAULT 20 CHECK (max_attempts >= 1),
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_error      TEXT NOT NULL DEFAULT '',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  completed_at    TIMESTAMPTZ
);

CREATE UNIQUE INDEX IF NOT EXISTS mailbox_cleanup_jobs_object_unique_idx
  ON mailbox_cleanup_jobs(mailbox_id, storage_backend, storage_key);
CREATE INDEX IF NOT EXISTS mailbox_cleanup_jobs_ready_idx
  ON mailbox_cleanup_jobs(next_attempt_at, created_at)
  WHERE status IN ('pending','retry');
CREATE INDEX IF NOT EXISTS mailbox_cleanup_jobs_mailbox_idx
  ON mailbox_cleanup_jobs(mailbox_id, created_at DESC);
