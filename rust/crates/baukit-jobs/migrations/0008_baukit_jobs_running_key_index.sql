-- no-transaction
--
-- Copy this file after 0007_baukit_jobs_validate_checks.sql.
--
-- The partial index keeps the per-key running count of
-- WorkerConfig.max_per_key to the running rows of one key. sqlx runs this file
-- outside a transaction, so the index builds concurrently while writes
-- continue.

-- sqlx sends the file as one query, and PostgreSQL runs a query with several
-- statements as one transaction block, where CONCURRENTLY fails. A SET in this
-- file would break it, so the file holds only the index. Its SHARE UPDATE
-- EXCLUSIVE lock does not queue reads or writes behind it.
-- squawk-ignore require-lock-timeout, require-statement-timeout
CREATE INDEX CONCURRENTLY IF NOT EXISTS job_outbox_running_key_idx
    ON job_outbox (concurrency_key, locked_until)
    WHERE status = 'running' AND concurrency_key IS NOT NULL;
