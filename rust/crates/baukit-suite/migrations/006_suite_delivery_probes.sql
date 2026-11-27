-- Copy after 005_suite_record_ids.sql.
--
-- The store counts failed delivery attempts against SUITE_CIRCUIT_FAILURES when each delivery
-- finishes, so terminal job failures are no longer accounted in SQL. This release drops that
-- accounting in one step; docs/migrations/README.md describes the cutover.
--
-- The new columns do not rewrite the table. The widened failure check and the probe check start
-- NOT VALID; 007 validates them without blocking writes.

SET lock_timeout = '5s';
SET statement_timeout = '1min';

DROP TRIGGER IF EXISTS suite_delivery_failed ON job_outbox;
-- The 0.32.0 store no longer calls these functions or reads this table.
-- squawk-ignore ban-drop-function
DROP FUNCTION IF EXISTS account_suite_delivery_failures(uuid);
-- squawk-ignore ban-drop-function
DROP FUNCTION IF EXISTS record_suite_delivery_failure();
-- squawk-ignore ban-drop-table
DROP TABLE IF EXISTS suite_failed_delivery_jobs;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'suite_links'::regclass
          AND conname = 'suite_links_consecutive_failures_nonnegative'
    ) THEN
        ALTER TABLE suite_links
            ADD CONSTRAINT suite_links_consecutive_failures_nonnegative
            CHECK (consecutive_failures >= 0) NOT VALID;
    END IF;
END
$$;

-- suite_links_consecutive_failures_nonnegative replaces the BETWEEN 0 AND 20 bound, which the
-- store no longer keeps.
-- squawk-ignore ban-drop-constraint
ALTER TABLE suite_links DROP CONSTRAINT IF EXISTS suite_links_consecutive_failures_check;

ALTER TABLE suite_links ADD COLUMN IF NOT EXISTS next_probe_at timestamptz;

-- One probe runs per backoff interval of at least an hour, and the store reads the count as i32.
-- squawk-ignore prefer-bigint-over-int
ALTER TABLE suite_links ADD COLUMN IF NOT EXISTS failed_probes integer NOT NULL DEFAULT 0;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conrelid = 'suite_links'::regclass
          AND conname = 'suite_links_failed_probes_check'
    ) THEN
        ALTER TABLE suite_links
            ADD CONSTRAINT suite_links_failed_probes_check CHECK (failed_probes >= 0) NOT VALID;
    END IF;
END
$$;

UPDATE suite_links SET next_probe_at = now() + interval '1 hour'
    WHERE delivery_health = 'disabled' AND status <> 'revoked';
