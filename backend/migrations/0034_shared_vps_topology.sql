-- Upgrade 27: shared-VPS production topology identity lock.
-- The provider namespace is deliberately persisted so a deployment cannot
-- silently change ownership prefixes and begin adopting another product's
-- Stalwart objects. Changing this value is an explicit operator migration.

CREATE TABLE IF NOT EXISTS platform_runtime_identity (
  singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  provider_namespace TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT platform_runtime_identity_namespace_check CHECK (
    provider_namespace ~ '^[a-z0-9][a-z0-9-]{2,31}$'
  )
);

INSERT INTO platform_runtime_identity(singleton, provider_namespace)
VALUES(TRUE, 'cs-mail')
ON CONFLICT (singleton) DO NOTHING;

-- Public tenant/provider bindings must always carry an ownership marker.
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='organization_domains_provider_marker_required') THEN
    ALTER TABLE organization_domains
      ADD CONSTRAINT organization_domains_provider_marker_required
      CHECK (is_system OR provider_domain_id IS NULL OR provider_domain_id='' OR provider_marker<>'') NOT VALID;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='mailboxes_provider_marker_required') THEN
    ALTER TABLE mailboxes
      ADD CONSTRAINT mailboxes_provider_marker_required
      CHECK (provider_account_id IS NULL OR provider_account_id='' OR provider_marker<>'') NOT VALID;
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname='business_addresses_provider_marker_required') THEN
    ALTER TABLE business_addresses
      ADD CONSTRAINT business_addresses_provider_marker_required
      CHECK (provider_object_id IS NULL OR provider_object_id='' OR provider_marker<>'') NOT VALID;
  END IF;
END $$;

-- Existing historical system-domain rows may predate ownership markers, so
-- constraints are introduced NOT VALID and enforced immediately for new/changed
-- rows. Operators can VALIDATE them after reconciling any legacy records.
