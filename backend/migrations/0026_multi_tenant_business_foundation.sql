-- Upgrade 19: multi-tenant business foundation.
--
-- A CS Mail login is now a platform identity, not implicitly a mailbox.
-- Organizations own domains/mailboxes; users participate through explicit
-- memberships. Existing single-business rows are migrated into a protected
-- CrescentSphere organization without rewriting earlier migrations.

-- Platform authorization is deliberately separate from organization roles.
ALTER TABLE users
  ADD COLUMN IF NOT EXISTS platform_role TEXT NOT NULL DEFAULT 'user'
    CHECK (platform_role IN ('user','platform_support','platform_admin'));

UPDATE users
SET platform_role = CASE WHEN role = 'admin' THEN 'platform_admin' ELSE 'user' END
WHERE platform_role = 'user';

-- A platform-only account legitimately has no mailbox yet.
ALTER TABLE users DROP CONSTRAINT IF EXISTS users_mail_sync_status_check;
ALTER TABLE users ADD CONSTRAINT users_mail_sync_status_check
  CHECK (mail_sync_status IN ('none','pending','ready','retrying','error'));

CREATE TABLE IF NOT EXISTS organizations (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name              TEXT NOT NULL,
  slug              TEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active','suspended','closed')),
  is_system         BOOLEAN NOT NULL DEFAULT FALSE,
  created_by        UUID REFERENCES users(id) ON DELETE SET NULL,
  created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT organizations_name_nonempty CHECK (length(btrim(name)) BETWEEN 1 AND 120),
  CONSTRAINT organizations_slug_format CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$')
);
CREATE UNIQUE INDEX IF NOT EXISTS organizations_slug_unique_idx ON organizations(lower(slug));
CREATE UNIQUE INDEX IF NOT EXISTS organizations_single_system_idx ON organizations(is_system) WHERE is_system;

CREATE TABLE IF NOT EXISTS organization_memberships (
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role            TEXT NOT NULL DEFAULT 'member'
                    CHECK (role IN ('owner','admin','billing','member')),
  status          TEXT NOT NULL DEFAULT 'active'
                    CHECK (status IN ('active','invited','suspended')),
  joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, user_id)
);
CREATE INDEX IF NOT EXISTS organization_memberships_user_idx
  ON organization_memberships(user_id, status, joined_at);
CREATE INDEX IF NOT EXISTS organization_memberships_org_role_idx
  ON organization_memberships(organization_id, role, status);

CREATE TABLE IF NOT EXISTS organization_invitations (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
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
  CONSTRAINT organization_invitation_email_nonempty CHECK (position('@' in email::text) > 1)
);
CREATE UNIQUE INDEX IF NOT EXISTS organization_invitations_pending_email_idx
  ON organization_invitations(organization_id, lower(email::text))
  WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS organization_invitations_expiry_idx
  ON organization_invitations(expires_at)
  WHERE status = 'pending';

-- Domain lifecycle is intentionally present in the foundation even though DNS
-- proof/provisioning is implemented in Upgrade 20/21. No public API may mark a
-- customer domain verified/active during Upgrade 19.
CREATE TABLE IF NOT EXISTS organization_domains (
  id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id       UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  domain                CITEXT NOT NULL,
  status                TEXT NOT NULL DEFAULT 'pending_verification'
                          CHECK (status IN (
                            'pending_verification','verified','provisioning','dns_pending',
                            'active','degraded','suspended','removing','failed'
                          )),
  is_primary            BOOLEAN NOT NULL DEFAULT FALSE,
  is_system             BOOLEAN NOT NULL DEFAULT FALSE,
  provider_domain_id    TEXT,
  verified_at           TIMESTAMPTZ,
  activated_at          TIMESTAMPTZ,
  last_error            TEXT NOT NULL DEFAULT '',
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT organization_domains_format CHECK (
    domain::text = lower(domain::text)
    AND length(domain::text) BETWEEN 3 AND 253
    AND position('.' in domain::text) > 1
  )
);
CREATE UNIQUE INDEX IF NOT EXISTS organization_domains_global_unique_idx
  ON organization_domains(lower(domain::text));
