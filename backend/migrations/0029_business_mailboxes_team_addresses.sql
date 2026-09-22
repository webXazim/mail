-- Upgrade 22: business mailboxes, employee invitations, aliases and groups.
--
-- Hosted addresses now belong to an organization/domain first and may be
-- assigned to a platform identity later. Mailbox invitations are separate
-- from ordinary organization invitations because accepting one also binds a
-- real hosted mailbox. Business aliases/groups are first-class tenant-owned
-- routing objects and are never mixed with platform-admin legacy aliases.

CREATE TABLE IF NOT EXISTS mailbox_invitations (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  mailbox_id      UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  email           CITEXT NOT NULL,
  role            TEXT NOT NULL DEFAULT 'member'
                    CHECK (role IN ('owner','admin','billing','member')),
  token_hash      TEXT NOT NULL UNIQUE,
  status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','accepted','revoked','expired')),
  invited_by      UUID REFERENCES users(id) ON DELETE SET NULL,
  accepted_by     UUID REFERENCES users(id) ON DELETE SET NULL,
  expires_at      TIMESTAMPTZ NOT NULL,
  accepted_at     TIMESTAMPTZ,
  revoked_at      TIMESTAMPTZ,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT mailbox_invitation_email_nonempty CHECK (position('@' in email::text) > 1)
);
CREATE UNIQUE INDEX IF NOT EXISTS mailbox_invitations_pending_mailbox_idx
  ON mailbox_invitations(mailbox_id) WHERE status='pending';
CREATE INDEX IF NOT EXISTS mailbox_invitations_org_idx
  ON mailbox_invitations(organization_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS mailbox_invitations_expiry_idx
  ON mailbox_invitations(expires_at) WHERE status='pending';

-- Provider ownership marker for each hosted mailbox. Existing rows are
-- populated lazily by reconciliation; new customer mailboxes always receive a
-- deterministic marker before provider provisioning.
ALTER TABLE mailboxes
  ADD COLUMN IF NOT EXISTS provider_marker TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS invited_email CITEXT,
  ADD COLUMN IF NOT EXISTS suspended_at TIMESTAMPTZ;
CREATE UNIQUE INDEX IF NOT EXISTS mailboxes_provider_marker_idx
  ON mailboxes(provider_marker) WHERE provider_marker <> '' AND deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS business_addresses (
  id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id    UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  domain_id          UUID NOT NULL REFERENCES organization_domains(id) ON DELETE RESTRICT,
  local_part         TEXT NOT NULL,
  address            CITEXT NOT NULL,
  kind               TEXT NOT NULL CHECK (kind IN ('alias','group')),
  enabled            BOOLEAN NOT NULL DEFAULT TRUE,
  provider_marker    TEXT NOT NULL,
  provider_object_id TEXT,
  sync_status        TEXT NOT NULL DEFAULT 'pending'
                       CHECK (sync_status IN ('pending','syncing','ready','error','deleted')),
  sync_error         TEXT NOT NULL DEFAULT '',
  sync_attempts      INTEGER NOT NULL DEFAULT 0,
  next_attempt_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  synced_at          TIMESTAMPTZ,
  deleted_at         TIMESTAMPTZ,
  created_by         UUID REFERENCES users(id) ON DELETE SET NULL,
  created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT business_addresses_local_part_nonempty CHECK (length(btrim(local_part)) BETWEEN 1 AND 64)
);
CREATE UNIQUE INDEX IF NOT EXISTS business_addresses_active_address_idx
  ON business_addresses(lower(address::text)) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS business_addresses_provider_marker_idx
  ON business_addresses(provider_marker) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS business_addresses_reconcile_idx
  ON business_addresses(next_attempt_at, created_at)
  WHERE sync_status IN ('pending','error');
CREATE INDEX IF NOT EXISTS business_addresses_org_idx
  ON business_addresses(organization_id, kind, created_at) WHERE deleted_at IS NULL;

CREATE TABLE IF NOT EXISTS business_address_members (
  business_address_id UUID NOT NULL REFERENCES business_addresses(id) ON DELETE CASCADE,
  mailbox_id          UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (business_address_id, mailbox_id)
);
CREATE INDEX IF NOT EXISTS business_address_members_mailbox_idx
  ON business_address_members(mailbox_id, business_address_id);

-- An address may not simultaneously be a real mailbox and a business alias/group.
CREATE OR REPLACE FUNCTION cs_mail_business_address_conflict_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_TABLE_NAME = 'mailboxes' THEN
    IF NEW.deleted_at IS NULL AND EXISTS (
      SELECT 1 FROM business_addresses a
      WHERE a.deleted_at IS NULL AND lower(a.address::text)=lower(NEW.address::text)
    ) THEN
      RAISE EXCEPTION 'address already exists as a business alias/group';
    END IF;
  ELSE
    IF NEW.deleted_at IS NULL AND EXISTS (
      SELECT 1 FROM mailboxes m
      WHERE m.deleted_at IS NULL AND lower(m.address::text)=lower(NEW.address::text)
    ) THEN
      RAISE EXCEPTION 'address already exists as a mailbox';
    END IF;
  END IF;
  RETURN NEW;
END $$;

DROP TRIGGER IF EXISTS mailboxes_business_address_conflict ON mailboxes;
CREATE TRIGGER mailboxes_business_address_conflict
BEFORE INSERT OR UPDATE OF address,deleted_at ON mailboxes
FOR EACH ROW EXECUTE FUNCTION cs_mail_business_address_conflict_guard();

DROP TRIGGER IF EXISTS business_addresses_mailbox_conflict ON business_addresses;
CREATE TRIGGER business_addresses_mailbox_conflict
BEFORE INSERT OR UPDATE OF address,deleted_at ON business_addresses
FOR EACH ROW EXECUTE FUNCTION cs_mail_business_address_conflict_guard();
