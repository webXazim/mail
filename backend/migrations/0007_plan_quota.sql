-- WS2.7: plan-based quota. `quota_bytes` already existed on users (0001) but
-- nothing keyed a mailbox to a tier, so upload/send caps could not vary. The
-- plan drives the mailbox disk quota handed to Stalwart at provisioning time
-- and the per-message attachment caps the API enforces on send.
--
-- Existing rows default to `solo`; the legacy 50 GiB `quota_bytes` is left in
-- place so downgrading the plan is a one-column change, not a data migration.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS plan TEXT NOT NULL DEFAULT 'solo'
  CHECK (plan IN ('solo', 'team', 'business'));

-- Keep the mailbox quota in step with the plan unless an operator has set an
-- explicit override via HARBOR_MAIL_ACCOUNT_QUOTA_BYTES at provisioning time.
-- New signups get their quota from the plan; see `domain::quota`.
CREATE INDEX IF NOT EXISTS users_plan_idx ON users(plan);
