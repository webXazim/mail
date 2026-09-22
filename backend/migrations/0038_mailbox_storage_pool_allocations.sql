-- Upgrade 32: organization storage pool with per-mailbox allocations.
--
-- `mailboxes.quota_bytes` remains the materialized provider quota. A NULL
-- `quota_override_bytes` means the mailbox follows the plan/business default;
-- a non-NULL value is an explicit allocation chosen by a business owner/admin.
-- The trigger below serializes allocation changes on the subscription row and
-- guarantees that live mailbox allocations never exceed the purchased pool.

ALTER TABLE mailboxes
  ADD COLUMN IF NOT EXISTS quota_override_bytes BIGINT
    CHECK (quota_override_bytes IS NULL OR quota_override_bytes >= 1048576),
  ADD COLUMN IF NOT EXISTS quota_updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS quota_updated_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS mailboxes_org_quota_override_idx
  ON mailboxes(organization_id, quota_override_bytes)
  WHERE deleted_at IS NULL;

COMMENT ON COLUMN mailboxes.quota_override_bytes IS
  'Business-admin mailbox storage allocation override. NULL follows the subscription default mailbox quota.';
COMMENT ON COLUMN mailboxes.quota_bytes IS
  'Materialized effective mailbox storage allocation enforced at the mail provider.';

CREATE OR REPLACE FUNCTION cs_mail_enforce_storage_pool_allocation()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  pool_bytes BIGINT;
  allocated_other BIGINT;
BEGIN
  -- A deleted mailbox no longer consumes allocation. During the deleting state
  -- it still consumes quota because the provider mailbox may still exist.
  IF NEW.deleted_at IS NOT NULL THEN
    RETURN NEW;
  END IF;

  SELECT COALESCE(
           s.storage_pool_override_bytes,
           (p.storage_pool_bytes::bigint +
            p.mailbox_bytes::bigint * GREATEST(s.purchased_mailbox_count - p.mailbox_limit, 0)::bigint)
         )
    INTO pool_bytes
  FROM organization_subscriptions s
  JOIN plans p ON p.code=s.plan_code
  WHERE s.organization_id=NEW.organization_id
  FOR UPDATE OF s;

  IF pool_bytes IS NULL THEN
    RETURN NEW;
  END IF;

  SELECT COALESCE(SUM(m.quota_bytes),0)::bigint
    INTO allocated_other
  FROM mailboxes m
  WHERE m.organization_id=NEW.organization_id
    AND m.deleted_at IS NULL
    AND m.id IS DISTINCT FROM NEW.id;

  IF allocated_other + NEW.quota_bytes > pool_bytes THEN
    RAISE EXCEPTION 'organization storage pool allocation exceeded'
      USING ERRCODE='23514';
  END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS mailboxes_storage_pool_allocation ON mailboxes;
CREATE TRIGGER mailboxes_storage_pool_allocation
BEFORE INSERT OR UPDATE OF quota_bytes,organization_id,deleted_at ON mailboxes
FOR EACH ROW EXECUTE FUNCTION cs_mail_enforce_storage_pool_allocation();

-- Fail the migration rather than carrying forward an already inconsistent
-- legacy allocation. Operators can rebalance the affected subscription before
-- retrying instead of discovering the issue during the next quota mutation.
DO $$
DECLARE
  bad_org UUID;
BEGIN
  SELECT s.organization_id INTO bad_org
  FROM organization_subscriptions s
  JOIN plans p ON p.code=s.plan_code
  LEFT JOIN LATERAL (
    SELECT COALESCE(SUM(m.quota_bytes),0)::bigint AS allocated
    FROM mailboxes m
    WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL
  ) a ON TRUE
  WHERE a.allocated > COALESCE(
    s.storage_pool_override_bytes,
    p.storage_pool_bytes::bigint +
    p.mailbox_bytes::bigint * GREATEST(s.purchased_mailbox_count-p.mailbox_limit,0)::bigint
  )
  LIMIT 1;

  IF bad_org IS NOT NULL THEN
    RAISE EXCEPTION 'existing mailbox allocations exceed storage pool for organization %', bad_org;
  END IF;
END $$;
