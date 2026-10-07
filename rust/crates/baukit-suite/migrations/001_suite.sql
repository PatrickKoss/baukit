CREATE TABLE suite_links (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL,
    peer_app text NOT NULL,
    role text NOT NULL CHECK (role IN ('initiator','authorizer')),
    remote_link_id uuid NOT NULL,
    remote_subject text NOT NULL,
    remote_display_name text,
    suite_subject text,
    status text NOT NULL DEFAULT 'active' CHECK (status IN ('active','needs_attention','revoked')),
    secret_ciphertext bytea NOT NULL,
    secret_nonce bytea NOT NULL,
    secret_key_version integer NOT NULL,
    sends text[] NOT NULL,
    receives text[] NOT NULL,
    share_xp boolean NOT NULL DEFAULT true,
    reward_mode text NOT NULL DEFAULT 'native' CHECK (reward_mode IN ('native','source_xp','off')),
    delivery_health text NOT NULL DEFAULT 'healthy'
        CHECK (delivery_health IN ('healthy','degraded','needs_attention','disabled')),
    consecutive_failures integer NOT NULL DEFAULT 0 CHECK (consecutive_failures BETWEEN 0 AND 20),
    last_delivery_at timestamptz,
    last_failure_at timestamptz,
    last_failure_code text,
    last_received_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz
);
CREATE UNIQUE INDEX suite_links_active_peer_idx ON suite_links (user_id, peer_app) WHERE status <> 'revoked';

CREATE TABLE suite_link_requests (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL,
    peer_app text NOT NULL,
    state_hash bytea NOT NULL UNIQUE,
    verifier_ciphertext bytea NOT NULL,
    verifier_nonce bytea NOT NULL,
    verifier_key_version integer NOT NULL,
    client_state_nonce text NOT NULL,
    return_url text NOT NULL,
    link_id uuid,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE suite_link_codes (
    code_hash bytea PRIMARY KEY,
    user_id uuid NOT NULL,
    peer_app text NOT NULL,
    code_challenge text NOT NULL,
    suite_subject text,
    auto_approved boolean NOT NULL,
    link_id uuid,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE suite_inbound_events (
    link_id uuid NOT NULL REFERENCES suite_links(id) ON DELETE CASCADE,
    event_id text NOT NULL,
    user_id uuid NOT NULL,
    event_type text NOT NULL,
    occurred_at timestamptz NOT NULL,
    payload_hash bytea NOT NULL,
    replay boolean NOT NULL,
    outcome text NOT NULL CHECK (outcome IN ('granted','no_rule','capped')),
    ledger_entry_id text,
    received_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (link_id, event_id)
);

CREATE INDEX job_outbox_suite_delivery_idx ON job_outbox ((payload->>'link_id'), created_at DESC)
    WHERE job_type = 'suite.events.deliver';
