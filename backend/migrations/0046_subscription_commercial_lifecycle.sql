-- Upgrade 46: immutable commercial plan versions, scheduled renewal changes,
-- cancellation-at-renewal, and post-suspension data-retention authority.
--
-- `plans` remains the editable public catalog. Every commercial assignment is
-- bound to an immutable `plan_versions` row so later catalog edits cannot alter
-- an already-purchased subscription's storage, quotas, limits, or features.

CREATE TABLE IF NOT EXISTS plan_versions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  plan_code TEXT NOT NULL REFERENCES plans(code) ON DELETE RESTRICT,
  version_no INTEGER NOT NULL CHECK (version_no > 0),
  name TEXT NOT NULL,
  price_cents INTEGER NOT NULL CHECK (price_cents >= 0),
  extra_mailbox_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (extra_mailbox_price_cents >= 0),
  currency TEXT NOT NULL,
  interval TEXT NOT NULL,
  mailbox_bytes BIGINT NOT NULL CHECK (mailbox_bytes > 0),
  storage_pool_bytes BIGINT NOT NULL CHECK (storage_pool_bytes > 0),
  mailbox_limit INTEGER NOT NULL CHECK (mailbox_limit > 0),
  max_mailboxes INTEGER NOT NULL CHECK (max_mailboxes >= mailbox_limit),
  alias_limit_per_mailbox INTEGER CHECK (alias_limit_per_mailbox IS NULL OR alias_limit_per_mailbox > 0),
  domain_limit INTEGER NOT NULL CHECK (domain_limit > 0),
  organization_daily_send_limit INTEGER NOT NULL CHECK (organization_daily_send_limit > 0),
  max_attachment_bytes BIGINT NOT NULL CHECK (max_attachment_bytes > 0),
  max_recipients INTEGER NOT NULL CHECK (max_recipients > 0),
  daily_send_limit INTEGER NOT NULL CHECK (daily_send_limit > 0),
  seats INTEGER NOT NULL CHECK (seats > 0),
  features JSONB NOT NULL DEFAULT '[]'::jsonb,
  feature_flags JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(plan_code, version_no)
);

ALTER TABLE plans ADD COLUMN IF NOT EXISTS current_version_id UUID REFERENCES plan_versions(id) ON DELETE RESTRICT;

-- Seed one immutable version for every existing catalog plan.
INSERT INTO plan_versions(
  plan_code,version_no,name,price_cents,extra_mailbox_price_cents,currency,interval,
  mailbox_bytes,storage_pool_bytes,mailbox_limit,max_mailboxes,alias_limit_per_mailbox,
  domain_limit,organization_daily_send_limit,max_attachment_bytes,max_recipients,
  daily_send_limit,seats,features,feature_flags
)
SELECT p.code,1,p.name,p.price_cents,p.extra_mailbox_price_cents,p.currency,p.interval,
       p.mailbox_bytes,p.storage_pool_bytes,p.mailbox_limit,p.max_mailboxes,p.alias_limit_per_mailbox,
       p.domain_limit,p.organization_daily_send_limit,p.max_attachment_bytes,p.max_recipients,
       p.daily_send_limit,p.seats,p.features,p.feature_flags
FROM plans p
WHERE NOT EXISTS (SELECT 1 FROM plan_versions pv WHERE pv.plan_code=p.code);

UPDATE plans p
SET current_version_id = (
  SELECT pv.id
  FROM plan_versions pv
  WHERE pv.plan_code=p.code
  ORDER BY pv.version_no DESC
  LIMIT 1
)
WHERE p.current_version_id IS NULL;

ALTER TABLE organization_subscriptions
  ADD COLUMN IF NOT EXISTS plan_version_id UUID REFERENCES plan_versions(id) ON DELETE RESTRICT,
  ADD COLUMN IF NOT EXISTS retention_started_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS data_retention_until TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS purge_eligible_at TIMESTAMPTZ;

ALTER TABLE orders
  ADD COLUMN IF NOT EXISTS plan_version_id UUID REFERENCES plan_versions(id) ON DELETE RESTRICT;

UPDATE organization_subscriptions s
SET plan_version_id=p.current_version_id
FROM plans p
WHERE p.code=s.plan_code AND s.plan_version_id IS NULL;

