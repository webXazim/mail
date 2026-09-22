-- Upgrade 13: authoritative sender identities and provider-backed aliases.
--
-- Local aliases are durable desired state in PostgreSQL and are reconciled to
-- the mailbox provider. External aliases are represented by managed provider
-- mailing-list objects so delivery to an external destination remains server-
-- side and does not depend on the web application being online.

ALTER TABLE aliases
  ADD COLUMN IF NOT EXISTS enabled BOOLEAN NOT NULL DEFAULT TRUE,
  ADD COLUMN IF NOT EXISTS provider_object_id TEXT,
  ADD COLUMN IF NOT EXISTS sync_status TEXT NOT NULL DEFAULT 'pending',
  ADD COLUMN IF NOT EXISTS sync_error TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS sync_attempts INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  ADD COLUMN IF NOT EXISTS synced_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now();

ALTER TABLE aliases DROP CONSTRAINT IF EXISTS aliases_sync_status_check;
ALTER TABLE aliases ADD CONSTRAINT aliases_sync_status_check
  CHECK (sync_status IN ('pending','syncing','ready','error','deleted'));

ALTER TABLE aliases DROP CONSTRAINT IF EXISTS aliases_domain_source_key;
CREATE UNIQUE INDEX IF NOT EXISTS aliases_active_address_idx
  ON aliases (lower(domain), lower(source))
  WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS aliases_reconcile_idx
  ON aliases (next_attempt_at, created_at)
  WHERE sync_status IN ('pending','error');
CREATE INDEX IF NOT EXISTS aliases_dest_user_active_idx
  ON aliases (dest_user, created_at)
  WHERE dest_user IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS address_sync_state (
  user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  desired_revision BIGINT NOT NULL DEFAULT 0,
  applied_revision BIGINT NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','syncing','ready','error')),
  last_error TEXT NOT NULL DEFAULT '',
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  synced_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS address_sync_state_reconcile_idx
  ON address_sync_state(next_attempt_at, updated_at)
  WHERE status IN ('pending','error');

INSERT INTO address_sync_state(user_id, desired_revision, applied_revision, status)
SELECT DISTINCT dest_user, 1, 0, 'pending'
FROM aliases
WHERE dest_user IS NOT NULL
ON CONFLICT(user_id) DO NOTHING;

ALTER TABLE sender_identities
  ADD COLUMN IF NOT EXISTS alias_id UUID REFERENCES aliases(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS verification_token_hash TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS verification_expires_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS verification_sent_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS verification_attempts INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS verified_at TIMESTAMPTZ;

CREATE UNIQUE INDEX IF NOT EXISTS sender_identities_alias_idx
  ON sender_identities(alias_id) WHERE alias_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS sender_identities_pending_verification_idx
  ON sender_identities(verification_expires_at)
  WHERE status = 'pending';

UPDATE sender_identities
SET verified_at = COALESCE(verified_at, created_at)
WHERE status = 'verified';

-- Existing aliases that terminate at a CS Mailer mailbox become immediately
-- usable sender identities. The provider reconciliation worker will make their
-- receive-side routing authoritative after migration.
INSERT INTO sender_identities
  (user_id, email, display_name, is_primary, status, source, alias_id, verified_at)
SELECT a.dest_user,
       (a.source || '@' || a.domain)::citext,
       COALESCE(NULLIF(u.display_name, ''), a.source),
       FALSE,
       'verified',
       'alias',
       a.id,
       now()
FROM aliases a
JOIN users u ON u.id = a.dest_user
WHERE a.dest_user IS NOT NULL AND a.deleted_at IS NULL
ON CONFLICT (user_id, email) DO UPDATE
SET source = 'alias',
    alias_id = EXCLUDED.alias_id,
    status = CASE WHEN sender_identities.status = 'disabled' THEN 'disabled' ELSE 'verified' END,
    verified_at = COALESCE(sender_identities.verified_at, now()),
    updated_at = now();
