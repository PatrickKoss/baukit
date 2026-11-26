-- Count releases that gave an attempt back.
--
-- A deferred job outcome and a release on worker shutdown return the job to
-- pending without consuming an attempt and increment this counter. Workers cap
-- free deferrals with WorkerConfig.max_deferrals. Copy this file after a
-- product's existing baukit-jobs migrations.

ALTER TABLE job_outbox
    ADD COLUMN deferrals INTEGER NOT NULL DEFAULT 0 CHECK (deferrals >= 0);