CREATE UNIQUE INDEX IF NOT EXISTS organization_domains_primary_idx
  ON organization_domains(organization_id) WHERE is_primary;
CREATE INDEX IF NOT EXISTS organization_domains_org_status_idx
  ON organization_domains(organization_id, status, created_at);

CREATE TABLE IF NOT EXISTS mailboxes (
  id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id       UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  domain_id             UUID NOT NULL REFERENCES organization_domains(id) ON DELETE RESTRICT,
  user_id               UUID REFERENCES users(id) ON DELETE SET NULL,
  address               CITEXT NOT NULL,
  local_part            TEXT NOT NULL,
  display_name          TEXT NOT NULL DEFAULT '',
  status                TEXT NOT NULL DEFAULT 'pending'
                          CHECK (status IN ('pending','active','suspended','deleting','error')),
  is_primary_for_user   BOOLEAN NOT NULL DEFAULT FALSE,
  provider_account_id   TEXT,
  sync_status           TEXT NOT NULL DEFAULT 'pending'
                          CHECK (sync_status IN ('none','pending','ready','retrying','error')),
  sync_error            TEXT NOT NULL DEFAULT '',
  quota_bytes           BIGINT NOT NULL DEFAULT 5368709120 CHECK (quota_bytes >= 1048576),
  created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
  deleted_at            TIMESTAMPTZ,
  CONSTRAINT mailboxes_local_part_nonempty CHECK (length(btrim(local_part)) BETWEEN 1 AND 128)
);
CREATE UNIQUE INDEX IF NOT EXISTS mailboxes_active_address_idx
  ON mailboxes(lower(address::text)) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS mailboxes_primary_user_idx
  ON mailboxes(user_id) WHERE is_primary_for_user AND deleted_at IS NULL AND user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS mailboxes_org_idx
  ON mailboxes(organization_id, status, created_at) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS mailboxes_domain_idx
  ON mailboxes(domain_id, status) WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS mailboxes_user_idx
  ON mailboxes(user_id, status) WHERE deleted_at IS NULL;

-- Provider jobs gain stable tenant/mailbox references. Upgrade 19 binds all
-- provider execution to these authoritative mailbox keys; Upgrade 21 extends
-- that foundation to newly verified customer domains.
ALTER TABLE provisioning_jobs
  ADD COLUMN IF NOT EXISTS organization_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS provisioning_jobs_mailbox_idx
  ON provisioning_jobs(mailbox_id, created_at DESC) WHERE mailbox_id IS NOT NULL;

-- Retain a convenient active/default selection on the platform identity.
ALTER TABLE users
  ADD COLUMN IF NOT EXISTS active_organization_id UUID REFERENCES organizations(id) ON DELETE SET NULL,
  ADD COLUMN IF NOT EXISTS primary_mailbox_id UUID REFERENCES mailboxes(id) ON DELETE SET NULL;

-- Protected built-in organization representing the pre-multi-tenant install.
INSERT INTO organizations(name, slug, is_system, created_by)
SELECT 'CrescentSphere', 'crescentsphere', TRUE,
       (SELECT id FROM users WHERE platform_role = 'platform_admin' ORDER BY created_at LIMIT 1)
WHERE NOT EXISTS (SELECT 1 FROM organizations WHERE is_system = TRUE);

-- Existing *hosted* users remain members so no current mailbox loses access
-- when the organization-aware UI lands. Platform-only/external-login users
-- are deliberately NOT swept into the protected CrescentSphere tenant.
INSERT INTO organization_memberships(organization_id, user_id, role, status)
SELECT o.id,
       u.id,
       CASE
         WHEN u.platform_role = 'platform_admin' OR u.role = 'admin' THEN 'owner'
         WHEN u.role = 'billing' THEN 'billing'
         ELSE 'member'
       END,
       'active'
FROM organizations o
JOIN users u ON (
  u.platform_role = 'platform_admin'
  OR lower(split_part(u.email::text, '@', 2)) = 'crescentsphere.com'
  OR COALESCE(u.mail_account_id, '') <> ''
)
WHERE o.is_system = TRUE
ON CONFLICT (organization_id, user_id) DO NOTHING;

