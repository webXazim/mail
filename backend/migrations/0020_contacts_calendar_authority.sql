-- Upgrade 14: Contacts + Calendar server authority, pagination, optimistic concurrency,
-- scalable lookup indexes, attendee integrity, recurrence persistence and import/export metadata.

ALTER TABLE contacts
  ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 1;

CREATE INDEX IF NOT EXISTS contacts_user_name_id_idx
  ON contacts (user_id, lower(name), id);
CREATE INDEX IF NOT EXISTS contacts_user_company_idx
  ON contacts (user_id, lower(company));
CREATE INDEX IF NOT EXISTS contacts_user_phone_idx
  ON contacts (user_id, phone);

ALTER TABLE calendar_events
  ADD COLUMN IF NOT EXISTS version BIGINT NOT NULL DEFAULT 1,
  ADD COLUMN IF NOT EXISTS timezone_offset_minutes INTEGER NOT NULL DEFAULT 0,
  ADD COLUMN IF NOT EXISTS recurrence JSONB,
  ADD COLUMN IF NOT EXISTS external_uid TEXT NOT NULL DEFAULT '';

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'calendar_timezone_offset_range'
  ) THEN
    ALTER TABLE calendar_events
      ADD CONSTRAINT calendar_timezone_offset_range
      CHECK (timezone_offset_minutes BETWEEN -840 AND 840);
  END IF;
END $$;

CREATE INDEX IF NOT EXISTS calendar_user_end_idx
  ON calendar_events(user_id, ends_at);
CREATE INDEX IF NOT EXISTS calendar_user_category_start_idx
  ON calendar_events(user_id, category, starts_at);
CREATE INDEX IF NOT EXISTS calendar_user_title_idx
  ON calendar_events(user_id, lower(title));
CREATE UNIQUE INDEX IF NOT EXISTS calendar_user_external_uid_unique
  ON calendar_events(user_id, external_uid)
  WHERE external_uid <> '';
