-- Upgrade 25: Deliverability, Abuse & Anti-Fraud.
--
-- Adds tenant-scoped recipient suppressions, provider delivery-event intake,
-- operator outbound controls, and hourly burst counters. The pre-existing
-- suppressed_addresses table remains the platform-wide emergency/legal block
-- list. Customer bounce/complaint/unsubscribe state belongs to a business.

CREATE TABLE IF NOT EXISTS recipient_suppressions (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  mailbox_id      UUID REFERENCES mailboxes(id) ON DELETE CASCADE,
  email           CITEXT NOT NULL,
  reason          TEXT NOT NULL,
  source          TEXT NOT NULL DEFAULT 'bounce',
  detail          TEXT NOT NULL DEFAULT '',
  created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT recipient_suppressions_email_nonempty CHECK (position('@' in email::text) > 1)
);
CREATE UNIQUE INDEX IF NOT EXISTS recipient_suppressions_org_email_idx
  ON recipient_suppressions(organization_id, lower(email::text)) WHERE mailbox_id IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS recipient_suppressions_mailbox_email_idx
  ON recipient_suppressions(mailbox_id, lower(email::text)) WHERE mailbox_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS recipient_suppressions_lookup_idx
  ON recipient_suppressions(organization_id, mailbox_id, lower(email::text));

CREATE TABLE IF NOT EXISTS organization_sending_controls (
  organization_id UUID PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
  state           TEXT NOT NULL DEFAULT 'active'
                  CHECK (state IN ('active','restricted','suspended')),
  hourly_limit_override INTEGER CHECK (hourly_limit_override IS NULL OR hourly_limit_override >= 0),
  daily_limit_override  INTEGER CHECK (daily_limit_override IS NULL OR daily_limit_override >= 0),
  reason          TEXT NOT NULL DEFAULT '',
  updated_by      UUID REFERENCES users(id) ON DELETE SET NULL,
  updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO organization_sending_controls(organization_id)
SELECT id FROM organizations
ON CONFLICT (organization_id) DO NOTHING;

CREATE OR REPLACE FUNCTION cs_mail_default_sending_control()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO organization_sending_controls(organization_id)
  VALUES(NEW.id) ON CONFLICT(organization_id) DO NOTHING;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS organizations_default_sending_control ON organizations;
CREATE TRIGGER organizations_default_sending_control
AFTER INSERT ON organizations
FOR EACH ROW EXECUTE FUNCTION cs_mail_default_sending_control();

CREATE TABLE IF NOT EXISTS mailbox_send_hourly_counters (
  mailbox_id UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  hour_start TIMESTAMPTZ NOT NULL,
  sent_count BIGINT NOT NULL DEFAULT 0 CHECK (sent_count >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (mailbox_id, hour_start)
);
CREATE TABLE IF NOT EXISTS organization_send_hourly_counters (
  organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  hour_start TIMESTAMPTZ NOT NULL,
  sent_count BIGINT NOT NULL DEFAULT 0 CHECK (sent_count >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (organization_id, hour_start)
);
CREATE TABLE IF NOT EXISTS domain_send_hourly_counters (
  domain CITEXT NOT NULL,
  hour_start TIMESTAMPTZ NOT NULL,
  sent_count BIGINT NOT NULL DEFAULT 0 CHECK (sent_count >= 0),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (domain, hour_start)
);

CREATE TABLE IF NOT EXISTS delivery_events (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  provider_event_id TEXT NOT NULL,
  provider          TEXT NOT NULL DEFAULT 'provider',
  event_type        TEXT NOT NULL CHECK (event_type IN ('delivered','soft_bounce','hard_bounce','complaint')),
  organization_id   UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
  mailbox_id        UUID NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
  send_request_id   UUID NOT NULL REFERENCES mail_send_requests(id) ON DELETE CASCADE,
  recipient         CITEXT NOT NULL,
  status            TEXT NOT NULL DEFAULT '',
  diagnostic        TEXT NOT NULL DEFAULT '',
  received_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
  processed_at      TIMESTAMPTZ,
  CONSTRAINT delivery_events_recipient_nonempty CHECK (position('@' in recipient::text) > 1),
  CONSTRAINT delivery_events_provider_event_unique UNIQUE(provider, provider_event_id)
);
CREATE INDEX IF NOT EXISTS delivery_events_org_received_idx
  ON delivery_events(organization_id, received_at DESC);
CREATE INDEX IF NOT EXISTS delivery_events_mailbox_received_idx
  ON delivery_events(mailbox_id, received_at DESC);

CREATE OR REPLACE FUNCTION cs_mail_delivery_event_scope_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  send_mailbox UUID;
  send_org UUID;
  send_recipients JSONB;
BEGIN
  SELECT r.mailbox_id, m.organization_id, r.recipients
    INTO send_mailbox, send_org, send_recipients
    FROM mail_send_requests r
    JOIN mailboxes m ON m.id=r.mailbox_id
   WHERE r.id=NEW.send_request_id;
  IF send_mailbox IS NULL OR send_mailbox <> NEW.mailbox_id OR send_org <> NEW.organization_id THEN
    RAISE EXCEPTION 'delivery event scope does not match send request';
  END IF;
  IF NOT EXISTS (
    SELECT 1 FROM jsonb_array_elements_text(COALESCE(send_recipients,'[]'::jsonb)) AS recipient(value)
    WHERE lower(recipient.value)=lower(NEW.recipient::text)
  ) THEN
    RAISE EXCEPTION 'delivery event recipient does not match send request';
  END IF;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS delivery_events_scope_guard ON delivery_events;
CREATE TRIGGER delivery_events_scope_guard
BEFORE INSERT OR UPDATE OF organization_id,mailbox_id,send_request_id,recipient ON delivery_events
FOR EACH ROW EXECUTE FUNCTION cs_mail_delivery_event_scope_guard();

CREATE OR REPLACE FUNCTION cs_mail_recipient_suppression_scope_guard()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE mailbox_org UUID;
BEGIN
  IF NEW.mailbox_id IS NOT NULL THEN
    SELECT organization_id INTO mailbox_org FROM mailboxes WHERE id=NEW.mailbox_id;
    IF mailbox_org IS NULL OR mailbox_org <> NEW.organization_id THEN
      RAISE EXCEPTION 'recipient suppression mailbox does not belong to organization';
    END IF;
  END IF;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS recipient_suppressions_scope_guard ON recipient_suppressions;
CREATE TRIGGER recipient_suppressions_scope_guard
BEFORE INSERT OR UPDATE OF organization_id,mailbox_id ON recipient_suppressions
FOR EACH ROW EXECUTE FUNCTION cs_mail_recipient_suppression_scope_guard();

COMMENT ON TABLE suppressed_addresses IS 'Platform-wide emergency/legal do-not-send list. Customer unsubscribe/bounce/complaint state is stored in recipient_suppressions from Upgrade 25.';
COMMENT ON TABLE delivery_events IS 'Idempotent normalized provider delivery events. (provider, provider_event_id) must remain stable across provider retries.';
