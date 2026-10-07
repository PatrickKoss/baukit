ALTER TABLE suite_links ADD COLUMN last_replay_at timestamptz;

ALTER TABLE suite_link_requests
    ADD COLUMN initiator_link_id uuid,
    ADD COLUMN secret_ciphertext bytea,
    ADD COLUMN secret_nonce bytea,
    ADD COLUMN secret_key_version integer,
    ADD CONSTRAINT suite_request_exchange_complete CHECK (
        (initiator_link_id IS NULL AND secret_ciphertext IS NULL AND secret_nonce IS NULL AND secret_key_version IS NULL)
        OR (initiator_link_id IS NOT NULL AND secret_ciphertext IS NOT NULL AND secret_nonce IS NOT NULL AND secret_key_version IS NOT NULL)
    );

CREATE TABLE suite_failed_delivery_jobs (
    job_id uuid PRIMARY KEY REFERENCES job_outbox(id) ON DELETE CASCADE,
    link_id uuid NOT NULL REFERENCES suite_links(id) ON DELETE CASCADE
);
CREATE INDEX suite_failed_delivery_jobs_link_idx ON suite_failed_delivery_jobs(link_id);

CREATE FUNCTION record_suite_delivery_failure() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    target_link uuid;
    recorded_job uuid;
BEGIN
    BEGIN
        target_link := (NEW.payload->>'link_id')::uuid;
    EXCEPTION WHEN invalid_text_representation THEN
        RETURN NEW;
    END;
    SELECT id INTO target_link FROM suite_links WHERE id=target_link AND status<>'revoked';
    IF target_link IS NULL THEN RETURN NEW; END IF;
    INSERT INTO suite_failed_delivery_jobs(job_id,link_id) VALUES(NEW.id,target_link)
        ON CONFLICT DO NOTHING RETURNING job_id INTO recorded_job;
    IF recorded_job IS NOT NULL THEN
        UPDATE suite_links SET
            consecutive_failures = LEAST(20,consecutive_failures+1),
            delivery_health = CASE
                WHEN delivery_health='disabled' OR consecutive_failures+1 >= 20 THEN 'disabled'
                WHEN delivery_health='needs_attention' THEN 'needs_attention'
                ELSE 'degraded' END,
            last_failure_at=NEW.updated_at,
            last_failure_code=CASE WHEN delivery_health='disabled' AND NEW.last_error='suite_link_disabled'
                THEN last_failure_code ELSE NEW.last_error END,
            updated_at=NEW.updated_at
        WHERE id=target_link AND status<>'revoked';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER suite_delivery_failed AFTER UPDATE OF status ON job_outbox
    FOR EACH ROW WHEN (NEW.job_type='suite.events.deliver' AND NEW.status='failed' AND OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION record_suite_delivery_failure();
