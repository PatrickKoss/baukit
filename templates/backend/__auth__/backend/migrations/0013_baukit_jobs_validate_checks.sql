-- Copy this file after 0006_baukit_jobs_concurrency_key.sql.
--
-- Validates the checks that 0005 and 0006 added NOT VALID. It runs in its own
-- transaction, so the scan holds only a SHARE UPDATE EXCLUSIVE lock and writes
-- continue.

SET lock_timeout = '5s';
SET statement_timeout = '15min';

ALTER TABLE job_outbox VALIDATE CONSTRAINT job_outbox_deferrals_check;
ALTER TABLE job_outbox VALIDATE CONSTRAINT job_outbox_concurrency_key_check;
