-- Upgrade 20: public customer-domain claiming and DNS ownership verification.
-- Verification proves control of DNS only. Provider/domain provisioning is a
-- separate Upgrade 21 state transition and cannot be triggered by this schema.

ALTER TABLE organization_domains
  ADD COLUMN IF NOT EXISTS verification_name TEXT,
  ADD COLUMN IF NOT EXISTS verification_token TEXT,
  ADD COLUMN IF NOT EXISTS verification_expires_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS verification_attempts INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS last_checked_at TIMESTAMPTZ,
  ADD COLUMN IF NOT EXISTS last_dns_value TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS verification_method TEXT NOT NULL DEFAULT 'dns_txt'
    CHECK (verification_method IN ('dns_txt'));

CREATE TABLE IF NOT EXISTS domain_verification_events (
  id                BIGSERIAL PRIMARY KEY,
  domain_id         UUID NOT NULL REFERENCES organization_domains(id) ON DELETE CASCADE,
  organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  actor_user_id     UUID REFERENCES users(id) ON DELETE SET NULL,
  outcome           TEXT NOT NULL CHECK (outcome IN ('created','rotated','not_found','verified','expired','rate_limited','error')),
  observed_values   JSONB NOT NULL DEFAULT '[]'::jsonb,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS domain_verification_events_domain_idx
  ON domain_verification_events(domain_id, created_at DESC);

-- Backfill the protected CrescentSphere domain as provider-owned/system state.
-- It is not required to pass the customer verification flow.
UPDATE organization_domains
SET verified_at = COALESCE(verified_at, now()),
    verification_name = COALESCE(verification_name, '_cs-mail-verify.' || domain::text),
    updated_at = now()
WHERE is_system = TRUE;
