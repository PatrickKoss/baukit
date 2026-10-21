CREATE TABLE data_subject_operations (
    id UUID PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('erasure', 'export')),
    client_id TEXT NOT NULL,
    key_hash BYTEA NOT NULL CHECK (octet_length(key_hash) = 32),
    request_hash BYTEA NOT NULL CHECK (octet_length(request_hash) = 32),
    subject_reference TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'completed', 'notFound', 'restricted', 'failed')),
    categories JSONB NOT NULL DEFAULT '[]',
    restrictions JSONB NOT NULL DEFAULT '[]',
    parts JSONB NOT NULL DEFAULT '[]',
    receipt JSONB,
    error_code TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    identity_deletion_started_at TIMESTAMPTZ,
    purge_after TIMESTAMPTZ NOT NULL,
    UNIQUE (client_id, kind, key_hash),
    CHECK ((status = 'pending') = (completed_at IS NULL)),
    CHECK ((status IN ('completed', 'notFound', 'restricted')) = (receipt IS NOT NULL)),
    CHECK ((status = 'failed') = (error_code IS NOT NULL))
);
CREATE INDEX data_subject_operations_purge_after ON data_subject_operations (purge_after, id);

CREATE TABLE data_subject_identity_operations (
    operation_id UUID NOT NULL REFERENCES data_subject_operations (id) ON DELETE CASCADE,
    identity_operation_id UUID NOT NULL REFERENCES erasure_operations (id),
    PRIMARY KEY (operation_id, identity_operation_id)
);

-- Includes timeout, panic and expired final lease failures recorded by the runner.
CREATE FUNCTION data_subject_job_failed() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.job_type IN ('data-subject.erasure', 'data-subject.finalize') AND NEW.status = 'failed' THEN
        UPDATE data_subject_operations
        SET status = 'failed', completed_at = now(),
            error_code = 'data_subject_operation_failed', receipt = NULL
        WHERE id = (NEW.payload->>'operationId')::uuid AND status = 'pending';
        NEW.payload = jsonb_build_object('operationId', NEW.payload->>'operationId');
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER data_subject_job_failed BEFORE UPDATE OF status ON job_outbox
    FOR EACH ROW EXECUTE FUNCTION data_subject_job_failed();
