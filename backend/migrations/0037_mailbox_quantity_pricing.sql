-- Upgrade 31: mailbox-quantity pricing inspired by mainstream private-email hosting.
--
-- Plans remain the entitlement/catalog authority, but each plan now has a base
-- mailbox bundle plus an annual per-mailbox add-on price. Orders snapshot the
-- selected mailbox quantity and price components so later catalog edits never
-- rewrite an issued invoice. Organization subscriptions retain the purchased
-- mailbox quantity; effective storage, seat, and organization send limits scale
-- with that quantity unless an administrator has set an explicit override.

ALTER TABLE plans
  ADD COLUMN IF NOT EXISTS extra_mailbox_price_cents INTEGER NOT NULL DEFAULT 0 CHECK (extra_mailbox_price_cents >= 0),
  ADD COLUMN IF NOT EXISTS max_mailboxes INTEGER,
  ADD COLUMN IF NOT EXISTS alias_limit_per_mailbox INTEGER CHECK (alias_limit_per_mailbox IS NULL OR alias_limit_per_mailbox > 0);

UPDATE plans SET max_mailboxes = GREATEST(COALESCE(max_mailboxes, mailbox_limit), mailbox_limit)
WHERE max_mailboxes IS NULL;
ALTER TABLE plans ALTER COLUMN max_mailboxes SET NOT NULL;

-- Preserve the pre-upgrade included quantity while the catalog is rewritten so
-- existing subscriptions never lose mailbox capacity merely by applying this migration.
CREATE TEMP TABLE cs_mail_u31_plan_snapshot ON COMMIT DROP AS
SELECT code, mailbox_limit AS old_mailbox_limit FROM plans;

ALTER TABLE plans DROP CONSTRAINT IF EXISTS plans_max_mailboxes_valid;
ALTER TABLE plans ADD CONSTRAINT plans_max_mailboxes_valid CHECK (max_mailboxes >= mailbox_limit AND max_mailboxes <= 500);

-- CS Mail catalog: annual base bundle + additional mailbox price. Stable plan
-- codes are intentionally retained so existing references do not need a
-- destructive migration.
UPDATE plans SET
  name='CS Mail Start', price_cents=5900, extra_mailbox_price_cents=3500,
  currency='SAR', interval='year',
  mailbox_bytes=5368709120, storage_pool_bytes=5368709120,
  mailbox_limit=1, max_mailboxes=50, alias_limit_per_mailbox=10,
  domain_limit=1, organization_daily_send_limit=2000,
  max_attachment_bytes=26214400, max_recipients=50, daily_send_limit=2000,
  seats=1,
  features='["1 mailbox included","5 GB storage per mailbox","10 aliases per mailbox","1 custom domain","Webmail + IMAP/SMTP","Additional mailboxes available"]'::jsonb,
  sort_order=1, active=TRUE, updated_at=now()
WHERE code='solo';

UPDATE plans SET
  name='CS Mail Grow', price_cents=15900, extra_mailbox_price_cents=9900,
  currency='SAR', interval='year',
  mailbox_bytes=10737418240, storage_pool_bytes=32212254720,
  mailbox_limit=3, max_mailboxes=50, alias_limit_per_mailbox=50,
  domain_limit=3, organization_daily_send_limit=6000,
  max_attachment_bytes=52428800, max_recipients=50, daily_send_limit=2000,
  seats=3,
  features='["3 mailboxes included","10 GB storage per mailbox","50 aliases per mailbox","Up to 3 custom domains","Webmail + IMAP/SMTP","Additional mailboxes available"]'::jsonb,
  sort_order=2, active=TRUE, updated_at=now()
WHERE code='team';

UPDATE plans SET
  name='CS Mail Scale', price_cents=26900, extra_mailbox_price_cents=14900,
  currency='SAR', interval='year',
  mailbox_bytes=16106127360, storage_pool_bytes=80530636800,
  mailbox_limit=5, max_mailboxes=50, alias_limit_per_mailbox=NULL,
  domain_limit=5, organization_daily_send_limit=10000,
  max_attachment_bytes=104857600, max_recipients=50, daily_send_limit=2000,
  seats=5,
  features='["5 mailboxes included","15 GB storage per mailbox","Unlimited aliases per mailbox","Up to 5 custom domains","Webmail + IMAP/SMTP","Additional mailboxes available"]'::jsonb,
  sort_order=3, active=TRUE, updated_at=now()
