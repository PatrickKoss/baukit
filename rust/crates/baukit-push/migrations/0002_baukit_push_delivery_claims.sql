-- Reference schema for baukit-push's PostgresDeliveryClaimStore.
--
-- Only products that send scheduled pushes need this table. Copy this file
-- into the product's own ordered migrations; baukit-push deliberately does not
-- migrate on startup.
--
-- Join ownership in a product migration so erasing an owner erases their
-- claims:
--
--     ALTER TABLE push_delivery_claims
--         ADD CONSTRAINT push_delivery_claims_owner_fk
--         FOREIGN KEY (owner_id) REFERENCES <owner table> (id) ON DELETE CASCADE;

CREATE TABLE push_delivery_claims (
    owner_id UUID NOT NULL,
    local_date DATE NOT NULL,
    kind TEXT NOT NULL CHECK (kind ~ '^[a-z0-9][a-z0-9_.-]{0,63}$'),
    claimed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (owner_id, local_date, kind)
);

COMMENT ON TABLE push_delivery_claims IS
    'At-most-once scheduled push deliveries claimed through baukit-push::PostgresDeliveryClaimStore';

CREATE INDEX push_delivery_claims_local_date_idx ON push_delivery_claims (local_date);
