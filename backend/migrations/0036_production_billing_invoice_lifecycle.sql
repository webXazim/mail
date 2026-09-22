-- Upgrade 29: production pricing, invoice snapshots, manual-payment lifecycle.
--
-- Orders now receive immutable invoice numbers and totals when created. During
-- the current testing phase, CS_MAIL_BILLING_INSTANT_ACTIVATION may activate
-- the selected plan immediately while the invoice remains due for manual
-- payment/review. Disabling that flag restores payment-gated activation.

ALTER TABLE billing_settings
  ADD COLUMN IF NOT EXISTS seller_legal_name TEXT NOT NULL DEFAULT 'CrescentSphere',
  ADD COLUMN IF NOT EXISTS seller_email CITEXT NOT NULL DEFAULT 'billing@crescentsphere.com',
  ADD COLUMN IF NOT EXISTS seller_cr_number TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS seller_vat_number TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS seller_address TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS tax_rate_bps INTEGER NOT NULL DEFAULT 1500 CHECK (tax_rate_bps BETWEEN 0 AND 10000),
  ADD COLUMN IF NOT EXISTS invoice_due_days INTEGER NOT NULL DEFAULT 7 CHECK (invoice_due_days BETWEEN 0 AND 90),
  ADD COLUMN IF NOT EXISTS grace_days INTEGER NOT NULL DEFAULT 7 CHECK (grace_days BETWEEN 0 AND 90);

-- Public launch catalog. Existing stable plan codes remain unchanged so
-- subscriptions/references do not need a destructive migration.
UPDATE plans SET
  name='CS Mail Starter', price_cents=2500, currency='SAR', interval='month',
  mailbox_bytes=16106127360, storage_pool_bytes=16106127360,
  mailbox_limit=1, domain_limit=1, organization_daily_send_limit=500,
  max_attachment_bytes=26214400, max_recipients=30, daily_send_limit=500,
  seats=1,
  features='["1 business mailbox","1 custom domain","15 GB pooled storage","Web inbox + IMAP/SMTP","Aliases and scheduled send"]'::jsonb,
  sort_order=1, active=TRUE, updated_at=now()
WHERE code='solo';

UPDATE plans SET
  name='CS Mail Team', price_cents=7900, currency='SAR', interval='month',
  mailbox_bytes=26843545600, storage_pool_bytes=80530636800,
  mailbox_limit=5, domain_limit=2, organization_daily_send_limit=3000,
  max_attachment_bytes=52428800, max_recipients=60, daily_send_limit=1000,
  seats=5,
  features='["5 business mailboxes","2 custom domains","75 GB pooled storage","Aliases and groups","Team administration","Mailbox migration"]'::jsonb,
  sort_order=2, active=TRUE, updated_at=now()
WHERE code='team';

UPDATE plans SET
  name='CS Mail Business', price_cents=19900, currency='SAR', interval='month',
  mailbox_bytes=53687091200, storage_pool_bytes=322122547200,
  mailbox_limit=15, domain_limit=5, organization_daily_send_limit=10000,
  max_attachment_bytes=104857600, max_recipients=100, daily_send_limit=2000,
  seats=15,
  features='["15 business mailboxes","5 custom domains","300 GB pooled storage","Advanced administration and audit","Priority support","Mailbox migration"]'::jsonb,
  sort_order=3, active=TRUE, updated_at=now()
WHERE code='business';

