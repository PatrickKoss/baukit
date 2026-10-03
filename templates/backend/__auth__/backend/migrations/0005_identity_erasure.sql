CREATE TABLE erasure_operations (
    id UUID PRIMARY KEY,
    subject_hash BYTEA NOT NULL CHECK (octet_length(subject_hash) = 32),
    key_hash BYTEA NOT NULL UNIQUE CHECK (octet_length(key_hash) = 32),
    state TEXT NOT NULL CHECK (state IN ('pending', 'completed', 'failed')),
    response JSONB NOT NULL,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((state = 'completed') = (completed_at IS NOT NULL))
);
CREATE TABLE erasure_fences (
    subject_hash BYTEA PRIMARY KEY CHECK (octet_length(subject_hash) = 32)
);

-- A runner timeout or expired final lease must also fail the operation.
CREATE FUNCTION erasure_job_failed() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.job_type = 'identity.account.delete' AND NEW.status = 'failed' THEN
        UPDATE erasure_operations SET state = 'failed',
            response = jsonb_build_object('status', 'failed', 'operationId', id)
        WHERE id = (NEW.payload->>'operationId')::uuid AND state <> 'completed';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER erasure_job_failed AFTER UPDATE OF status ON job_outbox
    FOR EACH ROW EXECUTE FUNCTION erasure_job_failed();
