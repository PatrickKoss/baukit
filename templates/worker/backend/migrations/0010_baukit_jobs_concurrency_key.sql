-- Limit how many jobs sharing a key run at once.
--
-- NewJob::concurrency_key fills the column. A runner with
-- WorkerConfig.max_per_key skips pending rows whose key already holds that many
-- live leases. The partial index keeps that per-candidate count to the running
-- rows of one key. Copy this file after a product's existing baukit-jobs
-- migrations.

ALTER TABLE job_outbox
    ADD COLUMN concurrency_key TEXT CHECK (char_length(concurrency_key) BETWEEN 1 AND 500);

CREATE INDEX job_outbox_running_key_idx
    ON job_outbox (concurrency_key, locked_until)
    WHERE status = 'running' AND concurrency_key IS NOT NULL;
