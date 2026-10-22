-- Add independent active-job and pending-only deduplication keys.
--
-- Copy this file after a product's existing baukit-jobs migrations. Products
-- own migration execution; baukit-jobs deliberately does not migrate on startup.
-- Validating the CHECK constraints scans job_outbox under ACCESS EXCLUSIVE;
-- building the indexes also scans it while this transaction holds that lock.
-- On large outboxes, add CHECK constraints as NOT VALID, then VALIDATE them,
-- and build the indexes with CREATE UNIQUE INDEX CONCURRENTLY outside a transaction.

ALTER TABLE job_outbox
    ADD COLUMN dedupe_key TEXT,
    ADD COLUMN pending_dedupe_key TEXT,
    ADD CONSTRAINT job_outbox_dedupe_key_check CHECK (
        dedupe_key IS NULL OR length(trim(dedupe_key)) BETWEEN 1 AND 500
    ),
    ADD CONSTRAINT job_outbox_pending_dedupe_key_check CHECK (
        pending_dedupe_key IS NULL OR length(trim(pending_dedupe_key)) BETWEEN 1 AND 500
    );

CREATE UNIQUE INDEX job_outbox_active_dedupe_idx
    ON job_outbox (job_type, dedupe_key)
    WHERE status IN ('pending', 'running') AND dedupe_key IS NOT NULL;

CREATE UNIQUE INDEX job_outbox_pending_dedupe_idx
    ON job_outbox (job_type, pending_dedupe_key)
    WHERE status = 'pending' AND pending_dedupe_key IS NOT NULL;
