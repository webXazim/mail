-- Upgrade 23: tenant + active-mailbox authority migration.
--
-- A platform login may now operate more than one assigned mailbox.  The active
-- organization and active mailbox are preferences only; every mailbox-scoped
-- row carries a mailbox FK so request authorization can be checked against the
-- concrete tenant/mailbox before data is read or mutated.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS active_mailbox_id UUID REFERENCES mailboxes(id) ON DELETE SET NULL;

UPDATE users
SET active_mailbox_id = primary_mailbox_id
WHERE active_mailbox_id IS NULL AND primary_mailbox_id IS NOT NULL;

-- Keep the organization preference coherent with the selected mailbox.
UPDATE users u
SET active_organization_id = m.organization_id
FROM mailboxes m
WHERE u.active_mailbox_id = m.id
  AND (u.active_organization_id IS NULL OR u.active_organization_id <> m.organization_id);

-- Mailbox-scoped product state.  user_id is deliberately retained as the actor /
-- assignee for compatibility and auditability, but mailbox_id is the authority.
ALTER TABLE contacts ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE calendar_events ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE mail_drafts ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE scheduled_sends ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE read_receipts ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE receipt_requests ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE sender_identities ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE mail_send_requests ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE staged_attachments ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE attachment_refs ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE mail_rules ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE mail_forwarding ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE mail_vacation ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE mail_automation_state ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE address_sync_state ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE aliases ADD COLUMN IF NOT EXISTS dest_mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;
ALTER TABLE user_notifications ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;

-- Exact sender-address matches are the strongest migration signal for identities.
UPDATE sender_identities i
SET mailbox_id = m.id
FROM mailboxes m
WHERE i.mailbox_id IS NULL
  AND m.user_id = i.user_id
  AND m.deleted_at IS NULL
  AND lower(m.address::text) = lower(i.email::text);

-- Backfill all remaining legacy state from the user's current/default mailbox.
UPDATE contacts x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE calendar_events x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE mail_drafts x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE scheduled_sends x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE read_receipts x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE receipt_requests x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE sender_identities x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE mail_send_requests x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE staged_attachments x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE mail_rules x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE mail_forwarding x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE mail_vacation x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE mail_automation_state x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE address_sync_state x SET mailbox_id=u.active_mailbox_id FROM users u WHERE x.user_id=u.id AND x.mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;
UPDATE aliases x SET dest_mailbox_id=u.active_mailbox_id FROM users u WHERE x.dest_user=u.id AND x.dest_mailbox_id IS NULL AND u.active_mailbox_id IS NOT NULL;

UPDATE attachment_refs r
SET mailbox_id = a.mailbox_id
FROM staged_attachments a
WHERE r.attachment_id=a.id AND r.mailbox_id IS NULL AND a.mailbox_id IS NOT NULL;

UPDATE mail_send_requests r
SET mailbox_id = d.mailbox_id
FROM mail_drafts d
WHERE r.draft_id=d.id AND r.mailbox_id IS NULL AND d.mailbox_id IS NOT NULL;

