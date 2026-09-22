-- Upgrade 16: production Admin authority.
--
-- Product-owned admin policy is durable in PostgreSQL, while provider-native
-- domain, queue and filtering state is read/written through the Stalwart JMAP
-- management boundary. Audit rows become append-only and retain actor ids even
-- after a user is erased.


-- Upgrade the durable provider queue so account suspension/reactivation is
-- retried across provider outages just like quota/password changes.
ALTER TABLE provisioning_jobs DROP CONSTRAINT IF EXISTS provisioning_jobs_operation_check;
ALTER TABLE provisioning_jobs ADD CONSTRAINT provisioning_jobs_operation_check
  CHECK (operation IN ('ensure_mailbox','set_quota','set_credentials','set_access','delete_mailbox'));

CREATE TABLE IF NOT EXISTS admin_mail_policy (
  singleton              BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  spam_threshold         DOUBLE PRECISION NOT NULL DEFAULT 5.0 CHECK (spam_threshold >= -100 AND spam_threshold <= 100),
  retention_days         INTEGER NOT NULL DEFAULT 30 CHECK (retention_days >= 0 AND retention_days <= 3650),
  trash_auto_purge       BOOLEAN NOT NULL DEFAULT TRUE,
  updated_by             UUID,
  updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO admin_mail_policy(singleton) VALUES (TRUE) ON CONFLICT(singleton) DO NOTHING;

-- Preserve actor identity in immutable audit history even when the user row is
-- later erased. The human-readable email is snapshotted into actor_email,
-- while actor_id remains useful for correlation.
ALTER TABLE audit_log DROP CONSTRAINT IF EXISTS audit_log_actor_id_fkey;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS actor_email CITEXT;
ALTER TABLE audit_log ADD COLUMN IF NOT EXISTS event_hash TEXT;

UPDATE audit_log a
SET actor_email = COALESCE(u.email::text, 'system')
FROM users u
WHERE a.actor_id = u.id AND (a.actor_email IS NULL OR a.actor_email = '');
UPDATE audit_log SET actor_email = 'system' WHERE actor_email IS NULL OR actor_email = '';

UPDATE audit_log
SET event_hash = encode(
  digest(
    concat_ws('|', id::text, at::text, coalesce(actor_id::text, ''), actor_email::text, action, detail::text),
    'sha256'
  ),
  'hex'
)
WHERE event_hash IS NULL OR event_hash = '';

ALTER TABLE audit_log ALTER COLUMN actor_email SET NOT NULL;
ALTER TABLE audit_log ALTER COLUMN actor_email SET DEFAULT 'system';
ALTER TABLE audit_log ALTER COLUMN event_hash SET NOT NULL;

CREATE OR REPLACE FUNCTION audit_log_seal() RETURNS trigger AS $$
BEGIN
  IF NEW.actor_id IS NULL THEN
    NEW.actor_email := 'system';
  ELSE
    SELECT email::text INTO NEW.actor_email FROM users WHERE id = NEW.actor_id;
    NEW.actor_email := COALESCE(NULLIF(NEW.actor_email::text, ''), 'system');
  END IF;
  NEW.event_hash := encode(
    digest(
      concat_ws('|', NEW.id::text, NEW.at::text, coalesce(NEW.actor_id::text, ''), NEW.actor_email::text, NEW.action, NEW.detail::text),
      'sha256'
    ),
    'hex'
  );
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS audit_log_seal_insert ON audit_log;
CREATE TRIGGER audit_log_seal_insert
BEFORE INSERT ON audit_log
FOR EACH ROW EXECUTE FUNCTION audit_log_seal();

CREATE OR REPLACE FUNCTION audit_log_append_only() RETURNS trigger AS $$
BEGIN
  RAISE EXCEPTION 'audit_log is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS audit_log_no_update ON audit_log;
CREATE TRIGGER audit_log_no_update
BEFORE UPDATE OR DELETE ON audit_log
FOR EACH ROW EXECUTE FUNCTION audit_log_append_only();

CREATE INDEX IF NOT EXISTS audit_action_at_idx ON audit_log(action, at DESC);
CREATE INDEX IF NOT EXISTS mail_forwarding_admin_idx
  ON mail_forwarding(updated_at DESC)
  WHERE target_email <> '';
