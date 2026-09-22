-- Upgrade 34: complete localhost-only platform operations console.
--
-- Preserve authoritative subscription period/lifecycle information in the
-- assignment ledger, add a renewal-grace timestamp for deterministic expiry,
-- and make platform-admin inspection of activation/expiration history cheap.

ALTER TABLE organization_subscriptions
  ADD COLUMN IF NOT EXISTS renewal_grace_end TIMESTAMPTZ;

-- Legacy/bootstrap subscriptions predate paid invoice periods. Give customer
-- businesses one complete current term at the Upgrade 34 cutover instead of
-- silently treating an old bootstrap timestamp as an already-expired term.
UPDATE organization_subscriptions s
SET current_period_start = now(),
    current_period_end = now() + CASE WHEN p.interval='year' THEN interval '1 year' ELSE interval '1 month' END,
    updated_at = now()
FROM plans p, organizations o
WHERE p.code=s.plan_code AND o.id=s.organization_id AND o.is_system=FALSE
  AND s.current_period_end IS NULL AND s.assignment_source='bootstrap'
  AND s.status IN ('active','trial','past_due');

UPDATE organization_subscriptions s
SET renewal_grace_end = COALESCE(
  renewal_grace_end,
  CASE WHEN current_period_end IS NULL THEN NULL
       ELSE current_period_end + ((SELECT grace_days FROM billing_settings WHERE id=TRUE)::text || ' days')::interval
  END
)
WHERE renewal_grace_end IS NULL AND current_period_end IS NOT NULL;

CREATE INDEX IF NOT EXISTS organization_subscriptions_period_end_idx
  ON organization_subscriptions(status, current_period_end)
  WHERE current_period_end IS NOT NULL;
CREATE INDEX IF NOT EXISTS organization_subscriptions_renewal_grace_idx
  ON organization_subscriptions(status, renewal_grace_end)
  WHERE renewal_grace_end IS NOT NULL;


ALTER TABLE subscription_assignment_history
  DROP CONSTRAINT IF EXISTS subscription_assignment_history_assignment_source_check;
ALTER TABLE subscription_assignment_history
  ADD CONSTRAINT subscription_assignment_history_assignment_source_check
  CHECK (assignment_source IN ('bootstrap','test_instant','payment_approval','admin_manual','system_lifecycle'));

ALTER TABLE subscription_assignment_history
  ADD COLUMN IF NOT EXISTS event_type TEXT NOT NULL DEFAULT 'assignment'
    CHECK (event_type IN ('assignment','status','period','limits')),
  ADD COLUMN IF NOT EXISTS period_start TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS period_end TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS status_after TEXT
    CHECK (status_after IS NULL OR status_after IN ('trial','active','past_due','suspended','cancelled')),
  ADD COLUMN IF NOT EXISTS reason TEXT NOT NULL DEFAULT '';

UPDATE subscription_assignment_history
SET period_start = COALESCE(period_start, NULLIF(detail->>'period_start','')::timestamptz),
    period_end = COALESCE(period_end, NULLIF(detail->>'period_end','')::timestamptz),
    status_after = COALESCE(status_after, NULLIF(detail->>'status','')),
    reason = CASE
      WHEN reason <> '' THEN reason
      WHEN detail->>'reason' IS NOT NULL THEN detail->>'reason'
      WHEN assignment_source='payment_approval' THEN 'Payment approved'
      WHEN assignment_source='test_instant' THEN 'Acceptance-test instant activation'
      WHEN assignment_source='admin_manual' THEN 'Platform administrator change'
      ELSE ''
    END;

-- Ensure every existing subscription has at least one operator-visible origin
-- event, including businesses bootstrapped before invoice-bound assignments.
INSERT INTO subscription_assignment_history(
  organization_id,plan_code,plan_name,purchased_mailbox_count,assignment_source,payment_confirmed,
  assigned_by,assigned_at,event_type,period_start,period_end,status_after,reason,detail
)
SELECT s.organization_id,s.plan_code,p.name,s.purchased_mailbox_count,s.assignment_source,FALSE,
       s.assigned_by,COALESCE(s.assigned_at,s.created_at),'assignment',s.current_period_start,s.current_period_end,
       s.status,
       CASE s.assignment_source
         WHEN 'bootstrap' THEN 'Migrated existing subscription'
         WHEN 'test_instant' THEN 'Acceptance-test instant activation'
         WHEN 'payment_approval' THEN 'Payment approved'
         ELSE 'Platform administrator change'
       END,
       jsonb_build_object('upgrade','34','backfilled',TRUE)
FROM organization_subscriptions s JOIN plans p ON p.code=s.plan_code
WHERE NOT EXISTS (
  SELECT 1 FROM subscription_assignment_history h WHERE h.organization_id=s.organization_id
);

CREATE INDEX IF NOT EXISTS subscription_assignment_history_admin_timeline_idx
  ON subscription_assignment_history(organization_id, assigned_at DESC, event_type);

COMMENT ON COLUMN organization_subscriptions.renewal_grace_end IS
  'When an expired manual-renewal subscription moves from past_due to suspended if no renewal is applied.';
COMMENT ON COLUMN subscription_assignment_history.event_type IS
  'assignment = plan/quantity authority changed; status/period = lifecycle operation; limits = entitlement override change.';