WHERE code='business';

ALTER TABLE organization_subscriptions
  ADD COLUMN IF NOT EXISTS purchased_mailbox_count INTEGER CHECK (purchased_mailbox_count IS NULL OR purchased_mailbox_count > 0);

UPDATE organization_subscriptions s
SET purchased_mailbox_count = GREATEST(
  COALESCE(s.purchased_mailbox_count, 0),
  COALESCE(s.mailbox_limit_override, 0),
  COALESCE((SELECT p.mailbox_limit FROM plans p WHERE p.code=s.plan_code), 1),
  COALESCE((SELECT old.old_mailbox_limit FROM cs_mail_u31_plan_snapshot old WHERE old.code=s.plan_code), 1),
  COALESCE((SELECT count(*)::int FROM mailboxes m WHERE m.organization_id=s.organization_id AND m.deleted_at IS NULL AND m.status <> 'deleted'), 0),
  COALESCE((SELECT count(*)::int FROM organization_memberships om WHERE om.organization_id=s.organization_id AND om.status IN ('active','invited')), 0)
);
ALTER TABLE organization_subscriptions ALTER COLUMN purchased_mailbox_count SET NOT NULL;

ALTER TABLE orders
  ADD COLUMN IF NOT EXISTS mailbox_count INTEGER CHECK (mailbox_count IS NULL OR mailbox_count > 0),
  ADD COLUMN IF NOT EXISTS included_mailbox_count INTEGER CHECK (included_mailbox_count IS NULL OR included_mailbox_count > 0),
  ADD COLUMN IF NOT EXISTS extra_mailbox_count INTEGER CHECK (extra_mailbox_count IS NULL OR extra_mailbox_count >= 0),
  ADD COLUMN IF NOT EXISTS extra_mailbox_unit_price_cents INTEGER CHECK (extra_mailbox_unit_price_cents IS NULL OR extra_mailbox_unit_price_cents >= 0),
  ADD COLUMN IF NOT EXISTS base_price_cents INTEGER CHECK (base_price_cents IS NULL OR base_price_cents >= 0);

-- Historical invoices predate add-on mailbox pricing. Backfill them as a
-- single base bundle so the new invoice renderer never invents retroactive
-- add-on charges from today's catalog prices.
UPDATE orders o SET
  included_mailbox_count = COALESCE(o.included_mailbox_count, GREATEST(o.seats,1)),
  mailbox_count = COALESCE(o.mailbox_count, GREATEST(o.seats,1)),
  extra_mailbox_count = COALESCE(o.extra_mailbox_count, 0),
  extra_mailbox_unit_price_cents = COALESCE(o.extra_mailbox_unit_price_cents, 0),
  base_price_cents = COALESCE(o.base_price_cents, COALESCE(o.subtotal_cents,o.amount_cents))
WHERE o.mailbox_count IS NULL OR o.included_mailbox_count IS NULL OR o.extra_mailbox_count IS NULL
   OR o.extra_mailbox_unit_price_cents IS NULL OR o.base_price_cents IS NULL;

ALTER TABLE orders ALTER COLUMN mailbox_count SET NOT NULL;
ALTER TABLE orders ALTER COLUMN included_mailbox_count SET NOT NULL;
ALTER TABLE orders ALTER COLUMN extra_mailbox_count SET NOT NULL;
ALTER TABLE orders ALTER COLUMN extra_mailbox_unit_price_cents SET NOT NULL;
ALTER TABLE orders ALTER COLUMN base_price_cents SET NOT NULL;

CREATE INDEX IF NOT EXISTS orders_org_plan_mailboxes_idx ON orders(organization_id, plan_code, mailbox_count, created_at DESC);

COMMENT ON COLUMN plans.price_cents IS 'Base subscription price for the included mailbox bundle.';
COMMENT ON COLUMN plans.extra_mailbox_price_cents IS 'Price for each mailbox above mailbox_limit for one plan interval.';
COMMENT ON COLUMN plans.mailbox_limit IS 'Mailboxes included in the base plan price.';
COMMENT ON COLUMN plans.max_mailboxes IS 'Maximum mailbox quantity selectable through self-service ordering.';
COMMENT ON COLUMN plans.alias_limit_per_mailbox IS 'Per-mailbox business alias limit; NULL means unlimited.';
COMMENT ON COLUMN organization_subscriptions.purchased_mailbox_count IS 'Total mailbox quantity purchased for the current subscription.';
COMMENT ON COLUMN orders.mailbox_count IS 'Invoice-snapshotted total mailbox quantity selected by the customer.';
COMMENT ON COLUMN orders.base_price_cents IS 'Invoice-snapshotted base plan price before mailbox add-ons and tax.';

