-- Reference schema for baukit-jobs.
--
-- Copy this file into the product's own ordered migrations. Products own
-- migration execution; baukit-jobs deliberately does not migrate on startup.

CREATE TABLE job_outbox (
    id UUID PRIMARY KEY,
    job_type TEXT NOT NULL CHECK (length(trim(job_type)) BETWEEN 1 AND 200),
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0 AND attempts <= max_attempts),
    run_after TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_by TEXT CHECK (locked_by IS NULL OR length(trim(locked_by)) BETWEEN 1 AND 300),
    locked_until TIMESTAMPTZ,
    idempotency_key TEXT CHECK (
        idempotency_key IS NULL OR length(trim(idempotency_key)) BETWEEN 1 AND 500
    ),
    last_error TEXT CHECK (
        last_error IS NULL OR length(trim(last_error)) BETWEEN 1 AND 10000
    ),
    cancel_requested_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (
        (status = 'running' AND locked_by IS NOT NULL AND locked_until IS NOT NULL)
        OR
        (status <> 'running' AND locked_by IS NULL AND locked_until IS NULL)
    ),
    CHECK (cancel_requested_at IS NULL OR status = 'running'),
    CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX job_outbox_idempotency_idx
    ON job_outbox (job_type, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX job_outbox_claim_idx
    ON job_outbox (run_after, created_at, id)
    WHERE status = 'pending';

CREATE INDEX job_outbox_expired_lease_idx
    ON job_outbox (locked_until, id)
    WHERE status = 'running';

COMMENT ON TABLE job_outbox IS
    'Durable product job outbox managed by baukit-jobs; payload data must not enter metric labels';

