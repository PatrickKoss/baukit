-- Independent of peer-link tables. Copy into the product's forward migrations.
CREATE TABLE platform_inbound_receipts (
    event_id text PRIMARY KEY,
    body_hash bytea NOT NULL CHECK (octet_length(body_hash) = 32),
    received_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE platform_entitlement_cache (
    subject text NOT NULL,
    product_id text NOT NULL,
    tier text NOT NULL,
    features jsonb NOT NULL CHECK (jsonb_typeof(features) = 'array'),
    revision bigint NOT NULL CHECK (revision >= 0),
    valid_until bigint NOT NULL,
    grace_until bigint,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (subject, product_id)
);
