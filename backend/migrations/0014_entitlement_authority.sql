-- Upgrade 06: one authoritative entitlement model.
--
-- `plans` owns the default limits for every tier. `users.quota_override_bytes`
-- is the only supported per-user storage exception; `users.quota_bytes` stays
-- as the materialized/effective value consumed by reconciliation and admin
-- reporting so provider synchronization remains cheap and deterministic.

ALTER TABLE plans
  ADD COLUMN IF NOT EXISTS feature_flags JSONB NOT NULL DEFAULT '{}'::jsonb;

-- Current server-backed product features are intentionally enabled on the
-- existing plans. Later upgrades may differentiate them per plan without
-- introducing a second source of truth in application code.
UPDATE plans
SET feature_flags = jsonb_build_object(
  'mail', true,
  'attachments', true,
  'scheduled_send', true,
  'read_receipts', true,
  'contacts', true,
  'calendar', true
)
WHERE feature_flags = '{}'::jsonb;

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS quota_override_bytes BIGINT
  CHECK (quota_override_bytes IS NULL OR quota_override_bytes >= 1048576);

-- Preserve any already-customized quota as an explicit override. Rows already
-- matching their plan remain plan-managed. This makes the migration safe for
-- live installations while ending the old ambiguous "quota_bytes may mean
-- either plan or override" behavior from this point forward.
UPDATE users u
SET quota_override_bytes = CASE
    WHEN u.quota_bytes IS DISTINCT FROM p.mailbox_bytes THEN u.quota_bytes
    ELSE NULL
  END
FROM plans p
WHERE p.code = u.plan
  AND u.quota_override_bytes IS NULL;

-- Materialize the effective value from the authoritative sources.
UPDATE users u
SET quota_bytes = COALESCE(u.quota_override_bytes, p.mailbox_bytes),
    updated_at = now()
FROM plans p
WHERE p.code = u.plan
  AND u.quota_bytes IS DISTINCT FROM COALESCE(u.quota_override_bytes, p.mailbox_bytes);

-- Plan codes are admin-extensible. The old CHECK constraint only allowed the
-- original three hard-coded codes, which made a newly-created plan impossible
-- to activate. Replace it with a real relational constraint.
ALTER TABLE users DROP CONSTRAINT IF EXISTS users_plan_check;
ALTER TABLE users DROP CONSTRAINT IF EXISTS users_plan_fk;
ALTER TABLE users
  ADD CONSTRAINT users_plan_fk
  FOREIGN KEY (plan) REFERENCES plans(code)
  ON UPDATE CASCADE ON DELETE RESTRICT;

CREATE INDEX IF NOT EXISTS users_plan_quota_override_idx
  ON users(plan, quota_override_bytes);