ALTER TABLE organization_subscriptions ALTER COLUMN purchased_mailbox_count SET DEFAULT 1;

-- Capacity guards must honor purchased mailbox quantity, not only the base
-- included quantity in the plan catalog.
CREATE OR REPLACE FUNCTION cs_mail_enforce_seat_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint; org_id uuid;
BEGIN
  org_id := NEW.organization_id;
  IF NEW.status NOT IN ('active','invited','pending') THEN RETURN NEW; END IF;
  SELECT COALESCE(s.seat_limit_override,GREATEST(s.purchased_mailbox_count,p.seats)) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    WHERE s.organization_id=org_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT
    (SELECT count(*) FROM organization_memberships om WHERE om.organization_id=org_id AND om.status IN ('active','invited')) +
    (SELECT count(*) FROM organization_invitations oi WHERE oi.organization_id=org_id AND oi.status='pending')
    INTO used_count;
  IF TG_TABLE_NAME='organization_memberships' THEN
    IF TG_OP='INSERT' THEN used_count:=used_count+1;
    ELSIF OLD.status NOT IN ('active','invited') THEN used_count:=used_count+1;
    END IF;
  ELSIF TG_TABLE_NAME='organization_invitations' THEN
    IF TG_OP='INSERT' THEN used_count:=used_count+1;
    ELSIF OLD.status<>'pending' THEN used_count:=used_count+1;
    END IF;
  END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization seat limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

CREATE OR REPLACE FUNCTION cs_mail_enforce_mailbox_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE lim integer; used_count bigint;
BEGIN
  IF NEW.deleted_at IS NOT NULL OR NEW.status='deleted' THEN RETURN NEW; END IF;
  SELECT COALESCE(s.mailbox_limit_override,GREATEST(s.purchased_mailbox_count,p.mailbox_limit)) INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    WHERE s.organization_id=NEW.organization_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count FROM mailboxes m
    WHERE m.organization_id=NEW.organization_id AND m.deleted_at IS NULL AND m.status<>'deleted';
  IF TG_OP='INSERT' THEN used_count:=used_count+1; END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'organization mailbox limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

-- Alias allowances are commercial entitlements and need a database guard in
-- addition to the API preflight so concurrent requests cannot oversubscribe a
-- mailbox. Groups are intentionally not counted as aliases.
CREATE OR REPLACE FUNCTION cs_mail_enforce_alias_capacity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE org_id uuid; address_kind text; lim integer; used_count bigint;
BEGIN
  SELECT ba.organization_id,ba.kind INTO org_id,address_kind
    FROM business_addresses ba
    WHERE ba.id=NEW.business_address_id AND ba.deleted_at IS NULL;
  IF org_id IS NULL OR address_kind <> 'alias' THEN RETURN NEW; END IF;
  SELECT p.alias_limit_per_mailbox INTO lim
    FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
    WHERE s.organization_id=org_id FOR UPDATE OF s;
  IF lim IS NULL THEN RETURN NEW; END IF;
  SELECT count(*) INTO used_count
    FROM business_address_members bam
    JOIN business_addresses ba ON ba.id=bam.business_address_id
    WHERE bam.mailbox_id=NEW.mailbox_id AND ba.organization_id=org_id
      AND ba.kind='alias' AND ba.deleted_at IS NULL;
  IF TG_OP='INSERT' THEN
    used_count:=used_count+1;
  ELSIF OLD.mailbox_id IS DISTINCT FROM NEW.mailbox_id OR OLD.business_address_id IS DISTINCT FROM NEW.business_address_id THEN
    used_count:=used_count+1;
  END IF;
  IF used_count > lim THEN RAISE EXCEPTION 'mailbox alias limit reached' USING ERRCODE='23514'; END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS business_address_members_alias_capacity ON business_address_members;
CREATE TRIGGER business_address_members_alias_capacity
BEFORE INSERT OR UPDATE OF mailbox_id,business_address_id ON business_address_members
FOR EACH ROW EXECUTE FUNCTION cs_mail_enforce_alias_capacity();