UPDATE orders o
SET plan_version_id=p.current_version_id
FROM plans p
WHERE p.code=o.plan_code AND o.plan_version_id IS NULL;

CREATE INDEX IF NOT EXISTS organization_subscriptions_plan_version_idx ON organization_subscriptions(plan_version_id);
CREATE INDEX IF NOT EXISTS orders_plan_version_idx ON orders(plan_version_id);

CREATE TABLE IF NOT EXISTS subscription_scheduled_changes (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  change_type TEXT NOT NULL CHECK (change_type IN ('plan_change','cancel')),
  target_plan_code TEXT REFERENCES plans(code) ON DELETE RESTRICT,
  target_plan_version_id UUID REFERENCES plan_versions(id) ON DELETE RESTRICT,
  target_mailbox_count INTEGER CHECK (target_mailbox_count IS NULL OR target_mailbox_count > 0),
  effective_at TIMESTAMPTZ NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','ready_for_renewal','applied','cancelled','blocked')),
  requested_by UUID REFERENCES users(id) ON DELETE SET NULL,
  requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  cancelled_by UUID REFERENCES users(id) ON DELETE SET NULL,
  cancelled_at TIMESTAMPTZ,
  applied_at TIMESTAMPTZ,
  blocked_reason TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX IF NOT EXISTS subscription_scheduled_changes_one_pending_org_uq
  ON subscription_scheduled_changes(organization_id)
  WHERE status IN ('pending','ready_for_renewal','blocked');
CREATE INDEX IF NOT EXISTS subscription_scheduled_changes_due_idx
  ON subscription_scheduled_changes(status,effective_at);

ALTER TABLE billing_settings
  ADD COLUMN IF NOT EXISTS retention_days INTEGER NOT NULL DEFAULT 30 CHECK (retention_days BETWEEN 1 AND 365),
  ADD COLUMN IF NOT EXISTS renewal_reminder_days INTEGER NOT NULL DEFAULT 14 CHECK (renewal_reminder_days BETWEEN 1 AND 90),
  ADD COLUMN IF NOT EXISTS suspension_warning_days INTEGER NOT NULL DEFAULT 2 CHECK (suspension_warning_days BETWEEN 0 AND 30);

CREATE TABLE IF NOT EXISTS billing_lifecycle_outbox (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('renewal_reminder','past_due','suspension_warning','suspended','reactivated','retention_warning','cancelled')),
  event_key TEXT NOT NULL,
  recipient CITEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','sending','sent','retry','failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_error TEXT NOT NULL DEFAULT '',
  sent_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(organization_id,kind,event_key)
);
CREATE INDEX IF NOT EXISTS billing_lifecycle_outbox_pending_idx
  ON billing_lifecycle_outbox(status,next_attempt_at) WHERE status IN ('pending','retry');

ALTER TABLE subscription_assignment_history
  DROP CONSTRAINT IF EXISTS subscription_assignment_history_event_type_check;
ALTER TABLE subscription_assignment_history
  ADD CONSTRAINT subscription_assignment_history_event_type_check
  CHECK (event_type IN ('assignment','status','period','limits','scheduled_change','retention'));

-- Subscription capacity must use the purchased immutable version when present.
CREATE OR REPLACE FUNCTION cs_mail_enforce_seat_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint; org_id uuid;
BEGIN
  org_id := NEW.organization_id;
  IF NEW.status NOT IN ('active','invited','pending') THEN RETURN NEW; END IF;
  SELECT COALESCE(s.seat_limit_override,GREATEST(s.purchased_mailbox_count,COALESCE(pv.seats,p.seats))) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
    WHERE s.organization_id=org_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT
    (SELECT count(*) FROM organization_memberships om WHERE om.organization_id=org_id AND om.status IN ('active','invited')) +
    (SELECT count(*) FROM organization_invitations oi WHERE oi.organization_id=org_id AND oi.status='pending')
    INTO used_count;
  IF TG_TABLE_NAME='organization_memberships' THEN
    IF TG_OP='INSERT' THEN used_count:=used_count+1;
    ELSIF OLD.status NOT IN ('active','invited') THEN used_count:=used_count+1; END IF;
  ELSIF TG_TABLE_NAME='organization_invitations' THEN
    IF TG_OP='INSERT' THEN used_count:=used_count+1;
    ELSIF OLD.status<>'pending' THEN used_count:=used_count+1; END IF;
  END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization seat limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION cs_mail_enforce_mailbox_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint;
