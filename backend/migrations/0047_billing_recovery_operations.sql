-- Upgrade 47: billing recovery operations, retention-expiry authority, and
-- explicit operator-controlled retained-data purge workflow.
--
-- Reaching the retention deadline NEVER deletes data by itself. A platform
-- administrator must explicitly confirm a purge. Purge progress is durable and
-- provider deletion remains asynchronous/retryable through provisioning jobs.

ALTER TABLE organization_subscriptions
  ADD COLUMN IF NOT EXISTS retention_expired_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS purge_started_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS data_purged_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS subscription_purge_runs (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE RESTRICT,
  status TEXT NOT NULL DEFAULT 'queued'
    CHECK (status IN ('queued','processing','completed','failed')),
  requested_by UUID REFERENCES users(id) ON DELETE SET NULL,
  reason TEXT NOT NULL DEFAULT '',
  eligible_at_snapshot TIMESTAMPTZ NOT NULL,
  mailbox_count_snapshot INTEGER NOT NULL DEFAULT 0 CHECK (mailbox_count_snapshot >= 0),
  address_count_snapshot INTEGER NOT NULL DEFAULT 0 CHECK (address_count_snapshot >= 0),
  storage_bytes_snapshot BIGINT NOT NULL DEFAULT 0 CHECK (storage_bytes_snapshot >= 0),
  completed_mailbox_count INTEGER NOT NULL DEFAULT 0 CHECK (completed_mailbox_count >= 0),
  completed_address_count INTEGER NOT NULL DEFAULT 0 CHECK (completed_address_count >= 0),
  last_error TEXT NOT NULL DEFAULT '',
  requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  started_at TIMESTAMPTZ,
  completed_at TIMESTAMPTZ,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS subscription_purge_runs_one_active_org_uq
  ON subscription_purge_runs(organization_id)
  WHERE status IN ('queued','processing');
CREATE INDEX IF NOT EXISTS subscription_purge_runs_status_idx
  ON subscription_purge_runs(status, requested_at DESC);

-- Make operational status queries cheap without changing the provisioning
-- worker's existing ready-queue indexes.
CREATE INDEX IF NOT EXISTS provisioning_jobs_org_status_idx
  ON provisioning_jobs(organization_id,status,updated_at DESC)
  WHERE organization_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS billing_lifecycle_outbox_org_status_idx
  ON billing_lifecycle_outbox(organization_id,status,updated_at DESC);

COMMENT ON COLUMN organization_subscriptions.retention_expired_at IS
  'First time the retained-data deadline was observed as expired. This marker is non-destructive.';
COMMENT ON COLUMN organization_subscriptions.purge_started_at IS
  'Time an explicit platform-admin purge began. Automatic lifecycle reconciliation never sets this.';
COMMENT ON COLUMN organization_subscriptions.data_purged_at IS
  'Time retained mailbox/provider data was purged in the current subscription lifecycle. Reset on a new activation; durable purge/history rows preserve prior evidence.';
COMMENT ON TABLE subscription_purge_runs IS
  'Durable, explicit platform-admin retained-data purge runs. Provider deletion is performed through provisioning/address reconciliation.';
