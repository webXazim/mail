-- Auth lifecycle: email verification + password reset tokens.
-- Tokens are single-use, hashed at rest, with explicit expiry. The raw value is
-- only ever handed to the user (via email link). Fields mirror frontend routes:
-- verify-email?token= and reset-password?token=.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS email_verified_at TIMESTAMPTZ;

CREATE TABLE IF NOT EXISTS email_tokens (
  id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  token_hash TEXT NOT NULL,                  -- sha256 of the raw token
  kind       TEXT NOT NULL CHECK (kind IN ('verify','reset')),
  expires_at TIMESTAMPTZ NOT NULL,
  used_at    TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS email_tokens_hash_idx   ON email_tokens(token_hash);
CREATE INDEX IF NOT EXISTS email_tokens_user_idx   ON email_tokens(user_id, kind);