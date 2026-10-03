CREATE TABLE federation_authorization_codes (
    code_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    challenge text NOT NULL,
    redirect_uri text NOT NULL,
    expires_at timestamptz NOT NULL,
    used_at timestamptz
);
CREATE INDEX federation_authorization_codes_expiry_idx ON federation_authorization_codes(expires_at);

CREATE TABLE federation_access_tokens (
    token_hash text PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE INDEX federation_access_tokens_expiry_idx ON federation_access_tokens(expires_at);