BEGIN
  IF NEW.deleted_at IS NOT NULL OR NEW.status='deleted' THEN RETURN NEW; END IF;
  SELECT COALESCE(s.mailbox_limit_override,GREATEST(s.purchased_mailbox_count,COALESCE(pv.mailbox_limit,p.mailbox_limit))) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
    WHERE s.organization_id=NEW.organization_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count FROM mailboxes m
    WHERE m.organization_id=NEW.organization_id AND m.deleted_at IS NULL AND m.status<>'deleted';
  IF TG_OP='INSERT' THEN used_count:=used_count+1; END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization mailbox limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION cs_mail_enforce_domain_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint;
BEGIN
  IF NEW.status='removing' THEN RETURN NEW; END IF;
  SELECT COALESCE(s.domain_limit_override,COALESCE(pv.domain_limit,p.domain_limit)) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
    WHERE s.organization_id=NEW.organization_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count FROM organization_domains d
    WHERE d.organization_id=NEW.organization_id AND d.status<>'removing';
  IF TG_OP='INSERT' THEN used_count:=used_count+1; END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization domain limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION cs_mail_enforce_storage_pool_allocation()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE pool_bytes BIGINT; allocated_other BIGINT;
BEGIN
  IF NEW.deleted_at IS NOT NULL THEN RETURN NEW; END IF;
  SELECT COALESCE(
           s.storage_pool_override_bytes,
           (COALESCE(pv.storage_pool_bytes,p.storage_pool_bytes)::bigint +
            COALESCE(pv.mailbox_bytes,p.mailbox_bytes)::bigint *
              GREATEST(s.purchased_mailbox_count - COALESCE(pv.mailbox_limit,p.mailbox_limit),0)::bigint)
         )
    INTO pool_bytes
  FROM organization_subscriptions s
  JOIN plans p ON p.code=s.plan_code
  LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
  WHERE s.organization_id=NEW.organization_id
  FOR UPDATE OF s;
  IF pool_bytes IS NULL THEN RETURN NEW; END IF;
  SELECT COALESCE(SUM(m.quota_bytes),0)::bigint INTO allocated_other
  FROM mailboxes m WHERE m.organization_id=NEW.organization_id AND m.deleted_at IS NULL AND m.id IS DISTINCT FROM NEW.id;
  IF allocated_other + NEW.quota_bytes > pool_bytes THEN
    RAISE EXCEPTION 'organization storage pool allocation exceeded' USING ERRCODE='23514';
  END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION cs_mail_enforce_alias_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE org_id uuid; address_kind text; lim integer; used_count bigint;
BEGIN
  SELECT ba.organization_id,ba.kind INTO org_id,address_kind FROM business_addresses ba
  WHERE ba.id=NEW.business_address_id AND ba.deleted_at IS NULL;
  IF org_id IS NULL OR address_kind <> 'alias' THEN RETURN NEW; END IF;
  SELECT COALESCE(pv.alias_limit_per_mailbox,p.alias_limit_per_mailbox) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    LEFT JOIN plan_versions pv ON pv.id=s.plan_version_id
    WHERE s.organization_id=org_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count FROM business_address_members bam
  JOIN business_addresses ba ON ba.id=bam.business_address_id
  WHERE bam.mailbox_id=NEW.mailbox_id AND ba.organization_id=org_id AND ba.kind='alias' AND ba.deleted_at IS NULL;
  IF TG_OP='INSERT' THEN used_count:=used_count+1;
  ELSIF OLD.mailbox_id IS DISTINCT FROM NEW.mailbox_id OR OLD.business_address_id IS DISTINCT FROM NEW.business_address_id THEN used_count:=used_count+1; END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'mailbox alias limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

COMMENT ON TABLE plan_versions IS 'Immutable commercial snapshots. Existing subscriptions and invoices remain bound to the purchased version when the editable catalog changes.';
COMMENT ON TABLE subscription_scheduled_changes IS 'One future-dated plan downgrade or cancellation request per business. Changes do not silently alter a paid active term.';
COMMENT ON COLUMN organization_subscriptions.data_retention_until IS 'When suspended/cancelled mailbox data becomes eligible for a separately controlled purge workflow.';
