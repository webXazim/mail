-- Realm enhancements for the frontend activity surfaces.
-- Contacts: name/phone (frontend Contact shape). Calendar: description/location/category.

ALTER TABLE contacts
  ADD COLUMN IF NOT EXISTS name  TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS phone TEXT NOT NULL DEFAULT '';

ALTER TABLE calendar_events
  ADD COLUMN IF NOT EXISTS description TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS location    TEXT NOT NULL DEFAULT '',
  ADD COLUMN IF NOT EXISTS category    TEXT NOT NULL DEFAULT 'personal'
    CHECK (category IN ('work','meeting','personal','holiday','reminder'));

CREATE INDEX IF NOT EXISTS contacts_user_email_idx ON contacts(user_id, lower(email));