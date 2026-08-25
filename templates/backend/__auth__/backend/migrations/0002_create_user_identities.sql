CREATE TABLE IF NOT EXISTS user_identities (
    user_id UUID PRIMARY KEY,
    subject TEXT NOT NULL UNIQUE CHECK (length(trim(subject)) > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

COMMENT ON TABLE user_identities IS
    'Maps a verified provider-neutral OIDC subject to a stable internal user ID';
