CREATE TABLE IF NOT EXISTS oauth_states (
    state_hash BYTEA PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    binding_hash BYTEA NOT NULL CHECK (octet_length(binding_hash) = 32),
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    redirect TEXT NOT NULL,
    pkce_verifier TEXT,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS oauth_states_expires_at ON oauth_states (expires_at);

CREATE TABLE IF NOT EXISTS oauth_states_used (
    state_hash BYTEA PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS oauth_states_used_expires_at ON oauth_states_used (expires_at);
