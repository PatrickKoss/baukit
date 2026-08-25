-- Upgrade the baukit-jobs v0.5.1 reference schema with stable failure reasons.
--
-- Copy this file after a product's existing baukit-jobs migration. Products
-- own migration execution; baukit-jobs deliberately does not migrate on startup.

ALTER TABLE job_outbox
    ADD COLUMN failure_reason TEXT;

UPDATE job_outbox
SET failure_reason = CASE
    WHEN attempts >= max_attempts THEN 'attempts_exhausted'
    ELSE 'permanent'
END
WHERE status = 'failed';

ALTER TABLE job_outbox
    ADD CONSTRAINT job_outbox_failure_reason_value_check CHECK (
        failure_reason IS NULL OR failure_reason IN ('permanent', 'attempts_exhausted')
    ),
    ADD CONSTRAINT job_outbox_failure_reason_status_check CHECK (
        (status = 'failed') = (failure_reason IS NOT NULL)
    );
