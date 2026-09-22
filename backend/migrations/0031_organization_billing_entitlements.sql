-- Upgrade 24: Business Billing, Seats & Entitlement Authority.
-- Commercial authority now belongs to organizations, never to an individual
-- login. Legacy users.plan/quota columns remain only as compatibility mirrors.

ALTER TABLE plans
  ADD COLUMN IF NOT EXISTS storage_pool_bytes BIGINT,
  ADD COLUMN IF NOT EXISTS mailbox_limit INTEGER,
  ADD COLUMN IF NOT EXISTS domain_limit INTEGER,
  ADD COLUMN IF NOT EXISTS organization_daily_send_limit INTEGER;

UPDATE plans SET
  storage_pool_bytes = COALESCE(storage_pool_bytes, mailbox_bytes),
  mailbox_limit = COALESCE(mailbox_limit, seats),
  domain_limit = COALESCE(domain_limit, CASE code WHEN 'solo' THEN 1 WHEN 'team' THEN 3 ELSE 10 END),
  organization_daily_send_limit = COALESCE(organization_daily_send_limit, daily_send_limit)
WHERE storage_pool_bytes IS NULL OR mailbox_limit IS NULL OR domain_limit IS NULL
   OR organization_daily_send_limit IS NULL;

ALTER TABLE plans
  ALTER COLUMN storage_pool_bytes SET NOT NULL,
  ALTER COLUMN mailbox_limit SET NOT NULL,
  ALTER COLUMN domain_limit SET NOT NULL,
  ALTER COLUMN organization_daily_send_limit SET NOT NULL;

ALTER TABLE plans
  ADD CONSTRAINT plans_storage_pool_positive CHECK (storage_pool_bytes > 0),
  ADD CONSTRAINT plans_mailbox_limit_positive CHECK (mailbox_limit > 0),
  ADD CONSTRAINT plans_domain_limit_positive CHECK (domain_limit > 0),
  ADD CONSTRAINT plans_org_send_nonnegative CHECK (organization_daily_send_limit >= 0);

CREATE TABLE IF NOT EXISTS organization_subscriptions (
  organization_id UUID PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
  plan_code TEXT NOT NULL REFERENCES plans(code),
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('trial','active','past_due','suspended','cancelled')),
  storage_pool_override_bytes BIGINT CHECK (storage_pool_override_bytes IS NULL OR storage_pool_override_bytes > 0),
  mailbox_quota_override_bytes BIGINT CHECK (mailbox_quota_override_bytes IS NULL OR mailbox_quota_override_bytes > 0),
  seat_limit_override INTEGER CHECK (seat_limit_override IS NULL OR seat_limit_override > 0),
  mailbox_limit_override INTEGER CHECK (mailbox_limit_override IS NULL OR mailbox_limit_override > 0),
  domain_limit_override INTEGER CHECK (domain_limit_override IS NULL OR domain_limit_override > 0),
  organization_daily_send_override INTEGER CHECK (organization_daily_send_override IS NULL OR organization_daily_send_override >= 0),
  current_period_start TIMESTAMPTZ NOT NULL DEFAULT now(),
  current_period_end TIMESTAMPTZ,
  cancelled_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS organization_subscriptions_plan_idx
  ON organization_subscriptions(plan_code,status);

-- Bootstrap each existing organization from its owner/creator's legacy plan.
INSERT INTO organization_subscriptions(organization_id, plan_code, status)
SELECT o.id,
       COALESCE((SELECT u.plan FROM organization_memberships om JOIN users u ON u.id=om.user_id
                 WHERE om.organization_id=o.id AND om.role='owner' ORDER BY om.joined_at LIMIT 1),
                (SELECT u.plan FROM users u WHERE u.id=o.created_by),
                'solo'),
       CASE WHEN o.status='active' THEN 'active' ELSE 'suspended' END
FROM organizations o
ON CONFLICT (organization_id) DO NOTHING;

ALTER TABLE orders ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE CASCADE;
UPDATE orders o SET organization_id = COALESCE(
  (SELECT u.active_organization_id FROM users u WHERE u.id=o.user_id),
  (SELECT om.organization_id FROM organization_memberships om WHERE om.user_id=o.user_id AND om.status='active'
   ORDER BY CASE om.role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, om.joined_at LIMIT 1)
) WHERE organization_id IS NULL;
CREATE INDEX IF NOT EXISTS orders_org_idx ON orders(organization_id, created_at DESC);