-- A protected system organization must always have an owner. If an old
-- installation never assigned an admin role, promote the oldest hosted member
-- within this organization only; this does not grant platform-admin rights.
WITH system_org AS (
  SELECT id FROM organizations WHERE is_system = TRUE LIMIT 1
), candidate AS (
  SELECT om.organization_id, om.user_id
  FROM organization_memberships om
  JOIN users u ON u.id=om.user_id
  JOIN system_org so ON so.id=om.organization_id
  WHERE om.status='active'
  ORDER BY CASE WHEN u.platform_role='platform_admin' THEN 0 ELSE 1 END, u.created_at
  LIMIT 1
)
UPDATE organization_memberships om
SET role='owner', updated_at=now()
FROM candidate c
WHERE om.organization_id=c.organization_id AND om.user_id=c.user_id
  AND NOT EXISTS (
    SELECT 1 FROM organization_memberships existing
    WHERE existing.organization_id=c.organization_id
      AND existing.status='active' AND existing.role='owner'
  );

-- The deployment's own business domain is permanently reserved from public
-- claims. Existing provider-linked legacy domains are also captured under the
-- protected system organization, because those addresses were already managed
-- by this installation before the tenant split.
INSERT INTO organization_domains(organization_id, domain, status, is_primary, is_system, activated_at)
SELECT o.id, 'crescentsphere.com'::citext, 'active', TRUE, TRUE, now()
FROM organizations o
WHERE o.is_system = TRUE
ON CONFLICT DO NOTHING;

INSERT INTO organization_domains(organization_id, domain, status, is_primary, is_system, activated_at)
SELECT DISTINCT o.id,
       lower(split_part(u.email::text, '@', 2))::citext,
       'active', FALSE, TRUE, now()
FROM organizations o
JOIN users u ON COALESCE(u.mail_account_id, '') <> ''
WHERE o.is_system = TRUE
  AND split_part(u.email::text, '@', 2) <> ''
  AND lower(split_part(u.email::text, '@', 2)) <> 'crescentsphere.com'
ON CONFLICT DO NOTHING;

-- Materialize one primary mailbox row for every currently managed mailbox and
-- every existing @crescentsphere.com account. Other login identities stay
-- platform-only until their business domain is verified in later upgrades.
INSERT INTO mailboxes(
  organization_id, domain_id, user_id, address, local_part, display_name,
  status, is_primary_for_user, provider_account_id, sync_status, sync_error, quota_bytes
)
SELECT o.id,
       d.id,
       u.id,
       lower(u.email::text)::citext,
       split_part(lower(u.email::text), '@', 1),
       u.display_name,
       CASE WHEN u.status = 'suspended' THEN 'suspended'
            WHEN COALESCE(u.mail_account_id, '') <> '' THEN 'active'
            ELSE 'pending' END,
       TRUE,
       NULLIF(u.mail_account_id, ''),
       CASE WHEN COALESCE(u.mail_account_id, '') <> '' THEN 'ready'
            ELSE 'pending' END,
       u.mail_sync_error,
       GREATEST(u.quota_bytes, 1048576)
FROM organizations o
JOIN users u ON (
  lower(split_part(u.email::text, '@', 2)) = 'crescentsphere.com'
  OR COALESCE(u.mail_account_id, '') <> ''
)
JOIN organization_domains d
  ON d.organization_id = o.id
 AND lower(d.domain::text) = lower(split_part(u.email::text, '@', 2))
WHERE o.is_system = TRUE
ON CONFLICT DO NOTHING;

UPDATE users u
SET active_organization_id = m.organization_id,
    primary_mailbox_id = m.id,
    mail_sync_status = CASE
      WHEN m.sync_status = 'ready' THEN 'ready'
      WHEN m.sync_status = 'pending' THEN 'pending'
      ELSE u.mail_sync_status
    END
FROM mailboxes m
WHERE m.user_id = u.id AND m.is_primary_for_user = TRUE
  AND (u.active_organization_id IS NULL OR u.primary_mailbox_id IS NULL);

