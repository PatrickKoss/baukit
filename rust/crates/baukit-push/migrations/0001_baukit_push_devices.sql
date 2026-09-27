-- Reference schema for baukit-push's PostgresDeviceRegistry.
--
-- Copy this file into the product's own ordered migrations. Products own
-- migration execution; baukit-push deliberately does not migrate on startup.
--
-- Baukit does not own the owner table and cannot name it here. Join ownership
-- in a product migration so erasing an owner erases their devices:
--
--     ALTER TABLE push_devices
--         ADD CONSTRAINT push_devices_owner_fk
--         FOREIGN KEY (owner_id) REFERENCES <owner table> (id) ON DELETE CASCADE;
--
-- The token is the primary key, so one device token belongs to at most one
-- owner. The registry enforces the per-owner cap in code, not here.

CREATE TABLE push_devices (
    token TEXT PRIMARY KEY CHECK (
        octet_length(token) BETWEEN 1 AND 512 AND token ~ '^[!-~]+$'
    ),
    owner_id UUID NOT NULL,
    platform TEXT NOT NULL CHECK (platform IN ('ios', 'android')),
    time_zone TEXT CHECK (
        octet_length(time_zone) BETWEEN 1 AND 64 AND time_zone ~ '^[A-Za-z0-9/_+-]+$'
    ),
    created_at TIMESTAMPTZ NOT NULL,
    last_registered_at TIMESTAMPTZ NOT NULL,
    CHECK (last_registered_at >= created_at)
);

COMMENT ON TABLE push_devices IS
    'Push device tokens stored by baukit-push::PostgresDeviceRegistry; a token addresses one device and must never be logged';

CREATE INDEX push_devices_owner_registered_idx
    ON push_devices (owner_id, last_registered_at DESC, created_at DESC, token DESC);
