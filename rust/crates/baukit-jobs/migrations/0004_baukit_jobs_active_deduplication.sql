-- Copy this file after the existing baukit-jobs product migrations.

ALTER TABLE job_outbox
    ADD COLUMN dedupe_key TEXT CHECK (
        dedupe_key IS NULL OR length(trim(dedupe_key)) BETWEEN 1 AND 500
    );

CREATE UNIQUE INDEX job_outbox_active_dedupe_idx
    ON job_outbox (job_type, dedupe_key)
    WHERE status IN ('pending', 'running') AND dedupe_key IS NOT NULL;
