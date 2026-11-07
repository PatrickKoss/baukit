-- Reference schema for baukit-auth's browser session backend-for-frontend
-- (ADR 0006). Copy this file into the product's own ordered migrations.
--
-- The cookie carries `<id>.<secret>`; only SHA-256 of the secret is stored.
-- refresh_token, id_token and grace_access_token hold a 12-byte nonce followed
-- by AES-256-GCM ciphertext from baukit-credential-vault, sealed under
-- key_version with associated data bound to the row id and subject.
-- A refresh rotates the row: the successor joins the same family and the old
-- row points at it through rotated_to. Presenting a rotated row after the
-- successor's grace_until revokes the whole family. idle_expires_at is the
-- configured idle timeout capped by the IdP's refresh_expires_in, renewed on
-- every rotation; absolute_expires_at is fixed when the family starts.

CREATE TABLE baukit_auth_browser_sessions (
    id UUID PRIMARY KEY,
    secret_hash BYTEA NOT NULL CHECK (octet_length(secret_hash) = 32),
    family_id UUID NOT NULL,
    subject TEXT NOT NULL CHECK (char_length(subject) > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    refresh_token BYTEA NOT NULL,
    id_token BYTEA,
    created_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ NOT NULL,
    idle_expires_at TIMESTAMPTZ NOT NULL,
    absolute_expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    rotated_to UUID REFERENCES baukit_auth_browser_sessions (id),
    grace_access_token BYTEA,
    grace_expires_at TIMESTAMPTZ,
    grace_until TIMESTAMPTZ,
    CHECK (
        (grace_access_token IS NULL AND grace_expires_at IS NULL AND grace_until IS NULL)
        OR (grace_access_token IS NOT NULL AND grace_expires_at IS NOT NULL AND grace_until IS NOT NULL)
    )
);

COMMENT ON TABLE baukit_auth_browser_sessions IS
    'Browser sessions of baukit-auth::browser_session; secret_hash is SHA-256 of the cookie secret, token columns are encrypted';

CREATE INDEX baukit_auth_browser_sessions_family_idx
    ON baukit_auth_browser_sessions (family_id, created_at DESC);
CREATE INDEX baukit_auth_browser_sessions_subject_idx
    ON baukit_auth_browser_sessions (subject);
CREATE INDEX baukit_auth_browser_sessions_rotated_to_idx
    ON baukit_auth_browser_sessions (rotated_to);
