-- Lead the pending claim index with job_type.
--
-- Claims filter by the handler's job types. With job_type first, a worker reads
-- only pending rows of the types it handles, however many rows of other types
-- wait in the same outbox. Copy this file after a product's existing
-- baukit-jobs migrations. Products own migration execution; baukit-jobs
-- deliberately does not migrate on startup.

DROP INDEX job_outbox_claim_idx;

CREATE INDEX job_outbox_claim_idx
    ON job_outbox (job_type, run_after, created_at, id)
    WHERE status = 'pending';
