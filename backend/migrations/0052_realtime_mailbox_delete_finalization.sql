-- Mailbox hard-delete realtime safety.
--
-- Hard-deleting a mailbox cascades through mailbox-owned tables. Several of
-- those tables have AFTER DELETE realtime triggers which historically tried to
-- insert realtime_events rows with the just-deleted mailbox_id. PostgreSQL then
-- rejected the insert via realtime_events_mailbox_id_fkey and rolled the entire
-- mailbox deletion transaction back, leaving a ghost mailbox in `deleting`.
--
-- Two protections are intentional:
--   1. mailbox finalization can suppress noisy per-child realtime invalidations;
--   2. the generic trigger defensively downgrades a missing mailbox FK to an
--      account-global realtime event instead of ever inserting a dangling FK.

CREATE OR REPLACE FUNCTION cs_realtime_row_change()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
  row_json          JSONB;
  owner_id          UUID;
  mailbox_fk        UUID;
  event_mailbox_fk  UUID;
  event_seq         BIGINT;
  row_id            TEXT;
  row_ver           JSONB;
BEGIN
  -- A mailbox hard-delete can cascade across contacts, drafts, automation,
  -- identities and other mailbox-owned resources. One final account-level
  -- mailbox event is emitted by the application after commit, so emitting one
  -- event per cascading child delete is both wasteful and unsafe.
  IF current_setting('cs_mail.suppress_realtime', TRUE) = 'on' THEN
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
  END IF;

  row_json := CASE WHEN TG_OP = 'DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
  owner_id := NULLIF(row_json ->> 'user_id', '')::UUID;
  mailbox_fk := NULLIF(row_json ->> 'mailbox_id', '')::UUID;
  IF owner_id IS NULL THEN
    RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
  END IF;

  -- Never allow a realtime side effect to break the business mutation. During
  -- a concurrent/cascading mailbox delete the old row may still carry its
  -- mailbox_id even though that parent no longer exists. Preserve the id in
  -- the JSON payload for context but store NULL in the FK column so the event
  -- is delivered as account-global and remains replayable.
  event_mailbox_fk := mailbox_fk;
  IF event_mailbox_fk IS NOT NULL
     AND NOT EXISTS (SELECT 1 FROM mailboxes WHERE id = event_mailbox_fk) THEN
    event_mailbox_fk := NULL;
  END IF;

  row_id := COALESCE(row_json ->> 'id', '');
  row_ver := row_json -> 'version';

  SELECT seq INTO event_seq
  FROM realtime_events
  WHERE transaction_id = txid_current()
    AND user_id = owner_id
    AND mailbox_id IS NOT DISTINCT FROM event_mailbox_fk
    AND kind = 'resource-changed'
    AND payload ->> 'resource' = TG_ARGV[0]
  ORDER BY seq DESC
  LIMIT 1;

  IF event_seq IS NULL THEN
    INSERT INTO realtime_events(user_id, mailbox_id, kind, payload)
    VALUES (
      owner_id,
      event_mailbox_fk,
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
