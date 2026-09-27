-- Reference schema for baukit-auth's PostgresApiTokenStore.
--
-- Copy this file into the product's own ordered migrations. Products own
-- migration execution; baukit-auth deliberately does not migrate on startup.
--
-- Baukit does not own the owner table and cannot name it here. Join ownership
-- in a product migration so erasing an owner erases their tokens:
--
--     ALTER TABLE api_tokens
--         ADD CONSTRAINT api_tokens_owner_fk
--         FOREIGN KEY (owner_id) REFERENCES <owner table> (id) ON DELETE CASCADE;
--
-- `grants` holds opaque product-defined strings. A product that wants the
-- database to reject unknown grants adds its own CHECK, for example
-- `CHECK (grants <@ ARRAY['<grant>', ...]::TEXT[])`.

CREATE TABLE api_tokens (
    id UUID PRIMARY KEY,
    owner_id UUID NOT NULL,
    name TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    token_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    token_prefix TEXT NOT NULL CHECK (char_length(token_prefix) > 0),
    grants TEXT[] NOT NULL DEFAULT '{}' CHECK (
        cardinality(grants) <= 64 AND array_position(grants, NULL) IS NULL
    ),
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ CHECK (expires_at > created_at),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);

COMMENT ON TABLE api_tokens IS
    'Personal access tokens stored by baukit-auth::PostgresApiTokenStore; token_hash is SHA-256 of the presented secret';

CREATE INDEX api_tokens_owner_created_idx ON api_tokens (owner_id, created_at DESC, id DESC);
