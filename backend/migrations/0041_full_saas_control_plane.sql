-- Upgrade 35: complete localhost-only SaaS control plane.
--
-- Adds explicit platform runtime controls plus auditable organization lifecycle
-- metadata. Runtime switches are enforced by public handlers; they are not UI-
-- only flags. The singleton row is intentionally database-backed so every API
-- replica observes the same operator decision.

ALTER TABLE organizations
  ADD COLUMN IF NOT EXISTS status_reason TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS status_changed_by UUID REFERENCES users(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS status_changed_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS platform_controls (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  public_signup_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  business_creation_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  plan_ordering_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  domain_onboarding_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  mailbox_provisioning_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  outbound_sending_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  maintenance_message TEXT NOT NULL DEFAULT '',
  updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Defensive for an interrupted/pre-release application of this migration.
ALTER TABLE platform_controls
  ADD COLUMN IF NOT EXISTS mailbox_provisioning_enabled BOOLEAN NOT NULL DEFAULT TRUE;

INSERT INTO platform_controls(singleton)
VALUES(TRUE)
ON CONFLICT(singleton) DO NOTHING;

CREATE INDEX IF NOT EXISTS organizations_status_changed_idx
  ON organizations(status, status_changed_at DESC)
  WHERE is_system=FALSE;
