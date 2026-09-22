-- Upgrade 17: server-authoritative notifications, personal activity, support workflow,
-- and public pricing/status authority.

CREATE TABLE IF NOT EXISTS user_notifications (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind          TEXT NOT NULL CHECK (kind IN ('security','scheduled','mail','billing','support','account')),
  title         TEXT NOT NULL,
  detail        TEXT NOT NULL DEFAULT '',
  action_url    TEXT NOT NULL DEFAULT '',
  dedupe_key    TEXT,
  read_at       TIMESTAMPTZ,
  dismissed_at  TIMESTAMPTZ,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS user_notifications_user_created_idx
  ON user_notifications(user_id, created_at DESC)
  WHERE dismissed_at IS NULL;
CREATE INDEX IF NOT EXISTS user_notifications_user_unread_idx
  ON user_notifications(user_id, created_at DESC)
  WHERE dismissed_at IS NULL AND read_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS user_notifications_dedupe_idx
  ON user_notifications(user_id, dedupe_key)
  WHERE dedupe_key IS NOT NULL;

CREATE SEQUENCE IF NOT EXISTS support_ticket_ref_seq START WITH 1000;

CREATE TABLE IF NOT EXISTS support_tickets (
  id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  reference     TEXT NOT NULL UNIQUE DEFAULT ('CS-' || lpad(nextval('support_ticket_ref_seq')::text, 7, '0')),
  user_id       UUID REFERENCES users(id) ON DELETE SET NULL,
  requester_name TEXT NOT NULL,
  requester_email CITEXT NOT NULL,
  topic         TEXT NOT NULL CHECK (topic IN ('billing','security','technical','feedback','press','other')),
  subject       TEXT NOT NULL,
  status        TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','pending','resolved','closed')),
  priority      TEXT NOT NULL DEFAULT 'normal' CHECK (priority IN ('normal','high','urgent')),
  assigned_to   UUID REFERENCES users(id) ON DELETE SET NULL,
  created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_reply_at TIMESTAMPTZ,
  resolved_at   TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS support_tickets_status_updated_idx ON support_tickets(status, updated_at DESC);
CREATE INDEX IF NOT EXISTS support_tickets_user_updated_idx ON support_tickets(user_id, updated_at DESC) WHERE user_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS support_tickets_email_updated_idx ON support_tickets(requester_email, updated_at DESC);

CREATE TABLE IF NOT EXISTS support_messages (
  id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  ticket_id      UUID NOT NULL REFERENCES support_tickets(id) ON DELETE CASCADE,
  author_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  author_kind    TEXT NOT NULL CHECK (author_kind IN ('customer','agent','system')),
  body           TEXT NOT NULL,
  created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS support_messages_ticket_created_idx ON support_messages(ticket_id, created_at ASC);

CREATE TABLE IF NOT EXISTS service_incidents (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  title       TEXT NOT NULL,
  status      TEXT NOT NULL CHECK (status IN ('investigating','identified','monitoring','resolved')),
  impact      TEXT NOT NULL DEFAULT 'minor' CHECK (impact IN ('minor','major','critical')),
  message     TEXT NOT NULL DEFAULT '',
  started_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  resolved_at TIMESTAMPTZ,
  created_by  UUID REFERENCES users(id) ON DELETE SET NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS service_incidents_started_idx ON service_incidents(started_at DESC);
CREATE INDEX IF NOT EXISTS service_incidents_active_idx ON service_incidents(status, started_at DESC)
  WHERE status <> 'resolved';

-- Public product naming follows the current CS Mailer brand. Existing custom plan
-- names are not overwritten; only the legacy defaults are migrated.
UPDATE plans SET name = 'CS Mailer Solo' WHERE code = 'solo' AND name = 'Harbor Solo';
UPDATE plans SET name = 'CS Mailer Team' WHERE code = 'team' AND name = 'Harbor Team';
UPDATE plans SET name = 'CS Mailer Business' WHERE code = 'business' AND name = 'Harbor Business';