-- Mailbox-specific settings preserve existing settings for the migrated active
-- mailbox while allowing another mailbox on the same login to diverge safely.
CREATE TABLE IF NOT EXISTS mailbox_settings (
  mailbox_id UUID PRIMARY KEY REFERENCES mailboxes(id) ON DELETE CASCADE,
  user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  payload    JSONB NOT NULL DEFAULT '{}'::jsonb,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO mailbox_settings(mailbox_id,user_id,payload,updated_at)
SELECT u.active_mailbox_id,s.user_id,s.payload,s.updated_at
FROM settings s JOIN users u ON u.id=s.user_id
WHERE u.active_mailbox_id IS NOT NULL
ON CONFLICT(mailbox_id) DO NOTHING;

-- Replace uniqueness that previously assumed one mailbox per login.
ALTER TABLE contacts DROP CONSTRAINT IF EXISTS contacts_user_id_email_key;
DROP INDEX IF EXISTS contacts_user_name_id_idx;
CREATE INDEX IF NOT EXISTS contacts_mailbox_name_id_idx ON contacts(mailbox_id, lower(name), id);
CREATE INDEX IF NOT EXISTS contacts_mailbox_company_idx ON contacts(mailbox_id, lower(company));
CREATE INDEX IF NOT EXISTS contacts_mailbox_phone_idx ON contacts(mailbox_id, phone);
CREATE UNIQUE INDEX IF NOT EXISTS contacts_mailbox_email_unique_idx ON contacts(mailbox_id, lower(email));

DROP INDEX IF EXISTS calendar_user_start_idx;
DROP INDEX IF EXISTS calendar_user_end_idx;
DROP INDEX IF EXISTS calendar_user_category_start_idx;
DROP INDEX IF EXISTS calendar_user_title_idx;
DROP INDEX IF EXISTS calendar_user_external_uid_unique;
CREATE INDEX IF NOT EXISTS calendar_mailbox_start_idx ON calendar_events(mailbox_id, starts_at);
CREATE INDEX IF NOT EXISTS calendar_mailbox_end_idx ON calendar_events(mailbox_id, ends_at);
CREATE INDEX IF NOT EXISTS calendar_mailbox_category_start_idx ON calendar_events(mailbox_id, category, starts_at);
CREATE INDEX IF NOT EXISTS calendar_mailbox_title_idx ON calendar_events(mailbox_id, lower(title));
CREATE UNIQUE INDEX IF NOT EXISTS calendar_mailbox_external_uid_unique ON calendar_events(mailbox_id, external_uid) WHERE external_uid <> '';

DROP INDEX IF EXISTS mail_drafts_user_client_key_idx;
CREATE UNIQUE INDEX IF NOT EXISTS mail_drafts_mailbox_client_key_idx ON mail_drafts(mailbox_id, client_key);
CREATE INDEX IF NOT EXISTS mail_drafts_mailbox_updated_idx ON mail_drafts(mailbox_id, updated_at DESC);

DROP INDEX IF EXISTS scheduled_sends_user_idempotency_idx;
CREATE UNIQUE INDEX IF NOT EXISTS scheduled_sends_mailbox_idempotency_idx ON scheduled_sends(mailbox_id, idempotency_key);
CREATE INDEX IF NOT EXISTS scheduled_sends_mailbox_send_idx ON scheduled_sends(mailbox_id, send_at DESC);

DROP INDEX IF EXISTS read_receipts_user_mail_idx;
CREATE UNIQUE INDEX IF NOT EXISTS read_receipts_mailbox_mail_idx ON read_receipts(mailbox_id, mail_id);
DROP INDEX IF EXISTS receipt_requests_user_mail_idx;
CREATE UNIQUE INDEX IF NOT EXISTS receipt_requests_mailbox_mail_idx ON receipt_requests(mailbox_id, mail_id);

ALTER TABLE sender_identities DROP CONSTRAINT IF EXISTS sender_identities_user_id_email_key;
DROP INDEX IF EXISTS sender_identities_one_primary_idx;
CREATE UNIQUE INDEX IF NOT EXISTS sender_identities_mailbox_email_idx ON sender_identities(mailbox_id, email);
CREATE UNIQUE INDEX IF NOT EXISTS sender_identities_one_primary_mailbox_idx ON sender_identities(mailbox_id) WHERE mailbox_id IS NOT NULL AND is_primary;
CREATE INDEX IF NOT EXISTS sender_identities_mailbox_status_idx ON sender_identities(mailbox_id, status, is_primary DESC, created_at);

ALTER TABLE mail_send_requests DROP CONSTRAINT IF EXISTS mail_send_requests_user_id_idempotency_key_key;
CREATE UNIQUE INDEX IF NOT EXISTS mail_send_requests_mailbox_idempotency_idx ON mail_send_requests(mailbox_id, idempotency_key);
CREATE INDEX IF NOT EXISTS mail_send_requests_mailbox_created_idx ON mail_send_requests(mailbox_id, created_at DESC);

CREATE INDEX IF NOT EXISTS staged_attachments_mailbox_status_idx ON staged_attachments(mailbox_id, status, created_at DESC);
CREATE INDEX IF NOT EXISTS attachment_refs_mailbox_owner_idx ON attachment_refs(mailbox_id, owner_type, owner_id);
CREATE INDEX IF NOT EXISTS mail_rules_mailbox_order_idx ON mail_rules(mailbox_id, position, created_at, id);
CREATE UNIQUE INDEX IF NOT EXISTS mail_forwarding_mailbox_unique_idx ON mail_forwarding(mailbox_id) WHERE mailbox_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS mail_vacation_mailbox_unique_idx ON mail_vacation(mailbox_id) WHERE mailbox_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS mail_automation_state_mailbox_unique_idx ON mail_automation_state(mailbox_id) WHERE mailbox_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS address_sync_state_mailbox_unique_idx ON address_sync_state(mailbox_id) WHERE mailbox_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS aliases_dest_mailbox_active_idx ON aliases(dest_mailbox_id, created_at) WHERE dest_mailbox_id IS NOT NULL AND deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS user_notifications_mailbox_created_idx ON user_notifications(mailbox_id, created_at DESC) WHERE mailbox_id IS NOT NULL AND dismissed_at IS NULL;

-- Every assigned mailbox has a primary sender identity independent of login email.
INSERT INTO sender_identities(user_id,mailbox_id,email,display_name,is_primary,status,source,verified_at)
SELECT m.user_id,m.id,m.address,COALESCE(NULLIF(m.display_name,''),m.local_part),TRUE,'verified','primary',now()
FROM mailboxes m
WHERE m.user_id IS NOT NULL AND m.deleted_at IS NULL
ON CONFLICT DO NOTHING;

-- Operational indexes used by request-context resolution and switching.
CREATE INDEX IF NOT EXISTS users_active_mailbox_idx ON users(active_mailbox_id) WHERE active_mailbox_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS mailboxes_user_org_active_idx ON mailboxes(user_id,organization_id,status,id) WHERE deleted_at IS NULL;

-- Legacy single-row-per-user automation tables need a mailbox-capable key.
ALTER TABLE mail_forwarding ADD COLUMN IF NOT EXISTS id UUID DEFAULT gen_random_uuid();
ALTER TABLE mail_vacation ADD COLUMN IF NOT EXISTS id UUID DEFAULT gen_random_uuid();
ALTER TABLE mail_automation_state ADD COLUMN IF NOT EXISTS id UUID DEFAULT gen_random_uuid();
ALTER TABLE address_sync_state ADD COLUMN IF NOT EXISTS id UUID DEFAULT gen_random_uuid();
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname='mail_forwarding_pkey') THEN
    ALTER TABLE mail_forwarding DROP CONSTRAINT mail_forwarding_pkey;
  END IF;
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname='mail_vacation_pkey') THEN
    ALTER TABLE mail_vacation DROP CONSTRAINT mail_vacation_pkey;
  END IF;
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname='mail_automation_state_pkey') THEN
    ALTER TABLE mail_automation_state DROP CONSTRAINT mail_automation_state_pkey;
  END IF;
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname='address_sync_state_pkey') THEN
    ALTER TABLE address_sync_state DROP CONSTRAINT address_sync_state_pkey;
  END IF;
