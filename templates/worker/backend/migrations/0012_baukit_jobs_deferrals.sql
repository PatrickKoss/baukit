-- Count releases that gave an attempt back.
--
-- A deferred job outcome and a release on worker shutdown return the job to
-- pending without consuming an attempt and increment this counter. Workers cap
-- free deferrals with WorkerConfig.max_deferrals. Copy this file after a
-- product's existing baukit-jobs migrations.
--
-- A constant default does not rewrite the table. The check starts NOT VALID;
-- 0007 validates it without blocking writes.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

-- The counter stops at WorkerConfig.max_deferrals, and the store reads it as i32.
-- squawk-ignore prefer-bigint-over-int
ALTER TABLE job_outbox ADD COLUMN IF NOT EXISTS deferrals INTEGER NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'job_outbox'::regclass
          AND conname = 'job_outbox_deferrals_check'
    ) THEN
        ALTER TABLE job_outbox
            ADD CONSTRAINT job_outbox_deferrals_check CHECK (deferrals >= 0) NOT VALID;
    END IF;
END
$$;
