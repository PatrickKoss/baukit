ALTER TABLE data_subject_operations
    ADD COLUMN export_expires_at TIMESTAMPTZ,
    ADD CONSTRAINT data_subject_operations_export_expiry
        CHECK ((export_expires_at IS NOT NULL) = (kind = 'export' AND status = 'completed')),
    ADD CONSTRAINT data_subject_operations_export_owner UNIQUE (id, kind, status);

CREATE TABLE data_subject_export_parts (
    operation_id UUID NOT NULL,
    part_number BIGINT NOT NULL CHECK (part_number BETWEEN 1 AND 4294967295),
    operation_kind TEXT NOT NULL DEFAULT 'export' CHECK (operation_kind = 'export'),
    operation_status TEXT NOT NULL DEFAULT 'completed' CHECK (operation_status = 'completed'),
    json_bytes BYTEA NOT NULL,
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    bytes BIGINT NOT NULL CHECK (bytes > 0 AND bytes = octet_length(json_bytes)),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (operation_id, part_number),
    FOREIGN KEY (operation_id, operation_kind, operation_status)
        REFERENCES data_subject_operations (id, kind, status) ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (sha256 = encode(sha256(json_bytes), 'hex')),
    CHECK (convert_from(json_bytes, 'UTF8')::jsonb IS NOT NULL)
);
CREATE INDEX data_subject_export_parts_expires_at
    ON data_subject_export_parts (expires_at, operation_id, part_number);

CREATE OR REPLACE FUNCTION data_subject_job_failed() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.job_type IN ('data-subject.erasure', 'data-subject.export', 'data-subject.finalize') AND NEW.status = 'failed' THEN
        UPDATE data_subject_operations
        SET status = 'failed', completed_at = now(),
            error_code = 'data_subject_operation_failed', receipt = NULL
        WHERE id = (NEW.payload->>'operationId')::uuid AND status = 'pending';
        NEW.payload = jsonb_build_object('operationId', NEW.payload->>'operationId');
    END IF;
    RETURN NEW;
END;
$$;
