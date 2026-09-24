-- A verified CS Mail business may attach mailboxes to a Stalwart domain
-- created by CrescentSphere Mailer. The provider entry remains Mailer-owned.
ALTER TABLE organization_domains
  ADD COLUMN IF NOT EXISTS shared_mailer_domain BOOLEAN NOT NULL DEFAULT FALSE;