END $$;
ALTER TABLE mail_forwarding ALTER COLUMN id SET NOT NULL;
ALTER TABLE mail_vacation ALTER COLUMN id SET NOT NULL;
ALTER TABLE mail_automation_state ALTER COLUMN id SET NOT NULL;
ALTER TABLE address_sync_state ALTER COLUMN id SET NOT NULL;
ALTER TABLE mail_forwarding ADD CONSTRAINT mail_forwarding_pkey PRIMARY KEY (id);
ALTER TABLE mail_vacation ADD CONSTRAINT mail_vacation_pkey PRIMARY KEY (id);
ALTER TABLE mail_automation_state ADD CONSTRAINT mail_automation_state_pkey PRIMARY KEY (id);
ALTER TABLE address_sync_state ADD CONSTRAINT address_sync_state_pkey PRIMARY KEY (id);

-- Durable realtime must follow the concrete mailbox as well. Events with a
-- NULL mailbox_id are intentionally account-global (security/billing/profile).
ALTER TABLE realtime_events
  ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE SET NULL;
ALTER TABLE realtime_mailbox_state
  ADD COLUMN IF NOT EXISTS mailbox_id UUID REFERENCES mailboxes(id) ON DELETE CASCADE;

UPDATE realtime_mailbox_state r
SET mailbox_id = COALESCE(u.active_mailbox_id,u.primary_mailbox_id)
FROM users u
WHERE r.user_id=u.id AND r.mailbox_id IS NULL;
DELETE FROM realtime_mailbox_state WHERE mailbox_id IS NULL;

DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_constraint WHERE conname='realtime_mailbox_state_pkey') THEN
    ALTER TABLE realtime_mailbox_state DROP CONSTRAINT realtime_mailbox_state_pkey;
  END IF;
END $$;
ALTER TABLE realtime_mailbox_state ALTER COLUMN mailbox_id SET NOT NULL;
ALTER TABLE realtime_mailbox_state ADD CONSTRAINT realtime_mailbox_state_pkey PRIMARY KEY (mailbox_id);
CREATE INDEX IF NOT EXISTS realtime_mailbox_state_user_idx ON realtime_mailbox_state(user_id,mailbox_id);
CREATE INDEX IF NOT EXISTS realtime_events_user_mailbox_seq_idx ON realtime_events(user_id,mailbox_id,seq);

-- Mailbox-specific notification dedupe prevents the same remote mail id in two
-- business mailboxes from collapsing into one notification.
DROP INDEX IF EXISTS user_notifications_dedupe_idx;
CREATE UNIQUE INDEX IF NOT EXISTS user_notifications_mailbox_dedupe_idx
  ON user_notifications(user_id,mailbox_id,dedupe_key)
  WHERE mailbox_id IS NOT NULL AND dedupe_key IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS user_notifications_global_dedupe_idx
  ON user_notifications(user_id,dedupe_key)
  WHERE mailbox_id IS NULL AND dedupe_key IS NOT NULL;

-- Recreate the row-change publisher so mailbox-owned tables write a concrete
-- mailbox scope into the durable event ledger. Global rows continue to use NULL.
CREATE OR REPLACE FUNCTION cs_realtime_row_change()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
  row_json   JSONB;
  owner_id   UUID;
  mailbox_fk UUID;
  event_seq  BIGINT;
  row_id     TEXT;
  row_ver    JSONB;
BEGIN
  row_json := CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
  owner_id := NULLIF(row_json ->> 'user_id', '')::UUID;
  mailbox_fk := NULLIF(row_json ->> 'mailbox_id', '')::UUID;
  IF owner_id IS NULL THEN
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
  END IF;

  row_id := COALESCE(row_json ->> 'id', '');
  row_ver := row_json -> 'version';

  SELECT seq INTO event_seq
  FROM realtime_events
  WHERE transaction_id = txid_current()
    AND user_id = owner_id
    AND mailbox_id IS NOT DISTINCT FROM mailbox_fk
    AND kind = 'resource-changed'
    AND payload ->> 'resource' = TG_ARGV[0]
  ORDER BY seq DESC
  LIMIT 1;

  IF event_seq IS NULL THEN
    INSERT INTO realtime_events(user_id, mailbox_id, kind, payload)
    VALUES (
      owner_id,
      mailbox_fk,
      'resource-changed',
      jsonb_strip_nulls(jsonb_build_object(
        'resource', TG_ARGV[0],
        'action', lower(TG_OP),
        'id', NULLIF(row_id, ''),
        'version', row_ver,
        'mailbox_id', mailbox_fk
      ))
    )
    RETURNING seq INTO event_seq;
    PERFORM pg_notify('cs_mail_realtime', event_seq::TEXT);
  END IF;
  RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$;

DROP TRIGGER IF EXISTS mailbox_settings_realtime_change ON mailbox_settings;
CREATE TRIGGER mailbox_settings_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON mailbox_settings
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('settings');
