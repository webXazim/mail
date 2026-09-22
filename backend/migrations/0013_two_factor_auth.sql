-- Upgrade 05: real TOTP two-factor authentication.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS two_factor_enabled_at timestamptz,
    ADD COLUMN IF NOT EXISTS totp_secret_ciphertext bytea,
    ADD COLUMN IF NOT EXISTS totp_pending_ciphertext bytea,
    ADD COLUMN IF NOT EXISTS totp_pending_expires_at timestamptz,
    ADD COLUMN IF NOT EXISTS totp_last_used_step bigint;

CREATE TABLE IF NOT EXISTS two_factor_recovery_codes (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    lookup_prefix text NOT NULL,
    code_hash text NOT NULL,
    used_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, lookup_prefix)
);

CREATE INDEX IF NOT EXISTS two_factor_recovery_codes_active_idx
    ON two_factor_recovery_codes(user_id, used_at);

CREATE TABLE IF NOT EXISTS two_factor_challenges (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash text NOT NULL UNIQUE,
    ip text NOT NULL DEFAULT '',
    user_agent text NOT NULL DEFAULT '',
    attempts integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL DEFAULT 8,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (attempts >= 0),
    CHECK (max_attempts BETWEEN 1 AND 20)
);

CREATE INDEX IF NOT EXISTS two_factor_challenges_user_idx
    ON two_factor_challenges(user_id, expires_at DESC);
CREATE INDEX IF NOT EXISTS two_factor_challenges_expiry_idx
    ON two_factor_challenges(expires_at)
    WHERE consumed_at IS NULL;
