ALTER TABLE suite_failed_delivery_jobs
    DROP CONSTRAINT suite_failed_delivery_jobs_link_id_fkey,
    ADD COLUMN accounted boolean NOT NULL DEFAULT true;
ALTER TABLE suite_failed_delivery_jobs ALTER COLUMN accounted SET DEFAULT false;

CREATE OR REPLACE FUNCTION record_suite_delivery_failure() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    target_link uuid;
BEGIN
    BEGIN
        target_link := (NEW.payload->>'link_id')::uuid;
    EXCEPTION WHEN invalid_text_representation THEN
        RETURN NEW;
    END;
    IF target_link IS NOT NULL THEN
        INSERT INTO suite_failed_delivery_jobs(job_id,link_id) VALUES(NEW.id,target_link)
            ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION account_suite_delivery_failures(target_link uuid) RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    owner_id uuid;
    failure record;
BEGIN
    SELECT user_id INTO owner_id FROM suite_links WHERE id=target_link;
    IF owner_id IS NULL THEN RETURN; END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('baukit_suite.owner:' || owner_id::text,0));
    PERFORM id FROM suite_links WHERE id=target_link FOR UPDATE;
    PERFORM j.id FROM job_outbox j JOIN suite_failed_delivery_jobs f ON f.job_id=j.id
        WHERE f.link_id=target_link AND NOT f.accounted ORDER BY j.id FOR UPDATE OF j;
    FOR failure IN
        SELECT j.id,j.updated_at,j.last_error FROM job_outbox j
        JOIN suite_failed_delivery_jobs f ON f.job_id=j.id
        WHERE f.link_id=target_link AND NOT f.accounted ORDER BY j.updated_at,j.id
    LOOP
        UPDATE suite_links SET
            consecutive_failures = LEAST(20,consecutive_failures+1),
            delivery_health = CASE
                WHEN delivery_health='disabled' OR consecutive_failures+1 >= 20 THEN 'disabled'
                WHEN delivery_health='needs_attention' THEN 'needs_attention'
                ELSE 'degraded' END,
            last_failure_at=failure.updated_at,
            last_failure_code=CASE WHEN delivery_health='disabled' AND failure.last_error='suite_link_disabled'
                THEN last_failure_code ELSE failure.last_error END,
            updated_at=GREATEST(updated_at,failure.updated_at)
        WHERE id=target_link AND status<>'revoked';
        UPDATE suite_failed_delivery_jobs SET accounted=true WHERE job_id=failure.id;
    END LOOP;
END;
$$;
