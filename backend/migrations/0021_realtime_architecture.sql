-- Upgrade 15: durable, multi-instance realtime architecture.
--
-- PostgreSQL is both the durable replay ledger and the cross-process wake-up
-- bus. Each committed event is inserted first, then pg_notify wakes every API
-- replica. WebSocket/long-poll clients replay by monotonically increasing seq,
-- so reconnects do not depend on an in-memory process buffer.

CREATE TABLE IF NOT EXISTS realtime_events (
  seq         BIGSERIAL PRIMARY KEY,
  user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,
  payload     JSONB NOT NULL DEFAULT '{}'::jsonb,
  transaction_id BIGINT NOT NULL DEFAULT txid_current(),
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (length(kind) BETWEEN 1 AND 80),
  CHECK (jsonb_typeof(payload) = 'object')
);
CREATE INDEX IF NOT EXISTS realtime_events_user_seq_idx
  ON realtime_events(user_id, seq);
CREATE INDEX IF NOT EXISTS realtime_events_created_idx
  ON realtime_events(created_at);
CREATE INDEX IF NOT EXISTS realtime_events_transaction_idx
  ON realtime_events(transaction_id, user_id, kind);

-- Mailbox polling/query state is durable and leased so several API replicas
-- can run the worker without every replica polling every mailbox.
CREATE TABLE IF NOT EXISTS realtime_mailbox_state (
  user_id                 UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  initialized             BOOLEAN NOT NULL DEFAULT FALSE,
  inbox_query_state       TEXT NOT NULL DEFAULT '',
  email_state             TEXT NOT NULL DEFAULT '',
  mailbox_state           TEXT NOT NULL DEFAULT '',
  quota_used              BIGINT,
  configured_quota_total  BIGINT,
  provider_quota_total    BIGINT,
  lease_owner             UUID,
  lease_until             TIMESTAMPTZ,
  watch_until             TIMESTAMPTZ,
  last_polled_at          TIMESTAMPTZ,
  last_error              TEXT NOT NULL DEFAULT '',
  updated_at              TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS realtime_mailbox_claim_idx
  ON realtime_mailbox_state(watch_until, lease_until, last_polled_at);

INSERT INTO realtime_mailbox_state(user_id)
SELECT id
FROM users
WHERE status = 'active' AND COALESCE(mail_account_id, '') <> ''
ON CONFLICT(user_id) DO NOTHING;

-- Transactional resource-change publisher. Application-owned tables can emit
-- invalidations without every handler remembering to publish one manually.
CREATE OR REPLACE FUNCTION cs_realtime_row_change()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
  row_json  JSONB;
  owner_id  UUID;
  event_seq BIGINT;
  row_id    TEXT;
  row_ver   JSONB;
BEGIN
  row_json := CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
  owner_id := NULLIF(row_json ->> 'user_id', '')::UUID;
  IF owner_id IS NULL THEN
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
  END IF;

  row_id := COALESCE(row_json ->> 'id', '');
  row_ver := row_json -> 'version';

  -- Row-heavy operations such as CSV/ICS imports can touch thousands of rows.
  -- Realtime is an invalidation channel, not an audit log. Collapse changes
  -- for the same user/resource within the current database transaction. This
  -- is deterministic even when an import transaction runs for many seconds.
  SELECT seq INTO event_seq
  FROM realtime_events
  WHERE transaction_id = txid_current()
    AND user_id = owner_id
    AND kind = 'resource-changed'
    AND payload ->> 'resource' = TG_ARGV[0]
  ORDER BY seq DESC
  LIMIT 1;

  IF event_seq IS NULL THEN
    INSERT INTO realtime_events(user_id, kind, payload)
    VALUES (
      owner_id,
      'resource-changed',
      jsonb_strip_nulls(jsonb_build_object(
        'resource', TG_ARGV[0],
        'action', lower(TG_OP),
        'id', NULLIF(row_id, ''),
        'version', row_ver
      ))
    )
    RETURNING seq INTO event_seq;

    PERFORM pg_notify('cs_mail_realtime', event_seq::TEXT);
  END IF;
  RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END;
$$;

CREATE OR REPLACE FUNCTION cs_realtime_user_change()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
  event_seq BIGINT;
BEGIN
  INSERT INTO realtime_events(user_id, kind, payload)
  VALUES (
    NEW.id,
    'resource-changed',
    jsonb_build_object(
      'resource', 'profile',
      'action', 'update',
      'id', NEW.id::TEXT
    )
  )
  RETURNING seq INTO event_seq;
  PERFORM pg_notify('cs_mail_realtime', event_seq::TEXT);
  RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS contacts_realtime_change ON contacts;
CREATE TRIGGER contacts_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON contacts
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('contacts');

DROP TRIGGER IF EXISTS calendar_realtime_change ON calendar_events;
CREATE TRIGGER calendar_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON calendar_events
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('calendar');

DROP TRIGGER IF EXISTS settings_realtime_change ON settings;
CREATE TRIGGER settings_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON settings
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('settings');

DROP TRIGGER IF EXISTS drafts_realtime_change ON mail_drafts;
CREATE TRIGGER drafts_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON mail_drafts
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('drafts');

DROP TRIGGER IF EXISTS scheduled_realtime_change ON scheduled_sends;
CREATE TRIGGER scheduled_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON scheduled_sends
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('schedule');

DROP TRIGGER IF EXISTS rules_realtime_change ON mail_rules;
CREATE TRIGGER rules_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON mail_rules
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('automation');

DROP TRIGGER IF EXISTS forwarding_realtime_change ON mail_forwarding;
CREATE TRIGGER forwarding_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON mail_forwarding
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('automation');

DROP TRIGGER IF EXISTS vacation_realtime_change ON mail_vacation;
CREATE TRIGGER vacation_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON mail_vacation
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('automation');

DROP TRIGGER IF EXISTS identities_realtime_change ON sender_identities;
CREATE TRIGGER identities_realtime_change
AFTER INSERT OR UPDATE OR DELETE ON sender_identities
FOR EACH ROW EXECUTE FUNCTION cs_realtime_row_change('identities');

DROP TRIGGER IF EXISTS users_realtime_profile_change ON users;
CREATE TRIGGER users_realtime_profile_change
AFTER UPDATE OF email, display_name, role, status, quota_bytes ON users
FOR EACH ROW
WHEN (
  OLD.email IS DISTINCT FROM NEW.email OR
  OLD.display_name IS DISTINCT FROM NEW.display_name OR
  OLD.role IS DISTINCT FROM NEW.role OR
  OLD.status IS DISTINCT FROM NEW.status OR
  OLD.quota_bytes IS DISTINCT FROM NEW.quota_bytes
)
EXECUTE FUNCTION cs_realtime_user_change();
