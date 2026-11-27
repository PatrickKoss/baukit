-- Limit how many jobs sharing a key run at once.
--
-- NewJob::concurrency_key fills the column. A runner with
-- WorkerConfig.max_per_key skips pending rows whose key already holds that many
-- live leases. Copy this file after a product's existing baukit-jobs
-- migrations.
--
-- The column is nullable with no default, so adding it does not rewrite the
-- table. The check starts NOT VALID; 0007 validates it and 0008 builds the
-- running-key index without blocking writes.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

ALTER TABLE job_outbox ADD COLUMN IF NOT EXISTS concurrency_key TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'job_outbox'::regclass
          AND conname = 'job_outbox_concurrency_key_check'
    ) THEN
        ALTER TABLE job_outbox
            ADD CONSTRAINT job_outbox_concurrency_key_check
            CHECK (char_length(concurrency_key) BETWEEN 1 AND 500) NOT VALID;
    END IF;
END
$$;
