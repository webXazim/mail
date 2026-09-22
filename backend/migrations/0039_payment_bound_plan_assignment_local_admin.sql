-- Upgrade 33: payment-bound subscription assignment + localhost-only platform admin.
--
-- A reviewed payment must identify exactly which invoice/order assigned the
-- active business plan and mailbox quantity. The public edge also moves the
-- platform-admin surface behind a localhost-only reverse proxy; application
-- authorization still remains mandatory.

ALTER TABLE organization_subscriptions
  ADD COLUMN IF NOT EXISTS assignment_source TEXT NOT NULL DEFAULT 'bootstrap'
    CHECK (assignment_source IN ('bootstrap','test_instant','payment_approval','admin_manual')),
  ADD COLUMN IF NOT EXISTS assigned_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS assigned_by UUID REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS assignment_invoice_number TEXT,
  ADD COLUMN IF NOT EXISTS assignment_order_user_id UUID REFERENCES users(id) ON DELETE SET NULL;

UPDATE organization_subscriptions
SET assigned_at = COALESCE(assigned_at, updated_at, created_at)
WHERE assigned_at IS NULL;

ALTER TABLE organization_subscriptions ALTER COLUMN assigned_at SET DEFAULT now();

ALTER TABLE orders
  ADD COLUMN IF NOT EXISTS subscription_assigned_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS subscription_assigned_by UUID REFERENCES users(id) ON DELETE SET NULL;

-- Preserve only the newest still-open invoice per business before adding the
-- database concurrency guard. Older duplicate open invoices are voided rather
-- than left able to assign a second plan later.
WITH ranked AS (
  SELECT id,
         row_number() OVER (PARTITION BY organization_id ORDER BY created_at DESC, id DESC) AS rn
  FROM orders
  WHERE organization_id IS NOT NULL
    AND status IN ('pending','submitted')
    AND invoice_status='issued'
)
UPDATE orders o
SET status='cancelled',
    invoice_status='void',
    admin_note=trim(concat_ws(' ', NULLIF(o.admin_note,''), 'Superseded while enforcing one open invoice per business.')),
    updated_at=now()
FROM ranked r
WHERE o.id=r.id AND r.rn>1;

CREATE UNIQUE INDEX IF NOT EXISTS orders_one_open_invoice_per_org_idx
  ON orders(organization_id)
  WHERE status IN ('pending','submitted') AND invoice_status='issued';

CREATE TABLE IF NOT EXISTS subscription_assignment_history (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  order_id UUID REFERENCES orders(id) ON DELETE SET NULL,
  invoice_number TEXT,
  order_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  plan_code TEXT NOT NULL REFERENCES plans(code),
  plan_name TEXT NOT NULL,
  purchased_mailbox_count INTEGER NOT NULL CHECK (purchased_mailbox_count > 0),
  assignment_source TEXT NOT NULL CHECK (assignment_source IN ('test_instant','payment_approval','admin_manual')),
  payment_confirmed BOOLEAN NOT NULL DEFAULT FALSE,
  assigned_by UUID REFERENCES users(id) ON DELETE SET NULL,
  assigned_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  detail JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE UNIQUE INDEX IF NOT EXISTS subscription_assignment_history_order_source_uq
  ON subscription_assignment_history(order_id, assignment_source)
  WHERE order_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS subscription_assignment_history_org_idx
  ON subscription_assignment_history(organization_id, assigned_at DESC);

COMMENT ON TABLE subscription_assignment_history IS
  'Immutable audit ledger showing which order/payment or explicit platform-admin action assigned a business subscription.';
COMMENT ON COLUMN organization_subscriptions.assignment_invoice_number IS
  'Invoice whose reviewed order most recently assigned the active subscription; NULL for manual/bootstrap assignments.';
COMMENT ON COLUMN organization_subscriptions.assignment_order_user_id IS
  'Customer account that placed the order which most recently assigned this subscription.';
