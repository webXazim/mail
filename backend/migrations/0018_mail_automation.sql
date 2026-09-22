-- Upgrade 12: server-authoritative incoming mail automation.
-- Desired state lives in PostgreSQL and is compiled into one CS Mailer-owned
-- Sieve script per mailbox. Provider application is retryable/idempotent.

CREATE TABLE IF NOT EXISTS mail_rules (
  id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  name        TEXT NOT NULL,
  enabled     BOOLEAN NOT NULL DEFAULT TRUE,
  position    INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0),
  conditions  JSONB NOT NULL DEFAULT '[]'::jsonb,
  actions     JSONB NOT NULL DEFAULT '[]'::jsonb,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (length(name) BETWEEN 1 AND 120),
  CHECK (jsonb_typeof(conditions) = 'array'),
  CHECK (jsonb_typeof(actions) = 'array')
);
CREATE INDEX IF NOT EXISTS mail_rules_user_order_idx
  ON mail_rules(user_id, position, created_at, id);

CREATE TABLE IF NOT EXISTS mail_forwarding (
  user_id                    UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  enabled                    BOOLEAN NOT NULL DEFAULT FALSE,
  target_email               CITEXT NOT NULL DEFAULT '',
  keep_copy                  BOOLEAN NOT NULL DEFAULT TRUE,
  verified_at                TIMESTAMPTZ,
  verification_token_hash    TEXT NOT NULL DEFAULT '',
  verification_expires_at    TIMESTAMPTZ,
  verification_sent_at       TIMESTAMPTZ,
  updated_at                 TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (target_email = '' OR position('@' in target_email::text) > 1)
);

CREATE TABLE IF NOT EXISTS mail_vacation (
  user_id        UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  enabled        BOOLEAN NOT NULL DEFAULT FALSE,
  subject        TEXT NOT NULL DEFAULT 'Out of office',
  message        TEXT NOT NULL DEFAULT '',
  only_contacts  BOOLEAN NOT NULL DEFAULT TRUE,
  starts_at      DATE,
  ends_at        DATE,
  updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  CHECK (length(subject) <= 200),
  CHECK (length(message) <= 10000),
  CHECK (starts_at IS NULL OR ends_at IS NULL OR ends_at >= starts_at)
);

CREATE TABLE IF NOT EXISTS mail_automation_state (
  user_id             UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  desired_revision    BIGINT NOT NULL DEFAULT 0,
  applied_revision    BIGINT NOT NULL DEFAULT 0,
  provider_script_id  TEXT,
  status               TEXT NOT NULL DEFAULT 'pending'
                       CHECK (status IN ('pending','syncing','ready','error','disabled')),
  last_error           TEXT NOT NULL DEFAULT '',
  retry_count          INTEGER NOT NULL DEFAULT 0,
  next_retry_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
  applied_at           TIMESTAMPTZ,
  updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS mail_automation_retry_idx
  ON mail_automation_state(status, next_retry_at)
  WHERE status IN ('pending','error');

-- Rules, forwarding and vacation are product features governed by the same
-- entitlement authority introduced in Upgrade 06.
UPDATE plans
SET feature_flags = feature_flags || '{"mail_rules":true,"forwarding":true,"vacation":true}'::jsonb;
