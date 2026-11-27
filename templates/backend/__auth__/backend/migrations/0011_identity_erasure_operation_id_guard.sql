-- A non-UUID operationId no longer aborts the job's failure update.
SET lock_timeout = '5s';
SET statement_timeout = '1min';
CREATE OR REPLACE FUNCTION erasure_job_failed() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.job_type = 'identity.account.delete' AND NEW.status = 'failed'
        AND NEW.payload->>'operationId' ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN
        UPDATE erasure_operations SET state = 'failed',
            response = jsonb_build_object('status', 'failed', 'operationId', id)
        WHERE id = (NEW.payload->>'operationId')::uuid AND state <> 'completed';
    END IF;
    RETURN NEW;
END;
$$;