CREATE TABLE IF NOT EXISTS organization_billing_profiles (
  organization_id UUID PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
  legal_name TEXT NOT NULL DEFAULT '',
  billing_email CITEXT NOT NULL DEFAULT '',
  vat_number TEXT NOT NULL DEFAULT '',
  cr_number TEXT NOT NULL DEFAULT '',
  address_line1 TEXT NOT NULL DEFAULT '',
  address_line2 TEXT NOT NULL DEFAULT '',
  city TEXT NOT NULL DEFAULT '',
  postal_code TEXT NOT NULL DEFAULT '',
  country TEXT NOT NULL DEFAULT 'Saudi Arabia',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO organization_billing_profiles(organization_id, legal_name, billing_email)
SELECT o.id, o.name,
       COALESCE((SELECT u.email FROM organization_memberships om JOIN users u ON u.id=om.user_id
                 WHERE om.organization_id=o.id AND om.role='owner' AND om.status='active'
                 ORDER BY om.joined_at LIMIT 1), '')
FROM organizations o
ON CONFLICT (organization_id) DO NOTHING;

ALTER TABLE orders
  ADD COLUMN IF NOT EXISTS issued_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS due_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS invoice_status TEXT NOT NULL DEFAULT 'issued'
    CHECK (invoice_status IN ('issued','paid','void')),
  ADD COLUMN IF NOT EXISTS subtotal_cents INTEGER CHECK (subtotal_cents IS NULL OR subtotal_cents >= 0),
  ADD COLUMN IF NOT EXISTS tax_rate_bps INTEGER CHECK (tax_rate_bps IS NULL OR tax_rate_bps BETWEEN 0 AND 10000),
  ADD COLUMN IF NOT EXISTS tax_cents INTEGER CHECK (tax_cents IS NULL OR tax_cents >= 0),
  ADD COLUMN IF NOT EXISTS total_cents INTEGER CHECK (total_cents IS NULL OR total_cents >= 0),
  ADD COLUMN IF NOT EXISTS seller_snapshot JSONB NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN IF NOT EXISTS buyer_snapshot JSONB NOT NULL DEFAULT '{}'::jsonb,
  ADD COLUMN IF NOT EXISTS period_start TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS period_end TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS activation_mode TEXT NOT NULL DEFAULT 'payment_approval'
    CHECK (activation_mode IN ('test_instant','payment_approval'));

-- Backfill invoice identity for pre-Upgrade-29 orders that reached paid state.
UPDATE orders SET
  invoice_number = COALESCE(invoice_number,
    'INV-' || to_char(COALESCE(paid_at,created_at), 'YYYY') || '-' ||
    lpad(nextval('invoice_number_seq')::text, 6, '0')),
  issued_at = COALESCE(issued_at, created_at),
  due_at = COALESCE(due_at, created_at + interval '7 days'),
  invoice_status = CASE WHEN status='paid' THEN 'paid' WHEN status IN ('cancelled','rejected') THEN 'void' ELSE 'issued' END,
  subtotal_cents = COALESCE(subtotal_cents, amount_cents),
  tax_rate_bps = COALESCE(tax_rate_bps, 0),
  tax_cents = COALESCE(tax_cents, 0),
  total_cents = COALESCE(total_cents, amount_cents),
  period_start = COALESCE(period_start, COALESCE(activated_at, paid_at, created_at)),
  period_end = COALESCE(period_end,
    COALESCE(activated_at, paid_at, created_at) + CASE WHEN interval='year' THEN interval '1 year' ELSE interval '1 month' END)
WHERE invoice_number IS NULL OR issued_at IS NULL OR total_cents IS NULL;

CREATE INDEX IF NOT EXISTS orders_invoice_status_idx ON orders(invoice_status, due_at, created_at DESC);
CREATE INDEX IF NOT EXISTS orders_org_due_idx ON orders(organization_id, due_at) WHERE invoice_status='issued';

ALTER TABLE organization_subscriptions
  ADD COLUMN IF NOT EXISTS payment_due_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS grace_period_end TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS renewal_mode TEXT NOT NULL DEFAULT 'manual'
    CHECK (renewal_mode IN ('manual')),
  ADD COLUMN IF NOT EXISTS last_order_id UUID REFERENCES orders(id) ON DELETE SET NULL;

CREATE TABLE IF NOT EXISTS billing_email_outbox (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  order_id UUID NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  recipient CITEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('invoice_issued','payment_received')),
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','sending','sent','retry','failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_error TEXT NOT NULL DEFAULT '',
  sent_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE(order_id, kind)
);
CREATE INDEX IF NOT EXISTS billing_email_outbox_pending_idx
  ON billing_email_outbox(status, next_attempt_at) WHERE status IN ('pending','retry');

COMMENT ON COLUMN orders.amount_cents IS 'Compatibility total charged for the invoice; use subtotal_cents/tax_cents/total_cents for invoice rendering.';
COMMENT ON COLUMN orders.activation_mode IS 'test_instant means plan activated when ordered; payment_approval means activation waits for verified payment.';

CREATE OR REPLACE FUNCTION cs_mail_seed_billing_profile() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO organization_billing_profiles(organization_id,legal_name)
  VALUES(NEW.id,NEW.name)
  ON CONFLICT(organization_id) DO NOTHING;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS organizations_billing_profile_seed ON organizations;
CREATE TRIGGER organizations_billing_profile_seed
AFTER INSERT ON organizations FOR EACH ROW EXECUTE FUNCTION cs_mail_seed_billing_profile();