CREATE TABLE IF NOT EXISTS mailbox_send_counters (
  mailbox_id UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  day DATE NOT NULL,
  sent_count BIGINT NOT NULL DEFAULT 0 CHECK (sent_count >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (mailbox_id, day)
);

CREATE TABLE IF NOT EXISTS organization_send_counters (
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  day DATE NOT NULL,
  sent_count BIGINT NOT NULL DEFAULT 0 CHECK (sent_count >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, day)
);

-- Business-scoped usage cache. Provider reconciliation may refresh this; request
-- paths must still fail closed if a quota check cannot be established.
CREATE TABLE IF NOT EXISTS organization_usage (
  organization_id UUID PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
  storage_bytes BIGINT NOT NULL DEFAULT 0 CHECK (storage_bytes >= 0),
  mailbox_count INTEGER NOT NULL DEFAULT 0 CHECK (mailbox_count >= 0),
  domain_count INTEGER NOT NULL DEFAULT 0 CHECK (domain_count >= 0),
  seat_count INTEGER NOT NULL DEFAULT 0 CHECK (seat_count >= 0),
  refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO organization_usage(organization_id, mailbox_count, domain_count, seat_count)
SELECT o.id,
  (SELECT count(*)::int FROM mailboxes m WHERE m.organization_id=o.id AND m.deleted_at IS NULL AND m.status <> 'deleted'),
  (SELECT count(*)::int FROM organization_domains d WHERE d.organization_id=o.id AND d.status <> 'removing'),
  (SELECT count(*)::int FROM organization_memberships om WHERE om.organization_id=o.id AND om.status IN ('active','invited'))
FROM organizations o
ON CONFLICT (organization_id) DO UPDATE SET
  mailbox_count=EXCLUDED.mailbox_count, domain_count=EXCLUDED.domain_count,
  seat_count=EXCLUDED.seat_count, refreshed_at=now();

COMMENT ON COLUMN users.plan IS 'Deprecated compatibility mirror; organization_subscriptions is authoritative from Upgrade 24.';
COMMENT ON COLUMN users.quota_override_bytes IS 'Deprecated compatibility override; organization subscription/mailbox quota is authoritative from Upgrade 24.';
COMMENT ON COLUMN plans.mailbox_bytes IS 'Per-mailbox provider quota. Aggregate business storage is limited by storage_pool_bytes.';
COMMENT ON COLUMN plans.seats IS 'Included active/invited organization member seats.';

-- Database-level capacity guards serialize on the organization subscription so
-- concurrent API requests cannot overbook a business limit.
CREATE OR REPLACE FUNCTION cs_mail_enforce_seat_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint; org_id uuid;
BEGIN
  org_id := NEW.organization_id;
  IF NEW.status NOT IN ('active','invited','pending') THEN RETURN NEW; END IF;
  SELECT COALESCE(s.seat_limit_override,p.seats) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    WHERE s.organization_id=org_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT
    (SELECT count(*) FROM organization_memberships om WHERE om.organization_id=org_id AND om.status IN ('active','invited')) +
    (SELECT count(*) FROM organization_invitations oi WHERE oi.organization_id=org_id AND oi.status='pending')
    INTO used_count;
  IF TG_TABLE_NAME='organization_memberships' THEN
    IF TG_OP='INSERT' THEN
      used_count:=used_count+1;
    ELSIF OLD.status NOT IN ('active','invited') THEN
      used_count:=used_count+1;
    END IF;
  ELSIF TG_TABLE_NAME='organization_invitations' THEN
    IF TG_OP='INSERT' THEN
      used_count:=used_count+1;
    ELSIF OLD.status<>'pending' THEN
      used_count:=used_count+1;
    END IF;
  END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization seat limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS organization_memberships_capacity_guard ON organization_memberships;
CREATE TRIGGER organization_memberships_capacity_guard BEFORE INSERT OR UPDATE OF status ON organization_memberships
FOR EACH ROW EXECUTE FUNCTION cs_mail_enforce_seat_capacity();
DROP TRIGGER IF EXISTS organization_invitations_capacity_guard ON organization_invitations;
CREATE TRIGGER organization_invitations_capacity_guard BEFORE INSERT OR UPDATE OF status ON organization_invitations
FOR EACH ROW EXECUTE FUNCTION cs_mail_enforce_seat_capacity();

CREATE OR REPLACE FUNCTION cs_mail_enforce_mailbox_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint;
BEGIN
  IF NEW.deleted_at IS NOT NULL OR NEW.status='deleted' THEN RETURN NEW; END IF;
  SELECT COALESCE(s.mailbox_limit_override,p.mailbox_limit) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    WHERE s.organization_id=NEW.organization_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count FROM mailboxes m
    WHERE m.organization_id=NEW.organization_id AND m.deleted_at IS NULL AND m.status<>'deleted';
  IF TG_OP='INSERT' THEN used_count:=used_count+1; END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization mailbox limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS mailboxes_capacity_guard ON mailboxes;
CREATE TRIGGER mailboxes_capacity_guard BEFORE INSERT OR UPDATE OF status,deleted_at ON mailboxes
FOR EACH ROW EXECUTE FUNCTION cs_mail_enforce_mailbox_capacity();

CREATE OR REPLACE FUNCTION cs_mail_enforce_domain_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint;
BEGIN
  IF NEW.status='removing' THEN RETURN NEW; END IF;
  SELECT COALESCE(s.domain_limit_override,p.domain_limit) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    WHERE s.organization_id=NEW.organization_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count FROM organization_domains d
    WHERE d.organization_id=NEW.organization_id AND d.status<>'removing';
  IF TG_OP='INSERT' THEN used_count:=used_count+1; END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization domain limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS organization_domains_capacity_guard ON organization_domains;
CREATE TRIGGER organization_domains_capacity_guard BEFORE INSERT OR UPDATE OF status ON organization_domains
FOR EACH ROW EXECUTE FUNCTION cs_mail_enforce_domain_capacity();