UPDATE users
SET active_organization_id = o.id
FROM organizations o
WHERE users.active_organization_id IS NULL
  AND o.is_system = TRUE
  AND EXISTS (
    SELECT 1 FROM organization_memberships om
    WHERE om.organization_id = o.id AND om.user_id = users.id AND om.status = 'active'
  );

-- Platform-only accounts should not be picked up by the legacy mailbox
-- reconciliation worker merely because they have a login email.
UPDATE users
SET mail_sync_status = 'none', mail_sync_error = ''
WHERE primary_mailbox_id IS NULL AND COALESCE(mail_account_id, '') = '';

UPDATE provisioning_jobs pj
SET organization_id = m.organization_id,
    mailbox_id = m.id
FROM mailboxes m
WHERE m.user_id = pj.user_id AND m.is_primary_for_user = TRUE
  AND pj.mailbox_id IS NULL;

-- Normalize active semantic dedupe keys to the mailbox identity. This matters
-- for in-flight credential/access jobs: after the deploy, a newly queued job
-- must supersede an older pre-Upgrade-19 retry rather than allowing an old
-- password/access state to replay after the newer state.
UPDATE provisioning_jobs pj
SET dedupe_key = CASE pj.operation
  WHEN 'ensure_mailbox' THEN 'mailbox.ensure:' || pj.mailbox_id::text
  WHEN 'set_quota' THEN 'mailbox.quota:' || pj.mailbox_id::text
  WHEN 'set_credentials' THEN 'mailbox.credentials:' || pj.mailbox_id::text
  WHEN 'set_access' THEN 'mailbox.access:' || pj.mailbox_id::text
  WHEN 'delete_mailbox' THEN 'mailbox.delete:' || pj.mailbox_id::text
  ELSE pj.dedupe_key
END,
updated_at = now()
WHERE pj.mailbox_id IS NOT NULL
  AND pj.status IN ('pending','retry','processing');

COMMENT ON COLUMN users.role IS
  'Legacy compatibility role. Platform authorization uses platform_role; organization authorization uses organization_memberships.role.';
COMMENT ON COLUMN users.email IS
  'Platform login/contact email. It is not implicitly a mailbox address after Upgrade 19.';
COMMENT ON TABLE mailboxes IS
  'Business mailboxes owned by organizations. Provider jobs are mailbox-authoritative from Upgrade 19; Upgrade 21 provisions newly verified customer domains.';

-- Legacy user-level mail state remains as a compatibility mirror during the
-- staged migration. Keep updates synchronized with the primary mailbox until
-- Upgrade 23 can remove the legacy fields safely.
CREATE OR REPLACE FUNCTION sync_primary_mailbox_from_user() RETURNS trigger AS $$
BEGIN
  IF NEW.primary_mailbox_id IS NOT NULL THEN
    UPDATE mailboxes
    SET provider_account_id = NULLIF(NEW.mail_account_id, ''),
        sync_status = CASE NEW.mail_sync_status
          WHEN 'ready' THEN 'ready'
          WHEN 'retrying' THEN 'retrying'
          WHEN 'error' THEN 'error'
          WHEN 'none' THEN 'none'
          ELSE 'pending'
        END,
        sync_error = NEW.mail_sync_error,
        quota_bytes = GREATEST(NEW.quota_bytes, 1048576),
        status = CASE
          WHEN NEW.status = 'suspended' THEN 'suspended'
          WHEN NEW.mail_sync_status = 'ready' THEN 'active'
          WHEN NEW.mail_sync_status = 'error' THEN 'error'
          ELSE 'pending'
        END,
        updated_at = now()
    WHERE id = NEW.primary_mailbox_id;
  END IF;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS users_primary_mailbox_compat_sync ON users;
CREATE TRIGGER users_primary_mailbox_compat_sync
AFTER UPDATE OF mail_account_id, mail_sync_status, mail_sync_error, quota_bytes, status ON users
FOR EACH ROW EXECUTE FUNCTION sync_primary_mailbox_from_user();
