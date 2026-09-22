-- Upgrade 04: account security, durable session metadata and credential sync.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active'
    CHECK (status IN ('active','suspended'));

ALTER TABLE sessions
  ADD COLUMN IF NOT EXISTS last_used_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS rotated_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS revoked_at TIMESTAMPTZ;

-- Access/refresh JWTs minted before this migration did not carry the stable
-- session id claim introduced by Upgrade 04. Revoke them deliberately so the
-- first post-upgrade request performs a clean login instead of leaving an
-- ambiguous half-compatible session.
UPDATE sessions SET revoked_at = COALESCE(revoked_at, now());

CREATE INDEX IF NOT EXISTS sessions_user_active_idx
  ON sessions(user_id, last_used_at DESC)
  WHERE revoked_at IS NULL;

-- Used refresh tokens are retained through the configured replay-detection
-- window so reuse can revoke only the compromised device/session rather than
-- every device belonging to the account.
CREATE TABLE IF NOT EXISTS session_refresh_history (
  token_hash TEXT PRIMARY KEY,
  session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  used_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS session_refresh_history_user_idx
  ON session_refresh_history(user_id, used_at DESC);

-- Upgrade 03 constrained the queue before credential synchronization existed.
ALTER TABLE provisioning_jobs DROP CONSTRAINT IF EXISTS provisioning_jobs_operation_check;
ALTER TABLE provisioning_jobs
  ADD CONSTRAINT provisioning_jobs_operation_check
  CHECK (operation IN ('ensure_mailbox','set_quota','set_credentials','delete_mailbox'));
